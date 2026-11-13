//! The app's library half.
//!
//! The binary is a thin shell around these modules, and they are exposed here so
//! the end-to-end tests can drive the real runtime — the real listener, the real
//! handshake, the real decoder — rather than a mock of it.

#![allow(
    clippy::field_reassign_with_default,
    clippy::collapsible_if,
    clippy::manual_is_multiple_of,
    clippy::upper_case_acronyms,
    clippy::manual_range_contains,
    clippy::redundant_closure,
    clippy::unnecessary_cast
)]

pub mod adb;
pub mod logging;
pub mod runtime;
pub mod settings;
pub mod tray;
pub mod vcam;
