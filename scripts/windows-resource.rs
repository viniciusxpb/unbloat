// Shared Windows SDK resource compilation for the executable and shell DLL.
use std::{env, path::PathBuf, process::Command};

fn compile_windows_resource(resource: &str) -> Option<PathBuf> {
    println!("cargo:rerun-if-changed=../../scripts/windows-resource.rs");
    println!("cargo:rerun-if-changed=../../assets/unbloat.ico");
    println!("cargo:rerun-if-changed=../../assets/{resource}");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return None;
    }
    let mut rc = PathBuf::from("rc.exe");
    if Command::new(&rc).arg("/?").output().is_err() {
        let kits = PathBuf::from(env::var_os("ProgramFiles(x86)").expect("Windows SDK required"))
            .join("Windows Kits/10/bin");
        let mut candidates: Vec<_> = std::fs::read_dir(kits)
            .expect("Install Windows SDK")
            .filter_map(Result::ok)
            .map(|e| e.path().join("x64/rc.exe"))
            .filter(|p| p.is_file())
            .collect();
        candidates.sort();
        rc = candidates.pop().expect("Windows SDK rc.exe required");
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("unbloat.res");
    let result = Command::new(rc)
        .current_dir("../../assets")
        .arg("/nologo")
        .arg("/fo")
        .arg(&output)
        .arg(resource)
        .status()
        .expect("Run Windows resource compiler");
    assert!(result.success(), "Resource compilation failed");
    Some(output)
}
