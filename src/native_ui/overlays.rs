//! Overlays: the "Novedades" side sheet (shown once after an update) and the
//! shortcut-conflict dialog.
use super::theme::px;
use super::{
    Conflict, Studio,
    theme::{self, Theme, hex, hexa},
    widgets::{cta, ghost, icon, kbd, mark},
};
use gpui::{prelude::*, *};
use std::time::Duration;

const RELEASES: &str = "https://github.com/00faithlss/Cut/releases";

const ITEMS: [(&str, &str, &str); 5] = [
    (
        "film",
        "Replay de 10 minutos",
        "Guarda los últimos momentos desde un historial comprimido en RAM, sin detener la captura.",
    ),
    (
        "mic",
        "Pulsar para hablar",
        "Asigna una tecla en Ajustes y tu voz solo entra al clip mientras la mantienes, también dentro del juego.",
    ),
    (
        "key",
        "Atajos con el teclado numérico",
        "Num 0–9, Num Enter y los operadores funcionan como atajos, aunque Bloq Num esté desactivado.",
    ),
    (
        "window",
        "Interfaz nativa",
        "Cut ya no depende de WebView2 y la ventana sigue el escalado de Windows.",
    ),
    (
        "film",
        "Mejoras de captura",
        "Colores corregidos, mejoras contra el audio duplicado y respaldo por CPU hasta 1080p a 30 FPS.",
    ),
];

fn rng(x: f64, y: f64) -> f64 {
    let v = (x * 127.1 + y * 311.7).sin() * 43758.5453;
    v - v.floor()
}

impl Studio {
    /// Decorative band: 3 × 32 tiles that echo the shark mosaic.
    fn news_band(&self, th: &Theme) -> Div {
        let ink = if th.light { 0x1C1C18 } else { 0xEBEBEA };
        let mut band = div().flex().flex_col().gap(px(3.)).mx(px(28.)).mt(px(26.));
        for r in 0..3 {
            let mut row = div().flex().gap(px(3.));
            for c in 0..32 {
                let k = c as f64 / 31.;
                let n = rng(c as f64, (r * 7 + 3) as f64);
                let hot = n > 0.9 - k * 0.25 && r != 1;
                let level = 0.04 + n * 0.16 * (1. - k * 0.6);
                let color = if hot {
                    if k > 0.6 { th.accent } else { hex(0xFF8A4C) }
                } else {
                    hexa(ink, level as f32)
                };
                row = row.child(div().flex_1().h(px(8.)).rounded(px(2.)).bg(color));
            }
            band = band.child(row);
        }
        band
    }

    pub(super) fn news_sheet(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let closing = self.news_closing;
        let generation = self.news_gen;
        let version = env!("CARGO_PKG_VERSION");
        let subtitle = if self.news_from.is_empty() || self.news_from == version {
            "Esto es lo nuevo en esta versión.".to_string()
        } else {
            format!("Actualizado desde {}.", self.news_from)
        };
        let progress = move |d: f32| {
            let k = super::widgets::ease_out(d);
            if closing { 1. - d } else { k }
        };

        let mut list = div()
            .id("news-list")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px(px(28.))
            .mt(px(24.));
        for (ic, title, body) in ITEMS {
            list = list.child(
                div()
                    .flex()
                    .gap(px(16.))
                    .py(px(18.))
                    .border_t_1()
                    .border_color(th.line)
                    .child(div().pt(px(1.)).child(icon(ic, 20., th.meter_hi)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(relative(1.55))
                                    .text_color(th.secondary)
                                    .child(body),
                            ),
                    ),
            );
        }

        let sheet = div()
            .id("news-sheet")
            .occlude()
            .absolute()
            // Starts below the title bar so the status pill and window buttons stay clear.
            .top(px(58.))
            .bottom(px(12.))
            .right(px(12.))
            .w(px(440.))
            .flex()
            .flex_col()
            .rounded(px(24.))
            .bg(th.sheet)
            .border_1()
            .border_color(th.ghost_border)
            .overflow_hidden()
            .shadow_2xl()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(px(22.))
                    .pr(px(22.))
                    .pl(px(28.))
                    .child(mark(th, 30.))
                    .child(
                        ghost(th, "news-x")
                            .w(px(36.))
                            .h(px(36.))
                            .justify_center()
                            .rounded(px(12.))
                            .px(px(0.))
                            .child(icon("x", 15., th.secondary))
                            .on_click(cx.listener(|this, _, _, cx| this.close_news(cx))),
                    ),
            )
            .child(self.news_band(th))
            .child(
                div()
                    .px(px(28.))
                    .pt(px(26.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(34.))
                            .line_height(relative(1.))
                            .child("Cut")
                            .child(
                                div()
                                    .font_family(theme::mono())
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(version),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(12.))
                            .text_size(px(13.))
                            .line_height(relative(1.55))
                            .text_color(th.secondary)
                            .child(subtitle),
                    ),
            )
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(28.))
                    .pt(px(20.))
                    .pb(px(24.))
                    .border_t_1()
                    .border_color(th.line)
                    .child(
                        div()
                            .id("news-ok")
                            .tab_stop(true)
                            .flex()
                            .items_center()
                            .h(px(44.))
                            .px(px(22.))
                            .rounded(px(14.))
                            .bg(th.chip_on)
                            .text_color(th.chip_on_text)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .child("Entendido")
                            .on_click(cx.listener(|this, _, _, cx| this.close_news(cx))),
                    )
                    .child({
                        let hover = th.text;
                        div()
                            .id("news-history")
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .h(px(44.))
                            .px(px(12.))
                            .text_size(px(13.))
                            .text_color(th.secondary)
                            .cursor_pointer()
                            .hover(move |s| s.text_color(hover))
                            .child("Historial de cambios")
                            .child(icon("chevr", 13., th.secondary))
                            .on_click(|_, _, cx| cx.open_url(RELEASES))
                    }),
            )
            .with_animation(
                ("news-sheet-anim", generation),
                Animation::new(Duration::from_millis(if closing { 300 } else { 600 })),
                move |el, d| {
                    let k = progress(d);
                    el.opacity(k).right(px(12. - 48. * (1. - k)))
                },
            );

        let scrim = div()
            .id("news-scrim")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(th.scrim)
            .on_click(cx.listener(|this, _, _, cx| this.close_news(cx)))
            .with_animation(
                ("news-scrim-anim", generation),
                Animation::new(Duration::from_millis(if closing { 280 } else { 500 })),
                move |el, d| el.opacity(progress(d)),
            );

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(scrim)
            .child(sheet)
    }

    pub(super) fn conflict_modal(
        &mut self,
        th: &Theme,
        c: Conflict,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let key = c.key.label();
        let ptt = self.state.settings.push_to_talk;
        let save_label = if c.talk {
            self.state.hotkey.label()
        } else {
            key.clone()
        };
        let talk_label = if c.talk {
            key.clone()
        } else {
            ptt.map_or("Sin tecla".to_string(), |b| b.label())
        };
        let (title, body) = if c.talk {
            (
                format!("{key} ya guarda clips"),
                "Una tecla no puede hacer dos cosas: cada vez que hablaras también guardarías un clip. Elige otra o intercambia los atajos.",
            )
        } else {
            (
                format!("{key} ya es tu tecla para hablar"),
                "Una tecla no puede hacer dos cosas: cada vez que guardes un clip también se abriría tu micrófono. Elige otra o intercambia los atajos.",
            )
        };
        let swap = self.swap_plan(c).map(|(save, talk)| {
            if c.talk {
                format!("Guardar pasa a {}", save.label())
            } else {
                format!("Hablar pasa a {}", talk.label())
            }
        });
        let preview_row = |ic: &'static str, label: &'static str, key: String, last: bool| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .py(px(13.))
                .when(!last, |d| d.border_b_1().border_color(th.line))
                .child(icon(ic, 18., th.secondary))
                .child(div().flex_1().text_size(px(13.)).child(label))
                .child(kbd(th, key, false))
        };
        let modal = div()
            .id("conflict-modal")
            .occlude()
            .w(px(480.))
            .p(px(28.))
            .rounded(px(26.))
            .bg(th.sheet)
            .border_1()
            .border_color(th.ghost_border)
            .shadow_2xl()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(46.))
                            .h(px(46.))
                            .rounded(px(14.))
                            .bg(th.accent_soft)
                            .border_1()
                            .border_color(th.accent_edge)
                            .child(icon("alert", 22., th.accent)),
                    )
                    .child(
                        ghost(th, "conflict-x")
                            .w(px(34.))
                            .h(px(34.))
                            .px(px(0.))
                            .justify_center()
                            .rounded(px(11.))
                            .child(icon("x", 15., th.secondary))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.conflict = None;
                                this.flash = "Los atajos no cambiaron.".into();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .mt(px(20.))
                    .text_size(px(24.))
                    .line_height(relative(1.2))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .mt(px(10.))
                    .text_size(px(13.))
                    .line_height(relative(1.6))
                    .text_color(th.secondary)
                    .child(body),
            )
            .child(
                div()
                    .mt(px(22.))
                    .rounded(px(16.))
                    .border_1()
                    .border_color(th.card_border)
                    .bg(th.field)
                    .overflow_hidden()
                    .child(preview_row("save", "Guardar clip", save_label, false))
                    .child(preview_row("mic", "Pulsar para hablar", talk_label, true)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .mt(px(22.))
                    .when_some(swap, |d, label| {
                        d.child(
                            ghost(th, "conflict-swap")
                                .flex_1()
                                .justify_center()
                                .h(px(46.))
                                .rounded(px(14.))
                                .text_size(px(13.))
                                .child(icon("swap", 16., th.text))
                                .child(label)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.swap_keys();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        cta(th, "conflict-retry")
                            .flex_1()
                            .h(px(46.))
                            .rounded(px(14.))
                            .text_size(px(13.))
                            .child("Elegir otra tecla")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.conflict = None;
                                this.begin_edit(c.talk);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .mt(px(16.))
                    .text_size(px(12.))
                    .text_color(th.muted)
                    .text_center()
                    .child("Esc cancela y deja los atajos como estaban."),
            )
            .with_animation(
                "conflict-pop",
                Animation::new(Duration::from_millis(450)),
                |el, d| {
                    let k = super::widgets::ease_out(d);
                    el.opacity(k).mt(px(16. * (1. - k)))
                },
            );
        div()
            .id("conflict-scrim")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(24.))
            .bg(th.modal_scrim)
            .child(modal)
    }
}
