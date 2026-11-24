use std::{
    ffi::OsString,
    io::{self, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use windows::{
    Win32::{
        Foundation::{ERROR_CANCELLED, HWND},
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
            },
            Threading::CREATE_NO_WINDOW,
        },
        UI::Shell::{
            FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
            SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
        },
    },
    core::{HRESULT, HSTRING, Result, w},
};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

const KEY: &str = r"Software\Cut";
const VALUE: &str = "ClipDirectory";
pub const PICKER_ARG: &str = "--choose-clip-folder";

pub fn load() -> PathBuf {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(KEY)
        .and_then(|key| key.get_value::<OsString, _>(VALUE))
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("Videos")
                .join("Cut")
        })
}

pub fn remember(path: &Path) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY)?;
    key.set_value(VALUE, &path.as_os_str())
}

pub fn open(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()?;
    Ok(())
}

pub fn choose(owner: isize, current: &Path) -> io::Result<Option<PathBuf>> {
    // El selector carga extensiones de Explorer. Al salir el auxiliar libera esa memoria.
    let output = Command::new(std::env::current_exe()?)
        .arg(PICKER_ARG)
        .arg(owner.to_string())
        .arg(current)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Falló el selector de Windows ({}) {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let (bytes, remaining) = output.stdout.as_chunks::<2>();
    if !remaining.is_empty() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let units: Vec<_> = bytes.iter().copied().map(u16::from_le_bytes).collect();
    Ok(Some(PathBuf::from(OsString::from_wide(&units))))
}

pub fn run_picker(owner: isize, current: &Path) -> crate::replay::Result<()> {
    if let Some(path) = native_dialog(owner, current)? {
        let bytes: Vec<_> = path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect();
        io::stdout().lock().write_all(&bytes)?;
    }
    Ok(())
}

fn native_dialog(owner: isize, current: &Path) -> Result<Option<PathBuf>> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let result = (|| {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetTitle(w!("Carpeta de clips"))?;
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
            if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                &HSTRING::from(current.as_os_str()),
                None,
            ) {
                dialog.SetFolder(&folder)?;
            }
            let owner = (owner != 0).then_some(HWND(owner as *mut _));
            if let Err(error) = dialog.Show(owner) {
                if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                    return Ok(None);
                }
                return Err(error);
            }
            let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = OsString::from_wide(name.as_wide());
            CoTaskMemFree(Some(name.0.cast()));
            Ok(Some(PathBuf::from(path)))
        })();
        CoUninitialize();
        result
    }
}
