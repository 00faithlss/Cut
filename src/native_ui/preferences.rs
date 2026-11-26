use serde::{Deserialize, Serialize};

/// Appearance and behaviour preferences stored in `apariencia.json`.
#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    /// Used when `system` is false.
    pub light: bool,
    /// Follow the Windows app theme instead of `light`.
    pub system: bool,
    /// Sharks paused (the "Tiburones animados" switch is the inverse).
    pub paused: bool,
    pub seen_version: String,
    /// Short tone when a clip is saved. Nothing is ever shown on screen.
    pub sound: bool,
    /// Close button minimizes to the taskbar instead of hiding to the tray.
    pub minimize_on_close: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            light: false,
            system: false,
            paused: false,
            seen_version: String::new(),
            sound: true,
            minimize_on_close: false,
        }
    }
}

impl Preferences {
    /// Las novedades pendientes solo se reconocen cuando se cierran explícitamente.
    pub fn needs_news(&self, returning: bool, current: &str) -> bool {
        if self.seen_version.is_empty() {
            returning
        } else {
            self.seen_version != current
        }
    }
    fn path() -> std::path::PathBuf {
        crate::mixer_cache::MixerCache::path().with_file_name("apariencia.json")
    }
    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temp, path)
    }
    /// Whether the interface should use the light theme right now.
    pub fn is_light(&self) -> bool {
        if self.system {
            system_prefers_light()
        } else {
            self.light
        }
    }
}

/// Reads `AppsUseLightTheme` from the registry (Windows 10 1809+ and 11).
pub fn system_prefers_light() -> bool {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|value| value != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn novedades_no_se_pierden_por_reiniciar_sin_reconocer() {
        let mut prefs = Preferences {
            seen_version: "0.2.3".into(),
            ..Default::default()
        };
        assert!(prefs.needs_news(true, "0.3.0"));
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert!(restored.needs_news(true, "0.3.0"));
        prefs.seen_version = "0.3.0".into();
        assert!(!prefs.needs_news(true, "0.3.0"));
        assert!(!Preferences::default().needs_news(false, "0.3.0"));
    }
}
