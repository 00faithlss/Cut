//! Reusable GPUI components for the Cut design system.
use super::theme::{self, Theme, hex, px};
use gpui::{prelude::*, *};
use std::time::{Duration, Instant};

struct SwitchMotion {
    from: f32,
    to: f32,
    started: Instant,
    generation: u64,
}

impl SwitchMotion {
    fn value(&self, now: Instant) -> f32 {
        let progress = (now.duration_since(self.started).as_secs_f32() / 0.18).min(1.);
        self.from + (self.to - self.from) * ease_out(progress)
    }
}

fn tint(from: Hsla, to: Hsla, at: f32) -> Hsla {
    Hsla {
        h: from.h + (to.h - from.h) * at,
        s: from.s + (to.s - from.s) * at,
        l: from.l + (to.l - from.l) * at,
        a: from.a + (to.a - from.a) * at,
    }
}

/// Exponential ease-out used by every transition (`cubic-bezier(.16,1,.3,1)` feel).
pub fn ease_out(t: f32) -> f32 {
    1. - (1. - t).powi(5)
}

pub fn icon(name: &str, size_px: f32, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .w(px(size_px))
        .h(px(size_px))
        .flex_none()
        .text_color(color)
}

/// The Cut hammerhead, drawn as stacked single-tint layers.
pub fn mark(th: &Theme, size_px: f32) -> Div {
    let h = size_px * 224. / 256.;
    let mid = if th.light {
        hex(0x8F8F8A)
    } else {
        hex(0x6B6B66)
    };
    let layer = |name: &str, color: Hsla| {
        svg()
            .path(format!("icons/{name}.svg"))
            .absolute()
            .top_0()
            .left_0()
            .w(px(size_px))
            .h(px(h))
            .text_color(color)
    };
    div()
        .relative()
        .flex_none()
        .w(px(size_px))
        .h(px(h))
        .child(layer("mark_body", th.text))
        .child(layer("mark_shade", mid))
        .child(layer("mark_fin", th.text))
        .child(layer("mark_accent", th.accent))
        .child(layer("mark_eye", th.knob_on))
}

pub fn card(th: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(24.))
        .bg(th.card)
        .border_1()
        .border_color(th.card_border)
}

/// Soft backing for text that sits straight on the sea, so a passing shark never
/// competes with it: the window colour, nearly opaque, with a feathered edge.
pub fn halo(th: &Theme) -> Div {
    let color = theme::hexa(th.bg, 0.9);
    div().rounded(px(12.)).bg(color).shadow(vec![BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(14.),
        spread_radius: px(4.),
    }])
}

/// Wraps a block so its halo hugs the content instead of the whole column.
pub fn backed(th: &Theme, content: impl IntoElement) -> Div {
    div()
        .flex()
        .child(halo(th).px(px(10.)).py(px(4.)).ml(px(-10.)).child(content))
}

pub fn h1(th: &Theme, text: impl Into<SharedString>) -> Div {
    backed(
        th,
        div()
            .font_family(theme::sans())
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(40.))
            .line_height(relative(1.))
            .child(text.into()),
    )
}

pub fn h2(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(theme::sans())
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(19.))
        .child(text.into())
}

pub fn mono_text(text: impl Into<SharedString>, size_px: f32, color: Hsla) -> Div {
    div()
        .font_family(theme::mono())
        .text_size(px(size_px))
        .text_color(color)
        .child(text.into())
}

pub fn ghost(th: &Theme, id: impl Into<ElementId>) -> Stateful<Div> {
    let hover = th.ghost_hover;
    let focus = th.accent;
    div()
        .id(id)
        .tab_stop(true)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(34.))
        .px(px(12.))
        .rounded(px(11.))
        .bg(th.ghost_bg)
        .border_1()
        .border_color(th.ghost_border)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(th.text)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .focus(move |s| s.border_color(focus))
}

pub fn cta(th: &Theme, id: impl Into<ElementId>) -> Stateful<Div> {
    let hover = hex(0xFF6E2E);
    div()
        .id(id)
        .tab_stop(true)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .bg(th.accent)
        .text_color(th.on_accent)
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

pub fn switch(th: &Theme, id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    let th = *th;
    div()
        .id(id)
        .tab_stop(true)
        .relative()
        .flex_none()
        .w(px(36.))
        .h(px(22.))
        .rounded_full()
        .cursor_pointer()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let target = if on { 1. } else { 0. };
                    let motion = window.use_keyed_state("switch-motion", cx, |_, _| SwitchMotion {
                        from: target,
                        to: target,
                        started: Instant::now(),
                        generation: 0,
                    });
                    let value = motion.update(cx, |motion, cx| {
                        let now = Instant::now();
                        if motion.to != target {
                            motion.from = motion.value(now);
                            motion.to = target;
                            motion.started = now;
                            motion.generation += 1;
                            let generation = motion.generation;
                            cx.spawn(async move |this, cx| {
                                loop {
                                    cx.background_executor()
                                        .timer(Duration::from_millis(16))
                                        .await;
                                    let keep = this.update(cx, |motion, cx| {
                                        if motion.generation != generation {
                                            return false;
                                        }
                                        cx.notify();
                                        motion.started.elapsed() < Duration::from_millis(180)
                                    });
                                    if !matches!(keep, Ok(true)) {
                                        break;
                                    }
                                }
                            })
                            .detach();
                        }
                        motion.value(now)
                    });
                    window.paint_quad(
                        fill(bounds, tint(th.switch_off, th.chip_on, value)).corner_radii(px(11.)),
                    );
                    let knob = Bounds::new(
                        point(
                            bounds.origin.x + px(3. + 14. * value),
                            bounds.origin.y + px(3.),
                        ),
                        size(px(16.), px(16.)),
                    );
                    window.paint_quad(
                        fill(knob, tint(th.knob_off, th.knob_on, value)).corner_radii(px(8.)),
                    );
                },
            )
            .size_full(),
        )
}

#[derive(Clone, Copy, PartialEq)]
pub enum Cap {
    Plain,
    Talk,
    Active,
}

/// Visual keycap (bottom edge thicker, like a real key).
pub fn keycap(th: &Theme, text: impl Into<SharedString>, kind: Cap, enabled: bool) -> Div {
    let (border, bg, color) = match kind {
        Cap::Active => (th.accent, th.accent_soft, th.accent_text),
        Cap::Talk => (th.accent_edge, th.ghost_bg, th.accent_text),
        Cap::Plain => (th.cap_border, th.ghost_bg, th.text),
    };
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .min_w(px(96.))
        .h(px(44.))
        .px(px(16.))
        .rounded(px(13.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(color)
        .font_family(theme::mono())
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(13.))
        .opacity(if enabled { 1. } else { 0.4 })
        .child(text.into())
        .child(key_lip(border))
}

/// Small key label used inside the orange button and the conflict dialog.
pub fn kbd(th: &Theme, text: impl Into<SharedString>, on_accent: bool) -> Div {
    let (bg, border, color) = if on_accent {
        (hexa_dark(0.14), hexa_dark(0.2), th.on_accent)
    } else {
        (th.accent_soft, th.accent, th.accent_text)
    };
    div()
        .font_family(theme::mono())
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(11.))
        .relative()
        .px(px(9.))
        .py(px(6.))
        .rounded(px(9.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(color)
        .child(text.into())
        .child(key_lip(border))
}

/// Thicker bottom edge that makes a keycap read as a physical key.
fn key_lip(color: Hsla) -> Div {
    div()
        .absolute()
        .left(px(6.))
        .right(px(6.))
        .bottom(px(-1.))
        .h(px(2.))
        .rounded(px(2.))
        .bg(color)
}

fn hexa_dark(a: f32) -> Hsla {
    theme::hexa(0x0A0A0A, a)
}

pub fn gate_chip(th: &Theme, talking: bool, label: &'static str) -> Div {
    div()
        .px(px(8.))
        .py(px(3.))
        .rounded(px(7.))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .bg(if talking { th.accent } else { th.nav_on })
        .text_color(if talking { th.on_accent } else { th.secondary })
        .child(label)
}

pub fn divider(th: &Theme) -> Div {
    div().w_full().h(px(1.)).flex_none().bg(th.line)
}

/// A fixed alphabet shaped once, so animated ASCII can be painted glyph by glyph each
/// frame instead of reshaping whole lines of text (GPUI caches this layout between
/// frames because the string never changes).
pub struct Glyphs {
    /// Per character of the alphabet: the font and glyph to paint, if the font has one.
    pub glyphs: Vec<Option<(FontId, GlyphId)>>,
    /// Offset from the top of a cell of height `line` to the text baseline.
    pub baseline: Pixels,
    pub size: Pixels,
}

impl Glyphs {
    pub fn new(
        window: &mut Window,
        alphabet: &[char],
        face: Font,
        size: Pixels,
        line: Pixels,
    ) -> Self {
        let text: String = alphabet.iter().collect();
        let run = TextRun {
            len: text.len(),
            font: face,
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped =
            window
                .text_system()
                .shape_line(SharedString::from(text.clone()), size, &[run], None);
        let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        let mut glyphs = vec![None; alphabet.len()];
        for run in &shaped.runs {
            for glyph in &run.glyphs {
                if let Ok(i) = starts.binary_search(&glyph.index) {
                    glyphs[i] = Some((run.font_id, glyph.id));
                }
            }
        }
        let baseline = (line - shaped.ascent - shaped.descent) / 2. + shaped.ascent;
        Self {
            glyphs,
            baseline,
            size,
        }
    }

    /// Paint character `index` of the alphabet in the cell whose top-left is `origin`.
    pub fn paint(&self, window: &mut Window, index: usize, origin: Point<Pixels>, color: Hsla) {
        if let Some(Some((font, glyph))) = self.glyphs.get(index) {
            let _ = window.paint_glyph(
                point(origin.x, origin.y + self.baseline),
                *font,
                *glyph,
                self.size,
                color,
            );
        }
    }
}

/// One line of monospace glyphs spread across the available width.
pub fn ascii_row(cells: Vec<(char, Hsla)>, size_px: f32, height_px: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            if cells.is_empty() {
                return;
            }
            let width: f32 = bounds.size.width.into();
            let step = width / cells.len() as f32;
            let face = font(theme::mono());
            let mut text = String::new();
            let mut runs: Vec<TextRun> = Vec::new();
            for (ch, color) in &cells {
                text.push(*ch);
                let len = ch.len_utf8();
                match runs.last_mut() {
                    Some(run) if run.color == *color => run.len += len,
                    _ => runs.push(TextRun {
                        len,
                        font: face.clone(),
                        color: *color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }),
                }
            }
            let shaped = window.text_system().shape_line(
                SharedString::from(text),
                px(size_px),
                &runs,
                Some(gpui::px(step)),
            );
            let _ = shaped.paint(bounds.origin, px(height_px), window, cx);
        },
    )
    .w_full()
    .h(px(height_px))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn interruptor_invierte_sin_salto_y_termina_en_su_destino() {
        let start = Instant::now();
        let forward = SwitchMotion {
            from: 0.,
            to: 1.,
            started: start,
            generation: 0,
        };
        let middle = start + Duration::from_millis(50);
        let value = forward.value(middle);
        assert!(value > 0. && value < 1.);
        let reverse = SwitchMotion {
            from: value,
            to: 0.,
            started: middle,
            generation: 1,
        };
        assert_eq!(reverse.value(middle), value);
        assert_eq!(reverse.value(middle + Duration::from_millis(200)), 0.);
        assert_eq!(forward.value(start + Duration::from_millis(200)), 1.);
    }
}
