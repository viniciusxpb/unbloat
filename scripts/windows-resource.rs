// Shared Windows SDK resource compilation for the executable and shell DLL.
use std::{env, path::PathBuf, process::Command};

fn compile_windows_resource(resource: &str) -> Option<PathBuf> {
    compile_localizations();
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
    // Cargo resolves version.workspace for both crates from the root manifest.
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numbers = ["MAJOR", "MINOR", "PATCH"].map(|part| {
        env::var(format!("CARGO_PKG_VERSION_{part}"))
            .unwrap()
            .parse::<u16>()
            .expect("Windows version components must fit in 16 bits")
    });
    let header = format!(
        "#define UNBLOAT_VERSION {},{},{},0\n#define UNBLOAT_VERSION_STRING \"{version}\\0\"\n",
        numbers[0], numbers[1], numbers[2]
    );
    std::fs::write(output.with_file_name("unbloat-version.h"), header)
        .expect("Write Windows version header");
    let result = Command::new(rc)
        .current_dir("../../assets")
        .arg("/nologo")
        .arg("/i")
        .arg(output.parent().unwrap())
        .arg("/fo")
        .arg(&output)
        .arg(resource)
        .status()
        .expect("Run Windows resource compiler");
    assert!(result.success(), "Resource compilation failed");
    Some(output)
}

/// Embed sorted static catalogs, with no parsing or heap cache inside Explorer.
fn compile_localizations() {
    use std::collections::BTreeMap;
    let mut generated = String::new();
    for (name, constant) in [("en", "EN"), ("pt-BR", "PT")] {
        let path = format!("../../locales/{name}.txt");
        println!("cargo:rerun-if-changed={path}");
        let source = std::fs::read_to_string(path).expect("Read UTF-8 localization catalog");
        let mut entries = BTreeMap::new();
        for line in source
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let (key, value) = line.split_once('=').expect("Invalid localization entry");
            assert!(
                entries
                    .insert(key, value.replace("\\r", "\r").replace("\\n", "\n"))
                    .is_none(),
                "Duplicate localization key: {key}"
            );
        }
        generated.push_str(&format!("const {constant}: &[(&str, &str)] = &[\n"));
        for (key, value) in entries {
            generated.push_str(&format!("({key:?}, {value:?}),\n"));
        }
        generated.push_str("];\n");
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("locales.rs");
    std::fs::write(output, generated).expect("Generate embedded localization catalogs");
}
