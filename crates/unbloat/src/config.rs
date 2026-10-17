use crate::{Result, error};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub compression: Compression,
    pub ignore: Ignore,
    pub output: Output,
    pub logging: Logging,
    pub development: Development,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compression {
    pub format: String,
    pub method: String,
    pub level: u32,
    pub solid: bool,
    pub solid_block_size_mb: u64,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ignore {
    pub gitignore: bool,
    pub dot_ignore: bool,
    pub exclude_git_directory: bool,
    pub include_hidden: bool,
    pub follow_symlinks: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub suffix: String,
    pub on_conflict: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logging {
    pub enabled: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Development {
    pub test_input: PathBuf,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| error(format!("Não foi possível ler {}: {e}", path.display())))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.compression.format != "7z" || self.compression.method != "lzma2" {
            return Err(error(
                "Apenas format = '7z' e method = 'lzma2' são suportados.",
            ));
        }
        if self.compression.level > 9 || !(1..=1024).contains(&self.compression.solid_block_size_mb)
        {
            return Err(error(
                "Use level entre 0 e 9 e solid_block_size_mb entre 1 e 1024.",
            ));
        }
        if self.ignore.follow_symlinks || !self.ignore.exclude_git_directory {
            return Err(error(
                "A v0.2 exige follow_symlinks = false e exclude_git_directory = true.",
            ));
        }
        if self.output.on_conflict != "rename"
            || self
                .output
                .suffix
                .chars()
                .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
            || self.output.suffix.ends_with([' ', '.'])
        {
            return Err(error(
                "Configuração de output inválida (on_conflict deve ser 'rename').",
            ));
        }
        Ok(())
    }
}

/// Installed builds always load config beside the executable; debug builds also
/// support the repository config. --dev-run explicitly uses the repository file.
pub fn location(development: bool) -> Result<PathBuf> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config.toml");
    if development {
        return Ok(repository);
    }
    let beside_exe = std::env::current_exe()?.with_file_name("config.toml");
    if beside_exe.is_file() || !cfg!(debug_assertions) {
        Ok(beside_exe)
    } else {
        Ok(repository)
    }
}
