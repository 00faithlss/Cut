//! Background: top-down ASCII sharks (port of `docs/.../concepts/cut-ascii.html`).
//! A hammerhead and two makos swim an elliptical circuit; each occupied cell of a
//! character grid gets one glyph on a faint tile, so the bodies read as ASCII mosaics.
use super::theme::{self, SCALE, Theme, hex, hexa, px};
use super::widgets::Glyphs;
use gpui::{prelude::*, *};

const TAU: f64 = std::f64::consts::TAU;
const MARKS: [char; 10] = ['.', ':', ';', 't', 'o', 'X', 'S', '%', '8', '@'];

struct Fish {
    place: f64,
    length: f64,
    beat: f64,
    seed: f64,
    hammer: bool,
}
const FISH: [Fish; 3] = [
    Fish {
        place: 0.09,
        length: 0.42,
        beat: 0.72,
        seed: 7.,
        hammer: true,
    },
    Fish {
        place: 0.42,
        length: 0.32,
        beat: 0.91,
        seed: 31.,
        hammer: false,
    },
    Fish {
        place: 0.76,
        length: 0.37,
        beat: 0.84,
        seed: 53.,
        hammer: false,
    },
];

fn rng(x: f64, y: f64) -> f64 {
    let v = (x * 127.1 + y * 311.7).sin() * 43758.5453;
    v - v.floor()
}
fn sign(v: f64) -> f64 {
    if v > 0. {
        1.
    } else if v < 0. {
        -1.
    } else {
        0.
    }
}

struct Circuit {
    pts: Vec<(f64, f64, f64)>,
    perimeter: f64,
}
impl Circuit {
    fn new(w: f64, h: f64) -> Self {
        let mut pts: Vec<(f64, f64, f64)> = Vec::with_capacity(241);
        let mut perimeter = 0.;
        for i in 0..=240 {
            let a = TAU * i as f64 / 240.;
            let x = w * (0.5 + 0.39 * a.cos());
            let y = h * (0.49 + 0.35 * a.sin() + 0.034 * (2. * a).sin());
            if i > 0 {
                let p = pts[i - 1];
                perimeter += (x - p.0).hypot(y - p.1);
            }
            pts.push((x, y, perimeter));
        }
        Self { pts, perimeter }
    }
    /// (x, y, dx, dy) at a distance along the loop.
    fn point(&self, distance: f64) -> (f64, f64, f64, f64) {
        let p = self.perimeter;
        let d = ((distance % p) + p) % p;
        let (mut l, mut r) = (0usize, self.pts.len() - 1);
        while r - l > 1 {
            let m = (l + r) >> 1;
            if self.pts[m].2 <= d {
                l = m;
            } else {
                r = m;
            }
        }
        let (a, b) = (self.pts[l], self.pts[r]);
        let span = if b.2 - a.2 == 0. { 1. } else { b.2 - a.2 };
        let (dx, dy) = ((b.0 - a.0) / span, (b.1 - a.1) / span);
        (a.0 + (d - a.2) * dx, a.1 + (d - a.2) * dy, dx, dy)
    }
}

struct Seg {
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
}

fn spine(c: &Circuit, fish: &Fish, time: f64, length: f64) -> (Vec<Seg>, f64) {
    let lead = c.perimeter * (fish.place + time / 36. + 0.008 * (time * 0.18 + fish.seed).sin());
    let parts = 28usize;
    let step = length / parts as f64;
    let mut segs = Vec::with_capacity(parts);
    let mut prev: Option<(f64, f64)> = None;
    for i in 0..=parts {
        let u = i as f64 / parts as f64;
        let (px_, py_, dx, dy) = c.point(lead - u * length);
        let bend = length
            * (0.004 + 0.055 * u * u)
            * (TAU * (fish.beat * time - 0.86 * u) + fish.seed).sin();
        let (x, y) = (px_ - dy * bend, py_ + dx * bend);
        if let Some((qx, qy)) = prev {
            let (ddx, ddy) = (x - qx, y - qy);
            let span = ddx.hypot(ddy);
            let span = if span == 0. { 1. } else { span };
            segs.push(Seg {
                x: qx,
                y: qy,
                dx: ddx / span,
                dy: ddy / span,
            });
        }
        prev = Some((x, y));
    }
    (segs, step)
}

/// One grid cell: (pigment row, density column).
type Cell = Option<(u8, u8)>;

struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
}

fn rasterize(
    grid: &mut Grid,
    c: &Circuit,
    fish: &Fish,
    time: f64,
    dimensions: (f64, f64, f64, f64),
) {
    let (cw, ch, w, h) = dimensions;
    let length = (w * fish.length).max(175.);
    let (segs, step) = spine(c, fish, time, length);
    let pad = 0.28 * length;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for s in &segs {
        min_x = min_x.min(s.x);
        min_y = min_y.min(s.y);
        max_x = max_x.max(s.x);
        max_y = max_y.max(s.y);
    }
    let x0 = ((min_x - pad) / cw).floor().max(0.) as usize;
    let y0 = ((min_y - pad) / ch).floor().max(0.) as usize;
    let x1 = ((w / cw).ceil().min(((max_x + pad) / cw).ceil())).max(0.) as usize;
    let y1 = ((h / ch).ceil().min(((max_y + pad) / ch).ceil())).max(0.) as usize;
    let hammer = fish.hammer;
    let last = segs.len() - 1;
    for row in y0..y1.min(grid.rows) {
        for col in x0..x1.min(grid.cols) {
            let x = (col as f64 + 0.5) * cw;
            let y = (row as f64 + 0.5) * ch;
            let (mut best, mut u, mut v) = (f64::INFINITY, 0.0f64, 0.0f64);
            for (i, s) in segs.iter().enumerate() {
                let (ax, ay) = (x - s.x, y - s.y);
                let along = ax * s.dx + ay * s.dy;
                let near = along.clamp(0., step);
                let (ex, ey) = (ax - s.dx * near, ay - s.dy * near);
                let d2 = ex * ex + ey * ey;
                if d2 >= best {
                    continue;
                }
                best = d2;
                if (i == 0 && along < 0.) || (i == last && along > step) {
                    u = -1.;
                    continue;
                }
                u = (i as f64 * step + near) / length;
                v = sign(ay * s.dx - ax * s.dy) * d2.sqrt() / length;
            }
            if u <= 0. || u >= 1. {
                continue;
            }
            let edge = v.abs();
            let nose = if hammer { 0.065 } else { 0.14 };
            let body = (u / nose).min(1.)
                * 0.009f64.max(0.093 * (1. - 0.79 * u) * 1f64.min((0.85 - u) / 0.23));
            let head = hammer
                && u < 0.087
                && edge < 0.167
                && u > 0.009 + edge * 0.10
                && u < 0.063 + edge * 0.15;
            let reach = 0f64.max((u - 0.23) * 1.63);
            let pectoral = u > 0.23
                && u < 0.48
                && edge < 0.245f64.min(reach)
                && edge > 0f64.max((u - 0.35) * 2.14);
            let tail = u > 0.78 && edge < (u - 0.78) * 0.91 && edge > 0f64.max((u - 0.832) * 1.12);
            if !head && !pectoral && !tail && (edge > body || u > 0.835) {
                continue;
            }
            let eye = if hammer {
                head && edge > 0.142
            } else {
                u > 0.085 && u < 0.115 && edge > 0.046
            };
            let gill =
                u > 0.15 && u < 0.235 && edge > body * 0.47 && (230. * u + edge * 19.).sin() > 0.27;
            let fin = edge > body;
            let relief = if body > 0. { 1f64.min(edge / body) } else { 1. };
            let grain = rng((u * 58.).floor() + fish.seed, (v * 77.).floor());
            let pigment: i32 = if eye {
                6
            } else if gill {
                5
            } else if fin {
                3
            } else {
                4.min(
                    (relief * 3.25 + if v > 0. { 0.70 } else { 0. } + grain * 0.65).floor() as i32,
                )
            };
            let density: i32 = if eye {
                9
            } else if gill {
                2
            } else {
                1.max(9.min(
                    (7.5 - relief * 3. + grain * 3. - if fin { 1. } else { 0. }).floor() as i32,
                ))
            };
            grid.cells[row * grid.cols + col] = Some((pigment as u8, density as u8));
        }
    }
}

fn paint_sea(bounds: Bounds<Pixels>, t: f64, inks: [u32; 7], window: &mut Window) {
    // Drawn in design pixels; `px` scales them back to the screen.
    let w = f32::from(bounds.size.width) / SCALE;
    let h = f32::from(bounds.size.height) / SCALE;
    if w < 8. || h < 8. {
        return;
    }
    let narrow = w < 670.;
    let (cw, ch) = if narrow { (6f32, 8f32) } else { (8f32, 10f32) };
    let cols = (w / cw).ceil() as usize;
    let rows = (h / ch).ceil() as usize;
    let mut grid = Grid {
        cols,
        rows,
        cells: vec![None; cols * rows],
    };
    let circuit = Circuit::new(w as f64, h as f64);
    for fish in FISH.iter() {
        rasterize(
            &mut grid,
            &circuit,
            fish,
            t,
            (cw as f64, ch as f64, w as f64, h as f64),
        );
    }
    let mut face = font(theme::mono());
    face.weight = FontWeight::SEMIBOLD;
    let glyphs = Glyphs::new(window, &MARKS, face, px((ch * 0.92).round()), px(ch));
    let glyph_dx = if narrow { 0.5 } else { 1.0 };
    // Precomputed colours: one ink and one faint tile per pigment.
    let ink: Vec<Hsla> = inks.iter().map(|c| hex(*c)).collect();
    let tile: Vec<Hsla> = inks.iter().map(|c| hexa(*c, 0.17)).collect();
    let tile_size = size(px(cw - 1.), px(ch - 1.));
    for row in 0..rows {
        let y = bounds.origin.y + px(row as f32 * ch);
        for col in 0..cols {
            let Some((pigment, density)) = grid.cells[row * cols + col] else {
                continue;
            };
            let x = bounds.origin.x + px(col as f32 * cw);
            window.paint_quad(fill(
                Bounds::new(point(x, y), tile_size),
                tile[pigment as usize],
            ));
            glyphs.paint(
                window,
                density as usize,
                point(x + px(glyph_dx), y),
                ink[pigment as usize],
            );
        }
    }
}

/// Background entity: owns the clock so only this view repaints at ~24 FPS.
pub struct SeaView {
    t: f64,
    pub paused: bool,
    pub visible: bool,
    pub light: bool,
}

impl SeaView {
    pub fn new(paused: bool, light: bool) -> Self {
        Self {
            t: 0.,
            paused,
            visible: true,
            light,
        }
    }

    /// Move the sharks forward; false when nothing should be redrawn.
    pub fn advance(&mut self, dt: f64) -> bool {
        if self.paused || !self.visible {
            return false;
        }
        self.t += dt;
        true
    }
}

impl Render for SeaView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let th = if self.light {
            Theme::light()
        } else {
            Theme::dark()
        };
        let inks = th.shark_inks();
        let t = self.t;
        let c = if self.light { 0xEFEFEB } else { 0x0A0A0A };
        let edge = |a: f32| hexa(c, a);
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(hex(th.bg))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| paint_sea(bounds, t, inks, window),
                )
                .absolute()
                .size_full(),
            )
            // Vignette: linear fades on the four edges keep text readable.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(relative(0.18))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(edge(0.75), 0.),
                        linear_color_stop(edge(0.), 1.),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .h_full()
                    .w(relative(0.18))
                    .bg(linear_gradient(
                        270.,
                        linear_color_stop(edge(0.75), 0.),
                        linear_color_stop(edge(0.), 1.),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(relative(0.16))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(edge(0.70), 0.),
                        linear_color_stop(edge(0.), 1.),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .w_full()
                    .h(relative(0.16))
                    .bg(linear_gradient(
                        0.,
                        linear_color_stop(edge(0.70), 0.),
                        linear_color_stop(edge(0.), 1.),
                    )),
            )
    }
}
