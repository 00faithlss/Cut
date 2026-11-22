use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, SYSTEMTIME},
        Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW},
        System::SystemInformation::GetLocalTime,
    },
    core::PCWSTR,
};

static EXPORT_NUMBER: AtomicU64 = AtomicU64::new(0);

pub fn new_name(game: Option<&str>) -> String {
    name_at(game, unsafe { GetLocalTime() })
}

fn name_at(game: Option<&str>, date: SYSTEMTIME) -> String {
    let date = format!("{:02}-{:02}-{:04}", date.wDay, date.wMonth, date.wYear);
    match game {
        Some(title) => format!("Cut_{}_{date}", safe_game_name(title)),
        None => format!("Cut_{date}"),
    }
}

fn safe_game_name(title: &str) -> String {
    let clean: String = title
        .trim()
        .chars()
        .take(70)
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    let clean = clean.trim_matches([' ', '.', '_']);
    if clean.is_empty() {
        "Juego".to_owned()
    } else {
        clean.to_owned()
    }
}

pub fn create_temporary(directory: &Path, stem: &str) -> io::Result<(PathBuf, File)> {
    loop {
        let id = EXPORT_NUMBER.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!("{stem}.{}-{id}.partial.mp4", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

pub fn publish(temporary: &Path, directory: &Path, stem: &str) -> io::Result<PathBuf> {
    let from = wide_path(&std::fs::canonicalize(temporary)?);
    let absolute_directory = std::fs::canonicalize(directory)?;
    for number in 1.. {
        let name = if number == 1 {
            format!("{stem}.mp4")
        } else {
            format!("{stem}_{number:02}.mp4")
        };
        let to = wide_path(&absolute_directory.join(&name));
        // El cambio de nombre es atómico y nunca reemplaza otro clip.
        let result = unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVE_FILE_FLAGS(0),
            )
        };
        match result {
            Ok(()) => return Ok(directory.join(name)),
            Err(error)
                if [ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS]
                    .iter()
                    .any(|code| code.to_hresult() == error.code()) => {}
            Err(error) => return Err(io::Error::other(error)),
        }
    }
    unreachable!()
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    #[test]
    fn nombra_capturas_y_sanitiza_el_titulo_del_juego() {
        let date = SYSTEMTIME {
            wYear: 2026,
            wMonth: 3,
            wDay: 7,
            ..Default::default()
        };
        assert_eq!(name_at(None, date), "Cut_07-03-2026");
        assert_eq!(
            name_at(Some("  The Witcher 3: Edición / GOTY?  "), date),
            "Cut_The Witcher 3_ Edición _ GOTY_07-03-2026"
        );
        assert_eq!(name_at(Some("..."), date), "Cut_Juego_07-03-2026");
    }

    #[test]
    fn publicar_dos_clips_con_la_misma_fecha_conserva_ambos() -> io::Result<()> {
        let directory = std::env::temp_dir().join(format!("cut-nombres-{}", std::process::id()));
        fs::create_dir_all(&directory)?;
        let stem = "Cut_22-11-2026";
        let (first, mut file) = create_temporary(&directory, stem)?;
        file.write_all(b"primero")?;
        drop(file);
        let first = publish(&first, &directory, stem)?;
        let (second, mut file) = create_temporary(&directory, stem)?;
        file.write_all(b"segundo")?;
        drop(file);
        let second = publish(&second, &directory, stem)?;
        assert_eq!(first.file_name().unwrap(), "Cut_22-11-2026.mp4");
        assert_eq!(second.file_name().unwrap(), "Cut_22-11-2026_02.mp4");
        assert_eq!(fs::read(&first)?, b"primero");
        assert_eq!(fs::read(&second)?, b"segundo");
        fs::remove_file(first)?;
        fs::remove_file(second)?;
        fs::remove_dir(directory)?;
        Ok(())
    }
}
