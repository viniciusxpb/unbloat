//! Thin COM context menu handler. The compression engine is never loaded into Explorer.
#[cfg(windows)]
mod com;
#[cfg(windows)]
pub mod selection;
