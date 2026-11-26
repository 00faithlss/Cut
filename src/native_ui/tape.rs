//! Niveles medidos mientras la interfaz está visible. La escala corresponde a la
//! duración del replay; los intervalos sin muestras quedan vacíos.
use super::theme::{self, SCALE, Theme, px};
use super::widgets::Glyphs;
use gpui::{prelude::*, *};
use std::collections::VecDeque;

const RAMP: [char; 8] = ['·', ':', ';', '+', 'x', 'X', '%', '#'];
const CW: f64 = 9.;
const CH: f64 = 13.;

pub struct TapeView {
    samples: VecDeque<(f64, f64)>,
    now: f64,
    available: u64,
    duration: u64,
    active: bool,
    level: f64,
    pub visible: bool,
    pub light: bool,
}

impl TapeView {
    pub fn new(_paused: bool, light: bool) -> Self {
        Self {
            samples: VecDeque::new(),
            now: 0.,
            available: 0,
            duration: 30,
            active: false,
            level: 0.,
            visible: true,
            light,
        }
    }

    pub fn observe(&mut self, available: u64, duration: u64, active: bool) {
        if available < self.available || duration != self.duration || available == 0 {
            self.samples.clear();
        }
        self.available = available.min(duration);
        self.duration = duration.max(1);
        self.active = active;
    }

    pub fn sample_level(&mut self, level: f64) {
        self.level = level.clamp(0., 1.);
        if self.active && self.available > 0 && self.visible {
            if let Some((_, peak)) = self
                .samples
                .back_mut()
                .filter(|(at, _)| self.now - *at < 0.25)
            {
                *peak = peak.max(self.level);
            } else {
                self.samples.push_back((self.now, self.level));
            }
            while self.samples.len() > 2401 {
                self.samples.pop_front();
            }
        }
    }

    /// Scroll the tape; false when nothing should be redrawn.
    pub fn advance(&mut self, dt: f64) -> bool {
        if !self.active {
            return false;
        }
        self.now += dt;
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| self.now - at > self.available as f64)
        {
            self.samples.pop_front();
        }
        self.visible
    }
}

fn paint_tape(
    bounds: Bounds<Pixels>,
    samples: VecDeque<(f64, f64)>,
    now: f64,
    duration: u64,
    th: Theme,
    window: &mut Window,
) {
    // Drawn in design pixels; `px` scales them back to the screen.
    let w: f64 = (f32::from(bounds.size.width) / SCALE) as f64;
    let h: f64 = (f32::from(bounds.size.height) / SCALE) as f64;
    if w < 20. || h < CH {
        return;
    }
    let rows = (h / CH).floor() as usize;
    let mid = (rows as f64 - 1.) / 2.;
    let head = w - 2.;
    let ox = bounds.origin.x;
    let oy = bounds.origin.y;
    // Marcas correspondientes a los cuartos de la duración elegida.
    for q in 0..4 {
        let x = ((w - 2.) * q as f64 / 4.).round() as f32;
        window.paint_quad(fill(
            Bounds::new(point(ox + px(x), oy), size(px(1.), bounds.size.height)),
            th.tape_tick,
        ));
    }
    // Visible columns, newest first.
    let total = (w / CW).ceil() as usize;
    let mut cols = vec![0f64; total];
    for (at, value) in samples {
        let bucket = (((now - at).max(0.) / duration as f64) * total as f64) as usize;
        if let Some(col) = cols.get_mut(bucket) {
            *col = col.max(value);
        }
    }
    let mut visible: Vec<(f64, f64)> = Vec::new(); // (amplitude, center x)
    for (i, a) in cols.into_iter().enumerate() {
        if a <= 0. {
            continue;
        }
        let x = head - i as f64 * CW - CW / 2.;
        if x < -CW {
            break;
        }
        visible.push((a, x));
    }
    if visible.is_empty() {
        return;
    }
    let mut face = font(theme::mono());
    face.weight = FontWeight::MEDIUM;
    let glyphs = Glyphs::new(window, &RAMP, face, px(10.), px(CH as f32));
    let accent = th.accent;
    let pad_y = (h - rows as f64 * CH) / 2.;
    for r in 0..rows {
        let d = (r as f64 - mid).abs();
        let y = oy + px((pad_y + r as f64 * CH) as f32);
        for (rank, (a, x)) in visible.iter().enumerate() {
            let reach = a * (mid + 0.5);
            if d > reach {
                continue;
            }
            let inten = 1. - d / (mid + 0.5);
            let lvl = ((a * 0.55 + inten * 0.45) * RAMP.len() as f64)
                .floor()
                .min(RAMP.len() as f64 - 1.) as usize;
            let age = (x / w).clamp(0., 1.);
            let hot = rank < 3 && *a > 0.72 && d < 1.5;
            let k = inten * (0.35 + 0.65 * age);
            let color = if hot {
                accent
            } else {
                let level = if th.light {
                    185. - 150. * k
                } else {
                    70. + 150. * k
                };
                let g = level.round() as f32 / 255.;
                let alpha = (0.25 + 0.75 * age) as f32;
                Rgba {
                    r: g,
                    g,
                    b: (g - 3. / 255.).max(0.),
                    a: alpha,
                }
                .into()
            };
            glyphs.paint(window, lvl, point(ox + px((x - CW / 2.) as f32), y), color);
        }
    }
    window.paint_quad(fill(
        Bounds::new(
            point(ox + px(head as f32), oy),
            size(px(2.), bounds.size.height),
        ),
        accent,
    ));
}

impl Render for TapeView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let th = if self.light {
            Theme::light()
        } else {
            Theme::dark()
        };
        let samples = self.samples.clone();
        let (now, duration) = (self.now, self.duration);
        div().size_full().overflow_hidden().child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| paint_tape(bounds, samples, now, duration, th, window),
            )
            .size_full(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn cinta_sin_datos_no_inventa_audio_y_limpia_un_replay_nuevo() {
        let mut tape = TapeView::new(false, false);
        assert!(tape.samples.is_empty());
        tape.observe(2, 600, true);
        tape.sample_level(0.6);
        assert_eq!(tape.samples.front().unwrap().1, 0.6);
        tape.visible = false;
        assert!(!tape.advance(3.));
        assert!(tape.samples.is_empty());
        tape.visible = true;
        tape.observe(1, 600, true);
        tape.sample_level(0.4);
        tape.observe(0, 600, false);
        assert!(tape.samples.is_empty());
    }

    #[test]
    fn cinta_acota_memoria_y_conserva_picos_reales() {
        let mut tape = TapeView::new(false, false);
        tape.observe(600, 600, true);
        for _ in 0..10000 {
            tape.sample_level(0.5);
            tape.advance(0.12);
        }
        assert!(tape.samples.len() <= 2401);
        assert!(tape.samples.iter().all(|(_, level)| *level == 0.5));
    }
}
