use crate::{
    audio,
    mixer_cache::MixerCache,
    replay::{Profile, Replay, Snapshot},
};

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use windows_capture::{monitor::Monitor, window::Window};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub label: String,
}

// Conserva el JSON anterior (Profile) y añade la identidad de la ventana para
// recuperarla cuando Windows le asigne otro HWND.
#[derive(serde::Deserialize, serde::Serialize)]
struct StoredCapture {
    #[serde(flatten)]
    profile: Profile,
    #[serde(default)]
    source_label: Option<String>,
}

fn resolve_source<'a>(
    profile: &Profile,
    label: Option<&str>,
    sources: &'a [Source],
) -> Option<&'a Source> {
    let by_id = sources.iter().find(|source| source.id == profile.id);
    if let Some(label) = label {
        if let Some(source) = by_id.filter(|source| source.label == label) {
            return Some(source);
        }
        let mut matches = sources.iter().filter(|source| source.label == label);
        if let Some(source) = matches.next()
            && matches.next().is_none()
        {
            return Some(source);
        }
        if !profile.monitor {
            return None;
        }
    }
    by_id.or_else(|| profile.monitor.then(|| sources.first()).flatten())
}

fn profile_should_start(paused: bool) -> bool {
    !paused
}

fn validate_shortcuts(
    save: crate::hotkey::Binding,
    talk: Option<crate::hotkey::Binding>,
    registered: bool,
) -> Result<(), &'static str> {
    if !save.valid() {
        return Err(
            "Para guardar, usa un botón del mouse, una tecla F, del teclado numérico o una combinación con Ctrl o Alt.",
        );
    }
    if talk.is_some_and(|talk| !talk.valid_talk() || talk == save || !registered) {
        return Err("Elige un atajo de voz válido y distinto al de guardar.");
    }
    Ok(())
}
#[derive(Clone)]
pub struct State {
    pub revision: u64,
    pub monitors: Vec<Source>,
    pub windows: Vec<Source>,
    pub profile: Profile,
    pub settings: audio::Settings,
    pub devices: audio::Catalog,
    pub tracks: Vec<audio::Source>,
    pub levels: std::collections::HashMap<String, f64>,
    pub mic_preview: bool,
    pub mic_preview_ready: bool,
    pub mic_preview_level: f64,
    pub notices: std::collections::HashMap<String, String>,
    pub seconds: u64,
    pub active: bool,
    pub software_encoding: bool,
    pub buffer_bytes: usize,
    pub paused: bool,
    pub notice: String,
    pub directory: PathBuf,
    pub hotkey: crate::hotkey::Binding,
    pub startup: bool,
    pub update: Option<crate::updates::Update>,
    pub progress: Option<u64>,
    pub exit_ready: bool,
}
pub enum Command {
    Profile(Profile),
    StartCurrent,
    Audio(audio::Settings),
    Save,
    Toggle,
    Sources,
    Hotkey(crate::hotkey::Binding),
    Shortcuts {
        save: crate::hotkey::Binding,
        talk: Option<crate::hotkey::Binding>,
    },
    Editing(bool),
    MicPreview(bool),
    MicPreviewSample(u64, crate::mic_preview::Event),
    Directory,
    ChooseDirectory(isize),
    Startup(bool),
    CheckUpdates,
    InstallUpdate,
    Exit,
    Visible(bool),
    Saved(Result<PathBuf, String>),
    Released,
    Devices(audio::Catalog),
    Tracks(Vec<audio::Source>),
    Error(String),
    Update(Result<Option<crate::updates::Update>, String>),
    Progress(u64),
    Download(Result<crate::updates::Download, String>),
    Picked(Result<Option<PathBuf>, String>),
}

fn release_command(pending: &mut Option<Profile>) -> Option<Command> {
    pending.take().map(|_| Command::StartCurrent)
}

fn resolve_start(command: Command, current: impl FnOnce() -> Profile) -> Command {
    match command {
        Command::StartCurrent => Command::Profile(current()),
        other => other,
    }
}
#[derive(Clone)]
pub struct Controller {
    tx: mpsc::Sender<Command>,
    state: Arc<Mutex<State>>,
}
impl Controller {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            revision: 0,
            monitors: vec![],
            windows: vec![],
            profile: Profile {
                id: String::new(),
                monitor: true,
                height: 1080,
                fps: 60,
                bitrate: 20,
                seconds: 30,
                cursor: false,
            },
            settings: MixerCache::load(&MixerCache::path())
                .unwrap_or_default()
                .settings,
            devices: audio::Catalog::default(),
            tracks: vec![],
            levels: Default::default(),
            mic_preview: false,
            mic_preview_ready: false,
            mic_preview_level: 0.,
            notices: Default::default(),
            seconds: 0,
            active: false,
            software_encoding: false,
            buffer_bytes: 0,
            paused: false,
            notice: "Preparando captura…".into(),
            directory: if std::env::var_os("CUT_NATIVE_VALIDATION").is_some() {
                std::env::temp_dir()
                    .join("CutNativeValidation")
                    .join("clips")
            } else {
                crate::export_folder::load()
            },
            hotkey: crate::hotkey::Binding::load(),
            startup: startup_value(),
            update: None,
            progress: None,
            exit_ready: false,
        }));
        let controller = Self { tx, state };
        let worker = controller.clone();
        thread::spawn(move || worker.run(rx));
        controller
    }
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
    pub fn snapshot(&self) -> State {
        self.state.lock().unwrap().clone()
    }
    fn read<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&self.state.lock().unwrap())
    }
    fn change(&self, f: impl FnOnce(&mut State)) {
        let mut s = self.state.lock().unwrap();
        f(&mut s);
        s.revision = s.revision.wrapping_add(1);
    }
    fn notice(&self, message: impl Into<String>) {
        self.change(|s| s.notice = message.into());
    }
    fn run(self, rx: mpsc::Receiver<Command>) {
        let _com = crate::com::Mta::enter();
        let settings = Arc::new(Mutex::new(Arc::new(self.read(|s| s.settings.clone()))));
        let binding = Arc::new(std::sync::atomic::AtomicU32::new(
            self.read(|s| s.hotkey.packed()),
        ));
        let editing = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let key_tx = self.tx.clone();
        let registered = std::env::var_os("CUT_NATIVE_VALIDATION").is_none()
            && crate::hotkey::listen(binding.clone(), editing.clone(), move || {
                let _ = key_tx.send(Command::Save);
            });
        crate::hotkey::configure_talk(self.read(|s| s.settings.push_to_talk));
        let mut replay: Option<Replay> = None;
        let mut mic_preview: Option<crate::mic_preview::Preview> = None;
        let mut preview_generation = 0_u64;
        let mut retiring = false;
        let mut pending: Option<Profile> = None;
        let mut saving = false;
        let mut queued: Option<(Snapshot, PathBuf)> = None;
        let mut exiting = false;
        let mut visible = true;
        let mut persist_at: Option<Instant> = None;
        let mut monitor = None;
        let mut next_update = Instant::now();
        let mut checking = false;
        let mut choosing = false;
        let mut next_retry = Instant::now();
        self.refresh_sources();
        let stored = std::fs::read(MixerCache::path().with_file_name("captura.json"))
            .ok()
            .and_then(|v| serde_json::from_slice::<StoredCapture>(&v).ok())
            .filter(|p| p.profile.dimensions().is_ok());
        let mut source_label = stored
            .as_ref()
            .and_then(|capture| capture.source_label.clone());
        if let Some(capture) = stored {
            let mut profile = capture.profile;
            let source = self.read(|s| {
                let sources = if profile.monitor {
                    &s.monitors
                } else {
                    &s.windows
                };
                resolve_source(&profile, source_label.as_deref(), sources).cloned()
            });
            if let Some(source) = source {
                profile.id = source.id;
                self.send(Command::Profile(profile));
            } else {
                self.change(|s| s.profile = profile);
                self.notice("Esperando la fuente de captura…");
            }
        } else if let Some(source) = self.read(|s| s.monitors.first().cloned()) {
            let mut profile = self.read(|s| s.profile.clone());
            profile.id = source.id;
            self.send(Command::Profile(profile));
        }
        loop {
            if let Ok(command) = rx.recv_timeout(Duration::from_millis(150)) {
                let command = resolve_start(command, || self.read(|s| s.profile.clone()));
                match command {
                    Command::Profile(profile) if !exiting => {
                        if profile.dimensions().is_err() {
                            self.notice("Perfil de captura inválido.");
                            continue;
                        }
                        if !profile.monitor {
                            let supported =
                                crate::compat::Capabilities::current().and_then(|cap| {
                                    cap.check_window_capture(profile.cursor)
                                        .map_err(std::io::Error::other)
                                });
                            if let Err(error) = supported {
                                self.notice(error.to_string());
                                continue;
                            }
                        }
                        let paused = self.read(|s| s.paused);
                        let unchanged = self.read(|s| s.profile == profile);
                        let selected_label = self.read(|s| {
                            let sources = if profile.monitor {
                                &s.monitors
                            } else {
                                &s.windows
                            };
                            sources
                                .iter()
                                .find(|source| source.id == profile.id)
                                .map(|source| source.label.clone())
                        });
                        if selected_label.is_some() || !unchanged {
                            source_label = selected_label;
                        }
                        if unchanged && !paused && replay.as_ref().is_some_and(|r| !r.stalled()) {
                            continue;
                        }
                        self.change(|s| {
                            s.profile = profile.clone();
                            if !paused {
                                s.notice = "Preparando replay…".into();
                            }
                        });
                        if let Ok(bytes) = serde_json::to_vec_pretty(&StoredCapture {
                            profile: profile.clone(),
                            source_label: source_label.clone(),
                        }) {
                            let path = MixerCache::path().with_file_name("captura.json");
                            let _ = std::fs::create_dir_all(path.parent().unwrap());
                            if let Err(e) = std::fs::write(path, bytes) {
                                self.notice(e.to_string());
                            }
                        }
                        if !profile_should_start(paused) {
                            pending = None;
                            continue;
                        }
                        if retiring {
                            pending = Some(profile);
                        } else if let Some(previous) = replay.take() {
                            previous.pause();
                            retiring = true;
                            pending = Some(profile);
                            let tx = self.tx.clone();
                            thread::spawn(move || {
                                drop(previous);
                                let _ = tx.send(Command::Released);
                            });
                        } else {
                            match Replay::start(profile, settings.clone()) {
                                Ok(value) => {
                                    self.change(|s| {
                                        s.profile = value.profile.clone();
                                        s.software_encoding = value.software_encoding;
                                    });
                                    let software = value.software_encoding;
                                    replay = Some(value);
                                    self.notice(if software {
                                        "H.264 por CPU: respaldo de Windows, máximo 1080p y 30 FPS."
                                    } else {
                                        ""
                                    });
                                }
                                Err(e) => self.notice(e.to_string()),
                            }
                        }
                    }
                    Command::Released => {
                        retiring = false;
                        if !exiting && let Some(command) = release_command(&mut pending) {
                            self.send(command);
                        }
                    }
                    Command::Toggle if !exiting => {
                        let (paused, active) = self.read(|s| (s.paused, s.active));
                        if !paused && (active || retiring) {
                            pending = None;
                            if let Some(replay) = &replay {
                                replay.pause();
                            }
                            self.change(|s| s.paused = true);
                        } else {
                            self.change(|s| s.paused = false);
                            self.send(Command::StartCurrent);
                        }
                    }
                    Command::Audio(value) => {
                        if value.push_to_talk.is_some_and(|b| {
                            !b.valid_talk() || b == self.read(|s| s.hotkey) || !registered
                        }) {
                            self.notice("Elige un atajo de voz válido y distinto al de guardar.");
                            continue;
                        }
                        crate::hotkey::configure_talk(value.push_to_talk);
                        *settings.lock().unwrap() = Arc::new(value.clone());
                        self.change(|s| s.settings = value);
                        persist_at = Some(Instant::now() + Duration::from_millis(350));
                    }
                    Command::Hotkey(value) => {
                        if self.read(|s| s.settings.push_to_talk) == Some(value) {
                            self.notice("Esta tecla está asignada a pulsar para hablar.");
                            continue;
                        }
                        match value.save() {
                            Ok(()) => {
                                binding.store(value.packed(), std::sync::atomic::Ordering::Relaxed);
                                self.change(|s| {
                                    s.hotkey = value;
                                    s.notice = format!("Guardar clip → {}", value.label());
                                });
                            }
                            Err(e) => self.notice(e.to_string()),
                        }
                    }
                    Command::Shortcuts { save, talk } => {
                        if let Err(error) = validate_shortcuts(save, talk, registered) {
                            self.notice(error);
                            continue;
                        }
                        let previous = self.read(|s| s.settings.clone());
                        let mut next = previous.clone();
                        next.push_to_talk = talk;
                        let path = MixerCache::path();
                        if let Err(error) = (MixerCache {
                            settings: next.clone(),
                        })
                        .save(&path)
                        {
                            self.notice(format!("No se pudieron guardar los atajos: {error}"));
                            continue;
                        }
                        if let Err(error) = save.save() {
                            let rollback = (MixerCache { settings: previous }).save(&path);
                            self.notice(match rollback {
                                Ok(()) => format!("No se pudieron guardar los atajos: {error}"),
                                Err(rollback) => format!("No se pudieron guardar los atajos: {error}. No se pudo restaurar la configuración: {rollback}"),
                            });
                            continue;
                        }
                        *settings.lock().unwrap() = Arc::new(next.clone());
                        binding.store(save.packed(), std::sync::atomic::Ordering::Relaxed);
                        crate::hotkey::configure_talk(talk);
                        persist_at = None;
                        self.change(|s| {
                            s.hotkey = save;
                            s.settings = next;
                            s.notice = format!(
                                "Atajos guardados: guardar {} · hablar {}",
                                save.label(),
                                talk.map_or_else(|| "desactivado".into(), |key| key.label())
                            );
                        });
                    }
                    Command::MicPreview(enabled) => {
                        preview_generation = preview_generation.wrapping_add(1);
                        mic_preview = None;
                        self.change(|s| {
                            s.mic_preview = enabled && visible && !exiting;
                            s.mic_preview_ready = false;
                            s.mic_preview_level = 0.;
                        });
                        if enabled && visible && !exiting {
                            let tx = self.tx.clone();
                            let generation = preview_generation;
                            match crate::mic_preview::Preview::start(
                                Arc::clone(&settings),
                                move |sample| {
                                    let _ = tx.send(Command::MicPreviewSample(generation, sample));
                                },
                            ) {
                                Ok(preview) => mic_preview = Some(preview),
                                Err(error) => {
                                    self.change(|s| s.mic_preview = false);
                                    self.notice(format!("No se pudo probar el micrófono: {error}"));
                                }
                            }
                        }
                    }
                    Command::MicPreviewSample(generation, sample)
                        if generation == preview_generation && mic_preview.is_some() =>
                    {
                        match sample {
                            crate::mic_preview::Event::Ready => {
                                self.change(|s| s.mic_preview_ready = true);
                            }
                            crate::mic_preview::Event::Level(level) => {
                                self.change(|s| s.mic_preview_level = level.clamp(0., 1.));
                            }
                            crate::mic_preview::Event::Error(error) => {
                                mic_preview = None;
                                preview_generation = preview_generation.wrapping_add(1);
                                self.change(|s| {
                                    s.mic_preview = false;
                                    s.mic_preview_ready = false;
                                    s.mic_preview_level = 0.;
                                });
                                self.notice(format!(
                                    "No se pudo continuar la prueba de micrófono: {error}"
                                ));
                            }
                        }
                    }
                    Command::Editing(value) => {
                        if value {
                            self.send(Command::MicPreview(false));
                        }
                        crate::hotkey::close_talk();
                        crate::hotkey::set_editing(value);
                        editing.store(value, std::sync::atomic::Ordering::Relaxed);
                    }
                    Command::Save if !exiting => {
                        if queued.is_some() {
                            self.notice("Ya hay otro clip en espera.");
                            continue;
                        }
                        match replay
                            .as_ref()
                            .ok_or_else(|| "Activa la captura para llenar el replay.".into())
                            .and_then(Replay::snapshot)
                        {
                            Ok(snapshot) => {
                                let directory = self.read(|s| s.directory.clone());
                                if saving {
                                    queued = Some((snapshot, directory));
                                    self.notice("Segundo clip en espera.");
                                } else {
                                    saving = true;
                                    self.export(snapshot, directory);
                                }
                            }
                            Err(error) => self.notice(error.to_string()),
                        }
                    }
                    Command::Saved(result) => {
                        saving = false;
                        match result {
                            Ok(path) => {
                                if crate::native_ui::preferences::Preferences::load().sound {
                                    play_saved();
                                }
                                self.notice(format!("Clip guardado en {}", path.display()));
                            }
                            Err(error) => {
                                self.notice(format!("No se pudo guardar el clip. {error}"))
                            }
                        }
                        if let Some((snapshot, path)) = queued.take() {
                            saving = true;
                            self.export(snapshot, path);
                        }
                    }
                    Command::Sources => self.refresh_sources(),
                    Command::Devices(value) => self.change(|s| s.devices = value),
                    Command::Tracks(value) => self.change(|s| s.tracks = value),
                    Command::Error(error) => self.notice(error),
                    Command::Directory => {
                        if let Err(e) =
                            crate::export_folder::open(&self.read(|s| s.directory.clone()))
                        {
                            self.notice(e.to_string());
                        }
                    }
                    Command::ChooseDirectory(owner) if !choosing => {
                        choosing = true;
                        let tx = self.tx.clone();
                        let path = self.read(|s| s.directory.clone());
                        thread::spawn(move || {
                            let result = crate::export_folder::choose(owner, &path)
                                .map_err(|e| e.to_string());
                            let _ = tx.send(Command::Picked(result));
                        });
                    }
                    Command::Picked(result) => {
                        choosing = false;
                        match result {
                            Ok(Some(path)) => match crate::export_folder::remember(&path) {
                                Ok(()) => self.change(|s| s.directory = path),
                                Err(e) => self.notice(e.to_string()),
                            },
                            Err(e) => self.notice(e),
                            _ => {}
                        }
                    }
                    Command::Startup(enabled) => match set_startup(enabled) {
                        Ok(()) => self.change(|s| s.startup = enabled),
                        Err(e) => self.notice(e.to_string()),
                    },
                    Command::Visible(value) => {
                        if visible && !value {
                            self.send(Command::MicPreview(false));
                            editing.store(false, std::sync::atomic::Ordering::Relaxed);
                            crate::hotkey::set_editing(false);
                            crate::hotkey::take_captured();
                            crate::hotkey::close_talk();
                        }
                        visible = value;
                    }
                    Command::CheckUpdates => {
                        next_update = Instant::now();
                        self.notice("Buscando actualizaciones…");
                    }
                    Command::Update(result) => {
                        checking = false;
                        next_update = Instant::now() + Duration::from_secs(21600);
                        match result {
                            Ok(value) => self.change(|s| {
                                s.update = value;
                                s.notice = if s.update.is_some() {
                                    "Hay una actualización disponible."
                                } else {
                                    "Tienes la última versión."
                                }
                                .into();
                            }),
                            Err(e) => self.notice(e),
                        }
                    }
                    Command::InstallUpdate if self.read(|s| s.progress.is_none()) => {
                        if let Some(release) = self.read(|s| s.update.clone()) {
                            self.change(|s| s.progress = Some(0));
                            let tx = self.tx.clone();
                            thread::spawn(move || {
                                let result = crate::updates::download(&release, |p| {
                                    let _ = tx.send(Command::Progress(p));
                                })
                                .map_err(|e| e.to_string());
                                let _ = tx.send(Command::Download(result));
                            });
                        }
                    }
                    Command::Progress(value) => self.change(|s| s.progress = Some(value)),
                    Command::Download(result) => match result {
                        Ok(value) => match crate::updates::install(value) {
                            Ok(()) => self.send(Command::Exit),
                            Err(e) => {
                                self.change(|s| s.progress = None);
                                self.notice(e.to_string());
                            }
                        },
                        Err(e) => {
                            self.change(|s| s.progress = None);
                            self.notice(e);
                        }
                    },
                    Command::Exit => {
                        mic_preview = None;
                        self.change(|s| {
                            s.mic_preview = false;
                            s.mic_preview_ready = false;
                            s.mic_preview_level = 0.;
                        });
                        exiting = true;
                        pending = None;
                        if let Some(previous) = replay.take() {
                            previous.pause();
                            retiring = true;
                            let tx = self.tx.clone();
                            thread::spawn(move || {
                                drop(previous);
                                let _ = tx.send(Command::Released);
                            });
                        }
                    }
                    _ => {}
                }
            }
            if persist_at.is_some_and(|at| Instant::now() >= at) || exiting {
                persist_at = None;
                if let Err(e) = (MixerCache {
                    settings: self.read(|s| s.settings.clone()),
                })
                .save(&MixerCache::path())
                {
                    self.notice(format!("No se pudo guardar la configuración: {e}"));
                }
            }
            if exiting && !retiring && !saving {
                self.change(|s| s.exit_ready = true);
                break;
            }
            if std::env::var_os("CUT_NATIVE_VALIDATION").is_none()
                && !exiting
                && !checking
                && Instant::now() >= next_update
            {
                checking = true;
                let tx = self.tx.clone();
                thread::spawn(move || {
                    let result = crate::updates::check().map_err(|e| e.to_string());
                    let _ = tx.send(Command::Update(result));
                });
            }
            if visible && monitor.is_none() && !exiting {
                let (tx, refresh) = mpsc::sync_channel(1);
                monitor = Some(tx);
                let out = self.tx.clone();
                thread::spawn(move || {
                    let result = audio::monitor_mixer(
                        refresh,
                        |v| {
                            let _ = out.send(Command::Devices(v));
                        },
                        |v| {
                            let _ = out.send(Command::Tracks(v));
                        },
                    );
                    if let Err(e) = result {
                        let _ = out.send(Command::Error(e.to_string()));
                    }
                });
            } else if !visible {
                monitor = None;
            }
            let paused = self.read(|s| s.paused);
            let active = !paused && replay.as_ref().is_some_and(|r| !r.stalled());
            if self.read(|s| s.active) != active {
                self.change(|s| s.active = active);
            }
            if !exiting && !paused && !active && !retiring && Instant::now() >= next_retry {
                next_retry = Instant::now() + Duration::from_secs(3);
                self.refresh_sources();
                let recovered = self.read(|s| {
                    let sources = if s.profile.monitor {
                        &s.monitors
                    } else {
                        &s.windows
                    };
                    resolve_source(&s.profile, source_label.as_deref(), sources).map(|source| {
                        let mut profile = s.profile.clone();
                        profile.id.clone_from(&source.id);
                        profile
                    })
                });
                if let Some(profile) = recovered {
                    self.send(Command::Profile(profile));
                }
            }
            if visible {
                let buffer_bytes = replay.as_ref().map_or(0, Replay::buffer_bytes);
                let (seconds, error) = replay.as_ref().map_or((0, None), Replay::status);
                let levels = replay
                    .as_ref()
                    .filter(|_| active)
                    .map(Replay::levels)
                    .unwrap_or_default();
                let notices = replay
                    .as_ref()
                    .map(Replay::audio_notices)
                    .unwrap_or_default();
                self.change(|s| {
                    s.seconds = seconds;
                    s.buffer_bytes = buffer_bytes;
                    s.active = active;
                    s.levels = levels;
                    s.notices = notices;
                    if let Some(error) = error {
                        s.notice = error;
                    }
                });
            }
        }
    }
    fn export(&self, snapshot: Snapshot, directory: PathBuf) {
        self.notice("Guardando clip…");
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                std::panic::catch_unwind(|| snapshot.save(&directory).map_err(|e| e.to_string()))
                    .unwrap_or_else(|_| Err("El exportador se interrumpió.".into()));
            let _ = tx.send(Command::Saved(result));
        });
    }
    fn refresh_sources(&self) {
        let (monitors, windows) = capture_sources();
        self.change(|s| {
            s.monitors = monitors;
            s.windows = windows;
        });
    }
}
fn capture_sources() -> (Vec<Source>, Vec<Source>) {
    let primary = Monitor::primary()
        .ok()
        .map(|monitor| monitor.as_raw_hmonitor());
    let mut monitors: Vec<_> = Monitor::enumerate()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let is_primary = Some(monitor.as_raw_hmonitor()) == primary;
            let name = monitor
                .name()
                .ok()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("Pantalla {}", index + 1));
            let label = format!("{} · {}", index + 1, name);
            (
                is_primary,
                Source {
                    id: index.to_string(),
                    label,
                },
            )
        })
        .collect();
    monitors.sort_by_key(|(is_primary, _)| !is_primary);
    let monitors: Vec<_> = monitors.into_iter().map(|(_, monitor)| monitor).collect();
    let windows: Vec<_> = Window::enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|window| {
            let title = window.title().ok()?;
            if title.trim().is_empty() || title == "Cut" {
                return None;
            }
            Some(Source {
                id: (window.as_raw_hwnd() as isize).to_string(),
                label: title,
            })
        })
        .collect();
    (monitors, windows)
}

fn startup_value() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .and_then(|key| key.get_value::<String, _>("Cut"))
        .is_ok()
}

fn set_startup(enabled: bool) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
    if enabled {
        key.set_value(
            "Cut",
            &format!("\"{}\"", std::env::current_exe()?.display()),
        )
    } else {
        match key.delete_value("Cut") {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

fn play_saved() {
    use windows::{
        Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT},
        core::PCWSTR,
    };
    static SOUND: &[u8] = include_bytes!("clip-saved.wav");
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(SOUND.as_ptr().cast()),
            None,
            SND_ASYNC | SND_MEMORY | SND_NODEFAULT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(monitor: bool, id: &str) -> Profile {
        Profile {
            id: id.into(),
            monitor,
            height: 1080,
            fps: 60,
            bitrate: 20,
            seconds: 30,
            cursor: false,
        }
    }

    #[test]
    fn edits_preserve_capture_pause() {
        assert!(!profile_should_start(true));
        assert!(profile_should_start(false));
    }

    #[test]
    fn released_capture_does_not_overwrite_a_newer_queued_profile() {
        let a = profile(true, "A");
        let mut b = profile(true, "B");
        b.fps = 120;
        let mut current = a.clone();
        let mut pending = Some(a);
        let (tx, rx) = mpsc::channel();
        tx.send(Command::Released).unwrap();
        tx.send(Command::Profile(b.clone())).unwrap();
        for _ in 0..3 {
            match resolve_start(rx.recv().unwrap(), || current.clone()) {
                Command::Released => tx.send(release_command(&mut pending).unwrap()).unwrap(),
                Command::Profile(profile) => current = profile,
                _ => unreachable!(),
            }
        }
        assert_eq!(current, b);
        assert!(pending.is_none());
    }

    #[test]
    fn window_recovery_resolves_a_new_handle_by_title() {
        let sources = vec![Source {
            id: "202".into(),
            label: "Juego".into(),
        }];
        assert_eq!(
            resolve_source(&profile(false, "101"), Some("Juego"), &sources).map(|s| s.id.as_str()),
            Some("202")
        );
        assert!(resolve_source(&profile(false, "101"), Some("Otro juego"), &sources).is_none());
    }

    #[test]
    fn recovered_window_does_not_follow_reused_handle() {
        let sources = vec![
            Source {
                id: "101".into(),
                label: "Correo".into(),
            },
            Source {
                id: "202".into(),
                label: "Juego".into(),
            },
        ];
        assert_eq!(
            resolve_source(&profile(false, "101"), Some("Juego"), &sources).map(|s| s.id.as_str()),
            Some("202")
        );
    }

    #[test]
    fn recovery_does_not_guess_between_duplicate_window_titles() {
        let sources = vec![
            Source {
                id: "202".into(),
                label: "Juego".into(),
            },
            Source {
                id: "303".into(),
                label: "Juego".into(),
            },
        ];
        assert!(resolve_source(&profile(false, "101"), Some("Juego"), &sources).is_none());
        assert_eq!(
            resolve_source(&profile(false, "202"), Some("Juego"), &sources).map(|s| s.id.as_str()),
            Some("202")
        );
    }

    #[test]
    fn loads_legacy_capture_profiles_and_remembers_identity() {
        let old = r#"{"id":"101","monitor":false,"height":1080,"fps":60,"bitrate":20,"seconds":30,"cursor":false}"#;
        let mut stored: StoredCapture = serde_json::from_str(old).unwrap();
        assert_eq!(stored.profile.id, "101");
        assert!(stored.source_label.is_none());
        stored.source_label = Some("Juego".into());
        let restored: StoredCapture =
            serde_json::from_slice(&serde_json::to_vec(&stored).unwrap()).unwrap();
        assert_eq!(restored.source_label.as_deref(), Some("Juego"));
        assert_eq!(restored.profile, stored.profile);
        // Older readers can still consume the flattened profile.
        let legacy: Profile =
            serde_json::from_slice(&serde_json::to_vec(&stored).unwrap()).unwrap();
        assert_eq!(legacy, stored.profile);
    }

    #[test]
    fn legacy_sources_keep_working_and_missing_monitors_use_available_display() {
        let sources = vec![Source {
            id: "0".into(),
            label: "Pantalla".into(),
        }];
        assert_eq!(
            resolve_source(&profile(true, "1"), None, &sources).map(|s| s.id.as_str()),
            Some("0")
        );
        assert_eq!(
            resolve_source(&profile(false, "0"), None, &sources).map(|s| s.id.as_str()),
            Some("0")
        );
        assert!(resolve_source(&profile(false, "1"), None, &sources).is_none());
    }

    #[test]
    fn shortcut_swap_rejects_an_unmodified_letter_for_save() {
        let f9 = serde_json::from_str(r#"{"key":120,"modifiers":0}"#).unwrap();
        let a = serde_json::from_str(r#"{"key":65,"modifiers":0}"#).unwrap();
        assert!(validate_shortcuts(a, Some(f9), true).is_err());
        assert!(validate_shortcuts(f9, Some(a), true).is_ok());
        assert!(validate_shortcuts(f9, Some(f9), true).is_err());
        assert!(validate_shortcuts(f9, Some(a), false).is_err());
    }
}
