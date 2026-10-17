use sevenz_rust2::{ArchiveReader, Password};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use unbloat::{archiver, config::Config, scanner};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "unbloat-test-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn put(&self, name: &str, data: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn config() -> Config {
    Config::parse(include_str!("../../../config.toml")).unwrap()
}
fn names(manifest: &unbloat::manifest::Manifest) -> BTreeSet<String> {
    manifest
        .items
        .iter()
        .map(|i| i.archive_path.clone())
        .collect()
}
fn scan(tree: &Tree, paths: &[&str]) -> unbloat::manifest::Manifest {
    scanner::scan(
        &paths.iter().map(|p| tree.0.join(p)).collect::<Vec<_>>(),
        &config().ignore,
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn extract(path: &Path, destination: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut reader = ArchiveReader::open(path, Password::empty()).unwrap();
    let mut contents = BTreeMap::new();
    reader
        .for_each_entries(|entry, stream| {
            let target = destination.join(entry.name());
            if entry.is_directory() {
                fs::create_dir_all(&target)?;
            } else {
                fs::create_dir_all(target.parent().unwrap())?;
                let mut bytes = Vec::new();
                stream.read_to_end(&mut bytes)?;
                fs::write(&target, &bytes)?;
                contents.insert(entry.name().to_owned(), bytes);
            }
            Ok(true)
        })
        .unwrap();
    contents
}

#[test]
fn preflight_real_two_projects_solid_roundtrip() {
    let tree = Tree::new();
    tree.put("frontend/.gitignore", "node_modules/\ndist/\n");
    tree.put("backend/.gitignore", "target/\n*.log\n");
    tree.put(
        "frontend/src/olá mundo.ts",
        "export const mensagem = 'Olá 🌍';\n",
    );
    tree.put("backend/src/main.rs", "fn main() { println!(\"oi\"); }\n");
    tree.put("frontend/node_modules/huge.js", "excluded");
    tree.put("frontend/dist/bundle.js", "excluded");
    tree.put("backend/target/program.exe", "excluded");
    tree.put("backend/error.log", "excluded");
    tree.put("backend/.git/objects/secret", "excluded");
    tree.put("backend/empty.txt", "");
    fs::create_dir_all(tree.0.join("frontend/empty dir")).unwrap();
    let manifest = scan(&tree, &["frontend", "backend"]);
    let expected: BTreeMap<String, Vec<u8>> = [
        ("frontend/.gitignore", "node_modules/\ndist/\n"),
        ("backend/.gitignore", "target/\n*.log\n"),
        (
            "frontend/src/olá mundo.ts",
            "export const mensagem = 'Olá 🌍';\n",
        ),
        ("backend/src/main.rs", "fn main() { println!(\"oi\"); }\n"),
        ("backend/empty.txt", ""),
    ]
    .into_iter()
    .map(|(p, s)| (p.into(), s.as_bytes().to_vec()))
    .collect();
    assert_eq!(scanner::included_files(&manifest), expected.len());
    let path = archiver::create(&manifest, &config(), &AtomicBool::new(false), &|_| {}).unwrap();
    assert!(
        ArchiveReader::open(&path, Password::empty())
            .unwrap()
            .archive()
            .is_solid
    );
    let extracted = tree.0.join("extracted");
    assert_eq!(extract(&path, &extracted), expected);
    assert!(extracted.join("frontend/empty dir").is_dir());
    assert!(!extracted.join("backend/target").exists());
    // Independent implementation, validation only; never needed by the product.
    let seven_zip = Path::new("C:/Program Files/7-Zip/7z.exe");
    if seven_zip.is_file() {
        let output = std::process::Command::new(seven_zip)
            .arg("t")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let independent = tree.0.join("7zip-extracted");
        assert!(
            std::process::Command::new(seven_zip)
                .arg("x")
                .arg(&path)
                .arg(format!("-o{}", independent.display()))
                .arg("-y")
                .output()
                .unwrap()
                .status
                .success()
        );
        for (name, bytes) in expected {
            assert_eq!(fs::read(independent.join(name)).unwrap(), bytes);
        }
    }
}

#[test]
fn ignore_scope_precedence_negation_hidden_and_no_git_repository() {
    let tree = Tree::new();
    tree.put(".gitignore", "project/\n"); // parent rules must not leak in
    tree.put(
        "project/.gitignore",
        "*.log\n!keep.log\nignored/\n!ignored/nope.txt\n",
    );
    tree.put("project/.ignore", "*.tmp\n!dot-keep.tmp\n");
    for name in [
        "keep.log",
        "drop.log",
        "drop.tmp",
        "dot-keep.tmp",
        ".hidden",
        "normal",
        "ignored/nope.txt",
        ".git/a",
        "sub/special",
        "sub/keep",
        "sub/only.log",
        "sub/nested.tmp",
    ] {
        tree.put(&format!("project/{name}"), "content");
    }
    tree.put("project/sub/.gitignore", "special\n!only.log\n");
    tree.put("project/sub/.ignore", "keep\n!nested.tmp\n");
    let first = scan(&tree, &["project"]);
    let all = names(&first);
    for included in [
        "keep.log",
        "dot-keep.tmp",
        ".hidden",
        "normal",
        "sub/only.log",
        "sub/nested.tmp",
        ".gitignore",
        ".ignore",
    ] {
        assert!(
            all.contains(&format!("project/{included}")),
            "missing {included}"
        );
    }
    for excluded in [
        "drop.log",
        "drop.tmp",
        "ignored/nope.txt",
        ".git/a",
        "sub/special",
        "sub/keep",
    ] {
        assert!(
            !all.contains(&format!("project/{excluded}")),
            "included {excluded}"
        );
    }
    assert_eq!(
        first
            .items
            .iter()
            .map(|i| &i.archive_path)
            .collect::<Vec<_>>(),
        scan(&tree, &["project"])
            .items
            .iter()
            .map(|i| &i.archive_path)
            .collect::<Vec<_>>()
    );
}

#[test]
fn no_ignore_empty_roots_and_pruned_large_directory() {
    let tree = Tree::new();
    tree.put("project/.env", "secret");
    fs::create_dir_all(tree.0.join("project/empty")).unwrap();
    assert!(names(&scan(&tree, &["project"])).contains("project/.env"));
    tree.put("project/.gitignore", "node_modules/\n");
    let baseline = scan(&tree, &["project"]).examined_entries;
    for n in 0..2000 {
        tree.put(&format!("project/node_modules/pkg{n}/file.js"), "ignored");
    }
    let manifest = scan(&tree, &["project"]);
    assert_eq!(manifest.examined_entries, baseline);
    assert!(
        manifest
            .items
            .iter()
            .all(|i| !i.archive_path.contains("node_modules"))
    );
}

#[test]
fn errors_cancel_mutation_missing_and_conflicts() {
    let tree = Tree::new();
    tree.put("project/a", "original");
    let manifest = scan(&tree, &["project"]);
    assert!(archiver::create(&manifest, &config(), &AtomicBool::new(true), &|_| {}).is_err());
    tree.put("project/a", "different and longer");
    assert!(archiver::create(&manifest, &config(), &AtomicBool::new(false), &|_| {}).is_err());
    fs::remove_file(tree.0.join("project/a")).unwrap();
    assert!(archiver::create(&manifest, &config(), &AtomicBool::new(false), &|_| {}).is_err());
    assert!(fs::read_dir(&tree.0).unwrap().all(|e| {
        !e.unwrap()
            .path()
            .to_string_lossy()
            .ends_with("unbloat-part")
    }));
    tree.put("project/a", "new content");
    let manifest = scan(&tree, &["project"]);
    fs::write(tree.0.join("project-unbloat.7z"), b"existing").unwrap();
    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            archiver::create(&manifest, &config(), &AtomicBool::new(false), &|_| {}).unwrap()
        });
        let b = scope.spawn(|| {
            archiver::create(&manifest, &config(), &AtomicBool::new(false), &|_| {}).unwrap()
        });
        let a = a.join().unwrap();
        let b = b.join().unwrap();
        assert_ne!(a, b);
    });
    assert_eq!(
        fs::read(tree.0.join("project-unbloat.7z")).unwrap(),
        b"existing"
    );
}

#[test]
fn non_solid_and_bounded_solid_blocks_roundtrip() {
    let tree = Tree::new();
    for n in 0..4 {
        tree.put(&format!("project/{n}.txt"), &"abcdefgh\n".repeat(70000));
    }
    let manifest = scan(&tree, &["project"]);
    let mut cfg = config();
    cfg.compression.solid_block_size_mb = 1;
    let path = archiver::create(&manifest, &cfg, &AtomicBool::new(false), &|_| {}).unwrap();
    assert_eq!(
        ArchiveReader::open(&path, Password::empty())
            .unwrap()
            .archive()
            .blocks
            .len(),
        4
    );
    assert_eq!(extract(&path, &tree.0.join("out")).len(), 4);
    cfg.compression.solid = false;
    let path = archiver::create(&manifest, &cfg, &AtomicBool::new(false), &|_| {}).unwrap();
    assert!(
        !ArchiveReader::open(path, Password::empty())
            .unwrap()
            .archive()
            .is_solid
    );
}

#[test]
fn rejects_invalid_selection_config_and_names() {
    let tree = Tree::new();
    tree.put("one/a", "1");
    tree.put("other/two/a", "2");
    assert!(
        scanner::scan(
            &[tree.0.join("one"), tree.0.join("other/two")],
            &config().ignore,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(
        scanner::scan(
            &[tree.0.join("missing")],
            &config().ignore,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    for name in ["../escape", "/absolute", "C:/drive"] {
        assert!(unbloat::manifest::archive_name(Path::new(name)).is_err());
    }
    let text = include_str!("../../../config.toml");
    assert!(Config::parse(&text.replace("level = 5", "level = 99")).is_err());
    assert!(Config::parse(&text.replace("solid = true", "solidd = true")).is_err());
}

#[test]
#[cfg(windows)]
fn junction_is_pruned_and_not_accepted_as_root() {
    let tree = Tree::new();
    tree.put("project/keep", "ok");
    tree.put("external/secret", "exclude");
    let link = tree.0.join("project/junction");
    let output = std::process::Command::new("cmd.exe")
        .arg("/d")
        .arg("/c")
        .arg("mklink")
        .arg("/J")
        .arg(link.to_string_lossy().replace('/', "\\"))
        .arg(tree.0.join("external").to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !names(&scan(&tree, &["project"]))
            .iter()
            .any(|p| p.contains("junction"))
    );
    assert!(
        scanner::scan(
            std::slice::from_ref(&link),
            &config().ignore,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    fs::remove_dir(link).unwrap(); // remove junction only, never its target
}

#[test]
#[cfg(windows)]
fn reports_locked_sources_and_unreadable_ignore_files() {
    use std::os::windows::fs::OpenOptionsExt;
    let tree = Tree::new();
    tree.put("project/secret.txt", "readable before locking");
    let manifest = scan(&tree, &["project"]);
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(tree.0.join("project/secret.txt"))
        .unwrap();
    let result = archiver::create(&manifest, &config(), &AtomicBool::new(false), &|_| {});
    assert!(result.is_err());
    drop(lock);
    tree.put("project/.gitignore", "secret.txt\n");
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(tree.0.join("project/.gitignore"))
        .unwrap();
    let result = scanner::scan(
        &[tree.0.join("project")],
        &config().ignore,
        &AtomicBool::new(false),
    );
    assert!(
        result.is_err(),
        "unreadable ignore must not silently change the selection"
    );
    drop(lock);
}

#[test]
fn cancellation_and_mutation_after_encoding_leave_no_published_archive() {
    let tree = Tree::new();
    tree.put("project/a.txt", "initial content");
    let manifest = scan(&tree, &["project"]);
    let cancel = AtomicBool::new(false);
    assert!(
        archiver::create(&manifest, &config(), &cancel, &|message| {
            if message.starts_with("Validando") {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .is_err()
    );
    assert!(
        archiver::create(&manifest, &config(), &AtomicBool::new(false), &|message| {
            if message.starts_with("Validando") {
                tree.put("project/a.txt", "changed after reading");
            }
        })
        .is_err()
    );
    assert!(
        fs::read_dir(&tree.0)
            .unwrap()
            .all(|e| e.unwrap().path().is_dir())
    );
}

#[test]
#[cfg(windows)]
fn symbolic_links_are_excluded_when_supported_by_host() {
    let tree = Tree::new();
    tree.put("project/regular", "keep");
    tree.put("external/secret", "exclude");
    let link = tree.0.join("project/link");
    match std::os::windows::fs::symlink_file(tree.0.join("external/secret"), &link) {
        Ok(()) => {
            assert!(!names(&scan(&tree, &["project"])).contains("project/link"));
            fs::remove_file(link).unwrap();
        }
        Err(e) if e.raw_os_error() == Some(1314) => eprintln!(
            "File-symlink creation requires Windows Developer Mode or privilege; junction coverage still runs."
        ),
        Err(e) => panic!("Unexpected symlink creation failure: {e}"),
    }
}
