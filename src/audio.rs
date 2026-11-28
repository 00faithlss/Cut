use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use wasapi::{
    AudioCaptureClient, AudioClient, AudioMeterInformation, AudioSessionControl, DeviceEnumerator,
    Direction, Handle, SampleType, SessionState, StreamMode, WaveFormat,
};
use win_native_media::audio::EncodedAudio;
use windows::Win32::{
    Foundation::CloseHandle,
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    },
};

use crate::replay::Result;
use crate::{microphone::Microphone, mix::Mix};

#[path = "process_loopback.rs"]
mod process_loopback;

const RATE: u32 = 48_000;
const SYSTEM_AUDIO_ID: &str = "system";
pub const OUTPUT_AUDIO_PREFIX: &str = "output:";
const SYSTEM_AUDIO_NOTICE: &str = "No se pudo iniciar el audio por aplicación. Audio del sistema graba todas las aplicaciones de la salida predeterminada en una pista, separada del micrófono.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AudioMode {
    Applications,
    System,
}

fn audio_mode() -> AudioMode {
    static PROBE: OnceLock<()> = OnceLock::new();
    static APPLICATIONS: AtomicBool = AtomicBool::new(false);
    PROBE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = thread::Builder::new()
            .name("cut-audio-probe".into())
            .spawn(move || {
                let Ok(_apartment) = crate::com::Mta::enter() else {
                    return;
                };
                for delay in [0, 5, 15] {
                    thread::sleep(Duration::from_secs(delay));
                    let supported =
                        Input::new(InputKind::Application(std::process::id()), "", false).is_ok();
                    if supported {
                        APPLICATIONS.store(true, Ordering::Release);
                    }
                    if delay == 0 {
                        let _ = tx.send(());
                    }
                    if supported {
                        break;
                    }
                }
            });
        if worker.is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(2));
        }
    });
    if APPLICATIONS.load(Ordering::Acquire) {
        AudioMode::Applications
    } else {
        AudioMode::System
    }
}

pub type SharedSettings = Arc<Mutex<Arc<Settings>>>;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipAudio {
    #[default]
    Separate,
    Mixed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub device: String,
    pub mono: bool,
    pub noise_suppression: bool,
    pub clip_audio: ClipAudio,
    pub push_to_talk: Option<crate::hotkey::Binding>,
    pub tracks: HashMap<String, Gain>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device: String::new(),
            mono: true,
            noise_suppression: true,
            clip_audio: ClipAudio::default(),
            push_to_talk: None,
            tracks: HashMap::new(),
        }
    }
}

impl Settings {
    pub fn gain(&self, id: &str) -> Gain {
        self.tracks.get(id).copied().unwrap_or(Gain {
            // El loopback del sistema no puede respetar exclusiones por aplicación.
            // Exige elegir esta pista antes de volver a incluir esas aplicaciones.
            include: !id.starts_with(OUTPUT_AUDIO_PREFIX)
                && (id != SYSTEM_AUDIO_ID
                    || !self
                        .tracks
                        .iter()
                        .any(|(id, gain)| id.starts_with("app:") && !gain.include)),
            volume: 100,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Gain {
    pub include: bool,
    pub volume: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub label: String,
}

#[derive(Serialize)]
pub struct Application {
    #[serde(flatten)]
    pub source: Source,
    #[serde(skip)]
    pids: Vec<u32>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize)]
pub struct Catalog {
    pub devices: Vec<Source>,
    pub default_microphone: Option<String>,
}

fn processes() -> HashMap<u32, (u32, String)> {
    let mut list = HashMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return list;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&ch| ch == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                list.insert(
                    entry.th32ProcessID,
                    (
                        entry.th32ParentProcessID,
                        name.trim_end_matches(".exe").to_owned(),
                    ),
                );
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    list
}

fn capture_devices(enumerator: &DeviceEnumerator) -> Catalog {
    let mut found = Catalog {
        default_microphone: enumerator
            .get_default_device(Direction::Capture)
            .and_then(|device| device.get_friendlyname())
            .ok(),
        ..Catalog::default()
    };
    if let Ok(devices) = enumerator.get_device_collection(Direction::Capture) {
        for device in (&devices).into_iter().flatten() {
            if let (Ok(id), Ok(label)) = (device.get_id(), device.get_friendlyname()) {
                found.devices.push(Source { id, label });
            }
        }
    }
    found
}

fn render_sessions(enumerator: &DeviceEnumerator, mut visit: impl FnMut(AudioSessionControl)) {
    if let Ok(speakers) = enumerator.get_device_collection(Direction::Render) {
        for device in (&speakers).into_iter().flatten() {
            let sessions = device
                .get_iaudiosessionmanager()
                .and_then(|manager| manager.get_audiosessionenumerator());
            if let Ok(sessions) = sessions {
                for index in 0..sessions.get_count().unwrap_or(0) {
                    if let Ok(control) = sessions.get_session(index) {
                        visit(control);
                    }
                }
            }
        }
    }
}

fn active_applications(enumerator: &DeviceEnumerator, settings: &Settings) -> CapturePlan {
    let mut pids = HashSet::new();
    render_sessions(enumerator, |control| {
        if control.get_state().ok() == Some(SessionState::Active)
            && let Ok(pid) = control.get_process_id()
            && pid != 0
            && pid != std::process::id()
        {
            pids.insert(pid);
        }
    });
    if pids.is_empty() {
        CapturePlan {
            sources: Vec::new(),
            blocked: HashMap::new(),
        }
    } else {
        let processes = processes();
        let sources = group_processes(&processes, pids, std::process::id());
        capture_plan(&processes, sources, settings)
    }
}

fn output_sources(enumerator: &DeviceEnumerator) -> Vec<Source> {
    let Ok(outputs) = enumerator.get_device_collection(Direction::Render) else {
        return Vec::new();
    };
    let mut sources: Vec<_> = (&outputs)
        .into_iter()
        .flatten()
        .filter_map(|device| {
            let id = device.get_id().ok()?;
            let label = device.get_friendlyname().ok()?;
            Some(Source {
                id: format!("{OUTPUT_AUDIO_PREFIX}{id}"),
                label,
            })
        })
        .collect();
    sources.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
    sources
}

fn capture_sources(
    mode: AudioMode,
    applications: Vec<Application>,
    outputs: Vec<Source>,
) -> Vec<Application> {
    match mode {
        AudioMode::Applications => applications,
        AudioMode::System => std::iter::once(Source {
            id: SYSTEM_AUDIO_ID.into(),
            label: "Audio del sistema".into(),
        })
        .chain(outputs)
        .map(|source| Application {
            source,
            pids: vec![0],
        })
        .collect(),
    }
}

fn default_output_covered(
    default: Option<&str>,
    sources: &[Application],
    settings: &Settings,
) -> bool {
    sources.iter().any(|app| {
        let id = &app.source.id;
        id.strip_prefix(OUTPUT_AUDIO_PREFIX)
            .is_some_and(|device| Some(device) == default && settings.gain(id).include)
    })
}

fn application_for(
    processes: &HashMap<u32, (u32, String)>,
    pid: u32,
    own_pid: u32,
) -> Option<(Source, u32)> {
    let (_, label) = processes.get(&pid)?;
    let mut root = pid;
    let mut current = pid;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        // El WebView y los auxiliares de Cut pertenecen al árbol de la app.
        if current == own_pid {
            return None;
        }
        let Some((parent, name)) = processes.get(&current) else {
            break;
        };
        if name.eq_ignore_ascii_case(label) {
            root = current;
        }
        current = *parent;
    }
    Some((
        Source {
            id: format!("app:{}", label.to_lowercase()),
            label: label.clone(),
        },
        root,
    ))
}

fn group_processes(
    processes: &HashMap<u32, (u32, String)>,
    pids: HashSet<u32>,
    own_pid: u32,
) -> Vec<Application> {
    let mut applications: HashMap<String, Application> = HashMap::new();
    let mut pids: Vec<_> = pids.into_iter().collect();
    pids.sort_unstable();
    for pid in pids {
        let Some((source, root)) = application_for(processes, pid, own_pid) else {
            continue;
        };
        let app = applications
            .entry(source.id.clone())
            .or_insert_with(|| Application {
                source,
                pids: Vec::new(),
            });
        if !app.pids.contains(&root) {
            app.pids.push(root);
        }
    }
    // El loopback de un proceso ya incluye sus hijos, incluso si tienen otro nombre.
    let roots: HashSet<_> = applications
        .values()
        .flat_map(|app| app.pids.iter().copied())
        .collect();
    for app in applications.values_mut() {
        app.pids.retain(|pid| {
            let mut current = *pid;
            let mut visited = HashSet::new();
            while visited.insert(current) {
                let Some((parent, _)) = processes.get(&current) else {
                    break;
                };
                if *parent != *pid && roots.contains(parent) {
                    return false;
                }
                current = *parent;
            }
            true
        });
    }
    let mut apps: Vec<_> = applications.into_values().collect();
    apps.retain(|app| !app.pids.is_empty());
    apps.sort_by(|a, b| a.source.id.cmp(&b.source.id));
    apps
}

struct CapturePlan {
    sources: Vec<Application>,
    blocked: HashMap<String, HashSet<u32>>,
}

fn capture_plan(
    processes: &HashMap<u32, (u32, String)>,
    mut sources: Vec<Application>,
    settings: &Settings,
) -> CapturePlan {
    let mut blocked = HashMap::<String, HashSet<u32>>::new();
    // INCLUDE_TARGET_PROCESS_TREE cannot subtract an excluded descendant.
    // Walk excluded processes towards their ancestors once, including dormant
    // children so opening a new audio stream cannot bypass the preference.
    let mut unsafe_roots = HashSet::new();
    for (&pid, (_, name)) in processes {
        if settings
            .gain(&format!("app:{}", name.to_lowercase()))
            .include
        {
            continue;
        }
        let mut current = pid;
        let mut visited = HashSet::new();
        while visited.insert(current) {
            unsafe_roots.insert(current);
            let Some((parent, _)) = processes.get(&current) else {
                break;
            };
            current = *parent;
        }
    }
    for source in &mut sources {
        if !settings.gain(&source.source.id).include {
            continue;
        }
        source.pids.retain(|pid| {
            if unsafe_roots.contains(pid) {
                blocked
                    .entry(source.source.id.clone())
                    .or_default()
                    .insert(*pid);
                false
            } else {
                true
            }
        });
    }
    CapturePlan { sources, blocked }
}

#[derive(Default)]
struct AudibleTracks {
    last_heard: HashMap<String, Instant>,
}

impl AudibleTracks {
    fn update<'a>(
        &mut self,
        now: Instant,
        active: impl IntoIterator<Item = (&'a Source, bool)>,
    ) -> Vec<Source> {
        let mut audible = Vec::new();
        let mut active_ids = HashSet::new();
        for (source, has_signal) in active {
            active_ids.insert(source.id.as_str());
            if has_signal {
                if let Some(last) = self.last_heard.get_mut(&source.id) {
                    *last = now;
                } else {
                    self.last_heard.insert(source.id.clone(), now);
                }
            }
            audible.push(source);
        }
        self.last_heard.retain(|id, last| {
            active_ids.contains(id.as_str()) && now.duration_since(*last) < Duration::from_secs(3)
        });
        audible.retain(|source| self.last_heard.contains_key(&source.id));
        audible.sort_by(|a, b| a.id.cmp(&b.id));
        audible.dedup_by(|a, b| a.id == b.id);
        audible.into_iter().cloned().collect()
    }
}

struct RenderMeter {
    source: Source,
    control: AudioSessionControl,
    meter: AudioMeterInformation,
}

pub fn monitor_mixer(
    refresh: Receiver<()>,
    mut devices_changed: impl FnMut(Catalog),
    mut audible_changed: impl FnMut(Vec<Source>),
) -> Result<()> {
    let _apartment = crate::com::Mta::enter()?;
    let mut mode = audio_mode();
    let enumerator = DeviceEnumerator::new()?;
    let mut devices = None;
    let mut sessions = Vec::<RenderMeter>::new();
    let mut outputs = Vec::new();
    let mut tracks = AudibleTracks::default();
    let mut previous = None;
    let mut refresh_at = Instant::now();
    loop {
        let now = Instant::now();
        if now >= refresh_at {
            mode = audio_mode();
            refresh_at = now + Duration::from_secs(2);
            let catalog = capture_devices(&enumerator);
            if devices.as_ref() != Some(&catalog) {
                devices_changed(catalog.clone());
                devices = Some(catalog);
            }
            sessions.clear();
            if mode == AudioMode::Applications {
                let processes = processes();
                render_sessions(&enumerator, |control| {
                    if let Ok(pid) = control.get_process_id()
                        && pid != 0
                        && let Some((source, _)) =
                            application_for(&processes, pid, std::process::id())
                        && let Ok(meter) = control.get_audiometerinformation()
                    {
                        sessions.push(RenderMeter {
                            source,
                            control,
                            meter,
                        });
                    }
                });
            } else {
                outputs = capture_sources(mode, Vec::new(), output_sources(&enumerator))
                    .into_iter()
                    .map(|app| app.source)
                    .collect();
            }
        }
        let active = sessions
            .iter()
            .filter(|session| session.control.get_state().ok() == Some(SessionState::Active))
            .map(|session| {
                let audible = session
                    .meter
                    .get_peak_value()
                    .is_ok_and(|peak| peak >= 0.001);
                (&session.source, audible)
            });
        let audible = if mode == AudioMode::System {
            // Keep the fallback visible before sound starts, so it can be
            // disabled before it records any application's output.
            outputs.clone()
        } else {
            tracks.update(now, active)
        };
        if previous.as_ref() != Some(&audible) {
            audible_changed(audible.clone());
            previous = Some(audible);
        }
        match refresh.recv_timeout(Duration::from_millis(100)) {
            Ok(()) => refresh_at = Instant::now(),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct History {
    pub name: String,
    pub channels: u16,
    pub packets: VecDeque<Arc<EncodedAudio>>,
    pub level: f64,
}

pub type Tracks = Arc<Mutex<HashMap<String, History>>>;
pub type Notices = Arc<Mutex<HashMap<String, String>>>;

pub struct Clock {
    frequency: i64,
    start: u64,
}

impl Clock {
    pub fn new() -> Result<Self> {
        let mut frequency = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency)? };
        let mut clock = Self {
            frequency,
            start: 0,
        };
        clock.start = clock.read()?;
        Ok(clock)
    }

    fn read(&self) -> Result<u64> {
        let mut count = 0;
        unsafe { QueryPerformanceCounter(&mut count)? };
        Ok((i128::from(count) * 10_000_000 / i128::from(self.frequency)) as u64)
    }

    pub fn now(&self) -> Result<Duration> {
        Ok(self.relative(self.read()?))
    }

    pub fn counter_time(&self, count: i64) -> Result<Duration> {
        if count == 0 {
            return self.now();
        }
        Ok(self.relative((i128::from(count) * 10_000_000 / i128::from(self.frequency)) as u64))
    }

    pub fn hns_time(&self, ticks: i64) -> Duration {
        self.relative(ticks.max(0) as u64)
    }

    fn relative(&self, ticks: u64) -> Duration {
        Duration::from_nanos(ticks.saturating_sub(self.start).saturating_mul(100))
    }
}

struct Input {
    _client: AudioClient,
    capture: AudioCaptureClient,
    _event: Option<Handle>,
    channels: u16,
    input_channels: u16,
    microphone: Option<Box<Microphone>>,
    staging: Vec<u8>,
    pcm: Vec<u8>,
    next_packet_at: Option<Duration>,
}

fn packet_start(
    next: &mut Option<Duration>,
    observed: Duration,
    duration: Duration,
    valid_timestamp: bool,
    discontinuity: bool,
) -> Duration {
    let tolerance = if valid_timestamp {
        Duration::from_micros(250)
    } else {
        Duration::from_millis(200)
    };
    let start = next
        .filter(|end| !discontinuity && end.abs_diff(observed) <= tolerance)
        .unwrap_or(observed);
    *next = Some(start + duration);
    start
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Microphone,
    System,
    Application(u32),
}

impl Input {
    fn new(kind: InputKind, device: &str, mono: bool) -> Result<Self> {
        let client = match kind {
            InputKind::Application(pid) => {
                process_loopback::Activation::start(pid)?.wait_for_probe()?
            }
            InputKind::Microphone | InputKind::System => {
                let enumerator = DeviceEnumerator::new()?;
                let source = if device.is_empty() {
                    let direction = if kind == InputKind::System {
                        Direction::Render
                    } else {
                        Direction::Capture
                    };
                    enumerator.get_default_device(direction)?
                } else {
                    enumerator.get_device(device)?
                };
                source.get_iaudioclient()?
            }
        };
        Self::start(client, kind, mono)
    }

    fn start(mut client: AudioClient, kind: InputKind, mono: bool) -> Result<Self> {
        let is_microphone = kind == InputKind::Microphone;
        let input_channels = if is_microphone
            && client
                .get_mixformat()
                .is_ok_and(|format| format.get_nchannels() == 1)
        {
            1
        } else {
            2
        };
        let channels = if mono { 1 } else { input_channels };
        let (bits, sample) = if is_microphone {
            (32, SampleType::Float)
        } else {
            (16, SampleType::Int)
        };
        let format = WaveFormat::new(
            bits,
            bits,
            sample,
            RATE as usize,
            input_channels as usize,
            None,
        );
        let mode = if kind == InputKind::System {
            // Polling works with endpoint loopback even on Windows versions
            // predating event-driven loopback support. The worker already polls.
            StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: 1_000_000,
            }
        } else {
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: 0,
            }
        };
        client.initialize_client(&format, Direction::Capture, mode, None)?;
        let event = if kind == InputKind::System {
            None
        } else {
            Some(client.set_get_eventhandle()?)
        };
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;
        Ok(Self {
            _client: client,
            capture,
            _event: event,
            channels,
            input_channels,
            microphone: is_microphone.then(|| Box::new(Microphone::new(channels))),
            staging: Vec::new(),
            pcm: Vec::new(),
            next_packet_at: None,
        })
    }

    fn read(
        &mut self,
        clock: &Clock,
        gain: u16,
        suppress: bool,
        output: impl FnMut(Duration, &[u8], u16),
    ) -> Result<()> {
        self.read_processed(clock, gain, suppress, true, output)
    }

    fn read_processed(
        &mut self,
        clock: &Clock,
        gain: u16,
        suppress: bool,
        gate: bool,
        mut output: impl FnMut(Duration, &[u8], u16),
    ) -> Result<()> {
        while let Some(size) = self.capture.get_next_packet_size()? {
            if size == 0 {
                break;
            }
            let input_width =
                usize::from(self.input_channels) * if self.microphone.is_some() { 4 } else { 2 };
            self.staging.resize(size as usize * input_width, 0);
            let (frames, info) = self.capture.read_from_device(&mut self.staging)?;
            if frames == 0 {
                break;
            }
            if info.flags.data_discontinuity {
                self.pcm.clear();
                if let Some(mic) = &mut self.microphone {
                    **mic = Microphone::new(self.channels);
                }
            }
            let packet_duration = Duration::from_secs_f64(f64::from(frames) / f64::from(RATE));
            let valid_timestamp = !info.flags.timestamp_error && info.timestamp != 0;
            let observed = if valid_timestamp {
                clock.relative(info.timestamp)
            } else {
                clock.now()?.saturating_sub(packet_duration)
            };
            let packet_time = packet_start(
                &mut self.next_packet_at,
                observed,
                packet_duration,
                valid_timestamp,
                info.flags.data_discontinuity,
            );
            self.pcm.clear();
            // Keep closed-gate voice out of the noise suppressor's delayed samples too.
            if gate && self.microphone.is_some() && !crate::hotkey::talk_open() {
                self.staging[..frames as usize * input_width].fill(0);
            }
            let input = &self.staging[..frames as usize * input_width];
            let start = if let Some(mic) = &mut self.microphone {
                let Some(start) = mic.process(
                    input,
                    self.input_channels,
                    packet_time,
                    gain,
                    suppress,
                    &mut self.pcm,
                ) else {
                    continue;
                };
                start
            } else if gain == 100 {
                output(packet_time, input, self.channels);
                continue;
            } else {
                for bytes in input.as_chunks::<2>().0 {
                    let sample = i32::from(i16::from_le_bytes(*bytes));
                    let sample = (sample * i32::from(gain.min(200)) / 100)
                        .clamp(i32::from(i16::MIN), i32::from(i16::MAX))
                        as i16;
                    self.pcm.extend_from_slice(&sample.to_le_bytes());
                }
                packet_time
            };
            if gate && self.microphone.is_some() && !crate::hotkey::talk_open() {
                self.pcm.fill(0);
            }
            output(start, &self.pcm, self.channels);
        }
        Ok(())
    }
}

/// Flujo independiente para escucharse, sin cambiar la activación de la grabación.
pub(crate) struct PreviewInput(Input);

impl PreviewInput {
    pub(crate) fn new(settings: &Settings) -> Result<Self> {
        Ok(Self(Input::new(
            InputKind::Microphone,
            &settings.device,
            settings.mono,
        )?))
    }

    pub(crate) fn read(
        &mut self,
        clock: &Clock,
        settings: &Settings,
        output: impl FnMut(Duration, &[u8], u16),
    ) -> Result<()> {
        self.0.read_processed(
            clock,
            settings.gain("mic").volume,
            settings.noise_suppression,
            false,
            output,
        )
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        let _ = self._client.stop_stream();
    }
}

struct Group {
    inputs: HashMap<u32, Input>,
    mix: Mix,
    output: History,
}

impl Group {
    fn new(label: &str, pid: u32, input: Input) -> Result<Self> {
        let mix = Mix::new(input.channels)?;
        let output = mix.new_history(label);
        Ok(Self {
            inputs: HashMap::from([(pid, input)]),
            mix,
            output,
        })
    }
}

pub fn run(
    stop: &AtomicBool,
    clock: &Clock,
    retain: Duration,
    settings: &SharedSettings,
    history: &Tracks,
    notices: &Notices,
) -> Result<()> {
    let _apartment = crate::com::Mta::enter()?;
    let mut mode = audio_mode();
    let mut groups = HashMap::<String, Group>::new();
    let mut pending = HashMap::<u32, process_loopback::Activation>::new();
    let mut sources = Vec::new();
    let mut discover_at = Instant::now();
    let mut previous = Arc::new(Settings::default());
    let mut mix = Mix::combined()?;
    let mut mixed = mix.new_history("Mezcla");
    let mut mixed_sources: HashSet<String> = HashSet::new();
    let mut blocked_sources = HashMap::<String, HashSet<u32>>::new();
    let mut output_device = None;
    while !stop.load(Ordering::Relaxed) {
        let preferences = settings.lock().unwrap().clone();
        if preferences.device != previous.device || preferences.mono != previous.mono {
            groups.remove("mic");
            history.lock().unwrap().remove("mic");
            notices.lock().unwrap().remove("mic");
        }
        let exclusions_changed = preferences
            .tracks
            .keys()
            .chain(previous.tracks.keys())
            .any(|id| preferences.gain(id).include != previous.gain(id).include);
        let discovery = Instant::now() >= discover_at || exclusions_changed;
        let mut newly_blocked = HashSet::new();
        if discovery {
            discover_at = Instant::now() + Duration::from_secs(2);
            if let Ok(enumerator) = DeviceEnumerator::new() {
                let detected = audio_mode();
                if detected != mode {
                    // Cierra el loopback anterior antes de abrir el de las aplicaciones.
                    let cutover = clock.now()?;
                    for (id, mut group) in groups.extract_if(|id, _| id != "mic") {
                        group.mix.finish(cutover, &mut group.output)?;
                        if let Some(track) = history.lock().unwrap().get_mut(&id) {
                            track.packets.append(&mut group.output.packets);
                        }
                    }
                    notices.lock().unwrap().retain(|id, _| id == "mic");
                    mode = detected;
                }
                sources = if mode == AudioMode::Applications {
                    let plan = active_applications(&enumerator, &preferences);
                    for (id, roots) in &plan.blocked {
                        if roots.iter().any(|pid| {
                            !blocked_sources.get(id).is_some_and(|old| old.contains(pid))
                        }) {
                            newly_blocked.insert(id.clone());
                        }
                    }
                    let mut current = notices.lock().unwrap();
                    for id in blocked_sources.keys() {
                        if !plan.blocked.contains_key(id) {
                            current.remove(id);
                        }
                    }
                    for id in plan.blocked.keys() {
                        current.insert(id.clone(), "No se graba este árbol de procesos porque contiene una aplicación excluida. Windows captura el árbol completo. Activa la aplicación excluida para volver a grabar este árbol.".into());
                    }
                    blocked_sources = plan.blocked;
                    plan.sources
                } else {
                    let current_output = enumerator
                        .get_default_device(Direction::Render)
                        .and_then(|device| device.get_id())
                        .ok();
                    if current_output != output_device {
                        groups.remove(SYSTEM_AUDIO_ID);
                        output_device = current_output;
                    }
                    capture_sources(mode, Vec::new(), output_sources(&enumerator))
                };
            }
            if !sources
                .iter()
                .any(|app: &Application| app.source.id == "mic")
            {
                sources.push(Application {
                    source: Source {
                        id: "mic".into(),
                        label: "Micrófono".into(),
                    },
                    pids: vec![0],
                });
            }
        }
        let default_covered = mode == AudioMode::System
            && default_output_covered(output_device.as_deref(), &sources, &preferences);
        let reconcile = discovery || !Arc::ptr_eq(&preferences, &previous);
        if reconcile {
            pending.retain(|pid, _| {
                sources
                    .iter()
                    .any(|app| app.pids.contains(pid) && preferences.gain(&app.source.id).include)
            });
            groups.retain(|id, _| {
                sources
                    .iter()
                    .any(|app| &app.source.id == id && !app.pids.is_empty())
                    && preferences.gain(id).include
                    && !(id == SYSTEM_AUDIO_ID && default_covered)
                    && !newly_blocked.contains(id)
            });
            let cutoff = clock.now()?.saturating_sub(retain + Duration::from_secs(3));
            let mut all = history.lock().unwrap();
            all.retain(|id, track| {
                if (id == SYSTEM_AUDIO_ID && default_covered)
                    || (id != "mix" && !sources.iter().any(|app| &app.source.id == id))
                {
                    track.level = 0.0;
                }
                while track
                    .packets
                    .front()
                    .is_some_and(|packet| packet.timestamp < cutoff)
                {
                    track.packets.pop_front();
                }
                !newly_blocked.contains(id)
                    && preferences.gain(id).include
                    && (id == "mix" || groups.contains_key(id) || !track.packets.is_empty())
            });
        }
        if reconcile
            && mixed_sources
                .iter()
                .any(|id| !preferences.gain(id).include || newly_blocked.contains(id))
        {
            mixed_sources.clear();
            mixed.packets.clear();
            mix = Mix::combined()?;
            history.lock().unwrap().remove("mix");
        }
        for app in &sources {
            let Source { id, label } = &app.source;
            let gain = preferences.gain(id);
            let is_system = id == SYSTEM_AUDIO_ID;
            if is_system && default_covered {
                if reconcile {
                    notices.lock().unwrap().insert(
                        id.clone(),
                        "La salida predeterminada está seleccionada como pista independiente."
                            .into(),
                    );
                }
                continue;
            }
            if !gain.include {
                if reconcile {
                    if is_system {
                        let message = if preferences.tracks.contains_key(SYSTEM_AUDIO_ID) {
                            SYSTEM_AUDIO_NOTICE.to_owned()
                        } else {
                            format!(
                                "{SYSTEM_AUDIO_NOTICE} Para conservar tus exclusiones previas, está desactivado. Activa Audio del sistema si quieres grabar todas las aplicaciones."
                            )
                        };
                        notices.lock().unwrap().insert(id.clone(), message);
                    } else {
                        notices.lock().unwrap().remove(id);
                    }
                }
                continue;
            }
            let is_mic = id == "mic";
            let update_inputs = discovery
                || !previous.gain(id).include
                || (is_mic
                    && (preferences.device != previous.device
                        || preferences.mono != previous.mono));
            if update_inputs && let Some(group) = groups.get_mut(id) {
                group.inputs.retain(|pid, _| app.pids.contains(pid));
            }
            let mut last_error = None;
            let mut opened = false;
            for &pid in &app.pids {
                if groups
                    .get(id)
                    .is_some_and(|group| group.inputs.contains_key(&pid))
                {
                    continue;
                }
                let input = if pid != 0 {
                    if let Some(activation) = pending.get_mut(&pid) {
                        let client = match activation.poll() {
                            process_loopback::ActivationStatus::Pending => continue,
                            process_loopback::ActivationStatus::Delayed => {
                                last_error = Some(
                                    "Windows todavía está preparando la captura de esta aplicación."
                                        .into(),
                                );
                                continue;
                            }
                            process_loopback::ActivationStatus::Complete(client) => client,
                        };
                        pending.remove(&pid);
                        client.and_then(|client| {
                            Input::start(client, InputKind::Application(pid), false)
                        })
                    } else {
                        if update_inputs {
                            match process_loopback::Activation::start(pid) {
                                Ok(activation) => {
                                    pending.insert(pid, activation);
                                }
                                Err(error) => last_error = Some(error.to_string()),
                            }
                        }
                        continue;
                    }
                } else if update_inputs {
                    let (kind, device) = if is_mic {
                        (InputKind::Microphone, preferences.device.as_str())
                    } else if is_system {
                        (
                            InputKind::System,
                            output_device.as_deref().unwrap_or_default(),
                        )
                    } else {
                        (
                            InputKind::System,
                            id.strip_prefix(OUTPUT_AUDIO_PREFIX).unwrap_or_default(),
                        )
                    };
                    Input::new(kind, device, is_mic && preferences.mono)
                } else {
                    continue;
                };
                let input = match input {
                    Ok(input) => input,
                    Err(error) => {
                        last_error = Some(error.to_string());
                        continue;
                    }
                };
                opened = true;
                if let Some(group) = groups.get_mut(id) {
                    group.inputs.insert(pid, input);
                } else {
                    groups.insert(id.clone(), Group::new(label, pid, input)?);
                }
            }
            if update_inputs || opened || last_error.is_some() {
                let mut current = notices.lock().unwrap();
                if groups.get(id).is_some_and(|group| !group.inputs.is_empty()) {
                    if is_system {
                        current.insert(id.clone(), SYSTEM_AUDIO_NOTICE.into());
                    } else if !blocked_sources.contains_key(id) {
                        current.remove(id);
                    }
                } else if let Some(error) = last_error {
                    current.insert(id.clone(), error);
                }
            }
            let Some(group) = groups.get_mut(id) else {
                continue;
            };
            let mut last_error = None;
            group.inputs.retain(|_, input| {
                match input.read(
                    clock,
                    gain.volume,
                    preferences.noise_suppression,
                    |time, data, channels| {
                        group.mix.add(time, data, channels);
                        mix.add(time, data, channels);
                        if !mixed_sources.contains(id) {
                            mixed_sources.insert(id.clone());
                        }
                    },
                ) {
                    Ok(()) => true,
                    Err(error) => {
                        last_error = Some(error.to_string());
                        false
                    }
                }
            });
            if group.inputs.is_empty()
                && let Some(error) = last_error
            {
                notices.lock().unwrap().insert(id.clone(), error);
            }
            group.mix.render(clock.now()?, &mut group.output)?;
            let mut all = history.lock().unwrap();
            let track = all
                .entry(id.clone())
                .or_insert_with(|| group.mix.new_history(label));
            track.level = group.output.level;
            track.packets.append(&mut group.output.packets);
        }
        mix.render(clock.now()?, &mut mixed)?;
        if !mixed.packets.is_empty() {
            history
                .lock()
                .unwrap()
                .entry("mix".into())
                .or_insert_with(|| mix.new_history("Mezcla"))
                .packets
                .append(&mut mixed.packets);
        }
        previous = preferences;
        thread::sleep(Duration::from_millis(12));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requiere un dispositivo de salida WASAPI en Windows"]
    fn abre_loopback_del_sistema_sin_procesarlo_como_microfono() -> Result<()> {
        let _apartment = crate::com::Mta::enter()?;
        let clock = Clock::new()?;
        let mut input = Input::new(InputKind::System, "", false)?;
        assert_eq!(input.channels, 2);
        assert!(input.microphone.is_none());
        for _ in 0..5 {
            input.read(&clock, 100, false, |_, data, channels| {
                assert_eq!(channels, 2);
                assert_eq!(data.len() % 4, 0);
            })?;
            thread::sleep(Duration::from_millis(12));
        }
        Ok(())
    }

    #[test]
    fn windows_antiguo_captura_el_sistema_una_vez_y_no_duplica_las_aplicaciones() {
        let applications = vec![Application {
            source: Source {
                id: "app:game".into(),
                label: "Game".into(),
            },
            pids: vec![42],
        }];
        let sources = capture_sources(AudioMode::System, applications, Vec::new());
        let ids: Vec<_> = sources.iter().map(|app| app.source.id.as_str()).collect();
        assert_eq!(ids, vec!["system"]);
        assert_eq!(sources[0].source.label, "Audio del sistema");
        assert_eq!(sources[0].pids, vec![0]);
        assert_eq!(
            capture_sources(AudioMode::System, Vec::new(), Vec::new()).len(),
            1
        );
    }

    #[test]
    fn windows_moderno_conserva_las_pistas_por_aplicacion_sin_pista_del_sistema() {
        let applications = vec![Application {
            source: Source {
                id: "app:game".into(),
                label: "Game".into(),
            },
            pids: vec![42],
        }];
        let sources = capture_sources(AudioMode::Applications, applications, Vec::new());
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source.id, "app:game");
        assert_eq!(sources[0].pids, vec![42]);
    }

    #[test]
    fn el_fallback_no_incluye_en_secreto_aplicaciones_excluidas_antes() {
        let mut settings = Settings::default();
        assert!(settings.gain("system").include);
        settings.tracks.insert(
            "app:private".into(),
            Gain {
                include: false,
                volume: 100,
            },
        );
        assert!(!settings.gain("system").include);
        assert!(settings.gain("mic").include);
        settings.tracks.insert(
            "system".into(),
            Gain {
                include: true,
                volume: 80,
            },
        );
        assert_eq!(
            settings.gain("system"),
            Gain {
                include: true,
                volume: 80
            }
        );
    }

    #[test]
    fn no_mezcla_dos_veces_el_audio_de_un_hijo_con_otro_nombre() {
        let tree = HashMap::from([
            (10, (1, "browser".into())),
            (11, (10, "renderer".into())),
            (12, (11, "utility".into())),
            (20, (1, "Discord".into())),
        ]);
        let apps = group_processes(&tree, HashSet::from([10, 11, 12, 20]), 99);
        let roots: Vec<_> = apps
            .iter()
            .flat_map(|app| app.pids.iter().copied())
            .collect();
        assert_eq!(roots, vec![10, 20]);
    }

    // Each fixture is the PCM returned by a Windows process-tree capture:
    // browser(10) includes its renderer(11), while player(20) is independent.
    fn render_capture_plan(sources: &[Application], settings: &Settings) -> Result<History> {
        let mut mix = Mix::combined()?;
        let mut output = mix.new_history("Prueba de captura");
        for source in sources {
            if !settings.gain(&source.source.id).include {
                continue;
            }
            for pid in &source.pids {
                let value: i16 = match pid {
                    0 => 4000,  // endpoint loopback already contains all apps
                    10 => 4000, // browser(1000) + renderer(3000)
                    11 => 3000,
                    20 => 2000,
                    30 => 500, // independent browser instance
                    _ => panic!("PID sin señal de prueba: {pid}"),
                };
                mix.add(
                    Duration::ZERO,
                    &value.to_le_bytes().repeat(1024 * 16 * 2),
                    2,
                );
            }
        }
        mix.render(Duration::from_secs(1), &mut output)?;
        Ok(output)
    }

    #[test]
    fn la_senal_del_hijo_entra_una_vez_en_la_mezcla_del_arbol() -> Result<()> {
        let tree = HashMap::from([
            (10, (1, "browser".into())),
            (11, (10, "renderer".into())),
            (20, (1, "player".into())),
        ]);
        let sources = group_processes(&tree, HashSet::from([10, 11, 20]), 99);
        let output = render_capture_plan(&sources, &Settings::default())?;
        // 1000 + 3000 + 2000; capturing renderer again would produce 9000.
        assert_eq!(output.level, 6000.0 / 32768.0);
        assert!(!output.packets.is_empty(), "Falta la salida AAC real");
        Ok(())
    }

    #[test]
    fn el_loopback_del_sistema_no_suma_tambien_el_loopback_de_aplicacion() -> Result<()> {
        let applications = vec![Application {
            source: Source {
                id: "app:browser".into(),
                label: "Browser".into(),
            },
            pids: vec![10],
        }];
        let sources = capture_sources(AudioMode::System, applications, Vec::new());
        let output = render_capture_plan(&sources, &Settings::default())?;
        assert_eq!(output.level, 4000.0 / 32768.0);
        assert!(!output.packets.is_empty());
        Ok(())
    }

    #[test]
    fn excluir_un_hijo_impide_que_su_senal_entre_por_el_padre() -> Result<()> {
        let tree = HashMap::from([
            (10, (1, "browser".into())),
            (11, (10, "renderer".into())),
            (20, (1, "player".into())),
        ]);
        let mut settings = Settings::default();
        settings.tracks.insert(
            "app:renderer".into(),
            Gain {
                include: false,
                volume: 100,
            },
        );
        let sources = group_processes(&tree, HashSet::from([10, 11, 20]), 99);
        let plan = capture_plan(&tree, sources, &settings);
        let output = render_capture_plan(&plan.sources, &settings)?;
        // Windows cannot subtract one child from an INCLUDE tree, so only
        // the independent player(2000) is safe to capture.
        assert_eq!(output.level, 2000.0 / 32768.0);
        assert!(
            !output.packets.is_empty(),
            "Se perdió la aplicación independiente"
        );
        assert!(plan.blocked.contains_key("app:browser"));
        Ok(())
    }

    #[test]
    fn exclusion_de_hijo_sin_sesion_conserva_la_instancia_independiente() -> Result<()> {
        let tree = HashMap::from([
            (10, (1, "browser".into())),
            (11, (10, "renderer".into())),
            (20, (1, "player".into())),
            (30, (1, "browser".into())),
        ]);
        let mut settings = Settings::default();
        settings.tracks.insert(
            "app:renderer".into(),
            Gain {
                include: false,
                volume: 100,
            },
        );
        // renderer(11) has no active audio session yet; its existing process
        // can start playing before the next discovery interval.
        let sources = group_processes(&tree, HashSet::from([10, 20, 30]), 99);
        let plan = capture_plan(&tree, sources, &settings);
        let output = render_capture_plan(&plan.sources, &settings)?;
        assert_eq!(output.level, 2500.0 / 32768.0);
        assert!(!output.packets.is_empty());
        Ok(())
    }

    #[test]
    fn muestra_solo_sesiones_con_audio_y_quita_las_cerradas() {
        let now = Instant::now();
        let spotify = Source {
            id: "app:spotify".into(),
            label: "Spotify".into(),
        };
        let discord = Source {
            id: "app:discord".into(),
            label: "Discord".into(),
        };
        let mut tracks = AudibleTracks::default();
        assert_eq!(
            tracks.update(now, [(&spotify, false), (&discord, true)]),
            vec![discord.clone()]
        );
        assert_eq!(
            tracks.update(
                now + Duration::from_secs(1),
                [(&spotify, true), (&discord, false)]
            ),
            vec![discord.clone(), spotify.clone()]
        );
        assert_eq!(
            tracks.update(now + Duration::from_millis(1100), [(&spotify, false)]),
            vec![spotify]
        );
        assert!(tracks.update(now + Duration::from_secs(4), []).is_empty());
    }

    #[test]
    fn agrupa_pestanas_e_instancias_y_excluye_los_auxiliares_de_cut() {
        let processes = HashMap::from([
            (10, (0, "VLC".into())),
            (11, (10, "vlc".into())),
            (12, (0, "vlc".into())),
            (21, (0, "zen".into())),
            (22, (21, "zen".into())),
            (31, (0, "Spotify".into())),
            (40, (0, "cut".into())),
            (41, (40, "msedgewebview2".into())),
        ]);
        let apps = group_processes(&processes, HashSet::from([11, 12, 22, 31, 41]), 40);
        let grouped: Vec<_> = apps
            .iter()
            .map(|app| (app.source.id.as_str(), app.pids.as_slice()))
            .collect();
        assert_eq!(
            grouped,
            vec![
                ("app:spotify", &[31][..]),
                ("app:vlc", &[10, 12][..]),
                ("app:zen", &[21][..]),
            ]
        );
    }
}
