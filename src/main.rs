#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod capture_gpu;
#[cfg(windows)]
mod clip_name;
#[cfg(windows)]
mod cursor;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod gpu;
#[cfg(windows)]
mod hotkey;
#[cfg(windows)]
mod instance;
#[cfg(windows)]
mod microphone;
#[cfg(windows)]
mod mix;
#[cfg(windows)]
mod mixer_cache;
#[cfg(windows)]
mod mp4;
#[cfg(windows)]
mod replay;
#[cfg(windows)]
mod scale;

#[cfg(windows)]
fn main() {
    if let Err(error) = desktop::run() {
        eprintln!("Cut: {error}");
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Cut requiere Windows.");
}
