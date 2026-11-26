//! "Ajustes" tab: shortcuts, global microphone, behaviour, appearance and about.
use super::theme::px;
use super::{
    Select, Slider, Studio,
    theme::{self, Theme},
    widgets::{Cap, backed, card, ghost, h1, h2, icon, keycap, mark, switch},
};
use crate::controller::Command as C;
use gpui::{prelude::*, *};
use std::time::Duration;

const SECTIONS: [(&str, usize); 5] = [
    ("Atajos", 1),
    ("Micrófono", 2),
    ("Comportamiento", 3),
    ("Apariencia", 4),
    ("Acerca de", 5),
];

fn set_theme(s: &mut Studio, i: usize) {
    match i {
        0 => {
            s.prefs.system = false;
            s.prefs.light = false;
        }
        1 => {
            s.prefs.system = false;
            s.prefs.light = true;
        }
        _ => s.prefs.system = true,
    }
    s.light = s.prefs.is_light();
    s.persist();
}

impl Studio {
    fn section_title(&self, th: &Theme, title: &'static str, note: Option<&'static str>) -> Div {
        div()
            .flex()
            .items_baseline()
            .justify_between()
            .gap(px(12.))
            .child(h2(title))
            .when_some(note, |d, note| {
                d.child(div().text_size(px(12.)).text_color(th.muted).child(note))
            })
    }

    fn setting_row(
        &self,
        th: &Theme,
        id: &'static str,
        label: &'static str,
        desc: &'static str,
        on: bool,
        last: bool,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(16.))
            .py(px(18.))
            .cursor_pointer()
            .when(!last, |d| d.border_b_1().border_color(th.line))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(div().text_size(px(12.)).text_color(th.muted).child(desc)),
            )
            .child(switch(th, (id, 1usize), on))
    }

    fn shortcuts_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let capturing = self.editing;
        let ptt = self.state.settings.push_to_talk;
        let save_label: SharedString = if capturing == Some(false) {
            "Pulsa…".into()
        } else {
            self.state.hotkey.label().into()
        };
        let talk_label: SharedString = if capturing == Some(true) {
            "Pulsa…".into()
        } else {
            ptt.or(self.last_talk)
                .map_or("Sin tecla".into(), |b| b.label().into())
        };
        let hint: SharedString = if capturing.is_some() {
            "Pulsa una tecla o combinación. Esc cancela.".into()
        } else if !self.flash.is_empty() {
            self.flash.clone()
        } else {
            "Haz clic en una tecla para cambiarla.".into()
        };
        let hint_color = if capturing.is_some() {
            th.accent_text
        } else if !self.flash.is_empty() {
            th.text
        } else {
            th.muted
        };
        card(th)
            .p(px(28.))
            .max_w(px(760.))
            .child(self.section_title(th, "Atajos", Some("Funcionan con Cut en segundo plano o dentro del juego.")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .pt(px(22.))
                    .pb(px(20.))
                    .mt(px(4.))
                    .border_b_1()
                    .border_color(th.line)
                    .child(icon("save", 22., th.meter_hi))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Guardar clip"))
                            .child(div().text_size(px(12.)).text_color(th.muted).child("Guarda el replay al instante.")),
                    )
                    .child(
                        div()
                            .id("cap-save")
                            .cursor_pointer()
                            .child(keycap(th, save_label, if capturing == Some(false) { Cap::Active } else { Cap::Plain }, true))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.begin_edit(false);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .py(px(20.))
                    .child(icon("mic", 22., if ptt.is_some() { th.accent } else { th.muted }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Pulsar para hablar"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(relative(1.4))
                                    .text_color(th.muted)
                                    .child("Tu micrófono solo entra al clip mientras mantienes la tecla. Acepta teclas del teclado numérico."),
                            ),
                    )
                    .child(switch(th, "talk-switch", ptt.is_some()).on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_talk();
                        cx.notify();
                    })))
                    .child(
                        div()
                            .id("cap-talk")
                            .when(ptt.is_some(), |d| d.cursor_pointer())
                            .child(keycap(
                                th,
                                talk_label,
                                if capturing == Some(true) { Cap::Active } else { Cap::Talk },
                                ptt.is_some() || capturing == Some(true),
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.state.settings.push_to_talk.is_some() {
                                    this.begin_edit(true);
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_h(px(18.))
                    .text_size(px(12.))
                    .text_color(hint_color)
                    .when(capturing.is_some(), |d| {
                        d.child(div().w(px(7.)).h(px(14.)).rounded(px(2.)).bg(th.accent).with_animation(
                            "caret",
                            Animation::new(Duration::from_millis(1000)).repeat(),
                            |el, t| el.opacity(if t < 0.5 { 1. } else { 0.2 }),
                        ))
                    })
                    .child(hint),
            )
    }

    fn microphone_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let volume = self.slider_value(Slider::Mic);
        let lit = ((volume as f32 / 200.) * 40.).round() as usize;
        let device: String = self
            .state
            .devices
            .devices
            .iter()
            .find(|d| d.id == self.state.settings.device)
            .map(|d| d.label.clone())
            .unwrap_or_else(|| "Predeterminado".into());
        let mic_gain = self.gain("mic");
        let (mono, noise) = (
            self.state.settings.mono,
            self.state.settings.noise_suppression,
        );
        let cell = self.slider_bounds(Slider::Mic);

        let tiles = div()
            .relative()
            .mt(px(14.))
            .h(px(30.))
            .id("mic-tiles")
            .cursor_pointer()
            .child(
                canvas(move |b, _, _| *cell.borrow_mut() = b, |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .children((0..40).map(|i| {
                        let on = i < lit;
                        let mid = i == 19;
                        let color = if on {
                            if i + 1 == lit { th.accent } else { th.text }
                        } else if mid {
                            th.tile_mid
                        } else {
                            th.tile_off
                        };
                        div()
                            .flex_1()
                            .h(px(if mid {
                                30.
                            } else if on {
                                22.
                            } else {
                                16.
                            }))
                            .rounded(px(3.))
                            .bg(color)
                    })),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.drag_done = false;
                    this.drag_to(Slider::Mic, ev.position);
                    cx.notify();
                }),
            );

        let toggle = |id: &'static str, label: &'static str, desc: &'static str, on: bool| {
            div()
                .id(id)
                .flex_1()
                .min_w(px(0.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .p(px(14.))
                .rounded(px(16.))
                .border_1()
                .border_color(if on { th.cap_border } else { th.field_border })
                .bg(if on { th.nav_on } else { th.field })
                .cursor_pointer()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label),
                        )
                        .child(div().text_size(px(12.)).text_color(th.muted).child(desc)),
                )
                .child(switch(th, (id, 1usize), on))
        };

        card(th)
            .p(px(28.))
            .max_w(px(760.))
            .child(self.section_title(
                th,
                "Micrófono global",
                Some("Para todas las capturas nuevas."),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .mt(px(24.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(th.secondary)
                            .child("Dispositivo"),
                    )
                    .child(self.select_button(th, Select::Mic, "mic-device", device, false, cx)),
            )
            .child(
                div()
                    .mt(px(26.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(th.secondary)
                                    .child("Volumen de grabación"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(16.))
                                    .child({
                                        let hover = th.text;
                                        div()
                                            .id("mic-reset")
                                            .cursor_pointer()
                                            .text_size(px(12.))
                                            .text_color(th.muted)
                                            .underline()
                                            .hover(move |s| s.text_color(hover))
                                            .child("Restablecer")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_gain("mic", |g| g.volume = 100);
                                                cx.notify();
                                            }))
                                    })
                                    .child(
                                        div()
                                            .font_family(theme::mono())
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_size(px(22.))
                                            .child(format!("{volume}%")),
                                    ),
                            ),
                    )
                    .child(tiles)
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .mt(px(8.))
                            .font_family(theme::mono())
                            .text_size(px(11.))
                            .text_color(th.muted)
                            .child("0%")
                            .child("100% · original")
                            .child("200%"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .mt(px(24.))
                    .child(
                        toggle("mic-mono", "Mono", "Voz centrada", mono).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.set_audio(|s| s.mono = !s.mono);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        toggle(
                            "mic-mute",
                            "Silenciar",
                            "No entra al clip",
                            !mic_gain.include,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_gain("mic", |g| g.include = !g.include);
                            cx.notify();
                        })),
                    )
                    .child(
                        toggle("mic-noise", "Supresión", "Reduce el ruido", noise).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.set_audio(|s| s.noise_suppression = !s.noise_suppression);
                                cx.notify();
                            }),
                        ),
                    ),
            )
    }

    fn behavior_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let startup = self.state.startup;
        let minimize = self.prefs.minimize_on_close;
        let cursor = self.state.profile.cursor;
        let sound = self.prefs.sound;
        card(th)
            .px(px(28.))
            .pt(px(28.))
            .pb(px(10.))
            .max_w(px(760.))
            .child(h2("Comportamiento").mb(px(8.)))
            .child(
                self.setting_row(
                    th,
                    "set-startup",
                    "Iniciar con Windows",
                    "Abrir Cut al iniciar sesión.",
                    startup,
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let value = !this.state.startup;
                    this.controller.send(C::Startup(value));
                    cx.notify();
                })),
            )
            .child(
                self.setting_row(
                    th,
                    "set-minimize",
                    "Minimizar al cerrar",
                    "Si está apagado, Cut sigue en la bandeja.",
                    minimize,
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.prefs.minimize_on_close = !this.prefs.minimize_on_close;
                    this.persist();
                    cx.notify();
                })),
            )
            .child(
                self.setting_row(
                    th,
                    "set-cursor",
                    "Capturar cursor",
                    "Se aplica al siguiente clip.",
                    cursor,
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_profile(|p| p.cursor = !p.cursor);
                    cx.notify();
                })),
            )
            .child(
                self.setting_row(
                    th,
                    "set-sound",
                    "Sonido al guardar",
                    "Un tono breve confirma cada clip, sin nada en pantalla.",
                    sound,
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.prefs.sound = !this.prefs.sound;
                    this.persist();
                    cx.notify();
                })),
            )
    }

    fn appearance_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let selected = if self.prefs.system {
            2
        } else if self.prefs.light {
            1
        } else {
            0
        };
        let animated = !self.prefs.paused;
        let segmented = self.segmented(
            th,
            "seg-theme",
            vec!["Oscuro".into(), "Claro".into(), "Sistema".into()],
            selected,
            false,
            34.,
            13.,
            false,
            set_theme,
            cx,
        );
        card(th)
            .p(px(28.))
            .max_w(px(760.))
            .child(h2("Apariencia").mb(px(20.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(20.))
                    .child(
                        div()
                            .w(px(112.))
                            .flex_none()
                            .text_size(px(13.))
                            .text_color(th.secondary)
                            .child("Tema"),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(segmented)),
            )
            .child(
                self.setting_row(
                    th,
                    "set-sharks",
                    "Tiburones animados",
                    "Nadan a 24 FPS y se detienen solos al minimizar Cut.",
                    animated,
                    true,
                )
                .mt(px(22.))
                .pt(px(2.))
                .border_t_1()
                .border_color(th.line)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.prefs.paused = !this.prefs.paused;
                    this.persist();
                    cx.notify();
                })),
            )
    }

    fn about_card(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let has_update = self.state.update.is_some();
        let downloading = self.state.progress.is_some();
        let label = if downloading {
            "Descargando…"
        } else if has_update {
            "Descargar actualización"
        } else {
            "Buscar actualizaciones"
        };
        card(th)
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(18.))
            .px(px(28.))
            .py(px(24.))
            .max_w(px(760.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(56.))
                    .h(px(56.))
                    .flex_none()
                    .rounded(px(16.))
                    .bg(theme::hex(th.bg))
                    .border_1()
                    .border_color(th.ghost_border)
                    .child(mark(th, 30.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Cut {}", env!("CARGO_PKG_VERSION"))),
                    ),
            )
            .child(
                ghost(th, "check-updates")
                    .h(px(38.))
                    .px(px(14.))
                    .rounded(px(12.))
                    .child(icon("refresh", 15., th.text))
                    .child(label)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let command = if this.state.update.is_some() {
                            C::InstallUpdate
                        } else {
                            C::CheckUpdates
                        };
                        this.controller.send(command);
                        cx.notify();
                    })),
            )
            .child(
                ghost(th, "open-news")
                    .h(px(38.))
                    .px(px(14.))
                    .rounded(px(12.))
                    .child(icon("spark", 15., th.text))
                    .child("Novedades")
                    .on_click(cx.listener(|this, _, _, cx| this.open_news(cx))),
            )
    }

    pub(super) fn settings(&mut self, th: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut chips = div().flex().flex_wrap().gap(px(6.)).mt(px(8.));
        for (i, (label, index)) in SECTIONS.iter().enumerate() {
            let hover_bg = th.ghost_hover;
            let hover_fg = th.text;
            let index = *index;
            chips = chips.child(
                div()
                    .id(("chip", i))
                    .flex()
                    .items_center()
                    .h(px(30.))
                    .px(px(12.))
                    .rounded_full()
                    .bg(th.ghost_bg)
                    .border_1()
                    .border_color(th.card_border)
                    .text_size(px(12.))
                    .text_color(th.secondary)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
                    .child(*label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(item) = this.scroll.bounds_for_item(index) {
                            let to = this.scroll.bounds().top() - item.top() + px(22.);
                            this.scroll_to(f32::from(to), cx);
                        }
                    })),
            );
        }
        vec![
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .mb(px(4.))
                .child(h1(th, "Ajustes"))
                .child(backed(th, chips))
                .into_any_element(),
            self.shortcuts_card(th, cx).into_any_element(),
            self.microphone_card(th, cx).into_any_element(),
            self.behavior_card(th, cx).into_any_element(),
            self.appearance_card(th, cx).into_any_element(),
            self.about_card(th, cx).into_any_element(),
        ]
    }
}
