//! Unbloat's manifest-based archive engine.
pub mod archiver;
pub mod config;
#[path = "../../../shared/i18n.rs"]
pub mod i18n;
pub mod manifest;
pub mod output;
pub mod scanner;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub fn error(message: impl Into<String>) -> Error {
    std::io::Error::other(message.into()).into()
}

/// Shows a path without the `\\?\` prefix that `canonicalize` adds on Windows.
/// File operations keep the verbatim path; long paths keep the prefix so a
/// copied path still works in tools without long-path support.
pub fn display_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text
        .strip_prefix(r"\\?\")
        .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
    {
        rest.to_owned()
    } else {
        return text;
    };
    if plain.encode_utf16().count() < 260 {
        plain
    } else {
        text
    }
}

pub fn check_cancel(cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(error(i18n::text("engine.cancelled")));
    }
    Ok(())
}
