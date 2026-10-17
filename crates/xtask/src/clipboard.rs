//! Plain-text clipboard: the Win32 API on Windows, the usual system tools elsewhere.
use crate::Result;

#[cfg(windows)]
pub fn copy(text: &str) -> Result {
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE},
        System::{DataExchange::*, Memory::*, Ole::CF_UNICODETEXT},
    };
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        // Another program may be holding the clipboard for a moment.
        let mut attempts = 0;
        while let Err(e) = OpenClipboard(None) {
            attempts += 1;
            if attempts == 20 {
                return Err(e.into());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let result = (|| -> Result {
            EmptyClipboard()?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * size_of::<u16>())?;
            let target = GlobalLock(memory).cast::<u16>();
            if target.is_null() {
                let error = windows::core::Error::from_thread();
                let _ = GlobalFree(Some(memory));
                return Err(error.into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
            let _ = GlobalUnlock(memory);
            // On success the clipboard owns the memory; otherwise it is still ours.
            if let Err(e) = SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0))) {
                let _ = GlobalFree(Some(memory));
                return Err(e.into());
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// An X11 or Wayland selection needs a process that stays alive to serve it,
/// which these tools provide; the first one that works wins.
#[cfg(not(windows))]
pub fn copy(text: &str) -> Result {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let tools: [(&str, &[&str]); 4] = [
        ("pbcopy", &[]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (program, args) in tools {
        let Ok(mut child) = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        let written = child
            .stdin
            .take()
            .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        if child.wait()?.success() && written {
            return Ok(());
        }
    }
    Err(crate::i18n::text("review.no_clipboard").into())
}
