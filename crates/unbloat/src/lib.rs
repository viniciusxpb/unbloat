//! Unbloat's manifest-based archive engine.
pub mod archiver;
pub mod config;
pub mod manifest;
pub mod output;
pub mod scanner;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub fn error(message: impl Into<String>) -> Error {
    std::io::Error::other(message.into()).into()
}

pub fn check_cancel(cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(error("Operação cancelada. Nenhum arquivo foi publicado."));
    }
    Ok(())
}
