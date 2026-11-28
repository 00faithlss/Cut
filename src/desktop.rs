use std::{
    borrow::Cow,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn},
        Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT},
    },
    core::PCWSTR,
};
use windows_capture::{monitor::Monitor, window::Window};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};
use wry::{
    WebView, WebViewBuilder, WebViewBuilderExtWindows,
    http::{Response, StatusCode},
};

use crate::{
    audio,
    mixer_cache::MixerCache,
    replay::{Profile, Replay, Result, Snapshot},
};

const ORIGIN: &str = "https://cut.local";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const ASSETS: &[(&str, &str, &[u8])] = &[
    (
        "/",
        "text/html; charset=utf-8",
        include_bytes!("../assets/legacy/cut-ascii.html"),
    ),
    (
        "/studio.css",
        "text/css",
        include_bytes!("../assets/legacy/studio.css"),
    ),
    (
        "/studio.js",
        "application/javascript",
        include_bytes!("../assets/legacy/studio.js"),
    ),
    (
        "/ui-assets/cut.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/cut.svg"),
    ),
    (
        "/ui-assets/waveform.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/waveform.svg"),
    ),
    (
        "/ui-assets/film-strip.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/film-strip.svg"),
    ),
    (
        "/ui-assets/sliders-horizontal.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/sliders-horizontal.svg"),
    ),
    (
        "/ui-assets/pause.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/pause.svg"),
    ),
    (
        "/ui-assets/play.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/play.svg"),
    ),
    (
        "/ui-assets/monitor.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/monitor.svg"),
    ),
    (
        "/ui-assets/floppy-disk.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/floppy-disk.svg"),
    ),
    (
        "/ui-assets/arrows-clockwise.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/arrows-clockwise.svg"),
    ),
    (
        "/ui-assets/game-controller.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/game-controller.svg"),
    ),
    (
        "/ui-assets/discord-logo.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/discord-logo.svg"),
    ),
    (
        "/ui-assets/spotify-logo.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/spotify-logo.svg"),
    ),
    (
        "/ui-assets/microphone.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/microphone.svg"),
    ),
    (
        "/ui-assets/browser.svg",
        "image/svg+xml",
        include_bytes!("../assets/legacy/ui-assets/browser.svg"),
    ),
    (
        "/ui-assets/Inter-LICENSE.txt",
        "text/plain",
        include_bytes!("../assets/legacy/ui-assets/Inter-LICENSE.txt"),
    ),
    (
        "/ui-assets/Plex-LICENSE.txt",
        "text/plain",
        include_bytes!("../assets/legacy/ui-assets/Plex-LICENSE.txt"),
    ),
    (
        "/ui-assets/Phosphor-LICENSE.txt",
        "text/plain",
        include_bytes!("../assets/legacy/ui-assets/Phosphor-LICENSE.txt"),
    ),
    (
        "/ui-assets/inter.woff2",
        "font/woff2",
        include_bytes!("../assets/legacy/ui-assets/inter.woff2"),
    ),
    (
        "/ui-assets/plex-mono.woff2",
        "font/woff2",
        include_bytes!("../assets/legacy/ui-assets/plex-mono.woff2"),
    ),
];

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Command {
    Ready,
    Sources,
    AudioSources,
    Hotkey {
        binding: crate::hotkey::Binding,
    },
    HotkeyEdit {
        editing: bool,
    },
    OpenDirectory,
    ChooseDirectory,
    CheckUpdates,
    OpenUpdate,
    Audio {
        settings: audio::Settings,
    },
    Start {
        profile: Profile,
    },
    AwaitSource {
        profile: Profile,
        label: Option<String>,
    },
    SelectPausedProfile {
        profile: Option<Profile>,
    },
    Pause,
    Save,
    Drag,
    Minimize,
    Maximize,
    Hide,
    Startup {
        enabled: bool,
    },
    MinimizeOnClose {
        enabled: bool,
    },
}

enum Message {
    Ui(Command),
    Hotkey,
    DirectoryPicked(std::result::Result<Option<PathBuf>, String>),
    UpdateChecked(std::result::Result<Option<crate::updates::Update>, String>),
    DownloadProgress(u64),
    Downloaded(std::result::Result<crate::updates::Download, String>),
    Saved(std::result::Result<PathBuf, String>),
    AudioDevices(u64, audio::Catalog),
    Audible(u64, Vec<audio::Source>),
    AudioError(u64, String),
    Show,
    ToggleCapture,
    Exit,
    CaptureReleased,
}

fn play_clip_saved() {
    static SOUND: &[u8] = include_bytes!("clip-saved.wav");
    // PlaySound puede leer el WAV en segundo plano durante toda la reproducción.
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(SOUND.as_ptr().cast()),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

#[derive(Serialize)]
struct Source {
    id: String,
    label: String,
}

fn update(web: &Option<WebView>, name: &str, value: impl Serialize) {
    if let Some(web) = web
        && let Ok(payload) = serde_json::to_string(&value)
    {
        let _ = web.evaluate_script(&format!("window.cutDesktop?.{name}({payload})"));
    }
}

fn save_async(snapshot: Snapshot, directory: PathBuf, proxy: EventLoopProxy<Message>) {
    let on_error = proxy.clone();
    if let Err(error) = thread::Builder::new()
        .name("cut-export".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(|| {
                snapshot.save(&directory).map_err(|error| error.to_string())
            })
            .unwrap_or_else(|_| Err("El exportador se interrumpió.".into()));
            let _ = proxy.send_event(Message::Saved(result));
        })
    {
        let _ = on_error.send_event(Message::Saved(Err(error.to_string())));
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

fn send_sources(web: &Option<WebView>) {
    let (monitors, windows) = capture_sources();
    update(
        web,
        "sources",
        json!({ "monitors": monitors, "windows": windows }),
    );
}

fn startup_value() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>("Cut"))
        .is_ok()
}

fn set_startup(enabled: bool) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
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

fn round_window(window: &tao::window::Window) {
    let hwnd = HWND(window.hwnd() as *mut _);
    unsafe {
        if window.is_maximized() {
            SetWindowRgn(hwnd, None, true);
            return;
        }
        let bounds = window.outer_size();
        if bounds.width == 0 || bounds.height == 0 {
            return;
        }
        let diameter = (32.0 * window.scale_factor()).round() as i32;
        let region = CreateRoundRectRgn(
            0,
            0,
            bounds.width as i32,
            bounds.height as i32,
            diameter,
            diameter,
        );
        if !region.is_invalid() && SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(region.into());
        }
    }
}

fn icon_rgba() -> Result<(Vec<u8>, u32, u32)> {
    let image = include_bytes!("../assets/legacy/ui-assets/cut.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(image)).read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size().ok_or("Icono inválido")?];
    let frame = reader.next_frame(&mut pixels)?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err("El icono de Cut requiere RGBA de 8 bits.".into());
    }
    pixels.truncate(frame.buffer_size());
    Ok((pixels, frame.width, frame.height))
}

fn create_webview(
    window: &tao::window::Window,
    ipc_proxy: EventLoopProxy<Message>,
) -> Result<WebView> {
    let data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("CutClipper")
        .join("WebView");
    let mut context = wry::WebContext::new(Some(data));
    Ok(WebViewBuilder::new_with_web_context(&mut context)
        .with_https_scheme(true)
        .with_background_color((18, 18, 18, 255))
        .with_default_context_menus(false)
        .with_initialization_script(include_str!("desktop.js"))
        .with_custom_protocol("cut".into(), |_, request| {
            let asset = ASSETS.iter().find(|entry| entry.0 == request.uri().path());
            let Some((_, mime, bytes)) = asset else {
                return Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Cow::Borrowed(&[][..]))
                    .unwrap();
            };
            Response::builder()
                .header("Content-Type", *mime)
                .header("X-Content-Type-Options", "nosniff")
                .body(Cow::Borrowed(*bytes))
                .unwrap()
        })
        .with_navigation_handler(|url| url == format!("{ORIGIN}/"))
        .with_ipc_handler(move |request| {
            if request.uri().host() == Some("cut.local")
                && let Ok(command) = serde_json::from_str(request.body())
            {
                let _ = ipc_proxy.send_event(Message::Ui(command));
            }
        })
        .with_url(format!("{ORIGIN}/"))
        .build(window)?)
}

pub fn run() -> Result<()> {
    let Some(instance) = crate::instance::Instance::open()? else {
        return Ok(());
    };
    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    let (pixels, width, height) = icon_rgba()?;
    let window = WindowBuilder::new()
        .with_title("Cut")
        .with_decorations(false)
        .with_undecorated_shadow(false)
        .with_window_icon(Some(tao::window::Icon::from_rgba(
            pixels.clone(),
            width,
            height,
        )?))
        .with_inner_size(LogicalSize::new(1140, 710))
        .with_min_inner_size(LogicalSize::new(720, 560))
        .with_visible(false)
        .build(&event_loop)?;
    round_window(&window);
    let event_proxy = event_loop.create_proxy();
    let tray_menu = Menu::new();
    let open_item = MenuItem::with_id("show", "Abrir Cut", true, None);
    let capture_item = MenuItem::with_id("capture", "Iniciar captura", true, None);
    let save_item = MenuItem::with_id("save", "Guardar clip", true, None);
    let exit_item = MenuItem::with_id("exit", "Salir de Cut", true, None);
    tray_menu.append_items(&[&open_item, &capture_item, &save_item, &exit_item])?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("Cut")
        .with_menu(Box::new(tray_menu))
        .with_icon(Icon::from_rgba(pixels, width, height)?)
        .build()?;
    let menu_proxy = event_proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = match event.id().as_ref() {
            "show" => Message::Show,
            "capture" => Message::ToggleCapture,
            "save" => Message::Ui(Command::Save),
            "exit" => Message::Exit,
            _ => return,
        };
        let _ = menu_proxy.send_event(action);
    }));
    let mut web = Some(create_webview(&window, event_proxy.clone())?);

    let mut clip_hotkey = crate::hotkey::Binding::load();
    let hotkey_binding = Arc::new(Mutex::new(clip_hotkey));
    let hotkey_editing = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hotkey_proxy = event_proxy.clone();
    let registered = crate::hotkey::listen(
        Arc::clone(&hotkey_binding),
        Arc::clone(&hotkey_editing),
        move || {
            let _ = hotkey_proxy.send_event(Message::Hotkey);
        },
    );
    let mut replay: Option<Replay> = None;
    let mixer_path = MixerCache::path();
    let loaded_mixer = MixerCache::load(&mixer_path);
    let restore_settings = loaded_mixer.is_some();
    let mut mixer = loaded_mixer.unwrap_or_default();
    crate::hotkey::configure_talk(mixer.settings.push_to_talk);
    let mut persist_at = None;
    let audio = Arc::new(Mutex::new(Arc::new(mixer.settings.clone())));
    let mut audio_monitor: Option<std::sync::mpsc::SyncSender<()>> = None;
    let mut audio_generation = 0;
    let mut ui_ready = false;
    let mut initialized = false;
    let mut last_profile: Option<Profile> = None;
    let mut awaiting_source: Option<(Profile, Option<String>)> = None;
    let mut next_source_scan = Instant::now();
    let mut next_capture_retry = Instant::now();
    let mut tray_active = false;
    let mut latest_save_notice = None;
    let mut hidden_since = None;
    let mut next_web_retry = Instant::now();
    let mut saving = false;
    let mut queued_save = None;
    let mut retiring = false;
    let mut pending_profile = None;
    let mut paused = false;
    let mut next_poll = Instant::now();
    let mut directory = crate::export_folder::load();
    let mut choosing_directory = false;
    let mut available_update = None;
    let mut installing_update = false;
    let mut update_progress = None;
    let mut checking_updates = false;
    let mut report_update = false;
    let mut next_update_check = Instant::now();
    let mut exiting = false;
    let mut minimize_on_close = false;
    let mut window_shape = (
        window.outer_size(),
        window.scale_factor(),
        window.is_maximized(),
    );
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(next_poll);
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. }
            | Event::UserEvent(Message::Ui(Command::Hide)) => {
                hotkey_editing.store(false, std::sync::atomic::Ordering::Relaxed);
                if minimize_on_close {
                    window.set_minimized(true);
                } else {
                    window.set_visible(false);
                }
            }
            Event::WindowEvent { event: WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }, .. } => {
                let shape = (window.outer_size(), window.scale_factor(), window.is_maximized());
                if shape != window_shape {
                    window_shape = shape;
                    round_window(&window);
                }
            }
            Event::UserEvent(Message::Show) => {
                window.set_visible(true);
                window.set_minimized(false);
                window.set_focus();
            }
            Event::UserEvent(Message::ToggleCapture) => {
                if let Some(web) = &web && ui_ready {
                    let _ = web.evaluate_script("window.cutDesktop?.toggleCapture()");
                } else {
                    let command = if !paused && (retiring || replay.as_ref().is_some_and(Replay::active)) {
                        Command::Pause
                    } else if let Some(profile) = last_profile.clone().filter(|_| awaiting_source.is_none()) {
                        Command::Start { profile }
                    } else {
                        let _ = event_proxy.send_event(Message::Show);
                        return;
                    };
                    let _ = event_proxy.send_event(Message::Ui(command));
                }
            }
            Event::UserEvent(Message::Exit) => {
                exiting = true;
                if persist_at.take().is_some() {
                    let _ = mixer.save(&mixer_path);
                }
                pending_profile = None;
                if let Some(previous) = replay.take() {
                    previous.pause();
                    retiring = true;
                    let proxy = event_proxy.clone();
                    thread::spawn(move || {
                        drop(previous);
                        let _ = proxy.send_event(Message::CaptureReleased);
                    });
                }
                if !retiring && !saving { *flow = ControlFlow::Exit; }
            }
            Event::UserEvent(Message::Ui(Command::Ready)) => {
                update(&web, "ready", json!({"startup": startup_value(), "version": env!("CARGO_PKG_VERSION"), "hotkey": registered,
                    "directory": directory.to_string_lossy(), "audioSettings": &mixer.settings, "restoreSettings": restore_settings || initialized, "paused": paused,
                    "clipHotkey": clip_hotkey,
                    "sourceId": last_profile.as_ref().filter(|_| initialized && awaiting_source.is_none()).map(|profile| &profile.id)}));
                send_sources(&web);
                update(&web, "updateAvailable", &available_update);
                update(&web, "updateProgress", update_progress);
                update(&web, "updateInstalling", installing_update);
                if let Some(notice) = &latest_save_notice { update(&web, "notice", notice); }
                if !initialized { window.set_visible(true); }
                ui_ready = true;
                if !initialized && !restore_settings {
                    persist_at = Some(Instant::now() + Duration::from_millis(500));
                }
                initialized = true;
            }
            Event::UserEvent(Message::Ui(Command::OpenDirectory)) => {
                if let Err(error) = crate::export_folder::open(&directory) {
                    update(&web, "notice", format!("No se pudo abrir la carpeta. {error}"));
                }
            }
            Event::UserEvent(Message::Ui(Command::CheckUpdates)) => {
                report_update = true;
                next_update_check = Instant::now();
                update(&web, "updateMessage", "Buscando actualizaciones…");
            }
            Event::UserEvent(Message::Ui(Command::OpenUpdate)) => {
                if !exiting && update_progress.is_none() && !installing_update
                    && let Some(release) = available_update.clone()
                {
                    update_progress = Some(0u64);
                    update(&web, "updateProgress", update_progress);
                    let proxy = event_proxy.clone();
                    thread::spawn(move || {
                        let result = crate::updates::download(&release, |percent| {
                            let _ = proxy.send_event(Message::DownloadProgress(percent));
                        }).map_err(|error| error.to_string());
                        let _ = proxy.send_event(Message::Downloaded(result));
                    });
                }
            }
            Event::UserEvent(Message::DownloadProgress(percent)) => {
                update_progress = Some(percent);
                update(&web, "updateProgress", update_progress);
            }
            Event::UserEvent(Message::Downloaded(result)) => {
                match result {
                    Ok(download) => {
                        match crate::updates::install(download) {
                        Ok(()) => {
                            installing_update = true;
                            update(&web, "updateInstalling", true);
                            update(&web, "updateMessage", "Terminando los clips pendientes. Cut se actualizará y volverá a abrirse…");
                            let _ = event_proxy.send_event(Message::Exit);
                        }
                        Err(error) => {
                            update_progress = None;
                            update(&web, "updateProgress", update_progress);
                            update(&web, "updateMessage", error.to_string());
                        }
                        }
                    }
                    Err(error) => {
                        update_progress = None;
                        update(&web, "updateProgress", update_progress);
                        update(&web, "updateMessage", error);
                    }
                }
            }
            Event::UserEvent(Message::UpdateChecked(result)) => {
                checking_updates = false;
                match result {
                    Ok(release) => {
                        available_update = release;
                        let tooltip = available_update.as_ref().map_or_else(
                            || "Cut".to_string(),
                            |release| format!("Cut · Actualización {} disponible", release.version),
                        );
                        let _ = tray.set_tooltip(Some(tooltip));
                        update(&web, "updateAvailable", &available_update);
                        if report_update {
                            let message = if available_update.is_some() { "" } else { "Tienes la última versión." };
                            update(&web, "updateMessage", message);
                        }
                    }
                    Err(error) if report_update => update(&web, "updateMessage", error),
                    Err(_) => {}
                }
                report_update = false;
                next_update_check = Instant::now() + Duration::from_secs(6 * 60 * 60);
            }
            Event::UserEvent(Message::Ui(Command::ChooseDirectory)) if !choosing_directory => {
                let owner = window.hwnd();
                let current = directory.clone();
                let proxy = event_proxy.clone();
                choosing_directory = true;
                thread::spawn(move || {
                    let result = crate::export_folder::choose(owner, &current).map_err(|e| e.to_string());
                    let _ = proxy.send_event(Message::DirectoryPicked(result));
                });
            }
            Event::UserEvent(Message::DirectoryPicked(result)) => {
                choosing_directory = false;
                match result {
                    Ok(Some(selected)) => match crate::export_folder::remember(&selected) {
                        Ok(()) => {
                            directory = selected;
                            update(&web, "directory", directory.to_string_lossy());
                        }
                        Err(error) => update(&web, "notice", format!("No se pudo guardar la ubicación. {error}")),
                    },
                    Err(error) => update(&web, "notice", format!("No se pudo elegir la carpeta. {error}")),
                    Ok(None) => {}
                }
            }
            Event::UserEvent(Message::Ui(Command::Sources)) => send_sources(&web),
            Event::UserEvent(Message::Ui(Command::Drag)) => {
                let _ = window.drag_window();
            }
            Event::UserEvent(Message::Ui(Command::Minimize)) => {
                hotkey_editing.store(false, std::sync::atomic::Ordering::Relaxed);
                window.set_minimized(true);
            }
            Event::UserEvent(Message::Ui(Command::Maximize)) => window.set_maximized(!window.is_maximized()),
            Event::UserEvent(Message::Ui(Command::MinimizeOnClose { enabled })) => {
                minimize_on_close = enabled;
            }
            Event::UserEvent(Message::Ui(Command::AudioSources)) => {
                if let Some(monitor) = &audio_monitor
                    && let Err(std::sync::mpsc::TrySendError::Disconnected(_)) = monitor.try_send(())
                {
                    audio_monitor = None;
                }
            }
            Event::UserEvent(Message::Ui(Command::Audio { settings })) => {
                if let Some(binding) = settings.push_to_talk
                    && (!binding.valid_talk() || binding == clip_hotkey || !registered) {
                    update(&web, "notice", "Elige una tecla válida para hablar, distinta de Guardar clip. Los atajos globales deben estar disponibles.");
                    return;
                }
                if mixer.settings != settings {
                    crate::hotkey::configure_talk(settings.push_to_talk);
                    *audio.lock().unwrap() = Arc::new(settings.clone());
                    mixer.settings = settings;
                    persist_at = Some(Instant::now() + Duration::from_millis(350));
                }
            }
            Event::UserEvent(Message::AudioDevices(generation, catalog)) if generation == audio_generation && audio_monitor.is_some() => {
                update(&web, "audioSources", catalog);
            }
            Event::UserEvent(Message::Audible(generation, apps)) if generation == audio_generation && audio_monitor.is_some() => {
                update(&web, "audible", apps);
            }
            Event::UserEvent(Message::AudioError(generation, error)) if generation == audio_generation && audio_monitor.is_some() => {
                update(&web, "audioError", error);
            }
            Event::UserEvent(Message::Ui(Command::SelectPausedProfile { profile })) if paused => {
                last_profile = profile;
            }
            Event::UserEvent(Message::Ui(Command::AwaitSource { profile, label })) => {
                awaiting_source = Some((profile, label));
                paused = false;
            }
            Event::UserEvent(Message::Ui(Command::Start { profile })) => {
                awaiting_source = None;
                if exiting { return; }
                if !paused && replay.as_ref().is_some_and(|current| current.active() && current.profile == profile) {
                    return;
                }
                paused = false;
                last_profile = Some(profile.clone());
                if retiring {
                    pending_profile = Some(profile);
                } else if let Some(previous) = replay.take() {
                    previous.pause();
                    pending_profile = Some(profile);
                    retiring = true;
                    let proxy = event_proxy.clone();
                    thread::spawn(move || {
                        drop(previous);
                        let _ = proxy.send_event(Message::CaptureReleased);
                    });
                } else {
                    match Replay::start(profile, Arc::clone(&audio)) {
                        Ok(capture) => replay=Some(capture),
                        Err(error) => update(&web, "notice", error.to_string()),
                    }
                }
            }
            Event::UserEvent(Message::CaptureReleased) => {
                retiring = false;
                if exiting {
                    if !saving { *flow = ControlFlow::Exit; }
                } else if let Some(profile) = pending_profile.take() {
                    let _ = event_proxy.send_event(Message::Ui(Command::Start { profile }));
                }
            }
            Event::UserEvent(Message::Ui(Command::Pause)) => {
                paused = true;
                awaiting_source = None;
                pending_profile = None;
                if let Some(capture) = &replay { capture.pause(); }
            }
            Event::UserEvent(Message::Ui(Command::Hotkey { binding })) => {
                if mixer.settings.push_to_talk == Some(binding) {
                    update(&web, "hotkeyError", "Esta tecla está asignada a pulsar para hablar.");
                    return;
                }
                match binding.save() {
                    Ok(()) => {
                        *hotkey_binding.lock().unwrap() = binding;
                        clip_hotkey = binding;
                        hotkey_editing.store(false, std::sync::atomic::Ordering::Relaxed);
                        update(&web, "hotkey", binding);
                    }
                    Err(error) => update(&web, "hotkeyError", error.to_string()),
                }
            }
            Event::UserEvent(Message::Ui(Command::HotkeyEdit { editing })) => {
                hotkey_editing.store(editing, std::sync::atomic::Ordering::Relaxed);
                update(&web, "hotkeyEditing", editing);
            }
            Event::UserEvent(Message::Ui(Command::Startup { enabled })) => {
                if let Err(error)=set_startup(enabled) {
                    update(&web, "startup", startup_value());
                    update(&web, "notice",format!("No se pudo cambiar el inicio con Windows. {error}"));
                }
            }
            Event::UserEvent(Message::Hotkey) if hotkey_editing.load(std::sync::atomic::Ordering::Relaxed) => {}
            Event::UserEvent(Message::Ui(Command::Save) | Message::Hotkey) => {
                if exiting { return; }
                if queued_save.is_some() {
                    update(&web, "notice", "Ya hay otro clip en espera.");
                    return;
                }
                let snapshot = replay.as_ref().ok_or_else(|| "Activa la captura para llenar el replay.".into())
                    .and_then(Replay::snapshot);
                match snapshot {
                    Ok(snapshot) => {
                        if saving {
                            queued_save = Some((snapshot, directory.clone()));
                            update(&web, "notice", "Segundo clip en espera.");
                        } else {
                            saving = true;
                            update(&web, "notice", "Guardando clip…");
                            save_async(snapshot, directory.clone(), event_proxy.clone());
                        }
                    }
                    Err(error) => {
                        let message = error.to_string();
                        update(&web, "notice", &message);
                        latest_save_notice = Some(message);
                    }
                }
            }
            Event::UserEvent(Message::Saved(result)) => {
                saving = false;
                let message = match result {
                    Ok(path) => {
                        play_clip_saved();
                        format!("Clip guardado en {}", path.display())
                    }
                    Err(error) => format!("No se pudo guardar el clip. {error}"),
                };
                update(&web, "notice", &message);
                latest_save_notice = Some(message);
                if let Some((snapshot, destination)) = queued_save.take() {
                    saving = true;
                    save_async(snapshot, destination, event_proxy.clone());
                } else if exiting && !retiring {
                    *flow = ControlFlow::Exit;
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) | Event::MainEventsCleared if Instant::now() >= next_poll => {
                next_poll=Instant::now()+Duration::from_millis(350);
                *flow=ControlFlow::WaitUntil(next_poll);
                if persist_at.is_some_and(|deadline| Instant::now() >= deadline) {
                    persist_at = None;
                    if let Err(error) = mixer.save(&mixer_path) {
                        update(&web, "notice", format!("No se pudo guardar la configuración del mezclador. {error}"));
                    }
                }
                if instance.take_open_request() {
                    window.set_visible(true);
                    window.set_minimized(false);
                    window.set_focus();
                }
                let visible = window.is_visible() && !window.is_minimized();
                if visible {
                    hidden_since = None;
                    if web.is_none() && !exiting && Instant::now() >= next_web_retry {
                        match create_webview(&window, event_proxy.clone()) {
                            Ok(view) => { web = Some(view); }
                            Err(error) => {
                                next_web_retry = Instant::now() + Duration::from_secs(5);
                                let _ = tray.set_tooltip(Some(format!("No se pudo abrir Cut. {error}")));
                            }
                        }
                    }
                } else if ui_ready {
                    let since = hidden_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= Duration::from_secs(3) {
                        web = None;
                        ui_ready = false;
                    }
                }
                let active = !paused && replay.as_ref().is_some_and(|capture| !capture.stalled());
                if !exiting && !paused && !active && !retiring && awaiting_source.is_none()
                    && Instant::now() >= next_capture_retry
                    && let Some(capture) = &replay
                    && capture.profile.monitor
                {
                    next_capture_retry = Instant::now() + Duration::from_secs(3);
                    // Detén también un codificador que dejó de entregar fotogramas.
                    capture.pause();
                    let _ = event_proxy.send_event(Message::Ui(Command::Start { profile: capture.profile.clone() }));
                }
                if active != tray_active {
                    capture_item.set_text(if active { "Pausar captura" } else { "Iniciar captura" });
                    tray_active = active;
                }
                if !exiting && web.is_none() && Instant::now() >= next_source_scan
                    && let Some((profile, preferred)) = &awaiting_source
                    && (profile.monitor || preferred.is_some())
                {
                    next_source_scan = Instant::now() + Duration::from_millis(3500);
                    let (monitors, windows) = capture_sources();
                    let sources = if profile.monitor { &monitors } else { &windows };
                    let target = sources.iter().find(|source| Some(&source.label) == preferred.as_ref())
                        .or_else(|| if profile.monitor { monitors.first() } else { None });
                    if let Some(source) = target {
                        let mut profile = profile.clone();
                        profile.id.clone_from(&source.id);
                        let _ = event_proxy.send_event(Message::Ui(Command::Start { profile }));
                    }
                }
                if !exiting && !checking_updates && Instant::now() >= next_update_check {
                    checking_updates = true;
                    let proxy = event_proxy.clone();
                    thread::spawn(move || {
                        let result = crate::updates::check().map_err(|error| error.to_string());
                        let _ = proxy.send_event(Message::UpdateChecked(result));
                    });
                }
                if !visible || !ui_ready {
                    if audio_monitor.take().is_some() {
                        update(&web, "audible", Vec::<audio::Source>::new());
                    }
                    return;
                }
                let (seconds, error)=replay.as_ref().map_or((0,None), Replay::status);
                update(&web, "status", json!({"seconds":seconds,"active":active,"error":error,"paused":paused}));
                if let Some(capture) = &replay {
                    update(&web, "audioNotices", capture.audio_notices());
                }
                update(&web, "levels", replay.as_ref().filter(|_| active).map(Replay::levels).unwrap_or_default());
                if !exiting && audio_monitor.is_none() {
                    let (tx, rx) = std::sync::mpsc::sync_channel(1);
                    audio_monitor = Some(tx);
                    audio_generation += 1;
                    let generation = audio_generation;
                    let proxy = event_proxy.clone();
                    thread::spawn(move || {
                        let result = audio::monitor_mixer(
                            rx,
                            |devices| { let _ = proxy.send_event(Message::AudioDevices(generation, devices)); },
                            |apps| { let _ = proxy.send_event(Message::Audible(generation, apps)); },
                        );
                        if let Err(error) = result {
                            let _ = proxy.send_event(Message::AudioError(generation, error.to_string()));
                        }
                    });
                }
            }
            _ => {}
        }
    })
}
