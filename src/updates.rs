use std::{os::windows::process::CommandExt, path::PathBuf, process::Command};

use serde::{Deserialize, Serialize};
use windows::{
    Win32::{
        System::Threading::CREATE_NO_WINDOW,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{HSTRING, PCWSTR, w},
};

const API: &str = "https://api.github.com/repos/00faithlss/Cut/releases/latest";
const RELEASES: &str = "https://github.com/00faithlss/Cut/releases/tag";

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Update {
    pub version: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
}

fn version(value: &str) -> Option<[u16; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    let [major, minor, patch] = parts.as_slice() else {
        return None;
    };
    let numbers = [
        major.parse().ok()?,
        minor.parse().ok()?,
        patch.parse().ok()?,
    ];
    (format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]) == value).then_some(numbers)
}

fn newer_release(release: Release, installed: [u16; 3]) -> Option<Update> {
    if release.draft || release.prerelease {
        return None;
    }
    let published = release.tag_name.strip_prefix('v')?;
    let actual = version(published)?;
    let expected = format!("Cut-{published}-Setup-x64.exe");
    if actual <= installed
        || !release
            .assets
            .iter()
            .any(|asset| asset.name == expected && asset.state == "uploaded")
    {
        return None;
    }
    Some(Update {
        version: published.into(),
    })
}

pub fn check() -> crate::replay::Result<Option<Update>> {
    let installed =
        version(env!("CARGO_PKG_VERSION")).ok_or("La versión local de Cut es inválida.")?;
    let system =
        std::env::var_os("SystemRoot").ok_or("Windows no indicó su directorio de sistema.")?;
    let output = Command::new(PathBuf::from(system).join("System32").join("curl.exe"))
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "5",
            "--max-time",
            "12",
            "--user-agent",
            "Cut-Windows",
            "--header",
            "Accept: application/vnd.github+json",
            API,
        ])
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()?;
    if !output.status.success() {
        return Err("GitHub no respondió con un release accesible.".into());
    }
    Ok(newer_release(
        serde_json::from_slice(&output.stdout)?,
        installed,
    ))
}

pub fn open(update: &Update) -> std::io::Result<()> {
    let url = HSTRING::from(format!("{RELEASES}/v{}", update.version));
    let status = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &url,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if status.0 as isize <= 32 {
        return Err(std::io::Error::other(
            "No se pudo abrir el release en el navegador.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ofrece_solo_versiones_estables_nuevas_con_instalador() {
        let make = |tag: &str, asset: &str, prerelease: bool| Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            assets: vec![Asset {
                name: asset.into(),
                state: "uploaded".into(),
            }],
        };
        let asset = "Cut-0.2.10-Setup-x64.exe";
        let installed = [0, 2, 9];
        assert_eq!(
            newer_release(make("v0.2.10", asset, false), installed)
                .unwrap()
                .version,
            "0.2.10"
        );
        assert!(newer_release(make("v0.2.10", asset, true), installed).is_none());
        assert!(newer_release(make("v0.2.10", "Cut-arm64.exe", false), installed).is_none());
        assert!(newer_release(make("v0.2.10", asset, false), [0, 2, 10]).is_none());
        assert!(newer_release(make("v0.02.10", asset, false), installed).is_none());
    }
}
