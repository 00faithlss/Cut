//! Registers Bricolage Grotesque + Martian Mono (SIL OFL) with GPUI.
//! Falls back to Segoe UI / Consolas when the files were not downloaded yet.
use super::theme;
use gpui::App;
use std::borrow::Cow;

include!(concat!(env!("OUT_DIR"), "/embedded_fonts.rs"));

pub fn register(cx: &mut App) {
    if !EMBEDDED_FONTS.is_empty() {
        let fonts: Vec<Cow<'static, [u8]>> = EMBEDDED_FONTS
            .iter()
            .map(|bytes| Cow::Borrowed(*bytes))
            .collect();
        let _ = cx.text_system().add_fonts(fonts);
    }
    let installed = cx.text_system().all_font_names();
    let has = |name: &str| installed.iter().any(|n| n == name);
    let sans = if has("Bricolage Grotesque 14pt") {
        "Bricolage Grotesque 14pt"
    } else {
        "Segoe UI"
    };
    let mono = if has("Martian Mono") {
        "Martian Mono"
    } else {
        "Consolas"
    };
    theme::set_families(sans, mono);
}
