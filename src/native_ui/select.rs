//! Dropdown selects: a trigger button plus a menu drawn over the whole window.
use super::{
    Studio,
    theme::{self, Theme, px},
    widgets::{ghost, icon},
};
use gpui::{prelude::*, *};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Select {
    /// Monitor or window being captured (depends on the profile).
    Source,
    /// Microphone device.
    Mic,
}

const ROW: f32 = 36.;
const PAD: f32 = 6.;
const MAX_HEIGHT: f32 = 336.;

impl Studio {
    /// (value, label) pairs of a select.
    fn options(&self, which: Select) -> Vec<(String, String)> {
        match which {
            Select::Source => {
                let list = if self.state.profile.monitor {
                    &self.state.monitors
                } else {
                    &self.state.windows
                };
                list.iter()
                    .map(|s| (s.id.clone(), s.label.clone()))
                    .collect()
            }
            Select::Mic => {
                let mut list = vec![(String::new(), "Predeterminado".to_string())];
                list.extend(
                    self.state
                        .devices
                        .devices
                        .iter()
                        .filter(|d| !d.id.is_empty())
                        .map(|d| (d.id.clone(), d.label.clone())),
                );
                list
            }
        }
    }

    fn selected(&self, which: Select) -> &str {
        match which {
            Select::Source => &self.state.profile.id,
            Select::Mic => &self.state.settings.device,
        }
    }

    fn choose(&mut self, which: Select, value: String) {
        match which {
            Select::Source => self.set_profile(|p| p.id = value),
            Select::Mic => self.set_audio(|s| s.device = value),
        }
    }

    /// The closed select: current value and a chevron. Records where it is drawn.
    pub(super) fn select_button(
        &mut self,
        th: &Theme,
        which: Select,
        id: &'static str,
        label: String,
        mono: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let cell = self.select_bounds.entry(which).or_default().clone();
        ghost(th, id)
            .relative()
            .h(px(38.))
            .pl(px(14.))
            .pr(px(12.))
            .rounded(px(12.))
            .gap(px(10.))
            .text_size(px(13.))
            .when(mono, |d| {
                d.font_family(theme::mono()).font_weight(FontWeight::NORMAL)
            })
            .child(
                canvas(
                    move |bounds, _, _| *cell.borrow_mut() = bounds,
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(div().max_w(px(340.)).truncate().child(label))
            .child(icon("chev", 15., th.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select = Some(which);
                cx.notify();
            }))
    }

    /// The open menu, if any. Drawn last so it sits above the page.
    pub(super) fn select_overlay(
        &mut self,
        th: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let which = self.select?;
        let trigger = *self.select_bounds.get(&which)?.borrow();
        let options = self.options(which);
        let selected = self.selected(which).to_string();
        let view = window.viewport_size();
        let margin = px(8.);

        // Below the trigger when it fits, otherwise on the side with more room.
        let wanted = px((options.len().max(1) as f32 * ROW + PAD * 2. + 2.).min(MAX_HEIGHT));
        let below = view.height - trigger.bottom() - margin * 2.;
        let above = trigger.top() - margin * 2.;
        let down = below >= wanted || below >= above;
        let height = wanted.min(if down { below } else { above });
        let width = trigger
            .size
            .width
            .max(px(360.))
            .min(view.width - margin * 2.);
        let left = (trigger.right() - width).max(margin);
        let top = if down {
            trigger.bottom() + px(6.)
        } else {
            trigger.top() - px(6.) - height
        };

        let mut list = div()
            .id("select-list")
            .size_full()
            .overflow_y_scroll()
            .p(px(PAD))
            .flex()
            .flex_col();
        if options.is_empty() {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(9.))
                    .text_size(px(13.))
                    .text_color(th.muted)
                    .child("No hay opciones disponibles."),
            );
        }
        for (i, (value, label)) in options.into_iter().enumerate() {
            let on = value == selected;
            let hover = th.nav_on;
            list = list.child(
                div()
                    .id(("select-option", i))
                    .tab_stop(true)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(ROW))
                    .px(px(12.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(if on { th.text } else { th.secondary })
                    .when(on, |d| d.bg(th.nav_on))
                    .hover(move |s| s.bg(hover))
                    .child(div().flex_1().min_w(px(0.)).truncate().child(label))
                    .child(icon("check", 15., th.accent).opacity(if on { 1. } else { 0. }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.select = None;
                        this.choose(which, value.clone());
                        cx.notify();
                    })),
            );
        }

        Some(
            div()
                .id("select-backdrop")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.select = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("select-menu")
                        .occlude()
                        .absolute()
                        .left(left)
                        .top(top)
                        .w(width)
                        .h(height)
                        .rounded(px(16.))
                        .bg(th.menu)
                        .border_1()
                        .border_color(th.ghost_border)
                        .shadow_lg()
                        .overflow_hidden()
                        .child(list),
                ),
        )
    }
}
