//! Embedded SVG icons (assets/icons/*.svg) served to GPUI's `svg()` element.
use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &str)] = &[
            $((concat!("icons/", $name, ".svg"), include_str!(concat!("../../assets/icons/", $name, ".svg")))),*
        ];
    };
}

icons!(
    "alert",
    "arrow",
    "check",
    "chev",
    "chevr",
    "clock",
    "cursor",
    "discord",
    "film",
    "folder",
    "game",
    "info",
    "key",
    "mark_accent",
    "mark_body",
    "mark_eye",
    "mark_fin",
    "mark_shade",
    "mic",
    "minus",
    "monitor",
    "moon",
    "pause",
    "play",
    "refresh",
    "save",
    "sliders",
    "spark",
    "spotify",
    "square",
    "sun",
    "swap",
    "wave",
    "window",
    "x",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, svg)| Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
