use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::audio::{OUTPUT_AUDIO_PREFIX, Settings};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MixerCache {
    pub settings: Settings,
}

impl MixerCache {
    pub fn path() -> PathBuf {
        if std::env::var_os("CUT_NATIVE_VALIDATION").is_some() {
            return std::env::temp_dir()
                .join("CutNativeValidation")
                .join("mezclador.json");
        }
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("CutClipper")
            .join("mezclador.json")
    }

    pub fn load(path: &Path) -> Option<Self> {
        let mut cache: Self = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        cache.settings.tracks.retain(|id, _| {
            id == "mic"
                || id == "system"
                || id.starts_with("app:")
                || id.starts_with(OUTPUT_AUDIO_PREFIX)
        });
        Some(cache)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Gain;

    #[test]
    fn migra_preferencias_del_mezclador_anterior() -> io::Result<()> {
        let path =
            std::env::temp_dir().join(format!("cut-mezclador-prueba-{}.json", std::process::id()));
        let mut cache = MixerCache::default();
        cache.save(&path)?;
        cache.settings.noise_suppression = false;
        cache.settings.tracks.insert(
            "app:vlc".into(),
            Gain {
                include: false,
                volume: 72,
            },
        );
        let mut legacy = serde_json::to_value(&cache)?;
        legacy["apps"] = serde_json::json!([{ "id": "app:vlc", "label": "VLC" }]);
        fs::write(&path, serde_json::to_vec(&legacy)?)?;
        let restored = MixerCache::load(&path).unwrap();
        assert_eq!(restored.settings, cache.settings);
        restored.save(&path)?;
        assert!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path)?)?
                .get("apps")
                .is_none()
        );
        fs::remove_file(path)
    }
}
