//! `make review`: the review instructions followed by the staged diff, on the clipboard.
use crate::{Result, i18n, root};
use std::{fs, path::Path, process::Command};

/// Read at run time, so edited instructions apply without rebuilding.
const INSTRUCTIONS: &str = "CODE_REVIEW.md";

pub fn copy() -> Result {
    let root = root();
    let instructions = fs::read_to_string(root.join(INSTRUCTIONS))?;
    // Leave the clipboard untouched when there is nothing to review.
    let staged = staged(&root)?.ok_or(i18n::text("review.nothing_staged"))?;
    let text = compose(&instructions, &staged);
    crate::clipboard::copy(&text)?;
    let kilobytes = text.len().div_ceil(1024);
    println!(
        "{}",
        i18n::message("review.copied", &[&staged.files, &kilobytes])
    );
    Ok(())
}

struct Staged {
    files: usize,
    stat: String,
    diff: String,
}

fn staged(repository: &Path) -> Result<Option<Staged>> {
    let git = |args: &[&str]| -> Result<String> {
        // Unquoted non-ASCII paths; no colors or external diff drivers in the text.
        let output = Command::new("git")
            .current_dir(repository)
            .args(["-c", "core.quotepath=off", "diff", "--staged"])
            .args(["--no-color", "--no-ext-diff"])
            .args(args)
            .output()
            .map_err(|e| i18n::message("review.git_failed", &[&e]))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(i18n::message("review.git_failed", &[&detail.trim()]).into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let files = git(&["--name-only"])?.lines().count();
    if files == 0 {
        return Ok(None);
    }
    Ok(Some(Staged {
        files,
        stat: git(&["--stat=200"])?,
        diff: git(&[])?,
    }))
}

/// Fences are longer than any backtick run in the diff, which may itself
/// contain Markdown, so the pasted text stays well-formed.
fn compose(instructions: &str, staged: &Staged) -> String {
    let longest = [&staged.stat, &staged.diff]
        .iter()
        .flat_map(|text| text.split(|c| c != '`'))
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    let block = |text: &str| {
        let newline = if text.ends_with('\n') { "" } else { "\n" };
        format!("{text}{newline}{fence}")
    };
    format!(
        "{}\n\n## git diff --staged --stat\n\n{fence}\n{}\n\n## git diff --staged\n\n{fence}diff\n{}\n",
        instructions.replace("\r\n", "\n").trim_end(),
        block(&staged.stat),
        block(&staged.diff),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Repository(std::path::PathBuf);
    impl Drop for Repository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copies_only_staged_changes_in_a_well_formed_fence() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = Repository(
            std::env::temp_dir().join(format!("xtask-review-{}-{nanos}", std::process::id())),
        );
        fs::create_dir(&repository.0).unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .current_dir(&repository.0)
                .args(args)
                .output()
        };
        if !git(&["init", "--quiet"]).is_ok_and(|o| o.status.success()) {
            eprintln!("git unavailable; skipping");
            return;
        }
        assert!(staged(&repository.0).unwrap().is_none());
        fs::write(repository.0.join("ação.md"), "olá\n```rust\n```\n").unwrap();
        assert!(
            staged(&repository.0).unwrap().is_none(),
            "untracked is not staged"
        );
        assert!(git(&["add", "ação.md"]).unwrap().status.success());
        let changes = staged(&repository.0).unwrap().unwrap();
        assert_eq!(changes.files, 1);
        assert!(changes.stat.contains("ação.md"), "{}", changes.stat);
        assert!(changes.diff.contains("+olá"));
        let text = compose("# Instruções\r\n", &changes);
        assert!(text.starts_with("# Instruções\n\n## git diff --staged --stat\n"));
        assert!(
            text.contains("\n````diff\n") && text.ends_with("\n````\n"),
            "{text}"
        );
    }
}
