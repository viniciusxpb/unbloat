//! Reads the localized title and buttons of the real native launcher window.
use crate::Result;
use std::{
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{Foundation::*, UI::WindowsAndMessaging::*},
    core::BOOL,
};

/// Control ids from crates/unbloat/src/launcher.rs.
const INSTALL: i32 = 10;
const CLOSE: i32 = 20;

pub fn check_launcher(executable: &Path, tag: &str, text: &dyn Fn(&str) -> &'static str) -> Result {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000); // CREATE_NO_WINDOW: debug builds use the console subsystem.
    if tag.is_empty() {
        command.env_remove("UNBLOAT_LANG");
    } else {
        command.env("UNBLOAT_LANG", tag);
    }
    let mut child = command.spawn()?;
    let result = (|| -> Result {
        let window = wait_for_window(child.id(), &mut child)?;
        for (control, key) in [
            (None, "launcher.title"),
            (Some(INSTALL), "launcher.install"),
            (Some(CLOSE), "common.close"),
        ] {
            let handle = match control {
                None => window,
                Some(id) => unsafe { GetDlgItem(Some(window), id)? },
            };
            if window_text(handle) != text(key) {
                return Err(format!("Native control mismatch: {tag:?}/{key}").into());
            }
        }
        unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0))? };
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait()?.is_none() {
            if Instant::now() > deadline {
                return Err("Application did not close.".into());
            }
            thread::sleep(Duration::from_millis(30));
        }
        Ok(())
    })();
    if child.try_wait()?.is_none() {
        let _ = child.kill();
    }
    result
}

fn wait_for_window(process: u32, child: &mut std::process::Child) -> Result<HWND> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait()?.is_some() {
            return Err("Application closed before showing its window.".into());
        }
        if let Some(window) = find_window(process) {
            return Ok(window);
        }
        if Instant::now() > deadline {
            return Err("Application window did not appear.".into());
        }
        thread::sleep(Duration::from_millis(30));
    }
}

/// The visible launcher window ("UnbloatWelcome" class) owned by `process`.
fn find_window(process: u32) -> Option<HWND> {
    struct Search {
        process: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(window: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
        let mut class = [0u16; 64];
        let length = unsafe { GetClassNameW(window, &mut class) } as usize;
        if owner == search.process
            && unsafe { IsWindowVisible(window) }.as_bool()
            && String::from_utf16_lossy(&class[..length]) == "UnbloatWelcome"
        {
            search.found = Some(window);
            return false.into();
        }
        true.into()
    }
    let mut search = Search {
        process,
        found: None,
    };
    // EnumWindows reports an error when the callback stops early; `found` is the result.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    search.found
}

fn window_text(window: HWND) -> String {
    let mut buffer = [0u16; 1024];
    let length = unsafe { GetWindowTextW(window, &mut buffer) } as usize;
    String::from_utf16_lossy(&buffer[..length])
}
