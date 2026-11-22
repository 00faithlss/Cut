use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::Deserialize;
use win_native_media::{
    CaptureTarget, VideoConfig,
    encoder::{EncodedSample, ParameterSets},
};

use crate::{
    audio, capture_gpu::Capture, clip_name, cursor::Cursor, gpu::Encoder, mp4, scale::Scaler,
};

const MEMORY_LIMIT: usize = 384 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

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

#[derive(Default)]
struct Buffer {
    bytes: usize,
    samples: VecDeque<Arc<EncodedSample>>,
    parameters: ParameterSets,
    error: Option<String>,
}

impl Buffer {
    fn push(&mut self, sample: EncodedSample, retain: Duration) {
        if self.samples.is_empty() && !sample.is_keyframe {
            return;
        }
        let cutoff = sample.timestamp.saturating_sub(retain);
        self.bytes += sample.data.len();
        self.samples.push_back(Arc::new(sample));
        let begin = self
            .samples
            .iter()
            .rposition(|s| s.is_keyframe && s.timestamp <= cutoff);
        for _ in 0..begin.unwrap_or_default() {
            self.bytes -= self.samples.pop_front().unwrap().data.len();
        }
        while self.bytes > MEMORY_LIMIT {
            self.bytes -= self.samples.pop_front().unwrap().data.len();
            while self.samples.front().is_some_and(|s| !s.is_keyframe) {
                self.bytes -= self.samples.pop_front().unwrap().data.len();
            }
        }
    }
}

pub struct Replay {
    pub profile: Profile,
    game_name: Option<String>,
    stop: Arc<AtomicBool>,
    data: Arc<Mutex<Buffer>>,
    audio: audio::Tracks,
    notices: audio::Notices,
    settings: Arc<Mutex<audio::Settings>>,
    worker: Option<JoinHandle<()>>,
    audio_worker: Option<JoinHandle<()>>,
}

impl Replay {
    pub fn start(profile: Profile, settings: Arc<Mutex<audio::Settings>>) -> Result<Self> {
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
        let seconds = data
            .samples
            .front()
            .zip(data.samples.back())
            .map_or(0, |(a, b)| {
                (b.timestamp.saturating_sub(a.timestamp)
                    + Duration::from_millis(1000 / u64::from(self.profile.fps)))
                .as_secs()
            });
        (seconds.min(self.profile.seconds), data.error.clone())
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let (width, height) = self.profile.dimensions()?;
        let data = self.data.lock().unwrap();
        if data.samples.len() < 2 || data.parameters.sps.is_empty() {
            return Err("Todavía no hay suficientes fotogramas para guardar un clip.".into());
        }
        Ok(Snapshot {
            name: clip_name::new_name(self.game_name.as_deref()),
            video: mp4::Video {
                width,
                height,
                fps: self.profile.fps,
                parameters: data.parameters.clone(),
            },
            samples: data.samples.iter().cloned().collect(),
            audio: {
                let settings = self.settings.lock().unwrap();
                self.audio
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(id, _)| settings.tracks.get(*id).is_none_or(|gain| gain.include))
                    .map(|(_, history)| history.clone())
                    .collect()
            },
        })
    }

    pub fn active(&self) -> bool {
        !self.stop.load(Ordering::Relaxed) && self.worker.as_ref().is_some_and(|w| !w.is_finished())
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
    samples: Vec<Arc<EncodedSample>>,
    audio: Vec<audio::History>,
}

impl Snapshot {
    pub fn save(self, directory: &Path) -> Result<PathBuf> {
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
    // El hilo posee la sesión de captura, las texturas y el codificador.
    unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        )
        .ok()?;
    }
    let mut capture =
        Capture::open(target, include_cursor).map_err(|error| format!("Captura: {error}"))?;
    let mut encoder = Encoder::new(capture.device(), config)
        .map_err(|error| format!("Inicio del codificador GPU: {error}"))?;
    let mut scaler = Scaler::new(capture.device(), config.width, config.height)?;
    let mut cursor = (include_cursor && matches!(target, CaptureTarget::Monitor(_)))
        .then(|| {
            Cursor::new(
                capture.device(),
                scaler.texture(),
                config.width,
                config.height,
            )
        })
        .transpose()?;
    let mut packets = Vec::new();
    let interval = Duration::from_nanos(1_000_000_000 / u64::from(config.fps));
    let mut next = Duration::ZERO;
    let initial = std::time::Instant::now();
    let mut has_output = false;
    let mut has_source = false;
    while !stop.load(Ordering::Relaxed) {
        encoder
            .poll(&mut packets)
            .map_err(|error| format!("Salida del codificador GPU: {error}"))?;
        if !packets.is_empty() {
            has_output = true;
            let mut buffer = data.lock().unwrap();
            if buffer.parameters.sps.is_empty() {
                buffer.parameters = encoder.parameters.clone();
            }
            for packet in packets.drain(..) {
                buffer.push(packet, retain);
            }
        }
        capture.with_frame(clock, cursor.as_mut(), |frame| {
            if !has_source || frame.timestamp >= next.saturating_sub(interval / 2) {
                scaler.stretch(frame)?;
                has_source = true;
            }
            Ok(())
        })?;
        // Una escena quieta puede dejar de entregar frames. Se mantiene el
        // reloj del replay con la última imagen válida, que sigue en la GPU.
        let now = clock.now()?;
        if has_source && now >= next {
            if now.saturating_sub(next) > interval * 2 {
                next = now;
            }
            let texture = scaler.texture();
            let texture = cursor
                .as_ref()
                .map_or(texture, |cursor| cursor.render(texture));
            encoder
                .feed(texture, next)
                .map_err(|error| format!("Entrada del codificador GPU: {error}"))?;
            next += interval;
        }
        if !has_output && initial.elapsed() > Duration::from_secs(8) {
            return Err("La GPU no entregó fotogramas H.264.".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
    drop(capture);
    let _ = encoder.finish();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_capture::window::Window;

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
