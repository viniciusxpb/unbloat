use crate::{Result, error};
use std::{
    collections::BTreeSet,
    fs::Metadata,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug)]
pub struct ArchiveItem {
    pub source_path: PathBuf,
    pub archive_path: String,
    pub is_directory: bool,
    pub size: u64,
    pub modified: SystemTime,
}

#[derive(Debug)]
pub struct Manifest {
    pub roots: Vec<PathBuf>,
    pub destination: PathBuf,
    pub items: Vec<ArchiveItem>,
    /// Entries returned by the ignore walker, not OS filesystem calls.
    pub examined_entries: usize,
}

pub fn archive_name(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for part in path.components() {
        let Component::Normal(name) = part else {
            return Err(error(
                "Caminho interno absoluto ou com travessia não permitido.",
            ));
        };
        let name = name
            .to_str()
            .ok_or_else(|| error("Nome de arquivo sem Unicode válido."))?;
        if name.is_empty() || name.chars().any(|c| c.is_control() || "/\\:".contains(c)) {
            return Err(error("Nome inseguro para o arquivo 7z."));
        }
        parts.push(name);
    }
    if parts.is_empty() {
        return Err(error("Caminho interno vazio."));
    }
    Ok(parts.join("/"))
}

pub fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // All reparse points, including junctions and cloud placeholders.
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// Recheck ancestors too: a directory can have been replaced by a junction since scanning.
pub fn check_source(item: &ArchiveItem) -> Result<()> {
    for ancestor in item.source_path.ancestors() {
        let meta = std::fs::symlink_metadata(ancestor)?;
        if is_link(&meta) {
            return Err(error(format!(
                "Link/reparse point recusado: {}",
                ancestor.display()
            )));
        }
    }
    let meta = std::fs::symlink_metadata(&item.source_path)?;
    if meta.is_dir() != item.is_directory
        || (!item.is_directory
            && (!meta.is_file() || meta.len() != item.size || meta.modified()? != item.modified))
    {
        return Err(error(format!(
            "Arquivo alterado durante a operação: {}",
            item.source_path.display()
        )));
    }
    Ok(())
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        let mut names = BTreeSet::new();
        for item in &self.items {
            if archive_name(Path::new(&item.archive_path))? != item.archive_path
                || !item.source_path.is_absolute()
                || !self
                    .roots
                    .iter()
                    .any(|root| item.source_path.starts_with(root))
                || !names.insert(item.archive_path.to_lowercase())
            {
                return Err(error("Manifesto inválido ou com caminhos duplicados."));
            }
        }
        Ok(())
    }
}
