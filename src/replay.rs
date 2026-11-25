use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::Deserialize;
use win_native_media::{
    CaptureTarget, VideoConfig,
    encoder::{EncodedSample, ParameterSets},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12},
        System::Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE,
            SYNCHRONIZATION_SYNCHRONIZE, SetWaitableTimer, TIMER_MODIFY_STATE, WaitForSingleObject,
        },
    },
    core::PCWSTR,
};

use crate::{
    audio, capture_gpu::Capture, clip_name, cursor::Cursor, gpu::Encoder, mp4, scale::Scaler,
};

#[path = "replay_store.rs"]
pub(crate) mod storage;
use storage::{Buffer, Frame};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_FRAME_DELAY: Duration = Duration::from_secs(3);

fn check_freshness(latest: Duration, now: Duration) -> Result<()> {
    if now.saturating_sub(latest) > MAX_FRAME_DELAY {
        return Err(
            "La captura se interrumpió. Espera a que el replay reciba video reciente.".into(),
        );
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub monitor: bool,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub seconds: u64,
    #[serde(default)]
    pub cursor: bool,
}

impl Profile {
    pub fn dimensions(&self) -> Result<(u32, u32)> {
        if ![720, 1080, 1440, 2160].contains(&self.height)
            || ![30, 60, 90, 120].contains(&self.fps)
            || ![10, 20, 40, 80].contains(&self.bitrate)
            || ![15, 30, 60, 120, 300].contains(&self.seconds)
        {
            return Err("Perfil de captura inválido.".into());
        }
        Ok((self.height * 16 / 9, self.height))
    }

    fn target(&self) -> Result<CaptureTarget> {
        if self.monitor {
            Ok(CaptureTarget::Monitor(self.id.parse()?))
        } else {
            Ok(CaptureTarget::Window(self.id.parse()?))
        }
    }
}

pub struct Replay {
    pub profile: Profile,
    game_name: Option<String>,
    stop: Arc<AtomicBool>,
    clock: Arc<audio::Clock>,
    data: Arc<Mutex<Buffer>>,
    audio: audio::Tracks,
    notices: audio::Notices,
    settings: audio::SharedSettings,
    worker: Option<JoinHandle<()>>,
    audio_worker: Option<JoinHandle<()>>,
}

impl Replay {
    pub fn start(profile: Profile, settings: audio::SharedSettings) -> Result<Self> {
        let (width, height) = profile.dimensions()?;
        let target = profile.target()?;
        let game_name = match target {
            CaptureTarget::Window(handle) => {
                let window = windows_capture::window::Window::from_raw_hwnd(handle as *mut _);
                Some(
                    window
                        .title()
                        .ok()
                        .filter(|title| !title.trim().is_empty())
                        .or_else(|| {
                            let process = window.process_name().ok()?;
                            Path::new(&process).file_stem()?.to_str().map(str::to_owned)
                        })
                        .unwrap_or_else(|| "Juego".into()),
                )
            }
            CaptureTarget::Monitor(_) => None,
        };
        let config = VideoConfig {
            width,
            height,
            fps: profile.fps,
            bitrate: profile.bitrate * 1_000_000,
            keyframe_interval: profile.fps,
        };
        let data = Arc::new(Mutex::new(Buffer::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let clock = Arc::new(audio::Clock::new()?);
        let history: audio::Tracks = Default::default();
        let notices: audio::Notices = Default::default();
        let retain = Duration::from_secs(profile.seconds);
        let worker = thread::Builder::new().name("cut-capture".into()).spawn({
            let data = Arc::clone(&data);
            let stop = Arc::clone(&stop);
            let clock = Arc::clone(&clock);
            let include_cursor = profile.cursor;
            move || {
                if let Err(error) =
                    record(target, config, retain, include_cursor, &stop, &data, &clock)
                {
                    data.lock().unwrap().error = Some(error.to_string());
                }
                stop.store(true, Ordering::Relaxed);
            }
        })?;
        let audio_start = thread::Builder::new().name("cut-audio".into()).spawn({
            let stop = Arc::clone(&stop);
            let clock = Arc::clone(&clock);
            let history = Arc::clone(&history);
            let notices = Arc::clone(&notices);
            let settings = Arc::clone(&settings);
            move || {
                if let Err(error) = audio::run(&stop, &clock, retain, &settings, &history, &notices)
                {
                    notices
                        .lock()
                        .unwrap()
                        .insert("mic".into(), error.to_string());
                }
            }
        });
        let audio_worker = match audio_start {
            Ok(handle) => handle,
            Err(error) => {
                stop.store(true, Ordering::Relaxed);
                let _ = worker.join();
                return Err(error.into());
            }
        };
        Ok(Self {
            profile,
            game_name,
            stop,
            clock,
            data,
            audio: history,
            notices,
            settings,
            worker: Some(worker),
            audio_worker: Some(audio_worker),
        })
    }

    pub fn audio_notices(&self) -> std::collections::HashMap<String, String> {
        self.notices.lock().unwrap().clone()
    }

    pub fn levels(&self) -> std::collections::HashMap<String, f64> {
        self.audio
            .lock()
            .unwrap()
            .iter()
            .map(|(id, track)| (id.clone(), track.level))
            .collect()
    }

    pub fn status(&self) -> (u64, Option<String>) {
        let data = self.data.lock().unwrap();
        let seconds = data.timestamps().map_or(0, |(a, b)| {
            (b.saturating_sub(a) + Duration::from_millis(1000 / u64::from(self.profile.fps)))
                .as_secs()
        });
        (seconds.min(self.profile.seconds), data.error.clone())
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let (width, height) = self.profile.dimensions()?;
        let data = self.data.lock().unwrap();
        if let Some(error) = &data.error {
            return Err(format!("La captura se interrumpió. {error}").into());
        }
        if data.frames().take(2).count() < 2 || data.parameters.sps.is_empty() {
            return Err("Todavía no hay suficientes fotogramas para guardar un clip.".into());
        }
        let (_, latest) = data.timestamps().ok_or("El replay está vacío.")?;
        check_freshness(latest, self.clock.now()?)?;
        Ok(Snapshot {
            name: clip_name::new_name(self.game_name.as_deref()),
            video: mp4::Video {
                width,
                height,
                fps: self.profile.fps,
                parameters: data.parameters.clone(),
            },
            samples: data.frames().cloned().collect(),
            audio: {
                let settings = self.settings.lock().unwrap();
                let history = self.audio.lock().unwrap();
                match settings.clip_audio {
                    audio::ClipAudio::Mixed => mp4::Audio::Mixed(history.get("mix").cloned()),
                    audio::ClipAudio::Separate => mp4::Audio::Separate(
                        history
                            .iter()
                            .filter(|(id, _)| {
                                settings.tracks.get(*id).is_none_or(|gain| gain.include)
                            })
                            .map(|(_, track)| track.clone())
                            .collect(),
                    ),
                }
            },
            audio_history: Arc::clone(&self.audio),
            audio_ready_at: std::time::Instant::now() + Duration::from_millis(250),
        })
    }

    pub fn active(&self) -> bool {
        !self.stop.load(Ordering::Relaxed) && self.worker.as_ref().is_some_and(|w| !w.is_finished())
    }

    pub fn stalled(&self) -> bool {
        if !self.active() {
            return true;
        }
        self.data
            .lock()
            .unwrap()
            .timestamps()
            .is_some_and(|(_, latest)| {
                self.clock
                    .now()
                    .is_ok_and(|now| check_freshness(latest, now).is_err())
            })
    }

    pub fn pause(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        self.pause();
        // DXGI necesita que la sesión anterior suelte la salida antes de abrir otra.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.audio_worker.take() {
            let _ = worker.join();
        }
    }
}

pub struct Snapshot {
    name: String,
    video: mp4::Video,
    samples: Vec<Frame>,
    audio: mp4::Audio,
    audio_history: audio::Tracks,
    audio_ready_at: std::time::Instant,
}

impl Snapshot {
    pub fn save(mut self, directory: &Path) -> Result<PathBuf> {
        // El hilo de exportación espera al audio pendiente del instante en que se pulsó guardar.
        thread::sleep(
            self.audio_ready_at
                .saturating_duration_since(std::time::Instant::now()),
        );
        let live = self.audio_history.lock().unwrap();
        let extend = |track: &mut audio::History| {
            if let Some(current) = live.values().find(|current| current.name == track.name) {
                let last = track.packets.back().map(|packet| packet.timestamp);
                track.packets.extend(
                    current
                        .packets
                        .iter()
                        .filter(|packet| last.is_none_or(|last| packet.timestamp > last))
                        .cloned(),
                );
            }
        };
        match &mut self.audio {
            mp4::Audio::Mixed(mix) => {
                if let Some(track) = mix {
                    extend(track);
                } else {
                    *mix = live.get("mix").cloned();
                }
            }
            mp4::Audio::Separate(tracks) => tracks.iter_mut().for_each(extend),
        }
        drop(live);
        fs::create_dir_all(directory)?;
        let (temp, file) = clip_name::create_temporary(directory, &self.name)?;
        let result = mp4::write(&temp, file, &self.video, &self.samples, &self.audio)
            .and_then(|_| clip_name::publish(&temp, directory, &self.name).map_err(Into::into));
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

fn record(
    target: CaptureTarget,
    config: VideoConfig,
    retain: Duration,
    include_cursor: bool,
    stop: &AtomicBool,
    data: &Mutex<Buffer>,
    clock: &audio::Clock,
) -> Result<()> {
    // La cola acotada aísla las escrituras del hilo de la GPU.
    thread::scope(|scope| {
        let (tx, rx) = mpsc::sync_channel::<(EncodedSample, Option<ParameterSets>)>(8);
        let writer = scope.spawn(move || -> std::io::Result<()> {
            let mut writer = storage::Writer::default();
            for (packet, parameters) in rx {
                let frame = writer.append(packet)?;
                let mut buffer = data.lock().unwrap();
                if let Some(parameters) = parameters {
                    buffer.parameters = parameters;
                }
                buffer.push(frame, retain);
            }
            Ok(())
        });
        let recording = record_frames(target, config, include_cursor, stop, tx, clock);
        writer
            .join()
            .map_err(|_| "Falló el hilo del historial temporal.")?
            .map_err(|error| format!("No se pudo escribir el historial temporal: {error}"))?;
        recording
    })
}

fn record_frames(
    target: CaptureTarget,
    config: VideoConfig,
    include_cursor: bool,
    stop: &AtomicBool,
    tx: mpsc::SyncSender<(EncodedSample, Option<ParameterSets>)>,
    clock: &audio::Clock,
) -> Result<()> {
    // El hilo posee la sesión de captura, las texturas y el codificador.
    let _apartment = crate::com::Mta::enter()?;
    let mut capture =
        Capture::open(target, include_cursor).map_err(|error| format!("Captura: {error}"))?;
    let mut encoder = Encoder::new(capture.device(), config)
        .map_err(|error| format!("Inicio del codificador GPU: {error}"))?;
    let mut nv12 = Scaler::new(
        capture.device(),
        config.width,
        config.height,
        config.fps,
        DXGI_FORMAT_NV12,
    )?;
    let mut cursor = if include_cursor && matches!(target, CaptureTarget::Monitor(_)) {
        let bgra = Scaler::new(
            capture.device(),
            config.width,
            config.height,
            config.fps,
            DXGI_FORMAT_B8G8R8A8_UNORM,
        )?;
        let target = bgra.create_target()?;
        let cursor = Cursor::new(capture.device(), &target, config.width, config.height)?;
        Some((bgra, target, cursor))
    } else {
        None
    };
    let timer = FrameTimer::new();
    let mut packets = Vec::new();
    let interval = Duration::from_nanos(1_000_000_000 / u64::from(config.fps));
    let mut next = Duration::ZERO;
    let initial = std::time::Instant::now();
    let mut has_output = false;
    let mut last_frame = None;
    while !stop.load(Ordering::Relaxed) {
        encoder
            .poll(&mut packets)
            .map_err(|error| format!("Salida del codificador GPU: {error}"))?;
        for packet in packets.drain(..) {
            let parameters = packet.is_keyframe.then(|| encoder.parameters.clone());
            tx.send((packet, parameters))
                .map_err(|_| "Se cerró el historial temporal.")?;
            has_output = true;
        }
        if !has_output && initial.elapsed() > Duration::from_secs(8) {
            return Err("La GPU no entregó fotogramas H.264.".into());
        }
        let now = clock.now()?;
        if now >= next && encoder.accepts_input() {
            let Some(input) = encoder.input_frame()? else {
                // Deja que el codificador recupere todas las superficies si se saturó.
                last_frame = None;
                timer.wait(Duration::from_millis(1));
                continue;
            };
            let source = match &mut cursor {
                Some((bgra, target, cursor)) => {
                    // Conserva el cursor como capa independiente durante la captura del monitor.
                    let updated = capture.with_frame(clock, Some(cursor), |frame| {
                        Ok(bgra.stretch(frame, target, 0)?)
                    })?;
                    if updated {
                        let rendered = cursor.render(target);
                        nv12.stretch_texture(
                            rendered,
                            config.width,
                            config.height,
                            &input.texture,
                            input.index,
                        )?;
                    }
                    updated
                }
                None => capture.with_frame(clock, None, |frame| {
                    Ok(nv12.stretch(frame, &input.texture, input.index)?)
                })?,
            };
            if source || last_frame.is_some() {
                if let Some(previous) = last_frame.as_ref().filter(|_| !source) {
                    encoder.repeat(previous, &input);
                }
                if now.saturating_sub(next) >= interval {
                    next = now;
                }
                encoder
                    .submit(&input, next)
                    .map_err(|error| format!("Entrada del codificador GPU: {error}"))?;
                // Retiene la muestra para que el pool no recicle la imagen que se repite.
                if source {
                    last_frame = Some(input);
                }
                next += interval;
            }
        }
        let until_frame = next.saturating_sub(clock.now()?);
        timer.wait(if until_frame.is_zero() {
            Duration::from_millis(1)
        } else {
            until_frame
        });
    }
    drop(capture);
    let _ = encoder.finish();
    Ok(())
}

struct FrameTimer(Option<HANDLE>);

impl FrameTimer {
    fn new() -> Self {
        Self(unsafe {
            CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_MODIFY_STATE.0 | SYNCHRONIZATION_SYNCHRONIZE.0,
            )
            .ok()
        })
    }

    fn wait(&self, delay: Duration) {
        let due = -(delay.as_nanos().div_ceil(100) as i64);
        let waited = self.0.is_some_and(|timer| unsafe {
            SetWaitableTimer(timer, &due, 0, None, None, false).is_ok()
                && WaitForSingleObject(timer, INFINITE) == WAIT_OBJECT_0
        });
        if !waited {
            thread::sleep(delay);
        }
    }
}

impl Drop for FrameTimer {
    fn drop(&mut self) {
        if let Some(timer) = self.0 {
            unsafe {
                let _ = CloseHandle(timer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_capture::window::Window;

    #[test]
    fn exporta_una_pista_mezclada_o_pistas_separadas() -> Result<()> {
        use win_native_media::audio::EncodedAudio;

        let old: audio::Settings = serde_json::from_str(r#"{"tracks":{}}"#)?;
        assert_eq!(old.clip_audio, audio::ClipAudio::Separate);

        let mut spool = storage::Writer::default();
        let frames = [spool.append(EncodedSample {
            timestamp: Duration::from_secs(1),
            is_keyframe: true,
            data: vec![0, 0, 0, 1, 0x65, 0x80],
        })?];
        let video = mp4::Video {
            width: 1280,
            height: 720,
            fps: 30,
            parameters: ParameterSets {
                sps: vec![0x67, 0x64, 0, 0x1f],
                pps: vec![0x68, 0xce, 0x3c, 0x80],
            },
        };
        let track = |name: &str| audio::History {
            name: name.into(),
            channels: 2,
            packets: [Arc::new(EncodedAudio {
                timestamp: Duration::from_secs(1),
                data: vec![0x21, 0x10, 0x04, 0x60, 0x8c, 0x1c],
            })]
            .into(),
            level: 0.0,
        };
        let cases = [
            (mp4::Audio::Mixed(Some(track("Mezcla"))), 1),
            (mp4::Audio::Mixed(None), 1),
            (
                mp4::Audio::Separate(vec![track("Mezcla"), track("Discord")]),
                2,
            ),
        ];
        for (index, (audio, expected)) in cases.into_iter().enumerate() {
            let path = std::env::temp_dir().join(format!(
                "cut-audio-export-{}-{index}.mp4",
                std::process::id()
            ));
            let result = (|| -> Result<_> {
                mp4::write(&path, fs::File::create(&path)?, &video, &frames, &audio)?;
                let file = fs::File::open(&path)?;
                let len = file.metadata()?.len();
                let reader = ::mp4::Mp4Reader::read_header(std::io::BufReader::new(file), len)?;
                let count = reader
                    .tracks()
                    .values()
                    .filter(|track| matches!(track.track_type(), Ok(::mp4::TrackType::Audio)))
                    .count();
                Ok(count)
            })();
            let _ = fs::remove_file(path);
            assert_eq!(result?, expected);
        }
        Ok(())
    }

    #[test]
    fn rechaza_historial_congelado() {
        let last = Duration::from_secs(120);
        assert!(check_freshness(last, Duration::from_secs(121)).is_ok());
        assert!(check_freshness(last, Duration::from_secs(129)).is_err());
    }

    #[test]
    #[ignore = "Requiere Windows interactivo y más de dos minutos de captura real"]
    fn historial_sigue_avanzando_despues_de_llenarse() -> Result<()> {
        let replay = Replay::start(
            Profile {
                id: "0".into(),
                monitor: true,
                height: 1080,
                fps: 120,
                bitrate: 40,
                seconds: 120,
                cursor: false,
            },
            Default::default(),
        )?;
        espera_frames(&replay, 120)?;
        let first = replay.snapshot()?;
        thread::sleep(Duration::from_secs(12));
        let second = replay.snapshot()?;
        let advance = |a: &Frame, b: &Frame| b.timestamp.saturating_sub(a.timestamp);
        assert!(advance(&first.samples[0], &second.samples[0]) >= Duration::from_secs(9));
        assert!(
            advance(
                first.samples.last().unwrap(),
                second.samples.last().unwrap()
            ) >= Duration::from_secs(11)
        );
        Ok(())
    }

    #[test]
    #[ignore = "Requiere una ventana de prueba 4:3 visible en Windows"]
    fn captura_ventana_y_guarda_mp4() -> Result<()> {
        let title = "Cut WGC 4x3 test pattern";
        let window = Window::from_name(title)?;
        prueba_clipear((window.as_raw_hwnd() as isize).to_string(), false)
    }

    #[test]
    #[ignore = "Requiere una sesión interactiva de Windows"]
    fn captura_monitor_y_guarda_mp4() -> Result<()> {
        prueba_clipear("0".into(), true)
    }

    #[test]
    #[ignore = "Requiere la ventana de prueba visible y mueve temporalmente el cursor"]
    fn capturar_cursor_en_monitor_y_ventana() -> Result<()> {
        use windows::Win32::{
            Foundation::POINT,
            UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos},
        };
        let window = Window::from_name("Cut WGC 4x3 test pattern")?;
        let rect = window.rect()?;
        let mut previous = POINT::default();
        unsafe { GetCursorPos(&mut previous)? };
        let result = (|| {
            for monitor in [true, false] {
                for cursor in [false, true] {
                    let profile = Profile {
                        id: if monitor {
                            "0".into()
                        } else {
                            (window.as_raw_hwnd() as isize).to_string()
                        },
                        monitor,
                        height: 720,
                        fps: 30,
                        bitrate: 10,
                        seconds: 15,
                        cursor,
                    };
                    let replay = Replay::start(profile, Default::default())?;
                    thread::sleep(Duration::from_millis(700));
                    unsafe {
                        SetCursorPos(rect.left + 100, rect.top + 110)?;
                        SetCursorPos(rect.left + 110, rect.top + 120)?;
                    }
                    espera_frames(&replay, 2)?;
                    let path = replay
                        .snapshot()?
                        .save(&std::env::temp_dir().join("cut-verificacion"))?;
                    println!("monitor={monitor} cursor={cursor} {}", path.display());
                }
            }
            Ok(())
        })();
        let _ = unsafe { SetCursorPos(previous.x, previous.y) };
        result
    }

    #[test]
    #[ignore = "Requiere una sesión interactiva de Windows y codificador H.264 por GPU"]
    fn captura_monitor_120fps_reanuda_y_supera_dos_segundos() -> Result<()> {
        let mut profile = Profile {
            id: "0".into(),
            monitor: true,
            height: 1080,
            fps: 60,
            bitrate: 20,
            seconds: 120,
            cursor: false,
        };
        let capture = Replay::start(profile.clone(), Default::default())?;
        espera_frames(&capture, 3)?;
        println!("Captura inicial: más de 2 s");
        capture.pause();
        drop(capture);
        println!("Sesión anterior liberada");
        profile.fps = 120;
        profile.bitrate = 80;
        let capture = Replay::start(profile, Default::default())?;
        espera_frames(&capture, 6)?;
        let output = std::env::temp_dir().join("cut-verificacion");
        let first = capture.snapshot()?.save(&output)?;
        espera_frames(&capture, 7)?;
        let second = capture.snapshot()?.save(&output)?;
        assert_ne!(first, second);
        println!("{}\n{}", first.display(), second.display());
        Ok(())
    }

    fn prueba_clipear(id: String, monitor: bool) -> Result<()> {
        let replay = Replay::start(
            Profile {
                id,
                monitor,
                height: 720,
                fps: 30,
                bitrate: 10,
                seconds: 15,
                cursor: false,
            },
            Default::default(),
        )?;
        espera_frames(&replay, 4)?;
        let path = replay
            .snapshot()?
            .save(&std::env::temp_dir().join("cut-verificacion"))?;
        println!("{}", path.display());
        Ok(())
    }

    fn espera_frames(replay: &Replay, expected: u64) -> Result<()> {
        let limit = std::time::Instant::now() + Duration::from_secs(expected + 10);
        loop {
            let (seconds, error) = replay.status();
            if let Some(error) = error {
                return Err(error.into());
            }
            if seconds >= expected {
                break;
            }
            if std::time::Instant::now() >= limit {
                return Err(format!(
                    "El replay se quedó en {seconds} s cuando se esperaban {expected} s."
                )
                .into());
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}
