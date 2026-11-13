//! PhoneCam desktop UI in GPUI.
//!
//! The window answers one question â€” "is the picture flowing?" â€” and offers one
//! action at a time. Three rules carry most of that, and each replaced a
//! specific failure in the Electron build:
//!
//! * **One status headline.** Two true statements about different subsystems
//!   read as a contradiction, so the priority order lives in one tested
//!   function (see [`theme::rules`]).
//! * **One accent, and it means "act", not "healthy".**
//! * **One action per state**, in the place the user is already looking.
//!
//! The preview is never a tab. It is always visible, because an answer you have
//! to navigate to is not an answer.

#![allow(
    clippy::field_reassign_with_default,
    clippy::assertions_on_constants,
    clippy::derivable_impls,
    clippy::chunks_exact_to_as_chunks,
    clippy::type_complexity,
    clippy::question_mark,
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::obfuscated_if_else
)]

pub mod theme;
pub mod views;

pub use theme::rules;
pub use views::{
    bind_keys, default_keybindings, drain_effects, push_effect, AppView, ExitFocusPreview,
    SelectTabCapture, SelectTabDiagnostics, SelectTabPublish, SelectTabSetup, SelectTabStream,
    SelectTabVideo, SideEffect, Tab, TabNext, TabPrev, ToggleFocusPreview, ToggleStream,
};
