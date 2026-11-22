use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use wasapi::{
    AudioCaptureClient, AudioClient, AudioClientProperties, DeviceEnumerator, Direction, Handle,
    SampleType, SessionState, StreamCategory, StreamMode, WaveFormat,
};
use win_native_media::audio::{EncodedAudio, aac::AacEncoder};
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

use crate::mix::Mix;
use crate::replay::Result;

const RATE: u32 = 48_000;
const AAC_FRAMES: usize = 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct Settings {
    pub device: String,
    pub mono: bool,
    pub voice: bool,
    pub tracks: HashMap<String, Gain>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device: String::new(),
            mono: true,
            voice: true,
            tracks: HashMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Gain {
    pub include: bool,
    pub volume: u16,
}

#[derive(Clone, Serialize)]
pub struct Source {
    pub id: String,
    pub label: String,
}

#[derive(Default, Serialize)]
pub struct Catalog {
    pub devices: Vec<Source>,
    pub apps: Vec<Source>,
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

pub fn discover() -> Result<Catalog> {
    let enumerator = DeviceEnumerator::new()?;
    let mut found = Catalog::default();
    if let Ok(devices) = enumerator.get_device_collection(Direction::Capture) {
        for device in (&devices).into_iter().flatten() {
            if let (Ok(id), Ok(label)) = (device.get_id(), device.get_friendlyname()) {
                found.devices.push(Source { id, label });
            }
        }
    }
    let mut pids = HashSet::new();
    if let Ok(speakers) = enumerator.get_device_collection(Direction::Render) {
        for device in (&speakers).into_iter().flatten() {
            let sessions = device
                .get_iaudiosessionmanager()
                .and_then(|manager| manager.get_audiosessionenumerator());
            if let Ok(sessions) = sessions {
                for index in 0..sessions.get_count().unwrap_or(0) {
                    if let Ok(control) = sessions.get_session(index)
                        && control.get_state().ok() == Some(SessionState::Active)
                        && let Ok(pid) = control.get_process_id()
                        && pid != 0
                        && pid != std::process::id()
                    {
                        pids.insert(pid);
                    }
                }
            }
        }
    }
    let process = processes();
    for &pid in &pids {
        let Some(&(mut parent, ref label)) = process.get(&pid) else {
            continue;
        };
        let mut visited = HashSet::new();
        while !pids.contains(&parent) && visited.insert(parent) {
            let Some(entry) = process.get(&parent) else {
                break;
            };
            parent = entry.0;
        }
        if parent != pid && pids.contains(&parent) {
            continue;
        }
        found.apps.push(Source {
            id: pid.to_string(),
            label: label.clone(),
        });
    }
    found
        .apps
        .sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    Ok(found)
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
    _event: Handle,
    encoder: AacEncoder,
    channels: u16,
    input_channels: u16,
    voice: Option<[Voice; 2]>,
    staging: Vec<u8>,
    pcm: Vec<u8>,
    start: Duration,
    samples: Vec<EncodedAudio>,
    peak: f64,
}

impl Input {
    fn new(pid: Option<u32>, device: &str, mono: bool, voice: bool) -> Result<Self> {
        let mut client = if let Some(pid) = pid {
            AudioClient::new_application_loopback_client(pid, true)?
        } else {
            let enumerator = DeviceEnumerator::new()?;
            let source = if device.is_empty() {
                enumerator.get_default_device(Direction::Capture)?
            } else {
                enumerator.get_device(device)?
            };
            source.get_iaudioclient()?
        };
        if voice {
            let _ = client
                .set_properties(AudioClientProperties::new().set_category(StreamCategory::Speech));
        }
        let input_channels = if pid.is_none()
            && client
                .get_mixformat()
                .is_ok_and(|format| format.get_nchannels() == 1)
        {
            1
        } else {
            2
        };
        let channels = if mono { 1 } else { 2 };
        let format = WaveFormat::new(
            16,
            16,
            SampleType::Int,
            RATE as usize,
            input_channels as usize,
            None,
        );
        client.initialize_client(
            &format,
            Direction::Capture,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: 0,
            },
            None,
        )?;
        let event = client.set_get_eventhandle()?;
        let capture = client.get_audiocaptureclient()?;
        let encoder = AacEncoder::new(RATE, channels, if mono { 96_000 } else { 128_000 })?;
        client.start_stream()?;
        Ok(Self {
            _client: client,
            capture,
            _event: event,
            encoder,
            channels,
            input_channels,
            voice: voice.then(|| [Voice::default(), Voice::default()]),
            staging: Vec::new(),
            pcm: Vec::new(),
            start: Duration::ZERO,
            samples: Vec::new(),
            peak: 0.0,
        })
    }

    fn read(&mut self, clock: &Clock, gain: u16, mix: &mut Mix) -> Result<()> {
        let mut peak = 0;
        while let Some(size) = self.capture.get_next_packet_size()? {
            if size == 0 {
                break;
            }
            let input_width = usize::from(self.input_channels) * 2;
            self.staging.resize(size as usize * input_width, 0);
            let (frames, info) = self.capture.read_from_device(&mut self.staging)?;
            if frames == 0 {
                break;
            }
            if info.flags.data_discontinuity {
                self.pcm.clear();
            }
            let packet_time = if info.flags.timestamp_error || info.timestamp == 0 {
                clock
                    .now()?
                    .saturating_sub(Duration::from_secs_f64(f64::from(frames) / f64::from(RATE)))
            } else {
                clock.relative(info.timestamp)
            };
            if self.pcm.is_empty() {
                self.start = packet_time;
            }
            let amplify = |sample: i32| {
                (sample * i32::from(gain.min(200)) / 100)
                    .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
            };
            let packet_start = self.pcm.len();
            for frame in self.staging[..frames as usize * input_width].chunks_exact(input_width) {
                let left = i32::from(i16::from_le_bytes([frame[0], frame[1]]));
                let right = if self.input_channels == 1 {
                    left
                } else {
                    i32::from(i16::from_le_bytes([frame[2], frame[3]]))
                };
                if self.channels == 1 {
                    let center = if left.abs() > 2 * right.abs() {
                        left
                    } else if right.abs() > 2 * left.abs() {
                        right
                    } else if left == 0 || right == 0 {
                        left + right
                    } else {
                        (left + right) / 2
                    };
                    let center = amplify(
                        self.voice
                            .as_mut()
                            .map_or(center, |filters| filters[0].process(center)),
                    );
                    peak = peak.max(center.unsigned_abs());
                    self.pcm.extend_from_slice(&center.to_le_bytes());
                } else {
                    let (left, right) = if let Some(filters) = &mut self.voice {
                        (filters[0].process(left), filters[1].process(right))
                    } else {
                        (left, right)
                    };
                    let (left, right) = (amplify(left), amplify(right));
                    peak = peak.max(left.unsigned_abs()).max(right.unsigned_abs());
                    self.pcm.extend_from_slice(&left.to_le_bytes());
                    self.pcm.extend_from_slice(&right.to_le_bytes());
                }
            }
            mix.add(packet_time, &self.pcm[packet_start..], self.channels);
            let chunk = AAC_FRAMES * usize::from(self.channels) * 2;
            let mut consumed = 0;
            while self.pcm.len() - consumed >= chunk {
                self.encoder.encode(
                    &self.pcm[consumed..consumed + chunk],
                    self.start,
                    &mut self.samples,
                )?;
                self.start +=
                    Duration::from_nanos(AAC_FRAMES as u64 * 1_000_000_000 / u64::from(RATE));
                consumed += chunk;
            }
            self.pcm.drain(..consumed);
        }
        self.peak = (self.peak * 0.91).max(f64::from(peak) / 32768.0);
        Ok(())
    }
}

#[derive(Default)]
struct Voice {
    previous: f32,
    high_pass: f32,
    low_pass: f32,
    envelope: f32,
}

impl Voice {
    fn process(&mut self, sample: i32) -> i32 {
        let sample = sample as f32;
        self.high_pass = 0.988 * (self.high_pass + sample - self.previous);
        self.previous = sample;
        self.low_pass += 0.63 * (self.high_pass - self.low_pass);
        self.envelope += (self.low_pass.abs() - self.envelope)
            * if self.low_pass.abs() > self.envelope {
                0.005
            } else {
                0.00025
            };
        let gate = ((self.envelope - 75.0) / 225.0).clamp(0.0, 1.0);
        let boost = 2.55 / (1.0 + self.envelope / 12_000.0);
        (self.low_pass * (0.10 + 0.90 * gate) * boost) as i32
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        let _ = self._client.stop_stream();
    }
}

pub fn run(
    stop: &AtomicBool,
    clock: &Clock,
    retain: Duration,
    settings: &Mutex<Settings>,
    history: &Tracks,
    notices: &Notices,
) -> Result<()> {
    wasapi::initialize_mta().ok()?;
    let mut inputs = HashMap::<String, Input>::new();
    let mut sources = HashMap::<String, String>::new();
    let mut discover_at = Instant::now();
    let mut device = String::new();
    let mut mono = false;
    let mut voice = false;
    let mut mix = Mix::new()?;
    let mut mixed = mix.new_history();
    let mut mixed_sources = HashSet::new();
    while !stop.load(Ordering::Relaxed) {
        let preferences = settings.lock().unwrap().clone();
        if preferences.device != device || preferences.mono != mono || preferences.voice != voice {
            inputs.remove("mic");
            history.lock().unwrap().remove("mic");
            notices.lock().unwrap().remove("mic");
            device.clone_from(&preferences.device);
            mono = preferences.mono;
            voice = preferences.voice;
        }
        let mut discovery = false;
        if Instant::now() >= discover_at {
            discover_at = Instant::now() + Duration::from_secs(2);
            discovery = true;
            if let Ok(catalog) = discover() {
                sources.extend(
                    catalog
                        .apps
                        .into_iter()
                        .map(|source| (source.id, source.label)),
                );
                let running = processes();
                sources.retain(|id, _| {
                    id.parse()
                        .ok()
                        .is_some_and(|pid| running.contains_key(&pid))
                });
                inputs.retain(|id, _| id == "mic" || sources.contains_key(id));
            }
            sources.insert("mic".into(), "Micrófono".into());
        }
        let cutoff = clock.now()?.saturating_sub(retain + Duration::from_secs(3));
        {
            let mut all = history.lock().unwrap();
            all.retain(|id, track| {
                track.packets.retain(|packet| packet.timestamp >= cutoff);
                preferences.tracks.get(id).is_none_or(|gain| gain.include)
                    && (id == "mix" || sources.contains_key(id) || !track.packets.is_empty())
            });
        }
        // Al excluir una pista también se descarta su contribución a la mezcla anterior.
        if mixed_sources
            .iter()
            .any(|id| preferences.tracks.get(id).is_some_and(|gain| !gain.include))
        {
            mixed_sources.clear();
            mixed.packets.clear();
            mix = Mix::new()?;
            history.lock().unwrap().remove("mix");
        }
        for (id, label) in &sources {
            let gain = preferences.tracks.get(id).copied().unwrap_or(Gain {
                include: true,
                volume: 100,
            });
            if !gain.include {
                inputs.remove(id);
                notices.lock().unwrap().remove(id);
                continue;
            }
            if !inputs.contains_key(id) {
                if !discovery && notices.lock().unwrap().contains_key(id) {
                    continue;
                }
                match Input::new(
                    id.parse().ok(),
                    &device,
                    id == "mic" && mono,
                    id == "mic" && voice,
                ) {
                    Ok(input) => {
                        notices.lock().unwrap().remove(id);
                        let mut all = history.lock().unwrap();
                        all.entry(id.clone()).or_insert_with(|| History {
                            name: label.clone(),
                            channels: input.channels,
                            packets: VecDeque::new(),
                            level: 0.0,
                        });
                        inputs.insert(id.clone(), input);
                    }
                    Err(error) => {
                        notices
                            .lock()
                            .unwrap()
                            .insert(id.clone(), error.to_string());
                        continue;
                    }
                }
            }
            let input = inputs.get_mut(id).unwrap();
            if let Err(error) = input.read(clock, gain.volume, &mut mix) {
                notices
                    .lock()
                    .unwrap()
                    .insert(id.clone(), error.to_string());
                inputs.remove(id);
                continue;
            }
            mixed_sources.insert(id.clone());
            if let Some(track) = history.lock().unwrap().get_mut(id) {
                track.level = input.peak;
                track.packets.extend(input.samples.drain(..).map(Arc::new));
            }
        }
        mix.render(clock.now()?, &mut mixed)?;
        if !mixed.packets.is_empty() {
            let mut tracks = history.lock().unwrap();
            tracks
                .entry("mix".into())
                .or_insert_with(|| mix.new_history())
                .packets
                .append(&mut mixed.packets);
        }
        thread::sleep(Duration::from_millis(12));
    }
    Ok(())
}
