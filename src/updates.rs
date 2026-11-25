use std::{
    fs,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

const API: &str = "https://api.github.com/repos/00faithlss/Cut/releases/latest";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Update {
    pub version: String,
    #[serde(skip)]
    url: String,
    #[serde(skip)]
    digest: String,
    #[serde(skip)]
    size: u64,
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
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    digest: String,
    #[serde(default)]
    size: u64,
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
    if actual <= installed {
        return None;
    }
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == expected && asset.state == "uploaded")?;
    let url = format!(
        "https://github.com/00faithlss/Cut/releases/download/{}/{expected}",
        release.tag_name
    );
    let digest = asset.digest.strip_prefix("sha256:")?;
    if asset.browser_download_url != url
        || digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        || asset.size == 0
    {
        return None;
    }
    Some(Update {
        version: published.into(),
        url,
        digest: digest.into(),
        size: asset.size,
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

fn system_tool(name: &str) -> std::io::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("SystemRoot")
            .ok_or_else(|| std::io::Error::other("No se encontró Windows."))?,
    )
    .join("System32")
    .join(name))
}

pub struct Download {
    path: PathBuf,
    digest: String,
}

impl Drop for Download {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

pub fn download(update: &Update, mut progress: impl FnMut(u64)) -> crate::replay::Result<Download> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("cut-update-{}-{stamp}", std::process::id()));
    fs::create_dir(&directory)?;
    let download = Download {
        path: directory.join("Cut-Setup.exe"),
        digest: update.digest.clone(),
    };
    let mut child = Command::new(system_tool("curl.exe")?)
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            "600",
            "--output",
        ])
        .arg(&download.path)
        .arg(&update.url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let size = fs::metadata(&download.path).map_or(0, |meta| meta.len());
        progress(
            size.saturating_mul(100)
                .checked_div(update.size)
                .unwrap_or(0)
                .min(99),
        );
        thread::sleep(Duration::from_millis(400));
    };
    if !status.success() || fs::metadata(&download.path)?.len() != update.size {
        return Err("La descarga se interrumpió. Puedes volver a intentarlo.".into());
    }
    let verified = Command::new(system_tool("WindowsPowerShell/v1.0/powershell.exe")?)
        .args(["-NoProfile","-NonInteractive","-Command","$ErrorActionPreference='Stop'; $hash=[BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes($env:CUT_UPDATE_FILE))).Replace('-',''); if($hash -ine $env:CUT_UPDATE_HASH){exit 1}"])
        .env("CUT_UPDATE_FILE",&download.path).env("CUT_UPDATE_HASH",&download.digest)
        .env_remove("PSModulePath")
        .creation_flags(CREATE_NO_WINDOW.0).status()?;
    if !verified.success() {
        return Err("La descarga no pasó la verificación SHA-256. Vuelve a intentarlo.".into());
    }
    progress(100);
    Ok(download)
}

pub fn install(download: Download) -> crate::replay::Result<()> {
    let exe = std::env::current_exe()?;
    let script = r#"
$ErrorActionPreference='Stop'
$file=$env:CUT_UPDATE_FILE
try {
  $parent=Get-Process -Id ([int]$env:CUT_UPDATE_PID) -ErrorAction SilentlyContinue
  if($parent -and !$parent.WaitForExit(180000)){throw 'Cut sigue guardando. Reinicia la actualización desde la app.'}
  $hash=[BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes($file))).Replace('-','')
  if($hash -ine $env:CUT_UPDATE_HASH){throw 'El instalador cambió después de descargarlo.'}
  $folder=Split-Path -Parent $env:CUT_UPDATE_EXE
  $args='/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR="'+$folder+'" /LOG="'+$env:CUT_UPDATE_LOG+'"'
  $setup=Start-Process -FilePath $file -ArgumentList $args -WindowStyle Hidden -Wait -PassThru
  if($setup.ExitCode -ne 0){throw ('El instalador terminó con código '+$setup.ExitCode)}
  Start-Process -FilePath $env:CUT_UPDATE_EXE -WindowStyle Hidden
} catch {
  $_ | Out-String | Add-Content -LiteralPath $env:CUT_UPDATE_LOG
  Add-Type -AssemblyName System.Windows.Forms
  [void][System.Windows.Forms.MessageBox]::Show('No se pudo actualizar Cut. Puedes abrirlo de nuevo y reintentar. Registro: '+$env:CUT_UPDATE_LOG,'Actualización de Cut')
} finally {
  Remove-Item -LiteralPath $file -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath (Split-Path -Parent $file) -ErrorAction SilentlyContinue
}
"#;
    let log = crate::mixer_cache::MixerCache::path().with_file_name("actualizacion.log");
    Command::new(system_tool("WindowsPowerShell/v1.0/powershell.exe")?)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("CUT_UPDATE_FILE", &download.path)
        .env("CUT_UPDATE_HASH", &download.digest)
        .env("CUT_UPDATE_EXE", exe)
        .env("CUT_UPDATE_PID", std::process::id().to_string())
        .env("CUT_UPDATE_LOG", log)
        .env_remove("PSModulePath")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;
    // El auxiliar asume la limpieza del instalador después de que Cut termine.
    std::mem::forget(download);
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
                browser_download_url: format!(
                    "https://github.com/00faithlss/Cut/releases/download/{tag}/{asset}"
                ),
                digest: format!("sha256:{}", "a".repeat(64)),
                size: 10,
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
        let mut unverified = make("v0.2.10", asset, false);
        unverified.assets[0].digest.clear();
        assert!(newer_release(unverified, installed).is_none());
        let mut redirected = make("v0.2.10", asset, false);
        redirected.assets[0].browser_download_url = "https://example.com/setup.exe".into();
        assert!(newer_release(redirected, installed).is_none());
    }

    #[test]
    #[ignore = "Descarga el release público de GitHub y verifica SHA-256"]
    fn descarga_verifica_y_limpia_el_instalador_publico() -> crate::replay::Result<()> {
        let mut release: Release = serde_json::from_slice(
            &Command::new(system_tool("curl.exe")?)
                .args([
                    "--disable",
                    "--fail",
                    "--silent",
                    "--max-time",
                    "15",
                    "--user-agent",
                    "Cut-test",
                    API,
                ])
                .output()?
                .stdout,
        )?;
        release.draft = false;
        let update = newer_release(release, [0, 0, 0]).ok_or("No hay release válido.")?;
        let installer = download(&update, |_| {})?;
        let path = installer.path.clone();
        assert!(path.is_file());
        drop(installer);
        assert!(!path.exists());
        Ok(())
    }
}
