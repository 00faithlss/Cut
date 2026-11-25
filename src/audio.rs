use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
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

const RATE: u32 = 48_000;

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
    pub tracks: HashMap<String, Gain>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device: String::new(),
            mono: true,
            noise_suppression: true,
            clip_audio: ClipAudio::default(),
            tracks: HashMap::new(),
        }
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

fn active_applications(enumerator: &DeviceEnumerator) -> Vec<Application> {
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
        Vec::new()
    } else {
        group_processes(&processes(), pids, std::process::id())
    }
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
    let enumerator = DeviceEnumerator::new()?;
    let mut devices = None;
    let mut sessions = Vec::<RenderMeter>::new();
    let mut tracks = AudibleTracks::default();
    let mut previous = None;
    let mut refresh_at = Instant::now();
    loop {
        let now = Instant::now();
        if now >= refresh_at {
            refresh_at = now + Duration::from_secs(2);
            let catalog = capture_devices(&enumerator);
            if devices.as_ref() != Some(&catalog) {
                devices_changed(catalog.clone());
                devices = Some(catalog);
            }
            let processes = processes();
            sessions.clear();
            render_sessions(&enumerator, |control| {
                if let Ok(pid) = control.get_process_id()
                    && pid != 0
                    && let Some((source, _)) = application_for(&processes, pid, std::process::id())
                    && let Ok(meter) = control.get_audiometerinformation()
                {
                    sessions.push(RenderMeter {
                        source,
                        control,
                        meter,
                    });
                }
            });
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
        let audible = tracks.update(now, active);
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
    _event: Handle,
    channels: u16,
    input_channels: u16,
    microphone: Option<Box<Microphone>>,
    staging: Vec<u8>,
    pcm: Vec<u8>,
}

impl Input {
    fn new(pid: Option<u32>, device: &str, mono: bool) -> Result<Self> {
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
        let input_channels = if pid.is_none()
            && client
                .get_mixformat()
                .is_ok_and(|format| format.get_nchannels() == 1)
        {
            1
        } else {
            2
        };
        let channels = if mono { 1 } else { input_channels };
        let (bits, sample) = if pid.is_none() {
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
        client.start_stream()?;
        Ok(Self {
            _client: client,
            capture,
            _event: event,
            channels,
            input_channels,
            microphone: pid.is_none().then(|| Box::new(Microphone::new(channels))),
            staging: Vec::new(),
            pcm: Vec::new(),
        })
    }

    fn read(
        &mut self,
        clock: &Clock,
        gain: u16,
        suppress: bool,
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
            let packet_time = if info.flags.timestamp_error || info.timestamp == 0 {
                clock
                    .now()?
                    .saturating_sub(Duration::from_secs_f64(f64::from(frames) / f64::from(RATE)))
            } else {
                clock.relative(info.timestamp)
            };
            self.pcm.clear();
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
            output(start, &self.pcm, self.channels);
        }
        Ok(())
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
    let mut groups = HashMap::<String, Group>::new();
    let mut sources = Vec::new();
    let mut discover_at = Instant::now();
    let mut previous = Arc::new(Settings::default());
    let mut mix = Mix::combined()?;
    let mut mixed = mix.new_history("Mezcla");
    let mut mixed_sources = HashSet::new();
    while !stop.load(Ordering::Relaxed) {
        let preferences = settings.lock().unwrap().clone();
        if preferences.device != previous.device || preferences.mono != previous.mono {
            groups.remove("mic");
            history.lock().unwrap().remove("mic");
            notices.lock().unwrap().remove("mic");
        }
        let discovery = Instant::now() >= discover_at;
        if discovery {
            discover_at = Instant::now() + Duration::from_secs(2);
            if let Ok(enumerator) = DeviceEnumerator::new() {
                sources = active_applications(&enumerator);
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
        let cutoff = clock.now()?.saturating_sub(retain + Duration::from_secs(3));
        {
            let mut all = history.lock().unwrap();
            all.retain(|id, track| {
                if id != "mix" && !sources.iter().any(|app| &app.source.id == id) {
                    track.level = 0.0;
                }
                while track
                    .packets
                    .front()
                    .is_some_and(|packet| packet.timestamp < cutoff)
                {
                    track.packets.pop_front();
                }
                preferences.tracks.get(id).is_none_or(|gain| gain.include)
                    && (id == "mix" || groups.contains_key(id) || !track.packets.is_empty())
            });
        }
        groups.retain(|id, _| {
            sources.iter().any(|app| &app.source.id == id)
                && preferences.tracks.get(id).is_none_or(|gain| gain.include)
        });
        if mixed_sources
            .iter()
            .any(|id| preferences.tracks.get(id).is_some_and(|gain| !gain.include))
        {
            mixed_sources.clear();
            mixed.packets.clear();
            mix = Mix::combined()?;
            history.lock().unwrap().remove("mix");
        }
        for app in &sources {
            let Source { id, label } = &app.source;
            let gain = preferences.tracks.get(id).copied().unwrap_or(Gain {
                include: true,
                volume: 100,
            });
            if !gain.include {
                notices.lock().unwrap().remove(id);
                continue;
            }
            let is_mic = id == "mic";
            if discovery
                || !previous.tracks.get(id).is_none_or(|gain| gain.include)
                || (is_mic
                    && (preferences.device != previous.device || preferences.mono != previous.mono))
            {
                if let Some(group) = groups.get_mut(id) {
                    group.inputs.retain(|pid, _| app.pids.contains(pid));
                }
                let mut last_error = None;
                for &pid in &app.pids {
                    if groups
                        .get(id)
                        .is_some_and(|group| group.inputs.contains_key(&pid))
                    {
                        continue;
                    }
                    let input = match Input::new(
                        (pid != 0).then_some(pid),
                        &preferences.device,
                        is_mic && preferences.mono,
                    ) {
                        Ok(input) => input,
                        Err(error) => {
                            last_error = Some(error.to_string());
                            continue;
                        }
                    };
                    if let Some(group) = groups.get_mut(id) {
                        group.inputs.insert(pid, input);
                    } else {
                        groups.insert(id.clone(), Group::new(label, pid, input)?);
                    }
                }
                let mut current = notices.lock().unwrap();
                if groups.get(id).is_some_and(|group| !group.inputs.is_empty()) {
                    current.remove(id);
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
