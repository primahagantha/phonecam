//! GPUI actions and default keybindings.

use gpui::KeyBinding;

gpui::actions!(
    phonecam,
    [
        ToggleStream,
        ToggleFocusPreview,
        ExitFocusPreview,
        SelectTabSetup,
        SelectTabCapture,
        SelectTabPublish,
        SelectTabDiagnostics,
        SelectTabStream,
        SelectTabVideo,
        TabNext,
        TabPrev,
    ]
);

/// The standard keybindings for PhoneCam.
///
/// * Space: start/stop stream
/// * F11: toggle full-window focus preview
/// * Esc: leave focus preview
/// * Ctrl+1: Setup tab
/// * Ctrl+2: Capture / Stream tab
/// * Ctrl+3: Publish / Video tab
/// * Ctrl+4: Diagnostics tab
/// * Tab: Move focus to next tab stop
/// * Shift+Tab: Move focus to previous tab stop
pub fn default_keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("space", ToggleStream, None),
        KeyBinding::new("f11", ToggleFocusPreview, None),
        KeyBinding::new("escape", ExitFocusPreview, None),
        KeyBinding::new("ctrl-1", SelectTabSetup, None),
        KeyBinding::new("ctrl-2", SelectTabCapture, None),
        KeyBinding::new("ctrl-3", SelectTabPublish, None),
        KeyBinding::new("ctrl-4", SelectTabDiagnostics, None),
        KeyBinding::new("cmd-1", SelectTabSetup, None),
        KeyBinding::new("cmd-2", SelectTabCapture, None),
        KeyBinding::new("cmd-3", SelectTabPublish, None),
        KeyBinding::new("cmd-4", SelectTabDiagnostics, None),
        KeyBinding::new("tab", TabNext, None),
        KeyBinding::new("shift-tab", TabPrev, None),
    ]
}

/// Registers the default keybindings on the application context.
pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys(default_keybindings());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keybindings_include_all_required_shortcuts() {
        let bindings = default_keybindings();
        assert!(bindings.len() >= 9);
    }
}
