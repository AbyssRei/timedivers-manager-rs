//! Browser picker dialog and the scrape entry point it feeds.

use gpui_kit::component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    notification::Notification,
    v_flex,
};
use gpui_kit::*;

use super::main_view::AppView;
use super::scraper_log::ScraperLogView;
use super::{push_notification, t};
use crate::scraper::{self, BrowserInfo};
use crate::util::window_options;

/// Starts a scrape for the currently selected game, opening the scraper log
/// window on success or an error notification otherwise.
pub(super) fn start_scrape(
    weak: &gpui_kit::WeakEntity<AppView>,
    browser: BrowserInfo,
    window: &mut Window,
    cx: &mut App,
) {
    weak.update(cx, |view, cx| {
        let Some(game) = view.current_game().cloned() else {
            return;
        };
        match scraper::start_scraper(
            game.app_id.clone(),
            game.depots.clone(),
            browser,
            view.paths.data_dir.clone(),
        ) {
            Ok(session) => ScraperLogView::open(view.weak.clone(), session, cx),
            Err(err) => push_notification(window, cx, Notification::error(format!("{err:#}"))),
        }
    })
    .ok();
}

/// Small modal window for choosing which browser to scrape with (mirrors the
/// original `Select Browser` dialog).
pub(super) struct BrowserSelectView {
    browsers: Vec<BrowserInfo>,
    selected: Option<usize>,
    app_view: gpui_kit::WeakEntity<AppView>,
}

impl BrowserSelectView {
    pub(super) fn open(
        app_view: gpui_kit::WeakEntity<AppView>,
        browsers: Vec<BrowserInfo>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let _ = open_window(
            window_options(
                t("scraper.select_browser_title"),
                (320., 260.),
                (320., 220.),
                true,
            ),
            cx,
            |_, cx| {
                cx.new(|_| Self {
                    browsers,
                    selected: Some(0),
                    app_view,
                })
            },
        );
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            let browser = self.browsers[ix].clone();
            let app_view = self.app_view.clone();
            window.remove_window();
            start_scrape(&app_view, browser, window, cx);
        }
    }

    fn on_select_click(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(window, cx);
    }
}

impl Render for BrowserSelectView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let list = uniform_list(
            "browsers",
            self.browsers.len(),
            move |range, _window, cx| {
                let this = view.read(cx);
                let hover = cx.theme().list_hover;
                let active = cx.theme().list_active;
                let fg = cx.theme().foreground;
                range
                    .map(|ix| {
                        let selected = this.selected == Some(ix);
                        let row_view = view.clone();
                        let name = this.browsers[ix].name.clone();
                        div()
                            .id(ix)
                            .w_full()
                            .h(px(28.))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded_sm()
                            .text_color(fg)
                            .bg(if selected {
                                active
                            } else {
                                hsla(0., 0., 0., 0.)
                            })
                            .hover(move |style| style.bg(hover))
                            .on_click(move |_, _window, cx| {
                                row_view.update(cx, |v, cx| {
                                    v.selected = Some(ix);
                                    cx.notify();
                                });
                            })
                            .child(SharedString::from(format!("  {name}")))
                    })
                    .collect()
            },
        )
        .flex_1()
        .min_h_0();

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .p_4()
            .gap_2()
            .child(div().child(t("scraper.select_browser")))
            .child(list)
            .child(
                div().flex().justify_center().child(
                    Button::new("select")
                        .label(t("scraper.select"))
                        .primary()
                        .small()
                        .on_click(cx.listener(Self::on_select_click)),
                ),
            )
    }
}
