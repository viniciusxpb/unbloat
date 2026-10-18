//! Portable development tasks, run as `cargo xtask <command>`; the Makefile calls these.
//! Checks of the Windows-only Explorer and installer integration stay in scripts/*.ps1.
mod clipboard;
#[path = "../../../shared/i18n.rs"]
mod i18n;
mod licenses;
mod localization;
mod package;
mod review;
#[cfg(windows)]
mod window;

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Targets described by `make help`, in Makefile order.
const MAKE_TARGETS: [&str; 9] = [
    "help",
    "build",
    "test",
    "run",
    "install",
    "uninstall",
    "package",
    "benchmark",
    "review",
];
const USAGE: &str = "commands: help, licenses, package [--no-open], install, uninstall, review, localization [--windows] [--executable <path>]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("help") => {
            help();
            Ok(())
        }
        Some("licenses") => licenses::collect(),
        Some("package") => package::create(&args[1..]),
        Some("install") => install(),
        Some("uninstall") => uninstall(),
        Some("review") => review::copy(),
        Some("localization") => localization::check(&args[1..]),
        Some(other) => Err(format!("unknown command '{other}'; {USAGE}").into()),
    };
    if let Err(e) = result {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}

fn help() {
    println!("{}", i18n::text("make.usage"));
    println!();
    println!("{}", i18n::text("make.heading"));
    for target in MAKE_TARGETS {
        println!("  {target:<11} {}", i18n::text(&format!("make.{target}")));
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask lives in crates/xtask")
        .to_path_buf()
}

/// The cargo running this task, so toolchain overrides apply to nested builds too.
fn cargo() -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root());
    command
}

fn run(command: &mut Command) -> Result {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("{command:?} failed: {status}").into());
    }
    Ok(())
}

fn cargo_metadata(dependencies: bool) -> Result<serde_json::Value> {
    let mut command = cargo();
    command.args(["metadata", "--format-version", "1", "--locked"]);
    // With dependencies, cargo may download sources of other platforms' crates,
    // which a build never fetches; --locked still pins the Cargo.lock versions.
    if !dependencies {
        command.args(["--no-deps", "--offline"]);
    }
    let output = command.stderr(Stdio::inherit()).output()?;
    if !output.status.success() {
        return Err(i18n::text("package.metadata_failed").into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn field<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("cargo metadata: missing '{key}'").into())
}

/// The workspace version, read from the `unbloat` package like the Windows resources.
fn version() -> Result<String> {
    let metadata = cargo_metadata(false)?;
    let members = metadata["workspace_members"].as_array();
    let packages: Vec<_> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|package| {
            package["name"] == "unbloat" && members.is_some_and(|m| m.contains(&package["id"]))
        })
        .collect();
    match packages.as_slice() {
        [package] => Ok(field(package, "version")?.to_owned()),
        _ => Err(i18n::text("package.missing_package").into()),
    }
}

/// Installs from the freshly built package, exactly as a user's double click would.
fn install() -> Result {
    package::create(&["--no-open".to_owned()])?;
    let exe = root().join("dist").join("Unbloat").join(executable());
    run(Command::new(exe).arg("--install"))
}

/// Any build of unbloat can remove the user's installation.
fn uninstall() -> Result {
    run(cargo().args(["build", "--release", "--locked", "--package", "unbloat"]))?;
    let exe = root().join("target").join("release").join(executable());
    run(Command::new(exe).arg("--uninstall"))
}

fn executable() -> String {
    format!("unbloat{}", std::env::consts::EXE_SUFFIX)
}
