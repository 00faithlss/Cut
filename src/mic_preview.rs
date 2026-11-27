use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use wasapi::{
    AudioClient, AudioRenderClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat,
};

use crate::{
    audio::{Clock, PreviewInput, SharedSettings},
    replay::Result,
};

const RATE: usize = 48_000;
const FRAME_BYTES: usize = 4;
// Evita acumular voz si un controlador o la interfaz se demora.
const MAX_QUEUE_BYTES: usize = RATE * FRAME_BYTES * 80 / 1_000;

pub struct Preview {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

pub enum Event {
    Ready,
    Level(f64),
    Error(String),
}

impl Preview {
    pub fn start(
        settings: SharedSettings,
        report: impl Fn(Event) + Send + 'static,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("mic-preview".into())
            .spawn(move || {
                if let Err(error) = run(&worker_stop, &settings, &report)
                    && !worker_stop.load(Ordering::Relaxed)
                {
                    report(Event::Error(error.to_string()));
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_millis(150);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            // Un controlador bloqueado no debe impedir cerrar la ventana.
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

struct Output {
    client: AudioClient,
    render: AudioRenderClient,
    device: String,
}

impl Output {
    fn new(stop: &AtomicBool) -> Result<Self> {
        check_stop(stop)?;
        let enumerator = DeviceEnumerator::new()?;
        check_stop(stop)?;
        let device = enumerator.get_default_device(Direction::Render)?;
        check_stop(stop)?;
        let id = device.get_id()?;
        check_stop(stop)?;
        let mut client = device.get_iaudioclient()?;
        check_stop(stop)?;
        let format = WaveFormat::new(16, 16, SampleType::Int, RATE, 2, None);
        client.initialize_client(
            &format,
            Direction::Render,
            StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: 300_000,
            },
            None,
        )?;
        check_stop(stop)?;
        let render = client.get_audiorenderclient()?;
        check_stop(stop)?;
        let output = Self {
            client,
            render,
            device: id,
        };
        output.client.start_stream()?;
        check_stop(stop)?;
        Ok(output)
    }

    fn write(&self, queue: &mut VecDeque<u8>, stop: &AtomicBool) -> Result<()> {
        check_stop(stop)?;
        let available = self.client.get_available_space_in_frames()? as usize;
        let frames = available.min(queue.len() / FRAME_BYTES);
        if frames == 0 {
            return Ok(());
        }
        check_stop(stop)?;
        self.render
            .write_to_device_from_deque(frames, queue, None)?;
        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

fn capture_device(settings: &crate::audio::Settings) -> Result<String> {
    if !settings.device.is_empty() {
        return Ok(settings.device.clone());
    }
    Ok(DeviceEnumerator::new()?
        .get_default_device(Direction::Capture)?
        .get_id()?)
}

fn check_stop(stop: &AtomicBool) -> Result<()> {
    if stop.load(Ordering::Relaxed) {
        return Err("Escucha cancelada.".into());
    }
    Ok(())
}

fn run(stop: &AtomicBool, settings: &SharedSettings, report: &impl Fn(Event)) -> Result<()> {
    check_stop(stop)?;
    let _apartment = crate::com::Mta::enter()?;
    check_stop(stop)?;
    let clock = Clock::new()?;
    let mut previous = settings.lock().unwrap().clone();
    let mut input_device = capture_device(&previous).map_err(|error| {
        format!("No se pudo abrir el micrófono. Revisa el dispositivo y los permisos: {error}")
    })?;
    check_stop(stop)?;
    let mut input = PreviewInput::new(&previous).map_err(|error| {
        format!("No se pudo abrir el micrófono. Revisa el dispositivo y los permisos: {error}")
    })?;
    check_stop(stop)?;
    let mut output = Output::new(stop)
        .map_err(|error| format!("No se pudo abrir la salida de sonido predeterminada: {error}"))?;
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    report(Event::Ready);
    let mut queue = VecDeque::with_capacity(MAX_QUEUE_BYTES);
    let mut discover_at = Instant::now() + Duration::from_secs(1);
    let mut report_at = Instant::now();
    let mut level = 0.0_f64;
    while !stop.load(Ordering::Relaxed) {
        let preferences = settings.lock().unwrap().clone();
        let discover = Instant::now() >= discover_at;
        let current_input = if discover || preferences.device != previous.device {
            capture_device(&preferences)
                .map_err(|error| format!("El micrófono dejó de estar disponible. Elige otro dispositivo y vuelve a escucharte: {error}"))?
        } else {
            input_device.clone()
        };
        check_stop(stop)?;
        if current_input != input_device
            || preferences.device != previous.device
            || preferences.mono != previous.mono
        {
            input = PreviewInput::new(&preferences)
                .map_err(|error| format!("No se pudo usar el micrófono seleccionado. Revisa el dispositivo y vuelve a escucharte: {error}"))?;
            check_stop(stop)?;
            input_device = current_input;
            queue.clear();
            level = 0.;
        }
        if discover {
            discover_at = Instant::now() + Duration::from_secs(1);
            let current_output = (|| -> Result<String> {
                Ok(DeviceEnumerator::new()?
                    .get_default_device(Direction::Render)?
                    .get_id()?)
            })()
            .map_err(|error| format!("No hay una salida de sonido disponible. Revisa el dispositivo predeterminado y vuelve a escucharte: {error}"))?;
            check_stop(stop)?;
            if current_output != output.device {
                output = Output::new(stop).map_err(|error| {
                    format!("No se pudo cambiar a la salida de sonido predeterminada: {error}")
                })?;
                queue.clear();
            }
        }
        previous = preferences.clone();
        input.read(&clock, &preferences, |_, pcm, channels| {
            for frame in pcm.chunks_exact(usize::from(channels) * 2) {
                let left = i16::from_le_bytes([frame[0], frame[1]]);
                let right = if channels == 1 { left } else { i16::from_le_bytes([frame[2], frame[3]]) };
                level = level.max(f64::from(left.unsigned_abs().max(right.unsigned_abs())) / 32768.);
                queue.extend(left.to_le_bytes());
                queue.extend(right.to_le_bytes());
            }
            if queue.len() > MAX_QUEUE_BYTES {
                queue.drain(..queue.len() - MAX_QUEUE_BYTES);
            }
        }).map_err(|error| format!("El micrófono dejó de responder. Revisa el dispositivo y vuelve a escucharte: {error}"))?;
        check_stop(stop)?;
        if let Err(error) = output.write(&mut queue, stop) {
            // El dispositivo puede invalidarse antes de comprobar la salida predeterminada.
            output = Output::new(stop).map_err(|reopen| format!("La salida de sonido dejó de responder. Revisa el dispositivo y vuelve a escucharte: {error}; {reopen}"))?;
            queue.clear();
        }
        if Instant::now() >= report_at {
            check_stop(stop)?;
            report(Event::Level(level));
            level *= 0.75;
            report_at = Instant::now() + Duration::from_millis(50);
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
