#![allow(non_snake_case)]
use std::{
    cell::RefCell,
    ffi::{OsString, c_void},
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::PathBuf,
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};
use windows::{
    Win32::{
        Foundation::*,
        System::{
            Com::*,
            LibraryLoader::GetModuleFileNameW,
            Ole::{CF_HDROP, ReleaseStgMedium},
            Registry::HKEY,
        },
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{Error, GUID, HRESULT, IUnknown, Interface, PSTR, PWSTR, Ref, Result, implement},
};

// Refresh the class identity so Explorer doesn't reuse a cached text-only handler.
// Older identities remain compatible until their in-process references drain.
pub const CLSID_UNBLOAT: GUID = GUID::from_u128(0xc74e4f58_593d_4fb5_ae90_5683c2d96373);
const CLSID_PREVIOUS: GUID = GUID::from_u128(0xc74e4f58_593d_4fb5_ae90_5683c2d96372);
const CLSID_LEGACY: GUID = GUID::from_u128(0xc74e4f58_593d_4fb5_ae90_5683c2d96371);
static MODULE: AtomicIsize = AtomicIsize::new(0);
static OBJECTS: AtomicUsize = AtomicUsize::new(0);
static LOCKS: AtomicUsize = AtomicUsize::new(0);
struct Lifetime;
impl Lifetime {
    fn new() -> Self {
        OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

fn failure(message: impl AsRef<str>) -> Error {
    Error::new(E_FAIL, message.as_ref())
}
fn executable() -> Result<PathBuf> {
    let mut path = vec![0u16; 32768];
    let size = unsafe {
        GetModuleFileNameW(
            Some(HMODULE(MODULE.load(Ordering::Relaxed) as *mut c_void)),
            &mut path,
        )
    } as usize;
    if size == 0 || size >= path.len() {
        return Err(Error::from_thread());
    }
    Ok(PathBuf::from(OsString::from_wide(&path[..size])).with_file_name("unbloat.exe"))
}

#[implement(IShellExtInit, IContextMenu)]
struct Handler {
    paths: RefCell<Vec<PathBuf>>,
    icon: RefCell<Option<crate::menu_icon::MenuIcon>>,
    _lifetime: Lifetime,
}
impl Handler {
    fn new() -> Self {
        Self {
            paths: RefCell::new(Vec::new()),
            icon: RefCell::new(None),
            _lifetime: Lifetime::new(),
        }
    }
}
impl IShellExtInit_Impl for Handler_Impl {
    fn Initialize(
        &self,
        _: *const Common::ITEMIDLIST,
        data: Ref<IDataObject>,
        _: HKEY,
    ) -> Result<()> {
        self.paths.borrow_mut().clear();
        let data = data.as_ref().ok_or_else(|| Error::from(E_INVALIDARG))?;
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        };
        let mut medium = unsafe { data.GetData(&format)? };
        let result = (|| {
            if medium.tymed != TYMED_HGLOBAL.0 as u32 {
                return Err(Error::from(E_INVALIDARG));
            }
            let drop = HDROP(unsafe { medium.u.hGlobal }.0);
            let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
            if count == 0 || count > 10000 {
                return Err(Error::from(E_INVALIDARG));
            }
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let len = unsafe { DragQueryFileW(drop, index, None) } as usize;
                if len == 0 || len > 32767 {
                    return Err(Error::from(E_INVALIDARG));
                }
                let mut name = vec![0u16; len + 1];
                if unsafe { DragQueryFileW(drop, index, Some(&mut name)) } as usize != len {
                    return Err(Error::from(E_FAIL));
                }
                let path = PathBuf::from(OsString::from_wide(&name[..len]));
                if !path.is_absolute() {
                    return Err(Error::from(E_INVALIDARG));
                }
                // Only a shallow metadata lookup; no recursion or archive code here.
                use std::os::windows::fs::MetadataExt;
                let meta = std::fs::symlink_metadata(&path).map_err(|e| failure(e.to_string()))?;
                if (!meta.is_dir() && !meta.is_file()) || meta.file_attributes() & 0x400 != 0 {
                    return Err(Error::from(E_INVALIDARG));
                }
                paths.push(path);
            }
            let parent = paths[0].parent();
            if paths.iter().any(|p| p.parent() != parent) {
                return Err(Error::from(E_INVALIDARG));
            }
            *self.paths.borrow_mut() = paths;
            Ok(())
        })();
        unsafe {
            ReleaseStgMedium(&mut medium);
        }
        result
    }
}
impl IContextMenu_Impl for Handler_Impl {
    fn QueryContextMenu(
        &self,
        menu: HMENU,
        index: u32,
        first: u32,
        last: u32,
        flags: u32,
    ) -> HRESULT {
        if flags & CMF_DEFAULTONLY != 0 || self.paths.borrow().is_empty() || first > last {
            return S_OK;
        }
        let insert = (|| -> Result<()> {
            let mut icon = self.icon.borrow_mut();
            if icon.is_none() {
                *icon = Some(crate::menu_icon::MenuIcon::load(HMODULE(
                    MODULE.load(Ordering::Relaxed) as *mut c_void,
                ))?);
            }
            let mut title: Vec<u16> = "Compactar com Unbloat"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let item = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_ID | MIIM_STRING | MIIM_BITMAP,
                wID: first,
                dwTypeData: PWSTR(title.as_mut_ptr()),
                hbmpItem: icon.as_ref().unwrap().bitmap(),
                ..Default::default()
            };
            unsafe { InsertMenuItemW(menu, index, true, &item) }
        })();
        match insert {
            Ok(()) => HRESULT(1),
            Err(e) => e.code(),
        }
    }
    fn InvokeCommand(&self, info: *const CMINVOKECOMMANDINFO) -> Result<()> {
        if info.is_null() {
            return Err(Error::from(E_POINTER));
        }
        let info = unsafe { &*info };
        if info.cbSize < size_of::<CMINVOKECOMMANDINFO>() as u32 {
            return Err(Error::from(E_INVALIDARG));
        }
        let verb = info.lpVerb.0 as usize;
        if verb > 0xffff {
            let verb = unsafe { info.lpVerb.to_string() }.map_err(|_| Error::from(E_INVALIDARG))?;
            if verb != "unbloat" {
                return Err(Error::from(E_INVALIDARG));
            }
        } else if verb != 0 {
            return Err(Error::from(E_INVALIDARG));
        }
        let selection =
            crate::selection::write(&self.paths.borrow()).map_err(|e| failure(e.to_string()))?;
        let launch = (|| {
            let exe = executable()?;
            std::process::Command::new(&exe)
                .arg("--shell")
                .arg("--selection-file")
                .arg(&selection)
                .current_dir(
                    exe.parent()
                        .ok_or_else(|| failure("Pasta da aplicação inválida"))?,
                )
                .creation_flags(0x08000000)
                .spawn()
                .map_err(|e| failure(format!("Não foi possível iniciar Unbloat: {e}")))?;
            Ok(())
        })();
        if launch.is_err() {
            let _ = std::fs::remove_file(selection);
        }
        launch
    }
    fn GetCommandString(
        &self,
        id: usize,
        kind: u32,
        _: *const u32,
        name: PSTR,
        capacity: u32,
    ) -> Result<()> {
        if id != 0 {
            return Err(Error::from(E_INVALIDARG));
        }
        if kind == GCS_VALIDATEA || kind == GCS_VALIDATEW {
            return Ok(());
        }
        if name.is_null() || capacity == 0 {
            return Err(Error::from(E_POINTER));
        }
        let text = match kind {
            GCS_VERBA | GCS_VERBW => "unbloat",
            GCS_HELPTEXTA | GCS_HELPTEXTW => "Compactar projetos respeitando .gitignore e .ignore",
            _ => return Err(Error::from(E_INVALIDARG)),
        };
        if kind & GCS_UNICODE != 0 {
            let dest =
                unsafe { std::slice::from_raw_parts_mut(name.0.cast::<u16>(), capacity as usize) };
            let units: Vec<_> = text.encode_utf16().collect();
            let len = units.len().min(dest.len() - 1);
            dest[..len].copy_from_slice(&units[..len]);
            dest[len] = 0;
        } else {
            let dest = unsafe { std::slice::from_raw_parts_mut(name.0, capacity as usize) };
            let len = text.len().min(dest.len() - 1);
            dest[..len].copy_from_slice(&text.as_bytes()[..len]);
            dest[len] = 0;
        }
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory {
    _lifetime: Lifetime,
}
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> Result<()> {
        if out.is_null() || iid.is_null() {
            return Err(Error::from(E_POINTER));
        }
        unsafe {
            *out = std::ptr::null_mut();
        }
        if outer.as_ref().is_some() {
            return Err(Error::from(CLASS_E_NOAGGREGATION));
        }
        let handler: IUnknown = Handler::new().into();
        unsafe { handler.query(iid, out).ok() }
    }
    fn LockServer(&self, lock: windows::core::BOOL) -> Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            let _ = LOCKS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
        }
        Ok(())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(
    module: HMODULE,
    reason: u32,
    _: *mut c_void,
) -> windows::core::BOOL {
    if reason == 1 {
        MODULE.store(module.0 as isize, Ordering::Relaxed);
    }
    true.into()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() || clsid.is_null() || iid.is_null() {
        return E_POINTER;
    }
    unsafe {
        *out = std::ptr::null_mut();
    }
    if ![CLSID_UNBLOAT, CLSID_PREVIOUS, CLSID_LEGACY].contains(&unsafe { *clsid }) {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IUnknown = Factory {
        _lifetime: Lifetime::new(),
    }
    .into();
    unsafe { factory.query(iid, out) }
}
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
