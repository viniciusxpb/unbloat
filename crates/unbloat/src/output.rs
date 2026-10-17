use crate::{Result, config::Output, error, i18n, manifest::Manifest, scanner::root_name};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TemporaryOutput {
    pub path: PathBuf,
}
impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
impl TemporaryOutput {
    pub fn create(directory: &Path) -> Result<(Self, File)> {
        loop {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let path = directory.join(format!(
                ".unbloat-{}-{time}-{id}.unbloat-part",
                std::process::id()
            ));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok((Self { path }, file)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(error(i18n::message(
                        "output.temporary_failed",
                        &[&crate::display_path(directory), &e],
                    )));
                }
            }
        }
    }

    pub fn publish(self, directory: &Path, stem: &str) -> Result<PathBuf> {
        for index in 1u64.. {
            let name = if index == 1 {
                format!("{stem}.7z")
            } else {
                format!("{stem}-{index}.7z")
            };
            let final_path = directory.join(name);
            // MoveFileEx without REPLACE_EXISTING is an atomic no-clobber publication.
            #[cfg(windows)]
            let result = {
                use std::os::windows::ffi::OsStrExt;
                unsafe extern "system" {
                    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
                }
                let from: Vec<u16> = self.path.as_os_str().encode_wide().chain(Some(0)).collect();
                let to: Vec<u16> = final_path
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect();
                if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 8) } != 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            };
            #[cfg(not(windows))]
            let result = std::fs::hard_link(&self.path, &final_path);
            match result {
                Ok(()) => return Ok(final_path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(error(i18n::message(
                        "output.publish_failed",
                        &[&crate::display_path(&final_path), &e],
                    )));
                }
            }
        }
        unreachable!()
    }
}

pub fn stem(manifest: &Manifest, config: &Output) -> Result<String> {
    if manifest.roots.len() == 1 {
        return Ok(format!(
            "{}{}",
            root_name(&manifest.roots[0])?,
            config.suffix
        ));
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let t = unsafe { GetLocalTime() };
        Ok(format!(
            "Unbloat-{:04}{:02}{:02}-{:02}{:02}{:02}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        ))
    }
    #[cfg(not(windows))]
    {
        Ok(format!(
            "Unbloat-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()
        ))
    }
}
