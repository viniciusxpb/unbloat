//! Exercises the real DLL using Windows shell IDataObject and a native popup menu.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        ffi::c_void,
        os::windows::ffi::OsStrExt,
        path::PathBuf,
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            Foundation::*,
            System::{Com::*, LibraryLoader::*},
            UI::{Shell::*, WindowsAndMessaging::*},
        },
        core::{GUID, Interface, PCWSTR, w},
    };
    type GetClass = unsafe extern "system" fn(
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> windows::core::HRESULT;
    type Unload = unsafe extern "system" fn() -> windows::core::HRESULT;
    const CLSID: GUID = GUID::from_u128(0xc74e4f58_593d_4fb5_ae90_5683c2d96372);
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let invoke = args.iter().any(|a| a == "--invoke");
    let cancel = args.iter().any(|a| a == "--cancel");
    let registered = args.iter().any(|a| a == "--registered");
    let explorer_menu = args.iter().any(|a| a == "--explorer-menu");
    args.retain(|a| {
        a != "--invoke" && a != "--cancel" && a != "--registered" && a != "--explorer-menu"
    });
    if args.len() < 2 {
        return Err(
            "Uso: shell_probe <dll> <arquivo-ou-pasta> [itens no mesmo diretório] [--invoke] [--cancel] [--registered]"
                .into(),
        );
    }
    let dll: Vec<u16> = PathBuf::from(&args[0])
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let paths: Vec<_> = args[1..]
        .iter()
        .map(|p| PathBuf::from(p).canonicalize())
        .collect::<Result<_, _>>()?;
    if explorer_menu {
        return check_explorer_menu(&paths);
    }
    let output_count = || {
        std::fs::read_dir(paths[0].parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "7z"))
            .count()
    };
    let before = output_count();
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let module = LoadLibraryW(PCWSTR(dll.as_ptr()))?;
        let get_class: GetClass = std::mem::transmute(
            GetProcAddress(module, windows::core::s!("DllGetClassObject"))
                .ok_or("export missing")?,
        );
        let can_unload: Unload = std::mem::transmute(
            GetProcAddress(module, windows::core::s!("DllCanUnloadNow")).ok_or("export missing")?,
        );
        assert_eq!(can_unload(), S_OK);
        let mut raw = std::ptr::null_mut();
        get_class(&CLSID, &IClassFactory::IID, &mut raw).ok()?;
        let factory = IClassFactory::from_raw(raw);
        assert_eq!(can_unload(), S_FALSE);
        let init: IShellExtInit = if registered {
            CoCreateInstance(&CLSID, None, CLSCTX_INPROC_SERVER)?
        } else {
            factory.CreateInstance(None)?
        };
        let menu: IContextMenu = init.cast()?;
        let wide = |p: &std::path::Path| {
            // Shell PIDL parsing expects DOS/UNC names rather than verbatim paths.
            let mut units: Vec<u16> = p.as_os_str().encode_wide().collect();
            if units.starts_with(&[92, 92, 63, 92]) {
                units.drain(..4);
                if units.starts_with(&[85, 78, 67, 92]) {
                    units.splice(..4, [92, 92]);
                }
            }
            units.push(0);
            units
        };
        let parent = ILCreateFromPathW(PCWSTR(wide(paths[0].parent().unwrap()).as_ptr()));
        if parent.is_null() {
            return Err("parent PIDL failed".into());
        }
        let full: Vec<_> = paths
            .iter()
            .map(|p| ILCreateFromPathW(PCWSTR(wide(p).as_ptr())))
            .collect();
        if full.iter().any(|p| p.is_null()) {
            return Err("child PIDL failed".into());
        }
        let children: Vec<_> = full
            .iter()
            .map(|p| ILFindLastID(*p) as *const Common::ITEMIDLIST)
            .collect();
        let data: IDataObject = SHCreateDataObject(Some(parent), Some(&children), None)?;
        init.Initialize(None, &data, None)?;
        let popup = CreatePopupMenu()?;
        assert_eq!(menu.QueryContextMenu(popup, 0, 1, 20, CMF_DEFAULTONLY).0, 0);
        assert_eq!(GetMenuItemCount(Some(popup)), 0);
        assert_eq!(menu.QueryContextMenu(popup, 0, 1, 20, 0).0, 1);
        assert_eq!(GetMenuItemCount(Some(popup)), 1);
        let mut text = [0u16; 128];
        GetMenuStringW(popup, 0, Some(&mut text), MF_BYPOSITION);
        assert_eq!(
            String::from_utf16_lossy(&text[..text.iter().position(|c| *c == 0).unwrap()]),
            "Compactar com Unbloat"
        );
        if invoke {
            let info = CMINVOKECOMMANDINFO {
                cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            let start = Instant::now();
            menu.InvokeCommand(&info)?;
            let launch_time = start.elapsed();
            println!(
                "Process launch returned in {:.3} s",
                launch_time.as_secs_f64()
            );
            // CreateProcess/antivirus startup can be slow on a cold Windows host.
            // This timeout detects a hung launch, not compression performance.
            assert!(launch_time < Duration::from_secs(10), "worker launch hung");
            let mut saw_window = false;
            let mut sent_cancel = false;
            loop {
                let window = FindWindowW(w!("UnbloatProgress"), PCWSTR::null()).ok();
                if let Some(window) = window {
                    saw_window = true;
                    if cancel && !sent_cancel {
                        PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                        sent_cancel = true;
                    }
                }
                if saw_window && window.is_none() {
                    break;
                }
                if start.elapsed() > Duration::from_secs(45) {
                    return Err("UI/worker did not finish in 45s".into());
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            assert!(saw_window, "native activity window missing");
            if cancel {
                assert_eq!(output_count(), before, "cancelled job published an archive");
            } else {
                assert_eq!(
                    output_count(),
                    before + 1,
                    "worker did not publish exactly one archive"
                );
            }
        }
        DestroyMenu(popup)?;
        drop(menu);
        drop(init);
        drop(data);
        drop(factory);
        for pidl in full {
            CoTaskMemFree(Some(pidl.cast()));
        }
        CoTaskMemFree(Some(parent.cast()));
        assert_eq!(can_unload(), S_OK, "COM reference leak");
        FreeLibrary(module)?;
        CoUninitialize();
    }
    println!(
        "PASS: real IDataObject, top-level menu, COM lifetime{}",
        if invoke {
            ", separate process and native activity UI"
        } else {
            ""
        }
    );
    Ok(())
}

/// Ask the Windows shell to compose its actual registered menu, rather than
/// instantiating our DLL directly. This catches missing file-class registration.
#[cfg(windows)]
fn check_explorer_menu(paths: &[std::path::PathBuf]) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Foundation::HWND,
            System::Com::*,
            UI::{Shell::*, WindowsAndMessaging::*},
        },
        core::PCWSTR,
    };
    let mut full = Vec::new();
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        for path in paths {
            let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
            if units.starts_with(&[92, 92, 63, 92]) {
                units.drain(..4);
                if units.starts_with(&[85, 78, 67, 92]) {
                    units.splice(..4, [92, 92]);
                }
            }
            units.push(0);
            let pidl = ILCreateFromPathW(PCWSTR(units.as_ptr()));
            if pidl.is_null() {
                return Err("Shell PIDL failed".into());
            }
            full.push(pidl);
        }
        let folder: IShellFolder = SHBindToParent(full[0], None)?;
        let children: Vec<_> = full
            .iter()
            .map(|p| ILFindLastID(*p) as *const Common::ITEMIDLIST)
            .collect();
        let menu: IContextMenu = folder.GetUIObjectOf(HWND::default(), &children, None)?;
        let popup = CreatePopupMenu()?;
        menu.QueryContextMenu(popup, 0, 1, 0x7fff, 0).ok()?;
        let count = GetMenuItemCount(Some(popup));
        let mut found = false;
        for index in 0..count {
            let mut text = [0u16; 512];
            let length = GetMenuStringW(popup, index as u32, Some(&mut text), MF_BYPOSITION);
            let title = String::from_utf16_lossy(&text[..length.max(0) as usize]).replace('&', "");
            if title == "Compactar com Unbloat" {
                found = true;
                println!(
                    "PASS: Windows shell composed a top-level '{title}' entry for {} selected item(s).",
                    paths.len()
                );
            }
        }
        DestroyMenu(popup)?;
        drop(menu);
        drop(folder);
        for pidl in full {
            CoTaskMemFree(Some(pidl.cast()));
        }
        CoUninitialize();
        if !found {
            return Err("Unbloat missing from the actual registered Windows shell menu".into());
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}
