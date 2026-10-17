use crate::i18n;
use crate::{Result, error};
use serde::Deserialize;
use std::path::{Path, PathBuf};

// Reinstallation preserves the user's config.toml, so every field has a default:
// a key added in a later version must not break an existing installation.
// Unknown keys are still rejected to catch typos; keep removed keys accepted.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub compression: Compression,
    pub ignore: Ignore,
    pub output: Output,
    pub logging: Logging,
    pub development: Development,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Compression {
    pub format: String,
    pub method: String,
    pub level: u32,
    pub solid: bool,
    pub solid_block_size_mb: u64,
}
impl Default for Compression {
    fn default() -> Self {
        Self {
            format: "7z".into(),
            method: "lzma2".into(),
            level: 5,
            solid: true,
            solid_block_size_mb: 64,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ignore {
    pub gitignore: bool,
    pub dot_ignore: bool,
    pub exclude_git_directory: bool,
    pub include_hidden: bool,
    pub follow_symlinks: bool,
}
impl Default for Ignore {
    fn default() -> Self {
        Self {
            gitignore: true,
            dot_ignore: true,
            exclude_git_directory: true,
            include_hidden: true,
            follow_symlinks: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    pub suffix: String,
    pub on_conflict: String,
}
impl Default for Output {
    fn default() -> Self {
        Self {
            suffix: "-unbloat".into(),
            on_conflict: "rename".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    pub enabled: bool,
}
impl Default for Logging {
    fn default() -> Self {
        Self { enabled: true }
    }
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Development {
    pub test_input: PathBuf,
}
impl Default for Development {
    fn default() -> Self {
        Self {
            test_input: "./tests/fixtures/workspace".into(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            error(i18n::message(
                "config.read_failed",
                &[&crate::display_path(path), &e],
            ))
        })?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.compression.format != "7z" || self.compression.method != "lzma2" {
            return Err(error(i18n::text("config.format")));
        }
        if self.compression.level > 9 || !(1..=1024).contains(&self.compression.solid_block_size_mb)
        {
            return Err(error(i18n::text("config.level")));
        }
        if self.ignore.follow_symlinks || !self.ignore.exclude_git_directory {
            return Err(error(i18n::text("config.links")));
        }
        if self.output.on_conflict != "rename"
            || self
                .output
                .suffix
                .chars()
                .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
            || self.output.suffix.ends_with([' ', '.'])
        {
            return Err(error(i18n::text("config.output")));
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
