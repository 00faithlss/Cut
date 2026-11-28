actions!(cut_ui, [NextFocus, PreviousFocus, Escape]);
mod assets;
mod capture;
mod chrome;
mod clips;
mod fonts;
mod overlays;
pub(crate) mod preferences;
mod sea;
mod select;
mod settings;
mod tape;
mod theme;
mod tray;
mod widgets;

use crate::{audio, controller::Command as C, hotkey::Binding};
use gpui::{prelude::*, *};
use raw_window_handle::HasWindowHandle;
use select::Select;
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};
use theme::{Theme, px};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Capture,
    Clips,
    Settings,
}

/// Controles que convierten la posición del ratón en un valor. Los índices de
/// Track pertenecen al catálogo estable de identidades, no a la lista visible.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Slider {
    Track(usize),
    Mic,
}

/// A captured key that collides with the other shortcut.
#[derive(Clone, Copy)]
struct Conflict {
    /// True when the key being assigned is "pulsar para hablar".
    talk: bool,
    key: Binding,
}

/// Memory for the sliding indicator of a segmented control.
#[derive(Clone, Copy)]
struct SegMem {
    from: f32,
    to: usize,
    generation: usize,
}

struct Studio {
    hwnd: isize,
    _tray: Option<tray_icon::TrayIcon>,
    _instance: Option<crate::instance::Instance>,
    prefs: preferences::Preferences,
    light: bool,
    tab: Tab,
    tab_gen: usize,
    news: bool,
    news_closing: bool,
    news_gen: usize,
    news_from: String,
    focus: FocusHandle,
    /// Some(false) = capturing the save key, Some(true) = capturing the talk key.
    editing: Option<bool>,
    conflict: Option<Conflict>,
    flash: SharedString,
    last_talk: Option<Binding>,
    menu: bool,
    refreshing: Option<std::time::Instant>,
    test_talk: bool,
    visible: bool,
    frame: u32,
    controller: crate::controller::Controller,
    state: crate::controller::State,
    sea: Entity<sea::SeaView>,
    tape: Entity<tape::TapeView>,
    seg: HashMap<&'static str, SegMem>,
    sliders: HashMap<Slider, Rc<RefCell<Bounds<Pixels>>>>,
    track_ids: Vec<String>,
    drag: Option<(Slider, u16)>,
    drag_done: bool,
    scroll: ScrollHandle,
    /// Where a smooth scroll is heading, while it is still moving.
    scroll_target: Option<f32>,
    scroll_tick: Option<std::time::Instant>,
    /// The dropdown that is open, and where each closed one is drawn.
    select: Option<Select>,
    select_bounds: HashMap<Select, Rc<RefCell<Bounds<Pixels>>>>,
    /// Choices sent to the engine but not confirmed yet.
    pending_profile: Option<(crate::replay::Profile, std::time::Instant)>,
    pending_settings: Option<(audio::Settings, std::time::Instant)>,
    /// Where the replay card leaves room for the tape, which `Shell` draws on top so its
    /// 24 FPS animation never forces the whole page to be rebuilt.
    tape_slot: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

impl Studio {
    fn theme(&self) -> Theme {
        if self.light {
            Theme::light()
        } else {
            Theme::dark()
        }
    }

    fn persist(&self) {
        if let Err(e) = self.prefs.save() {
            self.controller
                .send(C::Error(format!("No se pudo guardar la apariencia: {e}")));
        }
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab != tab {
            if self.tab == Tab::Settings {
                self.controller.send(C::MicPreview(false));
            }
            self.cancel_edit("");
            self.stop_talk_preview();
            self.tab = tab;
            self.tab_gen += 1;
            self.menu = false;
            self.select = None;
            self.scroll_target = None;
            self.scroll.set_offset(Point::default());
        }
        cx.notify();
    }

    /// Head the page smoothly toward a scroll offset (0 is the top, negative is down).
    fn scroll_to(&mut self, offset: f32, cx: &mut Context<Self>) {
        let max = f32::from(self.scroll.max_offset().height);
        self.scroll_target = Some(offset.clamp(-max, 0.));
        cx.notify();
    }

    /// One frame of the smooth scroll: cover part of the remaining distance.
    fn animate_scroll(&mut self, window: &mut Window) {
        let Some(target) = self.scroll_target else {
            self.scroll_tick = None;
            return;
        };
        let now = std::time::Instant::now();
        let dt = self
            .scroll_tick
            .map_or(1. / 60., |t| (now - t).as_secs_f32().min(0.05));
        let mut offset = self.scroll.offset();
        let current = f32::from(offset.y);
        let mut next = current + (target - current) * (1. - (-dt * 16.).exp());
        if (target - next).abs() < 0.5 {
            next = target;
            self.scroll_target = None;
            self.scroll_tick = None;
        } else {
            self.scroll_tick = Some(now);
            window.request_animation_frame();
        }
        offset.y = gpui::px(next);
        self.scroll.set_offset(offset);
    }

    fn open_news(&mut self, cx: &mut Context<Self>) {
        self.news = true;
        self.news_closing = false;
        self.news_gen += 1;
        cx.notify();
    }

    fn close_news(&mut self, cx: &mut Context<Self>) {
        if !self.news || self.news_closing {
            return;
        }
        self.news_closing = true;
        self.news_gen += 1;
        let closing_generation = self.news_gen;
        self.prefs.seen_version = env!("CARGO_PKG_VERSION").into();
        self.persist();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = this.update(cx, |s, cx| {
                if s.news_gen != closing_generation {
                    return;
                }
                s.news = false;
                s.news_closing = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ---------- shortcuts ----------

    fn begin_edit(&mut self, talk: bool) {
        self.stop_talk_preview();
        self.controller.send(C::MicPreview(false));
        crate::hotkey::set_editing(true);
        self.editing = Some(talk);
        self.flash = "".into();
        self.controller.send(C::Editing(true));
    }

    fn cancel_edit(&mut self, message: &str) {
        if self.editing.take().is_some() {
            crate::hotkey::set_editing(false);
            crate::hotkey::take_captured();
            self.controller.send(C::Editing(false));
        }
        self.flash = message.to_string().into();
    }

    fn stop_talk_preview(&mut self) {
        self.test_talk = false;
        crate::hotkey::set_talk_preview(false);
    }

    /// A key was captured for the save (`talk == false`) or talk (`true`) shortcut.
    fn captured(&mut self, talk: bool, binding: Binding) {
        let save = self.state.hotkey;
        let ptt = self.state.settings.push_to_talk;
        if talk {
            if binding == save {
                self.conflict = Some(Conflict {
                    talk: true,
                    key: binding,
                });
                return;
            }
            if !binding.valid_talk() {
                self.flash = "Esa tecla no sirve para hablar. Prueba con otra.".into();
                return;
            }
            let mut s = self.state.settings.clone();
            s.push_to_talk = Some(binding);
            self.send_audio(s);
            self.last_talk = Some(binding);
            self.flash = "".into();
        } else {
            if Some(binding) == ptt {
                self.conflict = Some(Conflict {
                    talk: false,
                    key: binding,
                });
                return;
            }
            if !binding.valid() {
                self.flash = "Elige una tecla, un botón del mouse o una combinación válida.".into();
                return;
            }
            self.controller.send(C::Hotkey(binding));
            self.flash = "".into();
        }
    }

    /// New (save, talk) pair for "Intercambiar", if it is valid.
    fn swap_plan(&self, c: Conflict) -> Option<(Binding, Binding)> {
        let save = self.state.hotkey;
        let ptt = self.state.settings.push_to_talk;
        let (new_save, new_talk) = if c.talk { (ptt?, c.key) } else { (c.key, save) };
        (new_save.valid() && new_talk.valid_talk() && new_save != new_talk)
            .then_some((new_save, new_talk))
    }

    fn swap_keys(&mut self) {
        let Some(c) = self.conflict.take() else {
            return;
        };
        let Some((save, talk)) = self.swap_plan(c) else {
            return;
        };
        self.controller.send(C::Shortcuts {
            save,
            talk: Some(talk),
        });
        self.last_talk = Some(talk);
        self.flash = "".into();
    }

    fn toggle_talk(&mut self) {
        let mut s = self.state.settings.clone();
        if let Some(current) = s.push_to_talk {
            self.last_talk = Some(current);
            s.push_to_talk = None;
            self.send_audio(s);
            self.flash = "Pulsar para hablar desactivado.".into();
        } else if let Some(key) = self
            .last_talk
            .filter(|k| *k != self.state.hotkey && k.valid_talk())
        {
            s.push_to_talk = Some(key);
            self.send_audio(s);
        } else {
            self.begin_edit(true);
        }
    }

    // ---------- audio helpers ----------

    /// Tracks shown in the mixer; the microphone is always present.
    fn sources(&self) -> Vec<audio::Source> {
        let mut sources = self.state.tracks.clone();
        if !sources.iter().any(|s| s.id == "mic") {
            sources.push(audio::Source {
                id: "mic".into(),
                label: "Micrófono".into(),
            });
        }
        sources
    }

    fn gain(&self, id: &str) -> audio::Gain {
        self.state.settings.gain(id)
    }

    fn set_gain(&mut self, id: &str, f: impl FnOnce(&mut audio::Gain)) {
        let initial = self.gain(id);
        let mut settings = self.state.settings.clone();
        let g = settings.tracks.entry(id.to_string()).or_insert(initial);
        f(g);
        self.send_audio(settings);
    }

    fn set_audio(&mut self, f: impl FnOnce(&mut audio::Settings)) {
        let mut settings = self.state.settings.clone();
        f(&mut settings);
        self.send_audio(settings);
    }

    fn set_profile(&mut self, f: impl FnOnce(&mut crate::replay::Profile)) {
        let mut p = self.state.profile.clone();
        f(&mut p);
        // Show the choice at once; the engine confirms it a moment later.
        self.state.profile = p.clone();
        self.pending_profile = Some((p.clone(), std::time::Instant::now()));
        self.controller.send(C::Profile(p));
    }

    /// Send audio settings, showing them right away (see `set_profile`).
    fn send_audio(&mut self, settings: audio::Settings) {
        self.state.settings = settings.clone();
        self.pending_settings = Some((settings.clone(), std::time::Instant::now()));
        self.controller.send(C::Audio(settings));
    }

    /// Take the engine's state, keeping a choice the engine has not applied yet so the
    /// control does not jump back for a moment.
    fn sync_state(&mut self) -> bool {
        fn keep<T: Clone + PartialEq>(
            pending: &mut Option<(T, std::time::Instant)>,
            fresh: &mut T,
        ) {
            if let Some((value, at)) = pending {
                if *fresh == *value || at.elapsed() > Duration::from_millis(1500) {
                    *pending = None;
                } else {
                    *fresh = value.clone();
                }
            }
        }
        let mut fresh = self.controller.snapshot();
        keep(&mut self.pending_profile, &mut fresh.profile);
        keep(&mut self.pending_settings, &mut fresh.settings);
        let old = &self.state;
        // Everything drawn outside the live meters.
        let changed = old.profile != fresh.profile
            || old.settings != fresh.settings
            || old.monitors != fresh.monitors
            || old.windows != fresh.windows
            || old.devices != fresh.devices
            || old.tracks != fresh.tracks
            || old.notice != fresh.notice
            || old.active != fresh.active
            || old.paused != fresh.paused
            || old.hotkey != fresh.hotkey
            || old.startup != fresh.startup
            || old.update != fresh.update
            || old.progress != fresh.progress
            || old.mic_preview != fresh.mic_preview
            || old.mic_preview_ready != fresh.mic_preview_ready
            || (old.mic_preview_level * 40.).round() != (fresh.mic_preview_level * 40.).round()
            || old.directory != fresh.directory;
        let changed = changed
            || old.seconds != fresh.seconds
            || old.notices != fresh.notices
            || old.software_encoding != fresh.software_encoding;
        self.state = fresh;
        changed
    }

    /// Whether the replay tape layer should be drawn over the page.
    fn tape_visible(&self) -> bool {
        self.tab == Tab::Capture && !self.news && self.conflict.is_none()
    }

    /// Whether the microphone is currently open to the clip.
    fn talking(&self) -> bool {
        let mic = self.gain("mic");
        self.state.active
            && mic.include
            && mic.volume > 0
            && (self.test_talk
                || (self.state.settings.push_to_talk.is_some() && crate::hotkey::talk_open()))
    }

    // ---------- sliders ----------

    fn slider_bounds(&mut self, key: Slider) -> Rc<RefCell<Bounds<Pixels>>> {
        self.sliders.entry(key).or_default().clone()
    }

    fn track_slider(&mut self, id: &str) -> Slider {
        let index = self
            .track_ids
            .iter()
            .position(|known| known == id)
            .unwrap_or_else(|| {
                self.track_ids.push(id.to_string());
                self.track_ids.len() - 1
            });
        Slider::Track(index)
    }

    fn slider_value(&self, key: Slider) -> u16 {
        if let Some((k, v)) = self.drag
            && k == key
        {
            return v;
        }
        match key {
            Slider::Track(i) => self.track_ids.get(i).map_or(100, |id| self.gain(id).volume),
            Slider::Mic => self.gain("mic").volume,
        }
    }

    fn drag_to(&mut self, key: Slider, position: Point<Pixels>) {
        let Some(bounds) = self.sliders.get(&key).map(|b| *b.borrow()) else {
            return;
        };
        let width: f32 = bounds.size.width.into();
        if width <= 0. {
            return;
        }
        let x: f32 = (position.x - bounds.origin.x).into();
        let frac = (x / width).clamp(0., 1.);
        let value = match key {
            Slider::Track(_) => (frac * 100.).round() as u16,
            Slider::Mic => ((frac * 200. / 5.).round() * 5.) as u16,
        };
        if self.drag.map(|d| d.1) == Some(value) && self.drag.map(|d| d.0) == Some(key) {
            return;
        }
        self.drag = Some((key, value));
        match key {
            Slider::Track(i) => {
                if let Some(id) = self.track_ids.get(i).cloned() {
                    self.set_gain(&id, |g| g.volume = value);
                }
            }
            Slider::Mic => self.set_gain("mic", |g| g.volume = value),
        }
    }

    /// Horizontal slider with a 4px track and a 16px thumb.
    fn slider(
        &mut self,
        th: &Theme,
        key: Slider,
        max: f32,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let value = self.slider_value(key) as f32;
        let frac = (value / max).clamp(0., 1.);
        let cell = self.slider_bounds(key);
        let id = match key {
            Slider::Track(i) => ElementId::from(("track-slider", i)),
            Slider::Mic => ElementId::from("mic-slider"),
        };
        div()
            .id(id)
            .tab_stop(true)
            .relative()
            .flex_none()
            .w(px(150.))
            .h(px(22.))
            .cursor_pointer()
            .child(
                canvas(
                    move |bounds, _, _| *cell.borrow_mut() = bounds,
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(9.))
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(th.switch_off),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(9.))
                    .h(px(4.))
                    .w(relative(frac))
                    .rounded(px(2.))
                    .bg(if enabled { th.chip_on } else { th.faint }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(8.))
                    .right(px(8.))
                    .top(px(3.))
                    .h(px(16.))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(relative(frac))
                            .ml(px(-8.))
                            .w(px(16.))
                            .h(px(16.))
                            .rounded_full()
                            .bg(th.text),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.drag_done = false;
                    this.drag_to(key, ev.position);
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                let delta: i32 = match ev.keystroke.key.as_str() {
                    "left" | "down" => -5,
                    "right" | "up" => 5,
                    "home" => -200,
                    "end" => 200,
                    _ => return,
                };
                let limit = if key == Slider::Mic { 200 } else { 100 };
                let value = (i32::from(this.slider_value(key)) + delta).clamp(0, limit) as u16;
                match key {
                    Slider::Mic => this.set_gain("mic", |g| g.volume = value),
                    Slider::Track(i) => {
                        if let Some(id) = this.track_ids.get(i).cloned() {
                            this.set_gain(&id, |g| g.volume = value);
                        }
                    }
                }
                cx.stop_propagation();
                cx.notify();
            }))
    }

    /// Segmented control with a sliding indicator.
    #[allow(clippy::too_many_arguments)]
    fn segmented(
        &mut self,
        th: &Theme,
        id: &'static str,
        labels: Vec<String>,
        selected: usize,
        mono: bool,
        height: f32,
        radius: f32,
        accent: bool,
        apply: fn(&mut Studio, usize),
        cx: &mut Context<Self>,
    ) -> Div {
        let n = labels.len().max(1);
        let mem = self.seg.get(id).copied().unwrap_or(SegMem {
            from: selected as f32,
            to: selected,
            generation: 0,
        });
        let mem = if mem.to != selected {
            SegMem {
                from: mem.to as f32,
                to: selected,
                generation: mem.generation + 1,
            }
        } else {
            mem
        };
        self.seg.insert(id, mem);
        let (from, to, nf) = (mem.from, mem.to as f32, n as f32);
        let indicator = div()
            .absolute()
            .top_0()
            .h_full()
            .w(relative(1. / nf))
            .left(relative(from / nf))
            .rounded(px(radius - 3.))
            .bg(if accent { th.accent } else { th.chip_on })
            .with_animation(
                (id, 1000 + mem.generation),
                Animation::new(Duration::from_millis(240)),
                move |el, d| el.left(relative((from + (to - from) * widgets::ease_out(d)) / nf)),
            );
        let mut row = div().relative().flex().w_full();
        for (i, label) in labels.into_iter().enumerate() {
            let on = i == selected;
            let color = if on {
                if accent {
                    th.on_accent
                } else {
                    th.chip_on_text
                }
            } else {
                th.secondary
            };
            let hover = th.text;
            row = row.child(
                div()
                    .id((id, i))
                    .tab_stop(true)
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(height))
                    .rounded(px(radius - 3.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .font_family(if mono { theme::mono() } else { theme::sans() })
                    .font_weight(if on {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(color)
                    .when(!on, move |d| d.hover(move |s| s.text_color(hover)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        apply(this, i);
                        cx.notify();
                    })),
            );
        }
        div()
            .relative()
            .w_full()
            .p(px(4.))
            .rounded(px(radius + 2.))
            .bg(th.field)
            .border_1()
            .border_color(th.field_border)
            .child(
                div()
                    .absolute()
                    .top(px(4.))
                    .bottom(px(4.))
                    .left(px(4.))
                    .right(px(4.))
                    .child(indicator),
            )
            .child(row)
    }

    /// Quiet status note (errors and hints from the engine). Success is never shown.
    /// It floats at the bottom of the page so appearing never pushes the layout around.
    fn notice_toast(&self, th: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = self.state.notice.clone();
        if text.is_empty() || text.starts_with("Clip guardado") {
            return None;
        }
        let bad = text.starts_with("No se pudo")
            || text.contains("inválid")
            || text.contains("Elige un atajo");
        let update = self.state.update.is_some() && text == "Hay una actualización disponible.";
        Some(
            div()
                .id("notice-toast")
                .absolute()
                .left(px(112.))
                .bottom(px(18.))
                .max_w(px(560.))
                .px(px(14.))
                .py(px(9.))
                .rounded(px(12.))
                .bg(th.menu)
                .border_1()
                .border_color(th.ghost_border)
                .shadow_lg()
                .text_size(px(12.))
                .line_height(relative(1.5))
                .text_color(if bad { th.warn_text } else { th.secondary })
                .child(text)
                .when(update, |d| {
                    d.cursor_pointer()
                        .child(" Pulsa para descargar.")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_tab(Tab::Settings, cx);
                            this.scroll_target = None;
                            this.scroll.scroll_to_top_of_item(5);
                        }))
                })
                .into_any_element(),
        )
    }

    fn source_label(&self) -> String {
        let p = &self.state.profile;
        let list = if p.monitor {
            &self.state.monitors
        } else {
            &self.state.windows
        };
        list.iter()
            .find(|s| s.id == p.id)
            .map(|s| s.label.clone())
            .unwrap_or_else(|| {
                if p.monitor {
                    "Pantalla completa".into()
                } else {
                    "Ventana del juego".into()
                }
            })
    }
}

impl Render for Studio {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.animate_scroll(window);
        let th = self.theme();
        let tab = self.tab;
        let page_children: Vec<AnyElement> = match tab {
            Tab::Capture => self.capture(&th, cx),
            Tab::Clips => self.clips(&th, cx),
            Tab::Settings => self.settings(&th, cx),
        };
        let page = div()
            .id(("page", tab as usize))
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(if tab == Tab::Settings { 24. } else { 0. }))
            .pt(px(22.))
            .pr(px(40.))
            .pb(px(56.))
            .pl(px(14.))
            // Hidden rather than scroll: the wheel is eased by `scroll_by` instead.
            .overflow_hidden()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                let delta = f32::from(ev.delta.pixel_delta(window.line_height()).y);
                let from = this
                    .scroll_target
                    .unwrap_or_else(|| f32::from(this.scroll.offset().y));
                this.scroll_to(from + delta, cx);
            }))
            .children(page_children)
            .with_animation(
                ("page-in", self.tab_gen),
                Animation::new(Duration::from_millis(260)),
                |el, d| {
                    let k = widgets::ease_out(d);
                    el.opacity(k).mt(px(12. * (1. - k)))
                },
            );
        let rail = self.rail(&th, cx);
        let news = self.news.then(|| self.news_sheet(&th, cx));
        let conflict = self.conflict.map(|c| self.conflict_modal(&th, c, cx));
        let select = self.select_overlay(&th, window, cx);
        let toast = self.notice_toast(&th, cx);
        div()
            .id("cut-root")
            .on_action(cx.listener(|_, _: &NextFocus, window, _| window.focus_next()))
            .on_action(cx.listener(|_, _: &PreviousFocus, window, _| window.focus_prev()))
            .on_action(cx.listener(|s, _: &Escape, _, cx| {
                if s.state.mic_preview {
                    s.controller.send(C::MicPreview(false));
                }
                if s.select.take().is_some() {
                } else if s.conflict.take().is_some() {
                    s.flash = "Los atajos no cambiaron.".into();
                } else if s.editing.is_some() {
                    s.cancel_edit("Edición cancelada.");
                } else if s.news {
                    s.close_news(cx);
                } else if s.menu {
                    s.menu = false;
                }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some((key, _)) = this.drag {
                    if ev.pressed_button == Some(MouseButton::Left) {
                        this.drag_to(key, ev.position);
                        cx.notify();
                    } else {
                        this.drag_done = true;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if this.drag.is_some() {
                        this.drag_done = true;
                    }
                }),
            )
            .relative()
            .size_full()
            .overflow_hidden()
            .text_color(th.text)
            .font_family(theme::sans())
            .text_size(px(16.))
            .flex()
            .flex_col()
            // Room for the title bar, which `Shell` draws (see there).
            .child(div().flex_none().h(px(46.)))
            // Focus lives here, not on the root: a focusable element under the title bar
            // would claim every press there and Windows would never move the window.
            .child(
                div()
                    .id("content")
                    .track_focus(&self.focus)
                    .flex_1()
                    .flex()
                    .min_h(px(0.))
                    .child(rail)
                    .child(page),
            )
            .children(toast)
            .children(select)
            .children(news)
            .children(conflict)
    }
}

pub fn run() -> crate::replay::Result<()> {
    // Cut 0.2.x left its mixer settings here but never wrote `seen_version`: if the file
    // exists before this run touches it, this is an update rather than a first install.
    let returning = crate::mixer_cache::MixerCache::path().exists();
    let instance = if std::env::var_os("CUT_NATIVE_VALIDATION").is_some() {
        None
    } else {
        let Some(instance) = crate::instance::Instance::open()? else {
            return Ok(());
        };
        Some(instance)
    };
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            fonts::register(cx);
            cx.bind_keys([
                KeyBinding::new("tab", NextFocus, None),
                KeyBinding::new("shift-tab", PreviousFocus, None),
                KeyBinding::new("escape", Escape, None),
            ]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1180.), px(780.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1000.), px(680.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Cut".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    let hwnd = match window.window_handle().unwrap().as_raw() {
                        raw_window_handle::RawWindowHandle::Win32(h) => h.hwnd.get(),
                        _ => 0,
                    };
                    let controller = crate::controller::Controller::new();
                    let tray_icon = if std::env::var_os("CUT_NATIVE_VALIDATION").is_some() {
                        None
                    } else {
                        match tray::create() {
                            Ok(icon) => Some(icon),
                            Err(error) => {
                                controller.send(C::Error(format!(
                                    "No se pudo crear la bandeja: {error}"
                                )));
                                None
                            }
                        }
                    };
                    let has_tray = tray_icon.is_some();
                    let close_controller = controller.clone();
                    window.on_window_should_close(cx, move |_, _| {
                        crate::hotkey::set_editing(false);
                        crate::hotkey::take_captured();
                        close_controller.send(C::Editing(false));
                        if preferences::Preferences::load().minimize_on_close {
                            tray::minimize(hwnd);
                        } else if has_tray {
                            tray::hide(hwnd);
                        } else {
                            close_controller.send(C::Exit);
                        }
                        false
                    });
                    let studio = cx.new(move |cx: &mut Context<Studio>| {
                        let focus = cx.focus_handle();
                        window.focus(&focus);
                        let state = controller.snapshot();
                        let mut prefs = preferences::Preferences::load();
                        let light = prefs.is_light();
                        // Una actualización mantiene las novedades pendientes hasta cerrarlas.
                        // En una instalación nueva no se interrumpe el primer inicio.
                        let current = env!("CARGO_PKG_VERSION");
                        let news_from = prefs.seen_version.clone();
                        let updated = prefs.needs_news(returning, current);
                        let news = updated;
                        if !updated && prefs.seen_version != current {
                            prefs.seen_version = current.into();
                            let _ = prefs.save();
                        }
                        let paused = prefs.paused;
                        let sea = cx.new(|_| sea::SeaView::new(paused, light));
                        let tape = cx.new(|_| tape::TapeView::new(paused, light));
                        cx.spawn(async move |this, cx| {
                            loop {
                                cx.background_executor()
                                    .timer(Duration::from_millis(120))
                                    .await;
                                if this
                                    .update(cx, |this, cx| {
                                        tray::poll(this.hwnd, &this.controller);
                                        if this
                                            ._instance
                                            .as_ref()
                                            .is_some_and(|instance| instance.take_open_request())
                                        {
                                            tray::show(this.hwnd);
                                        }
                                        let visible = tray::visible(this.hwnd);
                                        if !tray::active(this.hwnd) {
                                            this.stop_talk_preview();
                                            if this.state.mic_preview {
                                                this.controller.send(C::MicPreview(false));
                                            }
                                        }
                                        if !visible {
                                            this.select = None;
                                            this.cancel_edit("");
                                            this.stop_talk_preview();
                                            this.drag = None;
                                        }
                                        if this.visible != visible {
                                            this.visible = visible;
                                            this.controller.send(C::Visible(visible));
                                        }
                                        let mut redraw = this.sync_state();
                                        if let Some(talk) = this.editing {
                                            redraw = true;
                                            if let Some(binding) = crate::hotkey::take_captured() {
                                                this.editing = None;
                                                crate::hotkey::set_editing(false);
                                                this.controller.send(C::Editing(false));
                                                if binding.is_escape() {
                                                    this.flash = "Edición cancelada.".into();
                                                } else {
                                                    this.captured(talk, binding);
                                                }
                                            }
                                        }
                                        if this.drag_done {
                                            this.drag = None;
                                            this.drag_done = false;
                                            redraw = true;
                                        }
                                        if this.refreshing.is_some() {
                                            redraw = true;
                                            if this.refreshing.is_some_and(|t| {
                                                t.elapsed() > Duration::from_millis(1100)
                                            }) {
                                                this.refreshing = None;
                                            }
                                        }
                                        this.frame = this.frame.wrapping_add(1);
                                        if this.prefs.system && this.frame % 13 == 0 {
                                            let light = this.prefs.is_light();
                                            redraw |= light != this.light;
                                            this.light = light;
                                        }
                                        let (paused, light) = (this.prefs.paused, this.light);
                                        this.sea.update(cx, |s, cx| {
                                            if s.light != light {
                                                cx.notify();
                                            }
                                            s.paused = paused;
                                            s.visible = visible;
                                            s.light = light;
                                        });
                                        this.tape.update(cx, |t, cx| {
                                            if t.light != light {
                                                cx.notify();
                                            }
                                            t.visible = visible && this.tab == Tab::Capture;
                                            t.light = light;
                                            t.observe(
                                                this.state.seconds,
                                                this.state.profile.seconds,
                                                this.state.active,
                                            );
                                            let level = this
                                                .state
                                                .levels
                                                .values()
                                                .copied()
                                                .fold(0f64, f64::max);
                                            t.sample_level(level);
                                        });
                                        if this.state.exit_ready {
                                            cx.quit();
                                        }
                                        // The page is rebuilt only when something on it changed;
                                        // the live meters on Captura are the one steady exception.
                                        if visible && (redraw || this.tab == Tab::Capture) {
                                            cx.notify();
                                        }
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                        Studio {
                            hwnd,
                            _tray: tray_icon,
                            _instance: instance,
                            prefs,
                            light,
                            tab: Tab::Capture,
                            tab_gen: 0,
                            news,
                            news_closing: false,
                            news_gen: 0,
                            news_from,
                            focus,
                            editing: None,
                            conflict: None,
                            flash: "".into(),
                            last_talk: None,
                            menu: false,
                            refreshing: None,
                            test_talk: false,
                            visible: true,
                            frame: 0,
                            controller,
                            state,
                            sea,
                            tape,
                            seg: HashMap::new(),
                            sliders: HashMap::new(),
                            track_ids: Vec::new(),
                            drag: None,
                            drag_done: false,
                            scroll: ScrollHandle::new(),
                            scroll_target: None,
                            scroll_tick: None,
                            select: None,
                            select_bounds: HashMap::new(),
                            pending_profile: None,
                            pending_settings: None,
                            tape_slot: Rc::default(),
                        }
                    });
                    cx.new(|cx| Shell::new(studio, cx))
                },
            )
            .unwrap();
            cx.activate(true);
        });
    Ok(())
}

/// Window root. Three layers, each redrawn on its own: the sea (24 FPS), the page
/// (only when something on it changes) and the replay tape (24 FPS, on Captura).
/// One clock drives both animations so they share frames.
struct Shell {
    studio: Entity<Studio>,
    sea: Entity<sea::SeaView>,
    tape: Entity<tape::TapeView>,
    slot: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

impl Shell {
    fn new(studio: Entity<Studio>, cx: &mut Context<Self>) -> Self {
        let (sea, tape, slot) = {
            let s = studio.read(cx);
            (s.sea.clone(), s.tape.clone(), s.tape_slot.clone())
        };
        cx.observe(&studio, |_, _, cx| cx.notify()).detach();
        cx.spawn(async move |this, cx| {
            let mut prev = std::time::Instant::now();
            let mut idle = false;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(if idle { 250 } else { 41 }))
                    .await;
                let now = std::time::Instant::now();
                let dt = (now - prev).as_secs_f64();
                prev = now;
                let alive = this.update(cx, |shell, cx| {
                    let mut running = false;
                    let show_tape = shell.studio.read(cx).tape_visible();
                    shell.sea.update(cx, |s, cx| {
                        if s.advance(dt.min(0.1)) {
                            running = true;
                            cx.notify();
                        }
                    });
                    shell.tape.update(cx, |t, cx| {
                        if t.advance(dt) && show_tape {
                            running = true;
                            cx.notify();
                        }
                    });
                    !running
                });
                match alive {
                    Ok(sleep) => idle = sleep,
                    Err(_) => break,
                }
            }
        })
        .detach();
        Self {
            studio,
            sea,
            tape,
            slot,
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let studio = self.studio.read(cx);
        let tape = self.slot.get().filter(|_| studio.tape_visible());
        // The title bar is drawn here, outside the cached page: GPUI does not carry the
        // drag and caption areas over when it reuses a cached view, so inside the page
        // they would vanish on every frame the sea alone redraws.
        let th = studio.theme();
        let titlebar = studio
            .titlebar(&th)
            .text_color(th.text)
            .font_family(theme::sans())
            .text_size(px(16.));
        div()
            .size_full()
            .relative()
            .child(div().absolute().top_0().left_0().size_full().child(
                AnyView::from(self.sea.clone()).cached(StyleRefinement::default().size_full()),
            ))
            .child(
                AnyView::from(self.studio.clone()).cached(StyleRefinement::default().size_full()),
            )
            .when_some(tape, |d, b| {
                d.child(
                    div()
                        .absolute()
                        .left(b.origin.x)
                        .top(b.origin.y)
                        .w(b.size.width)
                        .h(b.size.height)
                        .child(
                            AnyView::from(self.tape.clone())
                                .cached(StyleRefinement::default().size_full()),
                        ),
                )
            })
            .child(div().absolute().top_0().left_0().w_full().child(titlebar))
    }
}
