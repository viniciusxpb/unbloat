//! The few HKEY_CURRENT_USER operations the per-user installation needs.
use windows::{
    Win32::{Foundation::*, System::Registry::*},
    core::{Error, HSTRING, PCWSTR, Result},
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

/// Only the values setup changes are saved. Unrelated values and subkeys are
/// left alone, and the original value types and bytes are restored verbatim.
pub struct Change<'a> {
    path: &'a str,
    name: Option<&'a str>,
    value: &'a str,
}

impl<'a> Change<'a> {
    pub fn new(path: &'a str, name: Option<&'a str>, value: &'a str) -> Self {
        Self { path, name, value }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Value {
    kind: REG_VALUE_TYPE,
    data: Vec<u8>,
}

#[derive(Clone)]
struct Saved {
    key_exists: bool,
    value: Option<Value>,
}

trait Backend {
    fn capture(&mut self, change: &Change) -> Result<Saved>;
    fn set(&mut self, change: &Change) -> Result<()>;
    fn restore(&mut self, change: &Change, saved: &Saved) -> Result<()>;
}

struct CurrentUser;

impl Backend for CurrentUser {
    fn capture(&mut self, change: &Change) -> Result<Saved> {
        let mut key = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(change.path),
                None,
                KEY_QUERY_VALUE,
                &mut key,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(Saved {
                key_exists: false,
                value: None,
            });
        }
        status.ok()?;
        let key = Key(key);
        let name = change.name.map(HSTRING::from);
        let name = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        let mut kind = REG_NONE;
        let mut size = 0u32;
        let mut data = Vec::new();
        loop {
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    name,
                    None,
                    Some(&mut kind),
                    if data.is_empty() {
                        None
                    } else {
                        Some(data.as_mut_ptr())
                    },
                    Some(&mut size),
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(Saved {
                    key_exists: true,
                    value: None,
                });
            }
            if status == ERROR_MORE_DATA || (status == ERROR_SUCCESS && size as usize > data.len())
            {
                data.resize(size as usize, 0);
                continue;
            }
            status.ok()?;
            data.truncate(size as usize);
            return Ok(Saved {
                key_exists: true,
                value: Some(Value { kind, data }),
            });
        }
    }

    fn set(&mut self, change: &Change) -> Result<()> {
        set(change.path, change.name, change.value)
    }

    fn restore(&mut self, change: &Change, saved: &Saved) -> Result<()> {
        if !saved.key_exists {
            // Never recursively delete a key: reverse order removes our child
            // keys first, and any unrelated subkey makes this fail safely.
            let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(change.path)) };
            return match status {
                ERROR_FILE_NOT_FOUND => Ok(()),
                status => status.ok(),
            };
        }
        let key = create(change.path)?;
        let name = change.name.map(HSTRING::from);
        let name = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        if let Some(value) = &saved.value {
            unsafe { RegSetValueExW(key.0, name, None, value.kind, Some(&value.data)).ok() }
        } else {
            match unsafe { RegDeleteValueW(key.0, name) } {
                ERROR_FILE_NOT_FOUND => Ok(()),
                status => status.ok(),
            }
        }
    }
}

pub struct Failure {
    pub cause: Error,
    pub rollback: Option<Error>,
}

impl From<Error> for Failure {
    fn from(cause: Error) -> Self {
        Self {
            cause,
            rollback: None,
        }
    }
}

pub fn apply(
    changes: &[Change],
    activate: impl FnOnce() -> Result<()>,
) -> std::result::Result<(), Failure> {
    apply_with(&mut CurrentUser, changes, activate)
}

fn apply_with(
    backend: &mut impl Backend,
    changes: &[Change],
    activate: impl FnOnce() -> Result<()>,
) -> std::result::Result<(), Failure> {
    // Capture everything before the first write. An unreadable old value must
    // abort rather than be mistaken for an absent registration.
    let saved = changes
        .iter()
        .map(|change| backend.capture(change))
        .collect::<Result<Vec<_>>>()?;
    let mut attempted = 0;
    let result = (|| {
        for change in changes {
            attempted += 1;
            backend.set(change)?;
        }
        activate()
    })();
    if let Err(cause) = result {
        let mut rollback = None;
        for (change, saved) in changes[..attempted].iter().zip(&saved).rev() {
            if let Err(e) = backend.restore(change, saved) {
                rollback.get_or_insert(e);
            }
        }
        return Err(Failure { cause, rollback });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Memory {
        keys: BTreeMap<String, BTreeMap<String, Value>>,
        fail_write: Option<String>,
        fail_read: bool,
        fail_restore: Option<String>,
    }

    fn failure() -> Error {
        Error::from(E_FAIL)
    }

    impl Backend for Memory {
        fn capture(&mut self, change: &Change) -> Result<Saved> {
            if self.fail_read {
                return Err(failure());
            }
            let key = self.keys.get(change.path);
            Ok(Saved {
                key_exists: key.is_some(),
                value: key
                    .and_then(|key| key.get(change.name.unwrap_or("")))
                    .cloned(),
            })
        }

        fn set(&mut self, change: &Change) -> Result<()> {
            let key = self.keys.entry(change.path.to_owned()).or_default();
            if self.fail_write.as_deref() == Some(change.path) {
                return Err(failure());
            }
            key.insert(
                change.name.unwrap_or("").to_owned(),
                Value {
                    kind: REG_SZ,
                    data: change
                        .value
                        .encode_utf16()
                        .chain(Some(0))
                        .flat_map(u16::to_le_bytes)
                        .collect(),
                },
            );
            Ok(())
        }

        fn restore(&mut self, change: &Change, saved: &Saved) -> Result<()> {
            if self.fail_restore.as_deref() == Some(change.path) {
                return Err(failure());
            }
            if !saved.key_exists {
                self.keys.remove(change.path);
            } else {
                let key = self.keys.entry(change.path.to_owned()).or_default();
                if let Some(value) = &saved.value {
                    key.insert(change.name.unwrap_or("").to_owned(), value.clone());
                } else {
                    key.remove(change.name.unwrap_or(""));
                }
            }
            Ok(())
        }
    }

    #[test]
    fn activation_failure_restores_old_values_and_preserves_unrelated_data() {
        let mut backend = Memory::default();
        backend.keys.insert(
            "class".into(),
            BTreeMap::from([
                (
                    "".into(),
                    Value {
                        kind: REG_EXPAND_SZ,
                        data: vec![1, 2, 3, 0],
                    },
                ),
                (
                    "unrelated".into(),
                    Value {
                        kind: REG_BINARY,
                        data: vec![42],
                    },
                ),
            ]),
        );
        backend.keys.insert("empty-handler".into(), BTreeMap::new());
        backend.keys.insert(
            "class\\server".into(),
            BTreeMap::from([
                (
                    "".into(),
                    Value {
                        kind: REG_SZ,
                        data: "old.dll\0"
                            .encode_utf16()
                            .flat_map(u16::to_le_bytes)
                            .collect(),
                    },
                ),
                (
                    "ThreadingModel".into(),
                    Value {
                        kind: REG_SZ,
                        data: "Both\0".encode_utf16().flat_map(u16::to_le_bytes).collect(),
                    },
                ),
            ]),
        );
        let original = backend.keys.clone();
        let changes = [
            Change::new("class", None, "new class"),
            Change::new("class\\server", None, "new.dll"),
            Change::new("class\\server", Some("ThreadingModel"), "Apartment"),
            Change::new("empty-handler", None, "new clsid"),
        ];
        let error = apply_with(&mut backend, &changes, || Err(failure()))
            .err()
            .unwrap();
        assert!(error.rollback.is_none());
        assert_eq!(backend.keys, original);
    }

    #[test]
    fn failed_writes_roll_back_and_failed_reads_never_write() {
        let mut backend = Memory {
            fail_write: Some("handler".into()),
            ..Default::default()
        };
        let changes = [
            Change::new("class", None, "new"),
            Change::new("class\\server", None, "new.dll"),
            Change::new("class\\server", Some("ThreadingModel"), "Apartment"),
            Change::new("handler", None, "new"),
        ];
        assert!(apply_with(&mut backend, &changes, || panic!("must not activate")).is_err());
        assert!(backend.keys.is_empty());
        backend.fail_read = true;
        assert!(apply_with(&mut backend, &changes, || panic!("must not activate")).is_err());
        assert!(backend.keys.is_empty());
    }

    #[test]
    fn successful_activation_keeps_changes_and_rollback_failures_are_reported() {
        let mut backend = Memory::default();
        let changes = [
            Change::new("class", None, "new"),
            Change::new("handler", None, "new"),
        ];
        assert!(apply_with(&mut backend, &changes, || Ok(())).is_ok());
        assert_eq!(backend.keys.len(), 2);
        backend.fail_restore = Some("handler".into());
        let error = apply_with(&mut backend, &changes, || Err(failure()))
            .err()
            .unwrap();
        assert!(error.rollback.is_some());
    }
}
