use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    notification::Notification,
    v_flex,
};
use gpui_kit::*;

use crate::scraper::{ScraperEvent, ScraperSession};
use crate::util::window_options;

/// The scraper log window (650x400, monospace), mirroring `ScraperLogWindow`.
pub struct ScraperLogView {
    text: String,
    cont: async_channel::Sender<()>,
    continued: bool,
    done: bool,
    scroll_handle: gpui_kit::ScrollHandle,
    app_view: gpui_kit::WeakEntity<super::main_view::AppView>,
}

impl ScraperLogView {
    pub fn open(
        app_view: gpui_kit::WeakEntity<super::main_view::AppView>,
        session: ScraperSession,
        cx: &mut App,
    ) {
        let events = Arc::new(session.events);
        let (any_handle, view) = open_window(
            window_options("Scraper", (160., 160.), (650., 400.), false),
            cx,
            |_, cx| {
                cx.new(|_| Self {
                    text: String::new(),
                    cont: session.cont,
                    continued: false,
                    done: false,
                    scroll_handle: gpui_kit::ScrollHandle::new(),
                    app_view,
                })
            },
        )
        .expect("open scraper log window");
        // The window handle's root type is the framework's Root wrapper, so it
        // cannot be downcast to the content view; drive the view through its
        // typed Entity and use the untyped handle only for window-level APIs.
        cx.spawn(async move |cx: &mut AsyncApp| {
            // The loop holds a strong view handle, so updates always succeed
            // even after the window is closed; the loop exits when the scraper
            // session closes the event channel.
            while let Ok(event) = events.recv().await {
                match event {
                    ScraperEvent::Log(line) => {
                        view.update(cx, |view, cx| {
                            view.append(&line);
                            cx.notify();
                        });
                    }
                    ScraperEvent::ReadyForLogin => {}
                    ScraperEvent::Finished(result) => {
                        let mut succeeded = false;
                        view.update(cx, |view, cx| {
                            view.append(
                                "
Scraping complete.",
                            );
                            view.done = true;
                            match result {
                                Ok(_manifest_path) => {
                                    succeeded = true;
                                    view.app_view
                                        .update(cx, |app, cx| app.on_scraper_finished(cx))
                                        .ok();
                                }
                                Err(err) => view.append(&format!(
                                    "
Error: {err:#}
"
                                )),
                            }
                            cx.notify();
                        });
                        if succeeded {
                            any_handle
                                .update(cx, |_root, window, cx| {
                                    window.push_notification(
                                        Notification::success(
                                            rust_i18n::t!("scraper.update_complete").to_string(),
                                        ),
                                        cx,
                                    );
                                })
                                .ok();
                        }
                    }
                }
            }
        })
        .detach();
    }

    fn append(&mut self, line: &str) {
        self.text.push_str(line);
        self.scroll_handle.scroll_to_bottom();
    }

    fn on_continue(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.continued {
            return;
        }
        self.continued = true;
        let _ = self.cont.try_send(());
        cx.notify();
    }

    fn on_close(&mut self, _: &gpui_kit::ClickEvent, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }
}

impl Render for ScraperLogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .gap_2()
            .p_3()
            .child(
                div()
                    .id("log-text")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().input)
                    .font_family("Consolas")
                    .text_size(px(12.))
                    .min_w_0()
                    .child(SharedString::from(self.text.clone())),
            )
            .child(
                div()
                    .flex()
                    .justify_center()
                    .gap_2()
                    .child(
                        Button::new("continue")
                            .label(rust_i18n::t!("scraper.continue").to_string())
                            .primary()
                            .small()
                            .disabled(self.continued || self.done)
                            .on_click(cx.listener(Self::on_continue)),
                    )
                    .child(
                        Button::new("close")
                            .label(rust_i18n::t!("common.close").to_string())
                            .small()
                            .disabled(!self.done)
                            .on_click(cx.listener(Self::on_close)),
                    ),
            )
    }
}
