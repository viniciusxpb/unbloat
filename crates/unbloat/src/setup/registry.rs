//! The few HKEY_CURRENT_USER operations the per-user installation needs.
use windows::{
    Win32::{Foundation::*, System::Registry::*},
    core::{HSTRING, PCWSTR, Result},
};

/// An open key, closed on drop.
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn create(path: &str) -> Result<Key> {
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()?;
    }
    Ok(Key(key))
}

/// Sets a string value, `None` meaning the key's default value.
pub fn set(path: &str, name: Option<&str>, value: &str) -> Result<()> {
    let key = create(path)?;
    let data: Vec<u8> = value
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let name = name.map(HSTRING::from);
    unsafe {
        RegSetValueExW(
            key.0,
            name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            None,
            REG_SZ,
            Some(&data),
        )
        .ok()
    }
}

/// The key's default string value, if the key exists and has one.
pub fn get(path: &str) -> Option<String> {
    let path = HSTRING::from(path);
    let mut size = 0u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &path,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        RegGetValueW(
            HKEY_CURRENT_USER,
            &path,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let length = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..length]))
    }
}

/// Deletes a key with everything below it; a missing key is not an error.
pub fn delete(path: &str) -> Result<()> {
    let path = HSTRING::from(path);
    let tolerate = |error: WIN32_ERROR| match error {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        error => error.ok(),
    };
    // The tree call empties the key; the key call then removes the key itself.
    unsafe {
        tolerate(RegDeleteTreeW(HKEY_CURRENT_USER, &path))?;
        tolerate(RegDeleteKeyW(HKEY_CURRENT_USER, &path))
    }
}
