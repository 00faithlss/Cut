//! Design tokens for Cut. Two grayscale tones plus one signal orange.
//! Values mirror the approved redesign (dark and light).
use gpui::{Hsla, rgb};
use std::sync::OnceLock;

pub const ACCENT: u32 = 0xFF5B14;

/// Size of the whole interface relative to the design's CSS pixels.
pub const SCALE: f32 = 0.9;

/// Design pixels to screen pixels. Every UI module imports this instead of
/// `gpui::px`. Hairlines (2px or less) keep their width so they do not fade out.
pub fn px(value: f32) -> gpui::Pixels {
    gpui::px(if value.abs() <= 2. {
        value
    } else {
        value * SCALE
    })
}

static FAMILIES: OnceLock<(&'static str, &'static str)> = OnceLock::new();

/// Called once after the bundled fonts were registered (or not).
pub fn set_families(sans: &'static str, mono: &'static str) {
    let _ = FAMILIES.set((sans, mono));
}
/// UI family: Bricolage Grotesque, or Segoe UI when the font files are missing.
pub fn sans() -> &'static str {
    FAMILIES.get().map_or("Segoe UI", |f| f.0)
}
/// Numeric/monospace family: Martian Mono, or Consolas when missing.
pub fn mono() -> &'static str {
    FAMILIES.get().map_or("Consolas", |f| f.1)
}

pub fn hex(c: u32) -> Hsla {
    rgb(c).into()
}
pub fn hexa(c: u32, a: f32) -> Hsla {
    let mut h = hex(c);
    h.a = a;
    h
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub light: bool,
    /// Window background.
    pub bg: u32,
    /// Card surface (barely translucent: text must win over the sharks).
    pub card: Hsla,
    pub card_border: Hsla,
    pub text: Hsla,
    pub secondary: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    /// Text drawn on top of the orange.
    pub on_accent: Hsla,
    /// Orange used for text (AA contrast on the surface).
    pub accent_text: Hsla,
    pub accent_soft: Hsla,
    pub accent_edge: Hsla,
    pub ghost_bg: Hsla,
    pub ghost_border: Hsla,
    pub ghost_hover: Hsla,
    /// Inset field behind segmented controls and tiles.
    pub field: Hsla,
    pub field_border: Hsla,
    pub line: Hsla,
    pub hover: Hsla,
    pub nav_on: Hsla,
    /// Selected segment / switch on.
    pub chip_on: Hsla,
    pub chip_on_text: Hsla,
    pub switch_off: Hsla,
    pub knob_off: Hsla,
    pub knob_on: Hsla,
    pub sheet: Hsla,
    pub menu: Hsla,
    pub scrim: Hsla,
    pub modal_scrim: Hsla,
    pub cap_border: Hsla,
    pub cap_edge: Hsla,
    pub tile_off: Hsla,
    pub close_hover: Hsla,
    pub warn_text: Hsla,
    /// ASCII meter ramp.
    pub meter_hi: Hsla,
    pub meter_mid: Hsla,
    pub meter_lo: Hsla,
    pub meter_off: Hsla,
    /// Replay-tape grid lines.
    pub tape_tick: Hsla,
    /// Volume tiles in Ajustes.
    pub tile_mid: Hsla,
    /// Disk-estimate tile ramp (cool → hot).
    pub est_a: Hsla,
    pub est_b: Hsla,
    pub est_c: Hsla,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            light: false,
            bg: 0x0A0A0A,
            card: hexa(0x10100F, 0.94),
            card_border: hexa(0xFFFFFF, 0.07),
            text: hex(0xEDEDEA),
            secondary: hex(0xA3A39F),
            muted: hex(0x8F8F8A),
            faint: hex(0x5A5A56),
            accent: hex(ACCENT),
            on_accent: hex(0x0A0A0B),
            accent_text: hex(0xFF8A4C),
            accent_soft: hexa(ACCENT, 0.10),
            accent_edge: hexa(ACCENT, 0.35),
            ghost_bg: hexa(0xFFFFFF, 0.04),
            ghost_border: hexa(0xFFFFFF, 0.09),
            ghost_hover: hexa(0xFFFFFF, 0.07),
            field: hexa(0x000000, 0.35),
            field_border: hexa(0xFFFFFF, 0.06),
            line: hexa(0xFFFFFF, 0.06),
            hover: hexa(0xFFFFFF, 0.04),
            nav_on: hexa(0xFFFFFF, 0.06),
            chip_on: hex(0xEDEDEA),
            chip_on_text: hex(0x0A0A0A),
            switch_off: hexa(0xFFFFFF, 0.10),
            knob_off: hex(0x8F8F8A),
            knob_on: hex(0x0A0A0A),
            sheet: hex(0x121211),
            menu: hex(0x151514),
            scrim: hexa(0x060606, 0.5),
            modal_scrim: hexa(0x050505, 0.72),
            cap_border: hexa(0xFFFFFF, 0.14),
            cap_edge: hexa(0xFFFFFF, 0.18),
            tile_off: hexa(0xFFFFFF, 0.06),
            close_hover: hex(0xC4261D),
            warn_text: hex(0xFFA071),
            meter_hi: hex(0xCFCFCB),
            meter_mid: hex(0x8F8F8A),
            meter_lo: hex(0x3A3A37),
            meter_off: hex(0x2A2A28),
            tape_tick: hexa(0xEDEDEA, 0.06),
            tile_mid: hexa(0xFFFFFF, 0.25),
            est_a: hex(0xFF8A4C),
            est_b: hex(0xFF5B14),
            est_c: hex(0xD9480F),
        }
    }

    pub fn light() -> Self {
        Self {
            light: true,
            bg: 0xEFEFEB,
            card: hexa(0xFFFFFF, 0.93),
            card_border: hexa(0x000000, 0.07),
            text: hex(0x141413),
            secondary: hex(0x54544F),
            muted: hex(0x63635D),
            faint: hex(0x9A9A94),
            accent: hex(ACCENT),
            on_accent: hex(0x121211),
            accent_text: hex(0xC2410C),
            accent_soft: hexa(ACCENT, 0.10),
            accent_edge: hexa(ACCENT, 0.45),
            ghost_bg: hexa(0x000000, 0.04),
            ghost_border: hexa(0x000000, 0.09),
            ghost_hover: hexa(0x000000, 0.07),
            field: hexa(0x1C1C18, 0.09),
            field_border: hexa(0x000000, 0.06),
            line: hexa(0x000000, 0.07),
            hover: hexa(0x000000, 0.04),
            nav_on: hexa(0x000000, 0.06),
            chip_on: hex(0x141413),
            chip_on_text: hex(0xF7F7F4),
            switch_off: hexa(0x000000, 0.12),
            knob_off: hex(0x63635D),
            knob_on: hex(0xF7F7F4),
            sheet: hex(0xFFFFFF),
            menu: hex(0xFFFFFF),
            scrim: hexa(0x1C1C18, 0.20),
            modal_scrim: hexa(0x1C1C18, 0.28),
            cap_border: hexa(0x000000, 0.14),
            cap_edge: hexa(0x000000, 0.20),
            tile_off: hexa(0x000000, 0.07),
            close_hover: hex(0xC4261D),
            warn_text: hex(0xB4430D),
            meter_hi: hex(0x2E2E2B),
            meter_mid: hex(0x63635D),
            meter_lo: hex(0xC6C6C0),
            meter_off: hex(0xD9D9D3),
            tape_tick: hexa(0x141412, 0.08),
            tile_mid: hexa(0x000000, 0.20),
            est_a: hex(0xFF8A4C),
            est_b: hex(0xFF5B14),
            est_c: hex(0xD9480F),
        }
    }

    /// Ink ramp for the ASCII sharks (bright → dim) plus the accent as last entry.
    pub fn shark_inks(&self) -> [u32; 7] {
        if self.light {
            [
                0x1F1F1D, 0x3D3D3A, 0x5A5A56, 0x76766F, 0x8F8F88, 0xA6A6A0, ACCENT,
            ]
        } else {
            [
                0xEBEBEA, 0xC8C8C5, 0xA8A8A4, 0x8B8B87, 0x727270, 0x5F5F5C, ACCENT,
            ]
        }
    }
}
