//! Reproducible, real engine benchmark. External 7-Zip is optional validation only.
use std::{fs, path::PathBuf, sync::atomic::AtomicBool, time::Instant};
use unbloat::{Result, archiver, config::Config, i18n, scanner};
fn main() -> Result<()> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/benchmark");
    fs::create_dir_all(&base)?;
    let unique = base.join(format!(
        "run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let root = unique.join("workspace");
    fs::create_dir_all(root.join("src"))?;
    fs::create_dir_all(root.join("node_modules"))?;
    fs::write(root.join(".gitignore"), "node_modules/\n")?;
    for n in 0..1500 {
        fs::write(
            root.join(format!("src/module-{n}.rs")),
            format!(
                "// module {n}\n{}",
                "pub fn hello() { println!(\"hello world\"); }\n".repeat(50)
            ),
        )?;
    }
    let config = Config::load(&base.join("../../config.toml"))?;
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    let baseline = scanner::scan(std::slice::from_ref(&root), &config.ignore, &cancel)?;
    let baseline_ms = start.elapsed().as_secs_f64() * 1000.;
    for n in 0..10000 {
        fs::write(
            root.join(format!("node_modules/ignored-{n}.js")),
            "excluded dependency\n".repeat(50),
        )?;
    }
    let start = Instant::now();
    let manifest = scanner::scan(std::slice::from_ref(&root), &config.ignore, &cancel)?;
    let scan_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(
        baseline.examined_entries, manifest.examined_entries,
        "ignored descendants were returned"
    );
    let start = Instant::now();
    let archive = archiver::create(&manifest, &config, &cancel, &|_| {})?;
    let archive_seconds = start.elapsed().as_secs_f64();
    let peak = peak_memory();
    let mut report = i18n::message(
        "benchmark.report",
        &[
            &scanner::included_files(&manifest),
            &manifest.examined_entries,
            &format!("{baseline_ms:.2}"),
            &format!("{scan_ms:.2}"),
            &format!("{archive_seconds:.3}"),
            &peak,
            &fs::metadata(&archive)?.len(),
        ],
    );
    let seven_zip = PathBuf::from("C:/Program Files/7-Zip/7z.exe");
    if seven_zip.is_file() {
        let comparison = unique.join("7zip.7z");
        let list = unique.join("approved.txt");
        fs::write(
            &list,
            manifest
                .items
                .iter()
                .filter(|i| !i.is_directory)
                .map(|i| i.archive_path.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        )?;
        let start = Instant::now();
        let output = std::process::Command::new(seven_zip)
            .current_dir(&unique)
            .args([
                "a",
                "-t7z",
                "-m0=LZMA2",
                "-mx=5",
                "-ms=64m",
                "-mmt=1",
                "-scsUTF-8",
            ])
            .arg(&comparison)
            .arg(format!("@{}", list.display()))
            .output()?;
        if !output.status.success() {
            return Err(unbloat::error(String::from_utf8_lossy(&output.stderr)));
        }
        report.push_str(&i18n::message(
            "benchmark.comparison",
            &[
                &format!("{:.3}", start.elapsed().as_secs_f64()),
                &fs::metadata(comparison)?.len(),
            ],
        ));
    } else {
        report.push_str(i18n::text("benchmark.no_comparison"));
    }
    report.push_str(i18n::text("benchmark.notes"));
    fs::write(base.join("RESULTS.md"), &report)?;
    print!("{report}");
    Ok(())
}
#[cfg(windows)]
fn peak_memory() -> usize {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(process: isize, info: *mut Counters, size: u32) -> i32;
    }
    let mut info: Counters = unsafe { std::mem::zeroed() };
    info.cb = size_of::<Counters>() as u32;
    if unsafe { GetProcessMemoryInfo(-1, &mut info, size_of::<Counters>() as u32) } != 0 {
        info.peak
    } else {
        0
    }
}
#[cfg(not(windows))]
fn peak_memory() -> usize {
    0
}
