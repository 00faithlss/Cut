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

#[derive(Clone, Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
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
            || ![15, 30, 60, 120, 300, 600].contains(&self.seconds)
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

#[cfg(test)]
mod profile_tests {
    use super::Profile;

    #[test]
    fn admite_diez_minutos_y_conserva_duraciones_anteriores() {
        let mut profile = Profile {
            id: "1".into(),
            monitor: true,
            height: 1080,
            fps: 60,
            bitrate: 20,
            seconds: 600,
            cursor: true,
        };
        for seconds in [15, 30, 60, 120, 300, 600] {
            profile.seconds = seconds;
            assert_eq!(profile.dimensions().unwrap(), (1920, 1080));
        }
        for seconds in [0, 599, 601, u64::MAX] {
            profile.seconds = seconds;
            assert!(profile.dimensions().is_err());
        }
    }
}

pub struct Replay {
    pub profile: Profile,
    pub software_encoding: bool,
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
        Self::start_with_encoder(
            profile,
            settings,
            std::env::var_os("CUT_FORCE_SOFTWARE_ENCODER").is_some(),
        )
    }

    fn start_with_encoder(
        mut profile: Profile,
        settings: audio::SharedSettings,
        force_software: bool,
    ) -> Result<Self> {
        let (width, height) = profile.dimensions()?;
        let target = profile.target()?;
        if matches!(target, CaptureTarget::Window(_)) {
            crate::compat::Capabilities::current()?.check_window_capture(profile.cursor)?;
        }
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
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new().name("cut-capture".into()).spawn({
            let data = Arc::clone(&data);
            let stop = Arc::clone(&stop);
            let clock = Arc::clone(&clock);
            let include_cursor = profile.cursor;
            move || {
                if let Err(error) = record(
                    RecordingRequest {
                        target,
                        config,
                        retain,
                        include_cursor,
                        force_software,
                    },
                    &stop,
                    &data,
                    &clock,
                    // Conserva el emisor hasta publicar un posible error de inicio.
                    ready_tx.clone(),
                ) {
                    data.lock().unwrap().error = Some(error.to_string());
                }
                stop.store(true, Ordering::Relaxed);
            }
        })?;
        let (effective, software_encoding) = match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(format) => format,
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                return Err(data
                    .lock()
                    .unwrap()
                    .error
                    .clone()
                    .unwrap_or_else(|| "El codificador no pudo iniciar a tiempo.".into())
                    .into());
            }
        };
        profile.height = effective.height;
        profile.fps = effective.fps;
        profile.bitrate = effective.bitrate / 1_000_000;
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
            software_encoding,
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

    pub fn buffer_bytes(&self) -> usize {
        self.data.lock().unwrap().bytes()
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
        let parameters = data.parameters.clone();
        let samples = data.frames().cloned().collect();
        drop(data);
        Ok(Snapshot {
            name: clip_name::new_name(self.game_name.as_deref()),
            video: mp4::Video {
                width,
                height,
                fps: self.profile.fps,
                parameters,
            },
            samples,
            audio: {
                let settings = self.settings.lock().unwrap();
                let history = self.audio.lock().unwrap();
                match settings.clip_audio {
                    audio::ClipAudio::Mixed => mp4::Audio::Mixed(history.get("mix").cloned()),
                    audio::ClipAudio::Separate => mp4::Audio::Separate(
                        history
                            .iter()
                            .filter(|(id, _)| settings.gain(id).include)
                            .map(|(id, track)| (id.clone(), track.clone()))
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
        let extend = |id: &str, track: &mut audio::History| {
            if let Some(current) = live.get(id) {
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
                    extend("mix", track);
                } else {
                    *mix = live.get("mix").cloned();
                }
            }
            mp4::Audio::Separate(tracks) => {
                for (id, track) in tracks {
                    extend(id, track);
                }
            }
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

struct RecordingRequest {
    target: CaptureTarget,
    config: VideoConfig,
    retain: Duration,
    include_cursor: bool,
    force_software: bool,
}

fn record(
    request: RecordingRequest,
    stop: &AtomicBool,
    data: &Mutex<Buffer>,
    clock: &audio::Clock,
    ready: mpsc::SyncSender<(VideoConfig, bool)>,
) -> Result<()> {
    // Una espera breve del historial no debe consumir el tiempo del siguiente fotograma.
    thread::scope(|scope| {
        let (tx, rx) = mpsc::sync_channel::<(EncodedSample, Option<ParameterSets>)>(8);
        let retain = request.retain;
        let history = thread::Builder::new()
            .name("cut-replay-history".into())
            .spawn_scoped(scope, move || -> std::io::Result<()> {
                let mut writer = storage::Writer::default();
                for (packet, parameters) in rx {
                    let frame = writer.append(packet)?;
                    let mut history = data.lock().unwrap();
                    if let Some(parameters) = parameters {
                        history.parameters = parameters;
                    }
                    history.push(frame, retain);
                }
                Ok(())
            })?;
        let result = record_frames(request, stop, tx, clock, ready);
        history
            .join()
            .map_err(|_| "Se interrumpió el historial de video.")??;
        result
    })
}

fn record_frames(
    request: RecordingRequest,
    stop: &AtomicBool,
    tx: mpsc::SyncSender<(EncodedSample, Option<ParameterSets>)>,
    clock: &audio::Clock,
    ready: mpsc::SyncSender<(VideoConfig, bool)>,
) -> Result<()> {
    let RecordingRequest {
        target,
        config,
        retain: _,
        include_cursor,
        force_software,
    } = request;
    // El hilo posee la sesión de captura, las texturas y el codificador.
    let _apartment = crate::com::Mta::enter()?;
    let mut capture =
        Capture::open(target, include_cursor).map_err(|error| format!("Captura: {error}"))?;
    let (mut encoder, config, software) =
        Encoder::select(capture.device(), config, force_software)?;
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
    ready
        .send((config, software))
        .map_err(|_| "Se canceló el inicio de la captura.")?;
    let mut packets = Vec::new();
    let interval = Duration::from_nanos(1_000_000_000 / u64::from(config.fps));
    let mut next = Duration::ZERO;
    let initial = std::time::Instant::now();
    let mut has_output = false;
    let mut last_frame = None;
    // Prepara las superficies mientras el MFT termina el fotograma anterior.
    // Serializar conversión y codificación reduce la cadencia con la GPU ocupada.
    let mut prepared = std::collections::VecDeque::new();
    // Deja cuatro de las doce muestras del pool para el MFT y la imagen retenida.
    // CPU sólo dispone de dos texturas alternadas: no preparar varias por delante.
    let capacity = if software { 1 } else { 8 };
    while !stop.load(Ordering::Relaxed) {
        encoder
            .poll(&mut packets)
            .map_err(|error| format!("Salida del codificador H.264: {error}"))?;
        for packet in packets.drain(..) {
            let parameters = packet.is_keyframe.then(|| encoder.parameters.clone());
            tx.send((packet, parameters))
                .map_err(|_| "Se cerró el historial de video.")?;
            has_output = true;
        }
        if !has_output && initial.elapsed() > Duration::from_secs(8) {
            return Err("El codificador no entregó fotogramas H.264.".into());
        }
        while encoder.accepts_input() {
            let Some((frame, timestamp)) = prepared.pop_front() else {
                break;
            };
            encoder
                .submit(&frame, timestamp)
                .map_err(|error| format!("Entrada del codificador H.264: {error}"))?;
        }
        let now = clock.now()?;
        if now >= next && prepared.len() < capacity {
            let Some(input) = encoder.input_frame()? else {
                // Deja que el codificador recupere todas las superficies si se saturó.
                last_frame = None;
                timer.wait(Duration::from_millis(1));
                continue;
            };
            let source = match &mut cursor {
                Some((bgra, target, cursor)) => {
                    // Conserva el cursor como capa independiente durante la captura del monitor.
                    capture.with_frame(clock, Some(cursor), |frame, cursor| {
                        if let Some(cursor) = cursor.filter(|cursor| cursor.needs_composition()) {
                            bgra.stretch(frame, target, 0)?;
                            nv12.stretch_texture(
                                cursor.render(target),
                                config.width,
                                config.height,
                                &input.texture,
                                input.index,
                            )?;
                        } else {
                            nv12.stretch(frame, &input.texture, input.index)?;
                        }
                        Ok(())
                    })?
                }
                None => capture.with_frame(clock, None, |frame, _| {
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
                // Retiene la muestra para que el pool no recicle la imagen que se repite.
                if source {
                    last_frame = Some(input.clone());
                }
                prepared.push_back((input, next));
                next += interval;
            }
        }
        let until_frame = next.saturating_sub(clock.now()?);
        timer.wait(if until_frame.is_zero() || !prepared.is_empty() {
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
    use win_native_media::encoder::{EncodedSample, ParameterSets};
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
                mp4::Audio::Separate(
                    [
                        ("mix".into(), track("Mezcla")),
                        ("app:discord".into(), track("Discord")),
                    ]
                    .into(),
                ),
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
                assert!(
                    fs::read(&path)?
                        .windows(15)
                        .any(|bytes| bytes == b"colrnclx\0\x01\0\x01\0\x01\0"),
                    "El MP4 debe declarar BT.709 limitado"
                );
                let file = fs::File::open(&path)?;
                let len = file.metadata()?.len();
                let reader = ::mp4::Mp4Reader::read_header(std::io::BufReader::new(file), len)?;
                let count = reader
                    .tracks()
                    .values()
                    .filter(|track| matches!(track.track_type(), Ok(::mp4::TrackType::Audio)))
                    .count();
                let mut audio: Vec<_> = reader
                    .tracks()
                    .values()
                    .filter(|track| matches!(track.track_type(), Ok(::mp4::TrackType::Audio)))
                    .collect();
                audio.sort_by_key(|track| track.track_id());
                for (i, track) in audio.iter().enumerate() {
                    assert_eq!(track.trak.tkhd.alternate_group, 1);
                    assert_eq!(track.trak.tkhd.flags & 1, u32::from(i == 0));
                }
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
    fn exportacion_conserva_diez_minutos_en_pelicula_y_pistas() -> Result<()> {
        let mut spool = storage::Writer::default();
        let frames = (0..18000)
            .map(|i| {
                spool.append(EncodedSample {
                    timestamp: Duration::from_nanos(i * 1_000_000_000 / 30),
                    is_keyframe: i % 30 == 0,
                    data: vec![0, 0, 0, 1, 0x65, 0x80],
                })
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        let video = mp4::Video {
            width: 1280,
            height: 720,
            fps: 30,
            parameters: ParameterSets {
                sps: vec![0x67, 0x64, 0, 0x1f],
                pps: vec![0x68, 0xce, 0x3c, 0x80],
            },
        };
        let path = std::env::temp_dir().join(format!(
            "cut-duration-regression-{}.mp4",
            std::process::id()
        ));
        mp4::write(
            &path,
            fs::File::create(&path)?,
            &video,
            &frames,
            &mp4::Audio::Mixed(None),
        )?;
        let file = fs::File::open(&path)?;
        let reader = ::mp4::Mp4Reader::read_header(
            std::io::BufReader::new(file),
            fs::metadata(&path)?.len(),
        )?;
        let movie = reader.duration();
        let tracks: Vec<_> = reader
            .tracks()
            .values()
            .map(|track| track.duration())
            .collect();
        drop(reader);
        fs::remove_file(path)?;
        assert_eq!(movie, Duration::from_secs(600));
        assert!(
            tracks
                .iter()
                .all(|duration| *duration == Duration::from_secs(600))
        );
        Ok(())
    }

    #[test]
    #[ignore = "Reutiliza un MP4 real indicado en CUT_VERIFY_MP4 sin volver a capturar"]
    fn reexporta_material_real_y_verifica_duracion() -> Result<()> {
        let input = std::env::var_os("CUT_VERIFY_MP4").ok_or("Falta CUT_VERIFY_MP4")?;
        let file = fs::File::open(&input)?;
        let mut reader = ::mp4::Mp4Reader::read_header(
            std::io::BufReader::new(file),
            fs::metadata(&input)?.len(),
        )?;
        let track = reader
            .tracks()
            .values()
            .find(|track| matches!(track.track_type(), Ok(::mp4::TrackType::Video)))
            .ok_or("Falta video")?;
        let (id, scale, count) = (track.track_id(), track.timescale(), track.sample_count());
        let video = mp4::Video {
            width: track.width() as u32,
            height: track.height() as u32,
            fps: track.frame_rate().round() as u32,
            parameters: ParameterSets {
                sps: track.sequence_parameter_set()?.to_vec(),
                pps: track.picture_parameter_set()?.to_vec(),
            },
        };
        let to_time = |ticks: u64, scale: u32| {
            Duration::from_nanos((u128::from(ticks) * 1_000_000_000 / u128::from(scale)) as u64)
        };
        let mut spool = storage::Writer::default();
        let mut frames = Vec::new();
        for i in 1..=count {
            let sample = reader.read_sample(id, i)?.ok_or("Falta muestra de video")?;
            let mut data = Vec::new();
            let mut avcc = sample.bytes.as_ref();
            while !avcc.is_empty() {
                let prefix: [u8; 4] = avcc.get(..4).ok_or("NAL incompleta")?.try_into()?;
                let length = u32::from_be_bytes(prefix) as usize;
                let nalu = avcc.get(4..4 + length).ok_or("NAL truncada")?;
                data.extend_from_slice(&[0, 0, 0, 1]);
                data.extend_from_slice(nalu);
                avcc = &avcc[4 + length..];
            }
            frames.push(spool.append(EncodedSample {
                timestamp: to_time(sample.start_time, scale),
                is_keyframe: sample.is_sync,
                data,
            })?);
        }
        let track = reader
            .tracks()
            .values()
            .filter(|track| matches!(track.track_type(), Ok(::mp4::TrackType::Audio)))
            .min_by_key(|track| track.track_id())
            .ok_or("Falta audio")?;
        let (id, scale, count) = (track.track_id(), track.timescale(), track.sample_count());
        let channels = if track.channel_config()? == ::mp4::ChannelConfig::Mono {
            1
        } else {
            2
        };
        let mut history = audio::History {
            name: "Mezcla".into(),
            channels,
            packets: Default::default(),
            level: 0.,
        };
        for i in 1..=count {
            let sample = reader.read_sample(id, i)?.ok_or("Falta muestra de audio")?;
            history
                .packets
                .push_back(Arc::new(win_native_media::audio::EncodedAudio {
                    timestamp: to_time(sample.start_time, scale),
                    data: sample.bytes.to_vec(),
                }));
        }
        let path = std::env::temp_dir()
            .join("cut-verificacion")
            .join(format!("Cut-10min-corregido-{}.mp4", std::process::id()));
        mp4::write(
            &path,
            fs::File::create(&path)?,
            &video,
            &frames,
            &mp4::Audio::Mixed(Some(history)),
        )?;
        let file = fs::File::open(&path)?;
        let reader = ::mp4::Mp4Reader::read_header(
            std::io::BufReader::new(file),
            fs::metadata(&path)?.len(),
        )?;
        assert!(reader.duration() >= Duration::from_secs(598));
        let video = reader
            .tracks()
            .values()
            .find(|track| matches!(track.track_type(), Ok(::mp4::TrackType::Video)))
            .unwrap();
        assert_eq!(video.sample_count() as usize, frames.len());
        assert!(reader.duration().abs_diff(video.duration()) < Duration::from_millis(25));
        println!(
            "{}: {:?}, {} fotogramas preservados",
            path.display(),
            reader.duration(),
            frames.len()
        );
        Ok(())
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
    #[ignore = "Requiere Windows interactivo, GPU y más de diez minutos de captura real"]
    fn retiene_y_exporta_diez_minutos_reales() -> Result<()> {
        let replay = Replay::start(
            Profile {
                id: "0".into(),
                monitor: true,
                height: 720,
                fps: 30,
                bitrate: 10,
                seconds: 600,
                cursor: false,
            },
            Default::default(),
        )?;
        espera_frames(&replay, 600)?;
        let snapshot = replay.snapshot()?;
        let elapsed = snapshot
            .samples
            .last()
            .unwrap()
            .timestamp
            .saturating_sub(snapshot.samples.first().unwrap().timestamp);
        assert!(
            elapsed >= Duration::from_secs(598),
            "Historial incompleto: {elapsed:?}"
        );
        let path = snapshot.save(&std::env::temp_dir().join("cut-verificacion"))?;
        let file = fs::File::open(&path)?;
        let length = file.metadata()?.len();
        let reader = ::mp4::Mp4Reader::read_header(std::io::BufReader::new(file), length)?;
        assert!(reader.duration() >= Duration::from_secs(598));
        println!(
            "Replay real de diez minutos: {} ({:?})",
            path.display(),
            reader.duration()
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
    #[ignore = "Requiere escritorio Windows y encoder H.264 de Windows; fuerza CPU sin desactivar la GPU"]
    fn respaldo_cpu_captura_y_exporta_1080p30_en_ram() -> Result<()> {
        let capture = Replay::start_with_encoder(
            Profile {
                id: "0".into(),
                monitor: true,
                height: 2160,
                fps: 120,
                bitrate: 80,
                seconds: 15,
                cursor: false,
            },
            Default::default(),
            true,
        )?;
        assert!(capture.software_encoding);
        assert_eq!(
            (
                capture.profile.height,
                capture.profile.fps,
                capture.profile.bitrate
            ),
            (1080, 30, 20)
        );
        espera_frames(&capture, 4)?;
        assert!(capture.buffer_bytes() > 0);
        let snapshot = capture.snapshot()?;
        assert_eq!(
            (
                snapshot.video.width,
                snapshot.video.height,
                snapshot.video.fps
            ),
            (1920, 1080, 30)
        );
        let path = snapshot.save(&std::env::temp_dir().join("cut-cpu-verificacion"))?;
        let file = fs::File::open(&path)?;
        let reader = ::mp4::Mp4Reader::read_header(
            std::io::BufReader::new(file),
            fs::metadata(&path)?.len(),
        )?;
        assert!(reader.duration() >= Duration::from_secs(3));
        println!(
            "Respaldo Windows H.264 CPU: {} ({:?})",
            path.display(),
            reader.duration()
        );
        Ok(())
    }

    #[test]
    #[ignore = "Requiere escritorio Windows, GPU y unos 42 segundos; verifica ventana móvil real en GPU/CPU"]
    fn replay_ram_exporta_ultimos_quince_segundos_y_sigue_capturando() -> Result<()> {
        for force_software in [false, true] {
            let capture = Replay::start_with_encoder(
                Profile {
                    id: "0".into(),
                    monitor: true,
                    height: 720,
                    fps: 30,
                    bitrate: 10,
                    seconds: 15,
                    cursor: false,
                },
                Default::default(),
                force_software,
            )?;
            espera_frames(&capture, 15)?;
            let first = capture.snapshot()?;
            let initial_start = first.samples.first().unwrap().timestamp;
            let initial_end = first.samples.last().unwrap().timestamp;
            let directory = std::env::temp_dir().join("cut-ram-verificacion");
            let saved_directory = directory.clone();
            let export =
                thread::spawn(move || first.save(&saved_directory).map_err(|e| e.to_string()));
            thread::sleep(Duration::from_secs(5));
            let second = capture.snapshot()?;
            let start = second.samples.first().unwrap().timestamp;
            let end = second.samples.last().unwrap().timestamp;
            assert!(
                start >= initial_start + Duration::from_secs(3),
                "El inicio no avanzó: {initial_start:?} -> {start:?}"
            );
            assert!(
                end >= initial_end + Duration::from_secs(4),
                "La captura se detuvo al guardar"
            );
            assert!(
                end - start <= Duration::from_secs(17),
                "Se acumuló video desde el inicio"
            );
            let first_path = export
                .join()
                .map_err(|_| "Falló el hilo de exportación")??;
            let second_path = second.save(&directory)?;
            assert_ne!(first_path, second_path);
            let file = fs::File::open(&second_path)?;
            let reader = ::mp4::Mp4Reader::read_header(
                std::io::BufReader::new(file),
                fs::metadata(&second_path)?.len(),
            )?;
            assert!(
                (Duration::from_secs(14)..Duration::from_secs(17)).contains(&reader.duration())
            );
            println!(
                "{}: {:?}, sesión {:?}, inicio {:?}, {} bytes RAM",
                if capture.software_encoding {
                    "CPU"
                } else {
                    "GPU"
                },
                reader.duration(),
                end,
                start,
                capture.buffer_bytes()
            );
        }
        Ok(())
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
        assert!(
            !capture.software_encoding,
            "Esta prueba requiere un encoder por hardware disponible."
        );
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
