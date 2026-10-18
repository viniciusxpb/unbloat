//! Distribution folder and ZIP for the host platform, from fresh release binaries.
use crate::{Result, cargo, i18n, root, run};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};
use zip::{CompressionMethod, DateTime, ZipWriter, write::SimpleFileOptions};

pub fn create(args: &[String]) -> Result {
    let open = match args {
        [] => std::env::var_os("CI").is_none(),
        [flag] if flag == "--no-open" => false,
        _ => return Err(format!("unsupported package option '{}'", args[0]).into()),
    };
    let root = root();
    let mut build = cargo();
    build.args(["build", "--release", "--locked", "--package", "unbloat"]);
    if cfg!(windows) {
        build.args(["--package", "unbloat-shell"]);
    }
    run(&mut build)?;
    let version = crate::version()?;
    // Generated notices stay out of Git, but must always ship with the binaries.
    crate::licenses::collect()?;
    let release = root.join("target").join("release");
    let mut files: Vec<(PathBuf, &str)> = if cfg!(windows) {
        vec![
            (release.join("unbloat.exe"), "unbloat.exe"),
            (release.join("unbloat_shell.dll"), "unbloat-shell.dll"),
        ]
    } else {
        // No file-manager integration outside Windows yet: the command line only.
        vec![(release.join("unbloat"), "unbloat")]
    };
    for document in ["config.toml", "LICENSE", "README.md", "VALIDATION.md"] {
        files.push((root.join(document), document));
    }
    let destination = root.join("dist").join("Unbloat");
    // Start empty so files from earlier versions never reach the ZIP.
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    fs::create_dir_all(&destination)?;
    for (source, name) in files {
        fs::copy(&source, destination.join(name))
            .map_err(|e| format!("{}: {e}", source.display()))?;
    }
    copy_tree(&root.join("licenses"), &destination.join("licenses"))?;
    let archive = root
        .join("dist")
        .join(format!("Unbloat-{version}-{}.zip", platform()));
    write_zip(&destination, &archive)?;
    println!(
        "{}",
        i18n::message("package.complete", &[&destination.display()])
    );
    println!(
        "{}",
        i18n::message("package.archive", &[&archive.display()])
    );
    if open && let Err(e) = reveal(&archive) {
        // The package is complete; failing to show it is only worth a note.
        eprintln!("{}", i18n::message("package.open_failed", &[&e]));
    }
    Ok(())
}

/// Shows the file in the system file manager, selected where supported.
/// Does not wait: the file manager outlives this task.
fn reveal(file: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer parses `/select,"path"` itself; Rust's own quoting would
        // wrap the whole argument and Explorer would open Documents instead.
        Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", file.display()))
            .spawn()
            .map(drop)
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg("-R").arg(file).spawn().map(drop)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let folder = file.parent().unwrap_or(file);
        Command::new("xdg-open").arg(folder).spawn().map(drop)
    }
}

/// For example `windows-x64`.
fn platform() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{}-{arch}", std::env::consts::OS)
}

fn copy_tree(source: &Path, destination: &Path) -> Result {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn files_in(directory: &Path, files: &mut Vec<PathBuf>) -> Result {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            files_in(&entry.path(), files)?;
        } else {
            files.push(entry.path());
        }
    }
    Ok(())
}

/// Entries use `/` separators as the ZIP specification requires, in a stable order.
fn write_zip(source: &Path, archive: &Path) -> Result {
    let mut files = Vec::new();
    files_in(source, &mut files)?;
    files.sort();
    let mut zip = ZipWriter::new(File::create(archive)?);
    for path in files {
        let name = path
            .strip_prefix(source)?
            .to_str()
            .ok_or("non-UTF-8 file name")?
            .replace('\\', "/");
        let metadata = fs::metadata(&path)?;
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(9))
            .last_modified_time(zip_time(metadata.modified()?));
        #[cfg(unix)]
        let options = {
            use std::os::unix::fs::PermissionsExt;
            options.unix_permissions(metadata.permissions().mode())
        };
        zip.start_file(name, options)?;
        io::copy(&mut File::open(&path)?, &mut zip)?;
    }
    zip.finish()?;
    Ok(())
}

/// ZIP stores a zone-less DOS time; like other archivers, use the packager's local time.
fn zip_time(modified: SystemTime) -> DateTime {
    let utc = OffsetDateTime::from(modified);
    let local = UtcOffset::current_local_offset().map_or(utc, |offset| utc.to_offset(offset));
    DateTime::try_from(PrimitiveDateTime::new(local.date(), local.time())).unwrap_or_default()
}
