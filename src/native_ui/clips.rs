//! "Clips" tab: capture source, video/audio profile and replay duration.
use super::theme::px;
use super::{
    Select, Studio,
    theme::{self, Theme},
    widgets::{card, divider, ghost, h1, h2, icon, mono_text},
};
use crate::{audio::ClipAudio, controller::Command as C};
use gpui::{prelude::*, *};

const RES: [u32; 4] = [720, 1080, 1440, 2160];
const FPS: [u32; 4] = [30, 60, 90, 120];
const BITRATE: [u32; 4] = [10, 20, 40, 80];
const DURATIONS: [u64; 6] = [15, 30, 60, 120, 300, 600];
const DURATION_LABELS: [&str; 6] = ["15 s", "30 s", "1 min", "2 min", "5 min", "10 min"];

fn index_of(values: &[u32], v: u64) -> usize {
    values.iter().position(|x| *x as u64 == v).unwrap_or(0)
}

fn out_res(height: u64) -> &'static str {
    match height {
        720 => "1280 × 720",
        1440 => "2560 × 1440",
        2160 => "3840 × 2160",
        _ => "1920 × 1080",
    }
}

fn set_res(s: &mut Studio, i: usize) {
    s.set_profile(|p| p.height = RES[i] as _);
}
fn set_fps(s: &mut Studio, i: usize) {
    s.set_profile(|p| p.fps = FPS[i] as _);
}
fn set_bitrate(s: &mut Studio, i: usize) {
    s.set_profile(|p| p.bitrate = BITRATE[i] as _);
}
fn set_audio_mode(s: &mut Studio, i: usize) {
    s.set_audio(|a| {
        a.clip_audio = if i == 0 {
            ClipAudio::Mixed
        } else {
            ClipAudio::Separate
        }
    });
}
fn set_duration(s: &mut Studio, i: usize) {
    s.set_profile(|p| p.seconds = DURATIONS[i] as _);
}

impl Studio {
    fn source_card(
        &self,
        th: &Theme,
        id: &'static str,
        copy: (&'static str, &'static str, &'static str),
        on: bool,
        cx: &mut Context<Self>,
        monitor: bool,
    ) -> Stateful<Div> {
        let (ic, title, text) = copy;
        let hover = th.cap_edge;
        let dot = if on {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(18.))
                .h(px(18.))
                .rounded_full()
                .bg(th.accent)
                .child(
                    div()
                        .w(px(8.))
                        .h(px(8.))
                        .rounded_full()
                        .bg(theme::hex(th.bg)),
                )
        } else {
            div()
                .w(px(18.))
                .h(px(18.))
                .rounded_full()
                .border_1()
                .border_color(th.faint)
        };
        div()
            .id(id)
            .tab_stop(true)
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(20.))
            .rounded(px(18.))
            .border_1()
            .border_color(if on { th.cap_edge } else { th.card_border })
            .bg(if on { th.nav_on } else { th.field })
            .cursor_pointer()
            .when(!on, move |d| d.hover(move |s| s.border_color(hover)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(icon(ic, 24., if on { th.accent } else { th.muted }))
                    .child(dot),
            )
            .child(
                div()
                    .mt(px(16.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(relative(1.5))
                    .text_color(th.muted)
                    .child(text),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pick_kind(monitor);
                cx.notify();
            }))
    }

    pub(super) fn clips(&mut self, th: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.state.profile.clone();
        let monitor = p.monitor;
        let mb = (p.bitrate as f64 * p.seconds as f64) / 8.;
        let heavy = mb >= 2000.;
        let est = if mb >= 1000. {
            format!("≈ {:.1} GB", mb / 1000.).replace('.', ",")
        } else {
            format!("≈ {} MB", mb.round() as u64)
        };
        let lit = ((mb / 6000.).min(1.) * 40.).round().max(1.) as usize;
        let dur_idx = DURATIONS.iter().position(|d| *d == p.seconds).unwrap_or(5);
        let source_value = self.source_label();
        let mix = self.state.settings.clip_audio == ClipAudio::Mixed;

        // --- Fuente de captura ---
        let source_section = card(th)
            .p(px(28.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .mb(px(20.))
                    .child(h2("Fuente de captura"))
                    .child(
                        ghost(th, "clips-refresh")
                            .child(icon("refresh", 15., th.text))
                            .child("Actualizar fuentes")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_sources();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(self.source_card(
                        th,
                        "src-window",
                        (
                            "window",
                            "Ventana del juego",
                            "Solo esa ventana. Si se cierra, Cut espera a que vuelva.",
                        ),
                        !monitor,
                        cx,
                        false,
                    ))
                    .child(self.source_card(
                        th,
                        "src-monitor",
                        (
                            "monitor",
                            "Pantalla completa",
                            "Todo el monitor, incluidas las demás ventanas.",
                        ),
                        monitor,
                        cx,
                        true,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .mt(px(20.))
                    .pt(px(20.))
                    .border_t_1()
                    .border_color(th.line)
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(th.secondary)
                            .child(if monitor { "Pantalla" } else { "Ventana" }),
                    )
                    .child(self.select_button(
                        th,
                        Select::Source,
                        "source-select",
                        source_value,
                        true,
                        cx,
                    )),
            );

        // --- Video y audio ---
        let group =
            |this: &mut Self,
             label: &'static str,
             id: &'static str,
             labels: Vec<String>,
             sel: usize,
             mono: bool,
             apply: fn(&mut Studio, usize),
             cx: &mut Context<Self>| {
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
                            .child(label),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(
                        this.segmented(th, id, labels, sel, mono, 34., 13., false, apply, cx),
                    ))
            };
        let res_labels = RES.iter().map(|v| format!("{v}p")).collect();
        let fps_labels = FPS.iter().map(|v| v.to_string()).collect();
        let br_labels = BITRATE.iter().map(|v| format!("{v} Mb/s")).collect();
        let video = card(th)
            .p(px(28.))
            .child(h2("Video y audio").mb(px(24.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .child(group(
                        self,
                        "Resolución",
                        "seg-res",
                        res_labels,
                        index_of(&RES, p.height as u64),
                        true,
                        set_res,
                        cx,
                    ))
                    .child(group(
                        self,
                        "Fotogramas",
                        "seg-fps",
                        fps_labels,
                        index_of(&FPS, p.fps as u64),
                        true,
                        set_fps,
                        cx,
                    ))
                    .child(group(
                        self,
                        "Bitrate",
                        "seg-br",
                        br_labels,
                        index_of(&BITRATE, p.bitrate as u64),
                        true,
                        set_bitrate,
                        cx,
                    ))
                    .child(group(
                        self,
                        "Pistas de audio",
                        "seg-audio",
                        vec!["Una pista (mezcla)".into(), "Varias pistas".into()],
                        if mix { 0 } else { 1 },
                        false,
                        set_audio_mode,
                        cx,
                    ))
                    .child(
                        div()
                            .ml(px(132.))
                            .text_size(px(12.))
                            .line_height(relative(1.5))
                            .text_color(th.muted)
                            .child(if mix {
                                "Todas las fuentes incluidas en una sola pista AAC."
                            } else {
                                "La mezcla, más una pista por aplicación y otra para el micrófono."
                            }),
                    ),
            );

        let left = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .gap(px(24.))
            .child(source_section)
            .child(video);

        // --- Duración del replay ---
        let tiles = div()
            .flex()
            .gap(px(3.))
            .mt(px(14.))
            .children((0..40).map(|i| {
                let k = i as f32 / 39.;
                let color = if i < lit {
                    if k < 0.5 {
                        th.est_a
                    } else if k < 0.8 {
                        th.est_b
                    } else {
                        th.est_c
                    }
                } else {
                    th.tile_off
                };
                div().flex_1().h(px(22.)).rounded(px(2.)).bg(color)
            }));
        let diagram = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .flex_none()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(34.))
                    .h(px(26.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(th.faint)
                    .child(
                        div()
                            .w(px(11.))
                            .h(px(11.))
                            .rounded_full()
                            .border_1()
                            .border_color(th.faint),
                    ),
            )
            .child(icon("arrow", 15., th.faint))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(50.))
                    .h(px(28.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(th.text)
                    .child(
                        div()
                            .w(px(17.))
                            .h(px(11.))
                            .rounded_full()
                            .border_1()
                            .border_color(th.accent),
                    ),
            );
        let duration_card = card(th)
            .flex_none()
            .w(px(400.))
            .p(px(28.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(h2("Duración del replay"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(th.muted)
                            .child(format!("Lo que guarda {}", self.state.hotkey.label())),
                    ),
            )
            .child(
                div()
                    .mt(px(28.))
                    .mb(px(24.))
                    .font_family(theme::mono())
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(88.))
                    .line_height(relative(1.))
                    .child(format!("{:02}:{:02}", p.seconds / 60, p.seconds % 60)),
            )
            .child(self.segmented(
                th,
                "seg-duration",
                DURATION_LABELS.iter().map(|s| s.to_string()).collect(),
                dur_idx,
                true,
                44.,
                14.,
                true,
                set_duration,
                cx,
            ))
            .child(
                div()
                    .mt(px(28.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .gap(px(10.))
                            .child(div().text_size(px(13.)).text_color(th.secondary).child("RAM del replay estimada"))
                            .child(
                                div()
                                    .font_family(theme::mono())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(22.))
                                    .text_color(if heavy { th.accent_text } else { th.text })
                                    .child(est),
                            ),
                    )
                    .child(tiles)
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .mt(px(8.))
                            .child(mono_text(
                                format!("{}p · {} FPS · {} Mb/s · sin audio", p.height, p.fps, p.bitrate),
                                11.,
                                th.muted,
                            ))
                            .child(mono_text("6 GB", 11., th.muted)),
                    )
                    .when(heavy, |d| {
                        d.child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .mt(px(14.))
                                .w_full()
                                .min_w(px(0.))
                                .text_size(px(12.))
                                .line_height(relative(1.5))
                                .text_color(th.warn_text)
                                .child(div().flex_none().pt(px(1.)).child(icon("alert", 15., th.warn_text)))
                                .child(div().flex_1().min_w(px(0.)).whitespace_normal().child("Replay largo: necesita RAM disponible. Cut libera los fotogramas que salen del historial; guardar comparte el buffer.")),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .mt(px(24.))
                    .p(px(16.))
                    .rounded(px(16.))
                    .bg(th.field)
                    .border_1()
                    .border_color(th.field_border)
                    .child(diagram)
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
                                    .child(format!("Salida 16:9 · {} px", out_res(p.height as u64))),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(th.muted)
                                    .child("Las fuentes 4:3 se estiran hasta llenar el cuadro."),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .mt(px(24.))
                    .pt(px(20.))
                    .border_t_1()
                    .border_color(th.line)
                    .child(icon("folder", 18., th.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().text_size(px(11.)).text_color(th.muted).child("Carpeta de clips"))
                            .child(
                                div()
                                    .font_family(theme::mono())
                                    .text_size(px(12.))
                                    .truncate()
                                    .child(self.state.directory.to_string_lossy().to_string()),
                            ),
                    )
                    .child(
                        ghost(th, "folder-open")
                            .child("Abrir")
                            .on_click(cx.listener(|this, _, _, _| this.controller.send(C::Directory))),
                    )
                    .child(
                        ghost(th, "folder-change")
                            .child("Cambiar…")
                            .on_click(cx.listener(|this, _, _, _| this.controller.send(C::ChooseDirectory(0)))),
                    ),
            );

        let _ = divider;
        vec![
            div()
                .flex()
                .flex_col()
                .gap(px(32.))
                .child(h1(th, "Clips"))
                .child(
                    div()
                        .flex()
                        .gap(px(24.))
                        .items_start()
                        .child(left)
                        .child(duration_card),
                )
                .into_any_element(),
        ]
    }
}
