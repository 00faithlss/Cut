use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::audio::{Settings, Source};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MixerCache {
    pub apps: Vec<Source>,
    pub settings: Settings,
}

impl MixerCache {
    pub fn path() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("CutClipper")
            .join("mezclador.json")
    }

    pub fn load(path: &Path) -> Option<Self> {
        let mut cache: Self = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        cache.apps.retain(|app| app.id.starts_with("app:"));
        cache.apps.sort_by(|a, b| a.id.cmp(&b.id));
        cache.apps.dedup_by(|a, b| a.id == b.id);
        cache
            .settings
            .tracks
            .retain(|id, _| id == "mic" || id.starts_with("app:"));
        Some(cache)
    }

    pub fn add_apps(&mut self, apps: impl Iterator<Item = Source>) -> bool {
        let mut changed = false;
        for app in apps {
            match self.apps.binary_search_by(|known| known.id.cmp(&app.id)) {
                Ok(index) if self.apps[index] != app => {
                    self.apps[index] = app;
                    changed = true;
                }
                Ok(_) => {}
                Err(index) => {
                    self.apps.insert(index, app);
                    changed = true;
                }
            }
        }
        changed
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
    fn restaura_aplicaciones_y_volumen_sin_duplicarlas() -> io::Result<()> {
        let path =
            std::env::temp_dir().join(format!("cut-mezclador-prueba-{}.json", std::process::id()));
        let app = Source {
            id: "app:vlc".into(),
            label: "VLC".into(),
        };
        let mut cache = MixerCache::default();
        assert!(cache.add_apps([app.clone()].into_iter()));
        assert!(!cache.add_apps([app.clone()].into_iter()));
        cache.save(&path)?;
        cache.settings.noise_suppression = false;
        cache.settings.tracks.insert(
            app.id.clone(),
            Gain {
                include: false,
                volume: 72,
            },
        );
        cache.save(&path)?;
        let restored = MixerCache::load(&path).unwrap();
        assert_eq!(restored.apps, vec![app]);
        assert_eq!(restored.settings, cache.settings);
        fs::remove_file(path)
    }
}
