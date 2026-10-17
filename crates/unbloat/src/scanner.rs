use crate::i18n;
use crate::{
    Result, check_cancel,
    config::Ignore,
    error,
    manifest::{ArchiveItem, Manifest, archive_name, is_link},
};
use ignore::WalkBuilder;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub fn scan(paths: &[PathBuf], config: &Ignore, cancel: &AtomicBool) -> Result<Manifest> {
    if paths.is_empty() {
        return Err(error(i18n::text("scanner.empty")));
    }
    let access_failed = |path: &Path, e: std::io::Error| {
        error(i18n::message(
            "scanner.access_failed",
            &[&crate::display_path(path), &e],
        ))
    };
    let mut roots = BTreeSet::new();
    for path in paths {
        // First, so a missing selection is reported with its path.
        let meta = std::fs::symlink_metadata(path).map_err(|e| access_failed(path, e))?;
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()?.join(path)
        };
        for ancestor in absolute.ancestors() {
            let ancestor_meta =
                std::fs::symlink_metadata(ancestor).map_err(|e| access_failed(ancestor, e))?;
            if is_link(&ancestor_meta) {
                return Err(error(i18n::text("scanner.link")));
            }
        }
        if (!meta.is_dir() && !meta.is_file()) || is_link(&meta) {
            return Err(error(i18n::text("scanner.regular_only")));
        }
        let root = path.canonicalize().map_err(|e| access_failed(path, e))?;
        if root.file_name().is_none()
            || (meta.is_dir()
                && root
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case(".git")))
        {
            return Err(error(i18n::text("scanner.invalid_root")));
        }
        roots.insert(root);
    }
    let roots: Vec<_> = roots.into_iter().collect();
    let destination = roots[0]
        .parent()
        .ok_or_else(|| error(i18n::text("scanner.parent_missing")))?
        .to_path_buf();
    if roots
        .iter()
        .any(|root| root.parent() != Some(destination.as_path()))
    {
        return Err(error(i18n::text("scanner.same_parent")));
    }
    let mut manifest = Manifest {
        roots: roots.clone(),
        destination,
        items: Vec::new(),
        examined_entries: 0,
    };
    for root in &roots {
        let base = root.parent().unwrap();
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(!config.include_hidden)
            .parents(false)
            .git_global(false)
            .git_exclude(false)
            .git_ignore(config.gitignore)
            .ignore(config.dot_ignore)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| {
                // Reading reparse attributes here prevents junction traversal by WalkDir.
                // Metadata errors are left to the main iterator, where they are reported.
                if entry.file_type().is_some_and(|t| t.is_dir())
                    && entry.file_name().eq_ignore_ascii_case(".git")
                {
                    return false;
                }
                !std::fs::symlink_metadata(entry.path()).is_ok_and(|m| is_link(&m))
            });
        for result in builder.build() {
            check_cancel(cancel)?;
            let entry = result.map_err(|e| error(i18n::message("scanner.failed", &[&e])))?;
            if let Some(e) = entry.error() {
                return Err(error(i18n::message("scanner.ignore_error", &[&e])));
            }
            manifest.examined_entries += 1;
            let meta = std::fs::symlink_metadata(entry.path())
                .map_err(|e| access_failed(entry.path(), e))?;
            if is_link(&meta) {
                continue;
            }
            if meta.is_dir() {
                check_ignore_readability(entry.path(), config)?;
            }
            if !meta.is_dir() && !meta.is_file() {
                return Err(error(i18n::message(
                    "scanner.special_file",
                    &[&crate::display_path(entry.path())],
                )));
            }
            manifest.items.push(ArchiveItem {
                source_path: entry.path().to_path_buf(),
                archive_path: archive_name(entry.path().strip_prefix(base)?)?,
                is_directory: meta.is_dir(),
                size: meta.len(),
                modified: meta.modified()?,
            });
        }
    }
    manifest
        .items
        .sort_by(|a, b| a.archive_path.cmp(&b.archive_path));
    manifest.validate()?;
    Ok(manifest)
}

/// `ignore` intentionally tolerates some I/O errors when loading ignore files.
/// Verify them separately so a sharing violation cannot silently include secrets.
/// This does not interpret rules or enumerate ignored subtrees.
fn check_ignore_readability(directory: &Path, config: &Ignore) -> Result<()> {
    for (name, enabled) in [
        (".gitignore", config.gitignore),
        (".ignore", config.dot_ignore),
    ] {
        if !enabled {
            continue;
        }
        let path = directory.join(name);
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(error(i18n::message(
                    "scanner.stat_failed",
                    &[&crate::display_path(&path), &e],
                )));
            }
            Ok(meta) if is_link(&meta) || !meta.is_file() => {
                return Err(error(i18n::message(
                    "scanner.ignore_regular",
                    &[&crate::display_path(&path)],
                )));
            }
            Ok(_) => {}
        }
        let result = std::fs::File::open(&path)
            .and_then(|mut file| std::io::copy(&mut file, &mut std::io::sink()));
        result.map_err(|e| {
            error(i18n::message(
                "scanner.ignore_read_failed",
                &[&crate::display_path(&path), &e],
            ))
        })?;
    }
    Ok(())
}

pub fn included_files(manifest: &Manifest) -> usize {
    manifest
        .items
        .iter()
        .filter(|item| !item.is_directory)
        .count()
}

pub fn root_name(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| error(i18n::text("scanner.invalid_name")))
}
