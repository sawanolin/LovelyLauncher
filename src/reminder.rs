//! The reminder card: pops up in the bottom-right corner when a 备忘录 reminder is
//! due, without taking the keyboard from what is in front.

use gpui::{
    BoxShadow, ClickEvent, Context, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, WeakEntity,
    Window, div, hsla, point, prelude::*, px,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{launcher::Launcher, theme::Theme, view::icon, win};

/// Window size (logical px); the card sits inside with room for its shadow.
pub const WIDTH: f32 = 380.;
pub const HEIGHT: f32 = 240.;
const PAD: f32 = 12.;

#[derive(Clone)]
pub struct Card {
    pub id: String,
    pub text: String,
    /// When it was due, and whether it repeats ("今天 15:00 · 每天").
    pub when: String,
    /// Due a while ago (the launcher wasn't running then).
    pub late: bool,
}

pub struct ReminderPopup {
    launcher: WeakEntity<Launcher>,
    hwnd: Option<isize>,
    card: Option<Card>,
    /// Further reminders waiting behind this one.
    more: usize,
}

impl ReminderPopup {
    pub fn new(launcher: WeakEntity<Launcher>, window: &mut Window) -> Self {
        let hwnd = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
            _ => None,
        };
        Self { launcher, hwnd, card: None, more: 0 }
    }

    pub fn show(&mut self, card: Card, more: usize, cx: &mut Context<Self>) {
        let new = self.card.as_ref().map(|c| &c.id) != Some(&card.id);
        self.card = Some(card);
        self.more = more;
        if new && let Some(raw) = self.hwnd {
            // Not synchronously: moving the window re-enters GPUI.
            cx.spawn(async move |_, _| win::place_corner_rect(raw, WIDTH, HEIGHT, 8.)).detach();
        }
        cx.notify();
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.card = None;
        if let Some(raw) = self.hwnd {
            cx.spawn(async move |_, _| win::hide_window(raw)).detach();
        }
        cx.notify();
    }

    fn act(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut Launcher, &str, &mut Context<Launcher>) + 'static) {
        let Some(id) = self.card.as_ref().map(|c| c.id.clone()) else { return };
        self.launcher.update(cx, |launcher, cx| f(launcher, &id, cx)).ok();
    }
}

impl Render for ReminderPopup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().size_full().p(px(PAD));
        let Some(card) = self.card.clone() else {
            return root;
        };
        let theme = Theme::get(cx).clone();
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(30.))
                .px(px(12.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .text_size(px(12.5))
                .cursor_pointer()
                .when(primary, |d| d.bg(theme.accent).text_color(theme.accent_text).hover(|s| s.opacity(0.9)))
                .when(!primary, |d| {
                    d.bg(theme.kbd_bg)
                        .text_color(theme.text)
                        .border_1()
                        .border_color(theme.border)
                        .hover(|s| s.border_color(theme.accent).text_color(theme.accent))
                })
                .child(label)
        };
        let surface = if theme.dark { gpui::rgb(0x2a2329) } else { gpui::rgb(0xfffbfc) };
        root.child(
            div()
                .id("reminder-card")
                .size_full()
                .rounded(px(14.))
                .bg(surface)
                .border_1()
                .border_color(theme.accent.opacity(0.45))
                .shadow(vec![BoxShadow {
                    color: hsla(0., 0., 0., if theme.dark { 0.45 } else { 0.18 }),
                    offset: point(px(0.), px(4.)),
                    blur_radius: px(12.),
                    spread_radius: px(0.),
                }])
                .p(px(14.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .text_color(theme.text)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon("bell", 16., theme.accent))
                        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("备忘录提醒"))
                        .child(div().flex_1())
                        .when(self.more > 0, |d| {
                            d.child(
                                div()
                                    .px(px(7.))
                                    .py(px(1.))
                                    .rounded(px(9.))
                                    .bg(theme.accent.opacity(0.12))
                                    .text_size(px(11.))
                                    .text_color(theme.accent)
                                    .child(format!("还有 {} 条", self.more)),
                            )
                        })
                        .child(
                            div()
                                .id("reminder-close")
                                .size(px(24.))
                                .rounded(px(6.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(theme.row_hover))
                                .child(icon("x", 13., theme.text_muted))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.act(cx, |launcher, id, cx| launcher.dismiss_reminder(id, cx))
                                })),
                        ),
                )
                .child(
                    div()
                        .mt(px(-4.))
                        .text_size(px(11.5))
                        .text_color(if card.late { theme.danger } else { theme.text_faint })
                        .child(SharedString::from(if card.late { format!("{}（错过了）", card.when) } else { card.when.clone() })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .text_size(px(14.))
                        .line_height(px(21.))
                        .line_clamp(3)
                        .child(card.text.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(button("reminder-done", "完成", true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.act(cx, |launcher, id, cx| launcher.finish_memo(id, cx))
                        })))
                        .child(button("reminder-10", "10 分钟后", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.act(cx, |launcher, id, cx| launcher.snooze_memo(id, 600, cx))
                        })))
                        .child(button("reminder-60", "1 小时后", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.act(cx, |launcher, id, cx| launcher.snooze_memo(id, 3600, cx))
                        })))
                        .child(div().flex_1())
                        .child(button("reminder-open", "打开", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.act(cx, |launcher, id, cx| launcher.open_memo_from_reminder(id, cx))
                        }))),
                ),
        )
    }
}
