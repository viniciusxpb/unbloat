//! Thin COM context menu handler. The compression engine is never loaded into Explorer.
#[cfg(windows)]
mod com;
#[path = "../../../shared/i18n.rs"]
pub mod i18n;
#[cfg(windows)]
mod menu_icon;
#[cfg(windows)]
pub mod selection;
