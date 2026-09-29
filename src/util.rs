//! Small helpers shared across modules: size parsing and window setup.

use gpui_kit::{
    Bounds, SharedString, TitlebarOptions, WindowBounds, WindowKind, WindowOptions, point, px, size,
};

/// Converts a size text like `"157 MB"` / `"1.2 GB"` into bytes.
pub fn parse_size_text(text: &str) -> Option<u64> {
    let text = text.trim().to_lowercase();
    let mut split = text.splitn(2, ' ');
    let number: f64 = split.next()?.replace(',', "").parse().ok()?;
    let unit = split.next()?.trim();
    let multiplier = match unit {
        "b" => 1.0,
        "kb" => 1024.0,
        "mb" => 1024.0 * 1024.0,
        "gb" => 1024.0 * 1024.0 * 1024.0,
        "tb" => 1024.0f64 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

/// Standard options for the app's windows: fixed bounds and a plain
/// (non-transparent) titlebar. `dialog` pins the window to a fixed-size
/// modal; otherwise it is freely resizable like the main window.
pub fn window_options(
    title: impl Into<SharedString>,
    origin: (f32, f32),
    win_size: (f32, f32),
    dialog: bool,
) -> WindowOptions {
    let mut options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(origin.0), px(origin.1)),
            size: size(px(win_size.0), px(win_size.1)),
        })),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        ..Default::default()
    };
    if dialog {
        options.kind = WindowKind::Dialog;
        options.is_resizable = false;
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_size_units() {
        assert_eq!(parse_size_text("157 MB"), Some(157 * 1024 * 1024));
        assert_eq!(parse_size_text("1.2 GB"), Some(1_288_490_188));
        assert_eq!(parse_size_text("512 KB"), Some(512 * 1024));
        assert_eq!(parse_size_text("42 B"), Some(42));
    }

    #[test]
    fn rejects_bad_size_text() {
        assert_eq!(parse_size_text(""), None);
        assert_eq!(parse_size_text("abc"), None);
        assert_eq!(parse_size_text("12 XB"), None);
    }
}
