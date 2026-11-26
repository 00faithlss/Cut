//! Window chrome: custom title bar (drag area + caption buttons) and the left rail.
use super::theme::px;
use super::{
    Studio, Tab,
    theme::{self, Theme, hexa},
    widgets::{self, icon, mark},
};
use crate::controller::Command as C;
use gpui::{prelude::*, *};

impl Studio {
    pub(super) fn titlebar(&self, th: &Theme) -> Div {
        let (status, live) = if self.state.active {
            (format!("Captura activa · {}", self.source_label()), true)
        } else if self.state.paused {
            ("Captura en pausa".to_string(), false)
        } else {
            ("Preparando replay…".to_string(), false)
        };
        let close_hover = th.close_hover;
        let hover = th.nav_on;
        let muted = th.muted;
        let text = th.text;
        let caption = move |id: &'static str,
                            name: &'static str,
                            size: f32,
                            area: WindowControlArea,
                            danger: bool| {
            div()
                .id(id)
                .window_control_area(area)
                .flex()
                .items_center()
                .justify_center()
                .w(px(46.))
                .h(px(46.))
                .text_color(muted)
                .hover(move |s| {
                    if danger {
                        s.bg(close_hover).text_color(gpui::white())
                    } else {
                        s.bg(hover).text_color(text)
                    }
                })
                .child(icon(name, size, muted))
        };
        // The drag area and the caption buttons are siblings: Windows asks GPUI which
        // area is under the cursor and the first match wins, so a drag area wrapping the
        // buttons would swallow minimise, maximise and close.
        let drag = div()
            .id("titlebar-drag")
            .window_control_area(WindowControlArea::Drag)
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .items_center()
            .gap(px(14.))
            .pl(px(30.))
            .pr(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(mark(th, 26.))
                    .px(px(8.))
                    .py(px(4.))
                    .ml(px(-8.))
                    .rounded(px(10.))
                    .bg(hexa(th.bg, 0.85))
                    .child(
                        div()
                            .font_family(theme::sans())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(19.))
                            .line_height(relative(1.))
                            .child("Cut"),
                    ),
            )
            .child(
                widgets::mono_text(env!("CARGO_PKG_VERSION"), 11., th.muted)
                    .px(px(6.))
                    .rounded(px(8.))
                    .bg(hexa(th.bg, 0.85)),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_shrink()
                    .min_w(px(0.))
                    .max_w(px(420.))
                    .items_center()
                    .gap(px(9.))
                    .h(px(28.))
                    .pl(px(11.))
                    .pr(px(12.))
                    .rounded_full()
                    .bg(hexa(th.bg, 0.92))
                    .border_1()
                    .border_color(th.ghost_border)
                    .text_size(px(12.))
                    .text_color(th.secondary)
                    .child(
                        div()
                            .flex_none()
                            .w(px(7.))
                            .h(px(7.))
                            .rounded_full()
                            .bg(if live { th.accent } else { th.faint }),
                    )
                    .child(div().min_w(px(0.)).truncate().child(status)),
            );
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(46.))
            .child(drag)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .bg(hexa(th.bg, 0.85))
                    .child(caption(
                        "win-min",
                        "minus",
                        15.,
                        WindowControlArea::Min,
                        false,
                    ))
                    .child(caption(
                        "win-max",
                        "square",
                        14.,
                        WindowControlArea::Max,
                        false,
                    ))
                    .child(caption(
                        "win-close",
                        "x",
                        15.,
                        WindowControlArea::Close,
                        true,
                    )),
            )
    }

    fn nav_item(
        &self,
        th: &Theme,
        id: &'static str,
        tab: Tab,
        label: &'static str,
        ic: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let on = self.tab == tab;
        let color = if on { th.text } else { th.muted };
        let hover_bg = th.hover;
        let hover_fg = th.text;
        div()
            .id(id)
            .tab_stop(true)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(7.))
            .w(px(70.))
            .h(px(68.))
            .rounded(px(16.))
            .cursor_pointer()
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            // Opaque enough that a shark passing behind never fights the label.
            .bg(if on {
                theme::hex(if th.light { 0xE1E1DD } else { 0x19191A })
            } else {
                hexa(th.bg, 0.85)
            })
            .when(!on, move |d| {
                d.hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            })
            .child(icon(ic, 22., color))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
    }

    pub(super) fn rail(&mut self, th: &Theme, cx: &mut Context<Self>) -> Div {
        let paused = self.prefs.paused;
        let hover_bg = th.hover;
        let hover_fg = th.text;
        let muted = th.muted;
        div()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .w(px(98.))
            .py(px(22.))
            .child(self.nav_item(th, "nav-capture", Tab::Capture, "Captura", "wave", cx))
            .child(self.nav_item(th, "nav-clips", Tab::Clips, "Clips", "film", cx))
            .child(self.nav_item(th, "nav-settings", Tab::Settings, "Ajustes", "sliders", cx))
            .child(div().flex_1())
            .child(
                div()
                    .id("nav-pause")
                    .tab_stop(true)
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .w(px(70.))
                    .h(px(68.))
                    .rounded(px(16.))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted)
                    .bg(hexa(th.bg, 0.85))
                    .text_center()
                    .line_height(relative(1.2))
                    .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
                    .child(icon(if paused { "play" } else { "pause" }, 20., muted))
                    .child(if paused {
                        "Animar fondo"
                    } else {
                        "Pausar fondo"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.prefs.paused = !this.prefs.paused;
                        this.persist();
                        cx.notify();
                    })),
            )
    }

    /// Used by the tray-less path and by the CTA: ask the engine to save.
    pub(super) fn save_clip(&self) {
        self.controller.send(C::Save);
    }
}
