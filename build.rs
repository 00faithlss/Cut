//! Embeds every font found in `assets/fonts` (run `scripts/get-fonts.ps1` once).
//! If the folder is empty the UI falls back to Segoe UI / Consolas and still builds.
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/fonts");
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dir = root.join("assets").join("fonts");
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    matches!(
                        p.extension().and_then(|e| e.to_str()),
                        Some("ttf") | Some("otf") | Some("TTF") | Some("OTF")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    let mut code = String::from("pub static EMBEDDED_FONTS: &[&[u8]] = &[\n");
    for path in &entries {
        let path = path.to_string_lossy().replace('\\', "/");
        code.push_str(&format!("    include_bytes!(r\"{path}\"),\n"));
    }
    code.push_str("];\n");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("embedded_fonts.rs"), code).unwrap();

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources(&root, &out_dir);
    }
}

/// Icon (resource 1: used by Explorer, the taskbar, the title bar and the tray) and the
/// version block Windows shows as the program name ("Cut") in Task Manager and Properties.
fn embed_windows_resources(root: &std::path::Path, out_dir: &std::path::Path) {
    let icon = root.join("scripts").join("installer").join("cut.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut numbers: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    numbers.resize(4, 0);
    let dotted = numbers
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let rc = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {dotted}
PRODUCTVERSION {dotted}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040A04B0"
    BEGIN
      VALUE "CompanyName", "00faithlss"
      VALUE "FileDescription", "Cut"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "Cut"
      VALUE "OriginalFilename", "Cut.exe"
      VALUE "ProductName", "Cut"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x040A, 1200
  END
END
"#,
        icon = icon.to_string_lossy().replace('\\', "\\\\"),
    );
    let path = out_dir.join("cut.rc");
    fs::write(&path, rc).unwrap();
    if let Err(error) = embed_resource::compile(&path, embed_resource::NONE).manifest_optional() {
        println!("cargo:warning=No se pudo incrustar el icono de Cut: {error}");
    }
}
