//! Per-user installation of the Explorer integration: files under
//! %LOCALAPPDATA%\Programs\Unbloat and COM registration in HKCU, without
//! elevation. Reads and upgrades installations made by the former PowerShell
//! installer: same marker file, DLL naming and CLSIDs.
mod registry;

use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use unbloat::{Result, display_path, error, i18n, manifest::is_link};
use windows::{
    Win32::{
        Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash},
        System::Com::*,
        UI::Shell::*,
    },
    core::{GUID, Interface},
};

/// Must match CLSID_UNBLOAT and CLSID_PREVIOUS in crates/unbloat-shell/src/com.rs.
const CLSID: GUID = GUID::from_u128(0xc74e4f58_593d_4fb5_ae90_5683c2d96374);
const LEGACY: [GUID; 3] = [
    GUID::from_u128(0xc74e4f58_593d_4fb5_ae90_5683c2d96373),
    GUID::from_u128(0xc74e4f58_593d_4fb5_ae90_5683c2d96372),
    GUID::from_u128(0xc74e4f58_593d_4fb5_ae90_5683c2d96371),
];
const PRODUCT: &str = "Unbloat";
const MARKER: &str = ".unbloat-install.json";
const HANDLERS: [&str; 2] = [
    r"Software\Classes\Directory\shellex\ContextMenuHandlers\Unbloat",
    r"Software\Classes\*\shellex\ContextMenuHandlers\Unbloat",
];
/// Files the package must contain besides the executable and the DLL.
const RESOURCES: [&str; 4] = [
    "config.toml",
    "LICENSE",
    r"licenses\THIRD_PARTY.md",
    r"licenses\DEPENDENCIES.md",
];
const NOTICES: &str = r"licenses\source-notices";
/// Prefix of files moved aside because they were in use.
const REMOVED: &str = "unbloat-removed-";

fn braced(clsid: &GUID) -> String {
    format!("{{{clsid:?}}}")
}

fn class_key(clsid: &GUID) -> String {
    format!(r"Software\Classes\CLSID\{}", braced(clsid))
}

/// The installation file list, `.unbloat-install.json`.
#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    product: String,
    clsid: String,
    #[serde(default)]
    shell_dll: String,
    /// Paths relative to the installation directory, with `\`.
    files: Vec<String>,
}

impl Marker {
    fn read(dir: &Path) -> Result<Option<Self>> {
        let text = match fs::read_to_string(dir.join(MARKER)) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        // Windows PowerShell 5.1 wrote it with a byte order mark.
        let marker: Self = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| error(i18n::text("uninstall.invalid_marker")))?;
        let known = std::iter::once(&CLSID)
            .chain(&LEGACY)
            .any(|clsid| marker.clsid.eq_ignore_ascii_case(&braced(clsid)));
        if marker.product != PRODUCT || !known {
            return Err(error(i18n::text("uninstall.invalid_marker")));
        }
        Ok(Some(marker))
    }

    fn write(&self, dir: &Path) -> Result<()> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(fs::write(dir.join(MARKER), text)?)
    }
}

/// `%LOCALAPPDATA%\Programs\Unbloat`, unless UNBLOAT_INSTALL_DIR overrides it
/// (scripts/test-launcher.ps1 installs into an isolated folder that way).
pub fn default_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("UNBLOAT_INSTALL_DIR").filter(|d| !d.is_empty()) {
        return Ok(dir.into());
    }
    let local = std::env::var_os("LOCALAPPDATA").ok_or_else(|| error("LOCALAPPDATA"))?;
    Ok(PathBuf::from(local).join("Programs").join(PRODUCT))
}

/// Validates an installation target before anything else is checked.
pub fn install_dir(dir: &Path) -> Result<PathBuf> {
    normalize(dir, "install.invalid_directory")
}

/// Absolute and without a trailing separator; drive roots are refused.
fn normalize(dir: &Path, invalid: &str) -> Result<PathBuf> {
    let dir: PathBuf = std::path::absolute(dir)?.components().collect();
    if dir.parent().is_none() || dir.file_name().is_none() {
        return Err(error(i18n::text(invalid)));
    }
    Ok(dir)
}

fn same_path(a: &Path, b: &Path) -> bool {
    let key = |p: &Path| {
        p.components()
            .collect::<PathBuf>()
            .to_string_lossy()
            .to_lowercase()
    };
    key(a) == key(b)
}

fn reparse_point(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| is_link(&meta))
}

/// The installation directory the registered COM class points to, if any.
pub fn registered_dir() -> Option<PathBuf> {
    registry::get(&format!(r"{}\InprocServer32", class_key(&CLSID)))
        .and_then(|dll| Path::new(&dll).parent().map(Path::to_path_buf))
}

/// The installation to manage: an explicit override, else the one registered
/// for this user (it may have been installed elsewhere), else the default.
pub fn target_dir() -> Result<PathBuf> {
    if std::env::var_os("UNBLOAT_INSTALL_DIR").is_some_and(|d| !d.is_empty()) {
        return default_dir();
    }
    match registered_dir().filter(|dir| is_installed(dir)) {
        Some(dir) => Ok(dir),
        None => default_dir(),
    }
}

/// Whether `dir` holds an installation (its marker file exists).
pub fn is_installed(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

/// An extracted distribution: the files `install` copies from.
#[derive(Clone)]
pub struct Package {
    dir: PathBuf,
    exe: PathBuf,
    dll: PathBuf,
}

impl Package {
    /// The package around the running executable, if complete.
    pub fn current() -> Result<Self> {
        let exe = std::env::current_exe()?;
        let dir = exe
            .parent()
            .ok_or_else(|| error(i18n::text("common.app_directory")))?
            .to_path_buf();
        // `cargo build` names the DLL unbloat_shell.dll; the package renames it.
        let dll = ["unbloat-shell.dll", "unbloat_shell.dll"]
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                let missing = display_path(&dir.join("unbloat-shell.dll"));
                error(i18n::message("install.missing_file", &[&missing]))
            })?;
        for resource in RESOURCES {
            if !dir.join(resource).is_file() {
                return Err(error(i18n::message(
                    "install.missing_resource",
                    &[&resource],
                )));
            }
        }
        if !dir.join(NOTICES).is_dir() {
            return Err(error(i18n::text("install.missing_notices")));
        }
        Ok(Self { dir, exe, dll })
    }
}

fn sha256(path: &Path) -> Result<String> {
    let data = fs::read(path)?;
    let mut digest = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, &data, &mut digest).ok()? };
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Content-named, so a new version installs beside one Explorer still has mapped.
fn dll_name(hash: &str) -> String {
    format!("unbloat-shell-{}.dll", &hash[..16])
}

/// Retries briefly while the target is in use: an Unbloat window that just
/// closed may still be exiting. A running compression still fails clearly.
fn copy(from: &Path, to: &Path) -> Result<()> {
    const SHARING_VIOLATION: i32 = 32;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match fs::copy(from, to) {
            Ok(_) => return Ok(()),
            Err(e)
                if e.raw_os_error() == Some(SHARING_VIOLATION)
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                return Err(error(i18n::message(
                    "install.copy_failed",
                    &[&display_path(to), &e],
                )));
            }
        }
    }
}

/// Relative paths of all files below `root\relative`, with `\` separators.
fn files_below(root: &Path, relative: &str, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let name = format!(r"{relative}\{}", entry.file_name().to_string_lossy());
        if entry.file_type()?.is_dir() {
            files_below(root, &name, out)?;
        } else {
            out.push(name);
        }
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            copy(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Deletes a file. One that is in use (the running executable, a DLL that
/// Explorer has loaded) can still be renamed, so it moves to %TEMP% instead,
/// where `sweep` deletes it once released.
fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let aside =
        std::env::temp_dir().join(format!("{REMOVED}{}-{nanos}-{name}", std::process::id()));
    fs::rename(path, aside)
}

/// Best effort: deletes files earlier runs moved aside.
fn sweep() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(REMOVED) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Removes the now-empty folders that contained `files`, deepest first.
fn remove_empty_dirs(dir: &Path, files: &[String]) {
    let mut folders: Vec<PathBuf> = files
        .iter()
        .flat_map(|file| {
            dir.join(file)
                .ancestors()
                .skip(1)
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .filter(|folder| folder.starts_with(dir) && folder != dir)
        .collect();
    folders.sort_by_key(|folder| std::cmp::Reverse(folder.components().count()));
    folders.dedup();
    for folder in folders {
        let _ = fs::remove_dir(folder); // Fails, as intended, when not empty.
    }
}

/// `relative` must stay inside the installation, without links on the way.
fn owned_path(dir: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(error(i18n::text("uninstall.outside")));
    }
    let path = dir.join(relative);
    if path
        .ancestors()
        .skip(1)
        .take_while(|ancestor| *ancestor != dir)
        .any(reparse_point)
    {
        return Err(error(i18n::text("uninstall.inner_link")));
    }
    Ok(path)
}

fn notify_shell() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Loads the registered handler the way Explorer does and asks for its interfaces.
fn activate() -> windows::core::Result<()> {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = CoCreateInstance::<_, IShellExtInit>(&CLSID, None, CLSCTX_INPROC_SERVER)
            .and_then(|handler| handler.cast::<IContextMenu>().map(drop));
        if com {
            CoUninitialize();
        }
        result
    }
}

fn unregister() {
    for key in HANDLERS {
        let _ = registry::delete(key);
    }
    let _ = registry::delete(&class_key(&CLSID));
}

/// Older class registrations that point into this installation.
fn retire_legacy(dir: &Path) {
    for clsid in &LEGACY {
        let key = class_key(clsid);
        let server = registry::get(&format!(r"{key}\InprocServer32"));
        if server.is_some_and(|dll| Path::new(&dll).parent().is_some_and(|d| same_path(d, dir))) {
            let _ = registry::delete(&key);
        }
    }
}

/// Copies the package into `dir` and registers the context menu for this user.
/// Reinstalling keeps the user's config.toml and cleans files of older versions.
pub fn install(package: &Package, dir: &Path) -> Result<PathBuf> {
    let dir = normalize(dir, "install.invalid_directory")?;
    if reparse_point(&dir) {
        return Err(error(i18n::text("install.link")));
    }
    sweep();
    let previous = Marker::read(&dir).ok().flatten();
    fs::create_dir_all(&dir)?;
    // Files first: the registry only ever points at a complete installation.
    let exe = dir.join("unbloat.exe");
    if !same_path(&package.exe, &exe) {
        copy(&package.exe, &exe)?;
    }
    let hash = sha256(&package.dll)?;
    let dll = dll_name(&hash);
    let dll_path = dir.join(&dll);
    if dll_path.is_file() {
        if sha256(&dll_path)? != hash {
            return Err(error(i18n::text("install.hash")));
        }
    } else {
        copy(&package.dll, &dll_path)?;
    }
    if !dir.join("config.toml").exists() {
        copy(&package.dir.join("config.toml"), &dir.join("config.toml"))?;
    }
    copy(&package.dir.join("LICENSE"), &dir.join("LICENSE"))?;
    copy_tree(&package.dir.join("licenses"), &dir.join("licenses"))?;
    let mut files = vec![
        "unbloat.exe".to_owned(),
        dll.clone(),
        "config.toml".to_owned(),
        "LICENSE".to_owned(),
    ];
    files_below(&dir, "licenses", &mut files)?;
    // Files of earlier versions: removed below, listed until then.
    let obsolete: Vec<String> = previous
        .map(|marker| marker.files)
        .unwrap_or_default()
        .into_iter()
        .filter(|old| !files.iter().any(|new| new.eq_ignore_ascii_case(old)))
        .collect();
    let mut marker = Marker {
        product: PRODUCT.to_owned(),
        clsid: braced(&CLSID),
        shell_dll: dll.clone(),
        files: files.iter().chain(&obsolete).cloned().collect(),
    };
    marker.write(&dir)?;
    let class = class_key(&CLSID);
    let registration = (|| {
        registry::set(&class, None, "Unbloat context menu")?;
        registry::set(
            &format!(r"{class}\InprocServer32"),
            None,
            &dll_path.to_string_lossy(),
        )?;
        registry::set(
            &format!(r"{class}\InprocServer32"),
            Some("ThreadingModel"),
            "Apartment",
        )?;
        for key in HANDLERS {
            registry::set(key, None, &braced(&CLSID))?;
        }
        activate()
    })();
    if let Err(e) = registration {
        unregister();
        return Err(error(i18n::message("install.com_failed", &[&e.message()])));
    }
    retire_legacy(&dir);
    notify_shell();
    // Old DLLs still mapped by Explorer move to %TEMP%; anything that can't stay listed.
    let leftover: Vec<String> = obsolete
        .into_iter()
        .filter(|old| owned_path(&dir, old).is_ok_and(|path| remove(&path).is_err()))
        .collect();
    remove_empty_dirs(&dir, &marker.files);
    marker.files = files.into_iter().chain(leftover).collect();
    marker.write(&dir)?;
    Ok(dir)
}

pub enum Uninstalled {
    Removed,
    AlreadyRemoved,
}

/// Unregisters the context menu and deletes the installed files, keeping any
/// file the user added. Works from the installed executable itself.
pub fn uninstall(dir: &Path) -> Result<Uninstalled> {
    let dir = normalize(dir, "uninstall.invalid_directory")?;
    let Some(mut marker) = Marker::read(&dir)? else {
        return match dir.exists() {
            false => Ok(Uninstalled::AlreadyRemoved),
            true => Err(error(i18n::text("uninstall.missing_marker"))),
        };
    };
    if reparse_point(&dir) {
        return Err(error(i18n::text("uninstall.link")));
    }
    sweep();
    let targets = marker
        .files
        .iter()
        .map(|file| owned_path(&dir, file))
        .collect::<Result<Vec<_>>>()?;
    let clsid = marker.clsid.clone();
    let class = format!(r"Software\Classes\CLSID\{clsid}");
    if let Some(dll) = registry::get(&format!(r"{class}\InprocServer32"))
        && !Path::new(&dll).parent().is_some_and(|d| same_path(d, &dir))
    {
        return Err(error(i18n::text("uninstall.other_install")));
    }
    for key in HANDLERS {
        registry::delete(key)?;
    }
    registry::delete(&class)?;
    retire_legacy(&dir);
    notify_shell();
    let mut locked = Vec::new();
    for (file, path) in marker.files.iter().zip(&targets) {
        if remove(path).is_err() {
            locked.push(file.clone());
        }
    }
    if !locked.is_empty() {
        let list = locked
            .iter()
            .map(|file| display_path(&dir.join(file)))
            .collect::<Vec<_>>()
            .join(", ");
        marker.files = locked;
        marker.write(&dir)?;
        return Err(error(i18n::message("uninstall.locked", &[&list])));
    }
    fs::remove_file(dir.join(MARKER))?;
    remove_empty_dirs(&dir, &marker.files);
    let _ = fs::remove_dir(&dir); // Kept when the user added files of their own.
    Ok(Uninstalled::Removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_match_the_powershell_installer() {
        assert_eq!(braced(&CLSID), "{C74E4F58-593D-4FB5-AE90-5683C2D96374}");
        assert_eq!(
            class_key(&LEGACY[2]),
            r"Software\Classes\CLSID\{C74E4F58-593D-4FB5-AE90-5683C2D96371}"
        );
        // Get-FileHash -Algorithm SHA256 of the same bytes, lowercased.
        let file = std::env::temp_dir().join(format!("unbloat-hash-{}", std::process::id()));
        fs::write(&file, b"abc").unwrap();
        let hash = sha256(&file).unwrap();
        fs::remove_file(&file).unwrap();
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(dll_name(&hash), "unbloat-shell-ba7816bf8f01cfea.dll");
    }

    #[test]
    fn markers_written_by_powershell_are_read() {
        let dir = std::env::temp_dir().join(format!("unbloat-marker-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let legacy = "\u{feff}{\r\n    \"clsid\":  \"{C74E4F58-593D-4FB5-AE90-5683C2D96374}\",\r\n    \"product\":  \"Unbloat\",\r\n    \"shell_dll\":  \"unbloat-shell-8efa8a316d2f5cdb.dll\",\r\n    \"files\":  [\r\n                  \"config.toml\",\r\n                  \"locales\\\\en.txt\"\r\n              ]\r\n}\r\n";
        fs::write(dir.join(MARKER), legacy).unwrap();
        let marker = Marker::read(&dir).unwrap().unwrap();
        assert_eq!(marker.files, ["config.toml", r"locales\en.txt"]);
        fs::write(dir.join(MARKER), legacy.replace("Unbloat\"", "Other\"")).unwrap();
        assert!(Marker::read(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
        assert!(Marker::read(&dir).unwrap().is_none());
    }

    #[test]
    fn owned_paths_stay_inside_the_installation() {
        let dir = Path::new(r"C:\Users\x\AppData\Local\Programs\Unbloat");
        assert!(owned_path(dir, r"licenses\LICENSE").is_ok());
        for outside in [
            r"..\evil.dll",
            r"C:\Windows\System32\kernel32.dll",
            r"\x",
            r"a\..\..\b",
        ] {
            assert!(owned_path(dir, outside).is_err(), "{outside}");
        }
        assert!(normalize(Path::new(r"C:\"), "install.invalid_directory").is_err());
        assert!(normalize(Path::new(r"C:\."), "install.invalid_directory").is_err());
        assert_eq!(
            normalize(Path::new(r"C:\x\Unbloat\"), "install.invalid_directory").unwrap(),
            Path::new(r"C:\x\Unbloat")
        );
    }
}
