// Locale files live in `locales/` relative to the crate root.
rust_i18n::i18n!("locales", fallback = "en");

mod config;
mod constants;
mod download;
mod manifest;
mod paths;
mod scraper;
mod steam;
mod ui;
mod util;

use gpui_kit::*;

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        use gpui_kit::component as gpui_component;
        rust_i18n::extend!(gpui_component);

        init(cx);
        cx.activate(true);
        cx.set_app_identity(paths::APP_ID, paths::APP_NAME);

        // The original tool ships a dark Steam look; pin the built-in
        // Dark theme instead of following the OS appearance.
        gpui_kit::component::theme::Theme::change(
            gpui_kit::component::theme::ThemeMode::Dark,
            None,
            cx,
        );
        gpui_kit::component::theme::Theme::sync_scrollbar_appearance(cx);

        open_window(
            util::window_options("Timedivers Manager V2", (60., 60.), (900., 600.), false),
            cx,
            |window, cx| cx.new(|cx| ui::main_view::AppView::new(window, cx)),
        )
        .expect("Failed to open window");
    });
}
