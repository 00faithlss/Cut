//! "Captura" tab: source picker, mixer, push-to-talk status, replay tape and the save button.
use super::theme::px;
use super::{
    Studio, Tab,
    theme::{self, Theme},
    widgets::{
        self, Cap, ascii_row, card, cta, divider, ghost, h1, h2, icon, kbd, keycap, mono_text,
        switch,
    },
};
use crate::controller::Command as C;
use gpui::{prelude::*, *};
use std::time::{Duration, Instant};

const RAMP: [char; 8] = ['·', ':', ';', '+', 'x', 'X', '%', '#'];
const METER_CELLS: usize = 56;

fn rng(x: f64, y: f64) -> f64 {
    let v = (x * 127.1 + y * 311.7).sin() * 43758.5453;
    v - v.floor()
}

/// "−1:30"-style tick label for a point `back` seconds before now.
fn tick(back: u64) -> String {
    format!("−{}:{:02}", back / 60, back % 60)
}

fn clock(seconds: u64) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

impl Studio {
    /// Switch between whole-monitor and single-window capture.
    pub(super) fn pick_kind(&mut self, monitor: bool) {
        let sources = if monitor {
            &self.state.monitors
        } else {
            &self.state.windows
        };
        let Some(first) = sources.first().cloned() else {
            self.controller.send(C::Error(
                "No hay fuentes disponibles. Actualiza la lista.".into(),
            ));
            return;
        };
        let keep = sources.iter().any(|s| s.id == self.state.profile.id)
            && self.state.profile.monitor == monitor;
        self.set_profile(|p| {
            p.monitor = monitor;
            if !keep {
                p.id = first.id.clone();
            }
        });
    }

    pub(super) fn refresh_sources(&mut self) {
        self.refreshing = Some(Instant::now());
        self.controller.send(C::Sources);
    }

    fn source_menu(&mut self, th: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let monitor = self.state.profile.monitor;
        let item =
            |id: &'static str, ic: &'static str, title: &'static str, sub: String, on: bool| {
                let hover = th.nav_on;
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .w_full()
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(11.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(icon(ic, 18., th.secondary))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(mono_text(sub, 11., th.muted).truncate()),
                    )
                    .child(icon("check", 16., th.accent).opacity(if on { 1. } else { 0. }))
            };
        // The chosen source when that kind is active, otherwise the one picking it would use.
        let current = &self.state.profile.id;
        let sub = |list: &[crate::controller::Source], empty: &str| {
            list.iter()
                .find(|s| &s.id == current)
                .or_else(|| list.first())
                .map_or_else(|| empty.to_string(), |s| s.label.clone())
        };
        let mon_sub = sub(&self.state.monitors, "Sin pantallas detectadas");
        let win_sub = sub(&self.state.windows, "Sin ventanas detectadas");
        deferred(
            div()
                .absolute()
                .top(px(46.))
                .left_0()
                .w(px(300.))
                .p(px(6.))
                .rounded(px(16.))
                .bg(th.menu)
                .border_1()
                .border_color(th.ghost_border)
                .shadow_lg()
                .occlude()
                .child(
                    item(
                        "menu-monitor",
                        "monitor",
                        "Pantalla completa",
                        mon_sub,
                        monitor,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pick_kind(true);
                        this.menu = false;
                        cx.notify();
                    })),
                )
                .child(
                    item(
                        "menu-window",
                        "window",
                        "Ventana del juego",
                        win_sub,
                        !monitor,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pick_kind(false);
                        this.menu = false;
                        cx.notify();
                    })),
                )
                .child(div().mx(px(8.)).my(px(6.)).child(divider(th)))
                .child(
                    div()
                        .id("menu-refresh")
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(11.))
                        .text_size(px(13.))
                        .text_color(th.secondary)
                        .cursor_pointer()
                        .hover({
                            let hover = th.nav_on;
                            move |s| s.bg(hover)
                        })
                        .child(icon("refresh", 16., th.secondary))
                        .child("Actualizar fuentes")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_sources();
                            this.menu = false;
                            cx.notify();
                        })),
                ),
        )
        .with_priority(10)
    }

    fn mixer(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let sources = self.sources();
        let talking = self.talking();
        let ptt = self.state.settings.push_to_talk.is_some();
        let included = sources.iter().filter(|s| self.gain(&s.id).include).count();
        let spinning = self.refreshing.is_some();
        let frame = self.frame as f64;
        let mut rows = div().flex().flex_col();
        for (i, source) in sources.iter().enumerate() {
            let gain = self.gain(&source.id);
            let on = gain.include;
            let is_mic = source.id == "mic";
            let lower = source.label.to_lowercase();
            let ic = if is_mic {
                "mic"
            } else if lower.contains("discord") {
                "discord"
            } else if lower.contains("spotify") {
                "spotify"
            } else {
                "game"
            };
            let gated = is_mic && ptt && !talking;
            let slider_key = self.track_slider(&source.id);
            let volume = self.slider_value(slider_key);
            let level = if on {
                (self
                    .state
                    .levels
                    .get(&source.id)
                    .copied()
                    .unwrap_or(0.)
                    .clamp(0., 1.)
                    * (volume as f64 / 100.))
                    .min(1.)
            } else {
                0.
            };
            let cells: Vec<(char, Hsla)> = (0..METER_CELLS)
                .map(|k| {
                    let phase = k as f64 * 0.31 + i as f64 * 2.6 + frame * 0.55;
                    let shape = ((0.08
                        + 0.92
                            * phase.sin().abs().powi(3)
                            * (0.35 + 0.65 * rng(k as f64 + frame, i as f64)))
                        * level)
                        .min(1.);
                    let idx = ((shape * RAMP.len() as f64) as usize).min(RAMP.len() - 1);
                    let hot = !gated && on && shape > 0.82;
                    let color = if !on {
                        th.meter_off
                    } else if gated {
                        th.meter_lo
                    } else if hot {
                        th.accent
                    } else if shape > 0.45 {
                        th.meter_hi
                    } else if shape > 0.18 {
                        th.meter_mid
                    } else {
                        th.meter_lo
                    };
                    (RAMP[idx], color)
                })
                .collect();
            let icon_color = if !on {
                th.faint
            } else if is_mic && talking {
                th.accent
            } else {
                th.meter_hi
            };
            let id = source.id.clone();
            let slider = self.slider(th, slider_key, 100., on, cx);
            rows = rows.child(
                div()
                    .flex()
                    .flex_col()
                    .mx(px(-12.))
                    .px(px(12.))
                    .pt(px(14.))
                    .pb(px(12.))
                    .rounded(px(14.))
                    .border_t_1()
                    .border_color(th.line)
                    .opacity(if on { 1. } else { 0.5 })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .child(switch(th, ("track-switch", i), on).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.set_gain(&id, |g| g.include = !g.include);
                                    cx.notify();
                                },
                            )))
                            .child(icon(ic, 20., icon_color))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .truncate()
                                            .child(source.label.clone()),
                                    )
                                    .when(is_mic && ptt, |d| {
                                        d.child(widgets::gate_chip(
                                            th,
                                            talking,
                                            if talking { "Hablando" } else { "Silenciado" },
                                        ))
                                    }),
                            )
                            .child(slider)
                            .child(
                                div()
                                    .w(px(40.))
                                    .flex_none()
                                    .text_right()
                                    .font_family(theme::mono())
                                    .text_size(px(12.))
                                    .text_color(if on { th.text } else { th.muted })
                                    .child(format!("{volume}%")),
                            ),
                    )
                    .child(div().mt(px(10.)).child(ascii_row(cells, 11., 14.)))
                    .when_some(self.state.notices.get(&source.id).cloned(), |d, notice| {
                        d.child(
                            div()
                                .mt(px(8.))
                                .text_size(px(12.))
                                .line_height(relative(1.4))
                                .text_color(th.warn_text)
                                .child(notice),
                        )
                    }),
            );
        }
        card(th)
            .px(px(28.))
            .pt(px(26.))
            .pb(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(12.))
                            .child(h2("Mezclador"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(th.muted)
                                    .child(format!("{included} de {} en el clip", sources.len())),
                            ),
                    )
                    .child(
                        ghost(th, "mixer-refresh")
                            .child(if spinning {
                                icon("refresh", 15., th.text)
                                    .with_animation(
                                        "spin",
                                        Animation::new(Duration::from_millis(800)).repeat(),
                                        |svg, d| {
                                            svg.with_transformation(Transformation::rotate(
                                                percentage(d),
                                            ))
                                        },
                                    )
                                    .into_any_element()
                            } else {
                                icon("refresh", 15., th.text).into_any_element()
                            })
                            .child("Buscar pistas")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_sources();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .mt(px(22.))
                    .pb(px(8.))
                    .text_size(px(12.))
                    .text_color(th.muted)
                    .child("Incluir en el clip")
                    .child("Volumen"),
            )
            .child(rows)
    }

    fn ptt_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let talking = self.talking();
        let binding = self.state.settings.push_to_talk;
        let (label, desc): (SharedString, &str) = match binding {
            Some(b) => (
                b.label().into(),
                "Tu voz entra al clip mientras mantienes las teclas o botones asignados.",
            ),
            None => (
                "Sin tecla".into(),
                "Tu micrófono está siempre abierto. Asigna un atajo en Ajustes.",
            ),
        };
        let mic = self.gain("mic");
        let gate = match (binding.is_some(), talking) {
            _ if !mic.include || mic.volume == 0 || !self.state.active => "Silenciado",
            (false, _) => "Siempre abierto",
            (true, true) => "Hablando",
            (true, false) => "Silenciado",
        };
        card(th)
            .flex_col()
            .items_start()
            .gap(px(12.))
            .px(px(20.))
            .py(px(18.))
            .when(talking && binding.is_some(), |d| {
                d.border_color(theme::hexa(theme::ACCENT, 0.6))
            })
            .child(
                keycap(
                    th,
                    label,
                    if talking && binding.is_some() {
                        Cap::Active
                    } else {
                        Cap::Plain
                    },
                    binding.is_some(),
                )
                .min_w(px(0.))
                .w_full()
                .h_auto()
                .min_h(px(44.))
                .py(px(10.))
                .px(px(12.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Pulsar para hablar"),
                            )
                            .child(widgets::gate_chip(th, talking && binding.is_some(), gate)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(relative(1.4))
                            .text_color(th.muted)
                            .child(desc),
                    ),
            )
            .when(binding.is_some(), |d| {
                d.child(
                    ghost(th, "ptt-test")
                        .h(px(36.))
                        .px(px(14.))
                        .child("Mantén para probar")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.test_talk = true;
                                crate::hotkey::set_talk_preview(true);
                                cx.notify();
                            }),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.stop_talk_preview();
                                cx.notify();
                            }),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.stop_talk_preview();
                                cx.notify();
                            }),
                        ),
                )
            })
    }

    fn replay_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let seconds = self.state.profile.seconds.max(4);
        let labels = [
            (seconds, 0.),
            (seconds * 3 / 4, 0.25),
            (seconds / 2, 0.5),
            (seconds / 4, 0.75),
        ];
        let mut ticks = div()
            .relative()
            .h(px(16.))
            .mt(px(10.))
            .text_size(px(11.))
            .font_family(theme::mono())
            .text_color(th.muted);
        for (back, at) in labels {
            ticks = ticks.child(
                div()
                    .absolute()
                    .top_0()
                    .left(relative(at))
                    .child(tick(back)),
            );
        }
        ticks = ticks.child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .text_color(th.accent_text)
                .child("ahora"),
        );
        card(th)
            .p(px(24.))
            .child({
                // Room for the tape; `Shell` draws it here on its own layer. A changed
                // position asks for one full redraw so the layer follows.
                let slot = self.tape_slot.clone();
                div().relative().h(px(132.)).mx(px(-2.)).child(
                    canvas(
                        move |bounds, window, _| {
                            if slot.get() != Some(bounds) {
                                slot.set(Some(bounds));
                                window.refresh();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .child(ticks)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .justify_between()
                    .gap(px(16.))
                    .mt(px(22.))
                    .pt(px(20.))
                    .border_t_1()
                    .border_color(th.line)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(th.secondary)
                                    .child(format!(
                                        "Disponible · {:.0} MB de video",
                                        self.state.buffer_bytes as f64 / 1_000_000.
                                    )),
                            )
                            .child(
                                div()
                                    .id("replay-time")
                                    .cursor_pointer()
                                    .font_family(theme::mono())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(60.))
                                    .line_height(relative(0.95))
                                    .child(clock(
                                        self.state.seconds.min(self.state.profile.seconds),
                                    ))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.set_tab(Tab::Clips, cx)),
                                    ),
                            ),
                    )
                    .child(
                        cta(th, "save-clip")
                            .h(px(52.))
                            .pl(px(18.))
                            .pr(px(10.))
                            .rounded(px(16.))
                            .text_size(px(15.))
                            .child(icon("save", 19., th.on_accent))
                            .child("Guardar clip")
                            .child(kbd(th, self.state.hotkey.label(), true))
                            .opacity(if self.state.seconds > 0 { 1. } else { 0.45 })
                            .on_click(cx.listener(|this, _, _, _| {
                                if this.state.seconds > 0 {
                                    this.save_clip();
                                }
                            })),
                    ),
            )
    }

    pub(super) fn capture(&mut self, th: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let monitor = self.state.profile.monitor;
        let p = self.state.profile.clone();
        let source_row = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                ghost(th, "source-button")
                    .h(px(38.))
                    .rounded(px(12.))
                    .text_size(px(13.))
                    .child(icon(
                        if monitor { "monitor" } else { "window" },
                        16.,
                        th.text,
                    ))
                    .child(if monitor {
                        "Pantalla completa"
                    } else {
                        "Ventana del juego"
                    })
                    .child(icon("chev", 15., th.muted))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = !this.menu;
                        cx.notify();
                    })),
            )
            .child(mono_text(
                format!(
                    "{}p · {} FPS · H.264 {}",
                    p.height,
                    p.fps,
                    if self.state.software_encoding {
                        "CPU"
                    } else {
                        "GPU"
                    }
                ),
                12.,
                th.secondary,
            ))
            .child(
                div()
                    .id("edit-profile")
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(th.muted)
                    .cursor_pointer()
                    .hover({
                        let c = th.text;
                        move |s| s.text_color(c)
                    })
                    .child("Editar")
                    .child(icon("chevr", 13., th.muted))
                    .on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Clips, cx))),
            )
            .when(self.menu, |d| d.child(self.source_menu(th, cx)));

        let left = div()
            .flex()
            .flex_col()
            .flex_grow()
            .flex_shrink()
            .flex_basis(px(520.))
            .min_w(px(420.))
            .max_w(px(640.))
            .gap(px(28.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(h1(th, "Captura"))
                    .child(widgets::backed(th, source_row).mt(px(6.))),
            )
            .child(self.mixer(th, cx));

        let right = div()
            .flex()
            .flex_col()
            .flex_grow()
            .flex_shrink()
            .flex_basis(px(340.))
            .min_w(px(340.))
            .justify_end()
            .gap(px(16.))
            .pt(px(24.))
            .child(self.ptt_card(th, cx))
            .child(self.replay_card(th, cx));

        vec![
            div()
                .flex()
                .gap(px(40.))
                .w_full()
                .h_full()
                .child(left)
                .child(right)
                .into_any_element(),
        ]
    }
}
