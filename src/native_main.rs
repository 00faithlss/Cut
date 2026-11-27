#![allow(non_snake_case)] // the binary is called "Cut"
// Never open a console behind the window, in debug builds either.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod annex_b;
#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod capture_gpu;
#[cfg(windows)]
mod clip_name;
#[cfg(windows)]
mod com;
#[cfg(windows)]
mod compat;
#[cfg(windows)]
mod controller;
#[cfg(windows)]
mod cursor;
#[cfg(windows)]
mod export_folder;
#[cfg(windows)]
mod gpu;
#[cfg(windows)]
mod hotkey;
#[cfg(windows)]
mod instance;
#[cfg(windows)]
mod mic_preview;
#[cfg(windows)]
mod microphone;
#[cfg(windows)]
mod mix;
#[cfg(windows)]
mod mixer_cache;
#[cfg(windows)]
mod mp4;
#[cfg(windows)]
mod native_ui;
#[cfg(windows)]
mod replay;
#[cfg(windows)]
mod scale;
#[cfg(windows)]
mod updates;

#[cfg(windows)]
fn main() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new(export_folder::PICKER_ARG)) {
        let owner = args.next().and_then(|value| value.to_str()?.parse().ok());
        let Some((owner, path)) = owner.zip(args.next()) else {
            std::process::exit(2);
        };
        if let Err(error) = export_folder::run_picker(owner, std::path::Path::new(&path)) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = native_ui::run() {
        eprintln!("Cut: {error}");
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Cut requiere Windows.");
}
