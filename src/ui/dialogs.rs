use std::rc::Rc;

use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    h_flex,
};
use gpui_kit::*;

/// Callback invoked when the dialog's OK button is pressed.
pub(crate) type DialogOkHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Everything a confirmation/message dialog needs. `on_ok` runs after the
/// dialog has been dismissed, so it may open a follow-up dialog.
#[derive(Clone)]
pub(crate) struct DialogSpec {
    pub title: SharedString,
    pub message: SharedString,
    pub ok_text: SharedString,
    pub ok_variant: ButtonVariant,
    pub cancel_text: Option<SharedString>,
    pub on_ok: Option<DialogOkHandler>,
}

impl DialogSpec {
    /// OK-only information/warning dialog.
    pub fn info(title: impl Into<SharedString>, message: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            ok_text: rust_i18n::t!("common.ok").to_string().into(),
            ok_variant: ButtonVariant::Primary,
            cancel_text: None,
            on_ok: None,
        }
    }

    /// OK + Cancel confirmation; `on_ok` fires only when OK is pressed.
    pub fn confirm(
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
        on_ok: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            cancel_text: Some(rust_i18n::t!("common.cancel").to_string().into()),
            on_ok: Some(Rc::new(on_ok)),
            ..Self::info(title, message)
        }
    }

    pub fn ok_text(mut self, ok_text: impl Into<SharedString>) -> Self {
        self.ok_text = ok_text.into();
        self
    }

    pub fn ok_variant(mut self, ok_variant: ButtonVariant) -> Self {
        self.ok_variant = ok_variant;
        self
    }
}

/// Opens a modal dialog whose footer buttons are rendered explicitly
/// (imperative `open_dialog` does not render `button_props` buttons).
pub(crate) fn show_dialog(window: &mut Window, cx: &mut App, spec: DialogSpec) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let title = spec.title.clone();
        let message = spec.message.clone();
        let ok_text = spec.ok_text.clone();
        let ok_variant = spec.ok_variant;
        let cancel_text = spec.cancel_text.clone();
        let on_ok = spec.on_ok.clone();
        dialog
            .title(title)
            .content(move |content, _, _| {
                let message = message.clone();
                content.child(div().child(message))
            })
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .children(cancel_text.map(|cancel_text| {
                        Button::new("dialog-cancel")
                            .small()
                            .label(cancel_text)
                            .on_click(|_, window, cx| window.close_dialog(cx))
                    }))
                    .child(
                        Button::new("dialog-ok")
                            .small()
                            .with_variant(ok_variant)
                            .label(ok_text)
                            .on_click(move |event, window, cx| {
                                window.close_dialog(cx);
                                if let Some(on_ok) = &on_ok {
                                    on_ok(event, window, cx);
                                }
                            }),
                    ),
            )
    });
}
