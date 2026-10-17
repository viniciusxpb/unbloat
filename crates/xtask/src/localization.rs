//! UTF-8 output, language selection and fallback of make help, the CLI and,
//! on Windows, the PowerShell installer diagnostics and the native launcher.
use crate::{MAKE_TARGETS, Result, i18n, root};
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Explicit tags, regional variants, an unsupported locale and the system default.
const LANGUAGES: [&str; 5] = ["pt-BR", "en", "pt-PT", "fr-FR", ""];

pub fn check(args: &[String]) -> Result {
    let mut windows = false;
    let mut executable = root()
        .join("target/debug")
        .join(format!("unbloat{}", std::env::consts::EXE_SUFFIX));
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--windows" if cfg!(windows) => windows = true,
            "--executable" => executable = args.next().ok_or("--executable needs a path")?.into(),
            other => return Err(format!("unsupported localization option '{other}'").into()),
        }
    }
    if !executable.is_file() {
        return Err("Build unbloat before running localization checks.".into());
    }
    let version = crate::version()?;
    for tag in LANGUAGES {
        let language = if tag.is_empty() {
            i18n::system_language()
        } else {
            i18n::from_tag(tag)
        };
        let text = |key: &str| i18n::text_for(language, key);
        let help = capture(
            &mut Command::new("make"),
            &["--no-print-directory", "help"],
            tag,
            0,
        )?
        .stdout;
        ensure(
            help.starts_with(text("make.usage")) && help.contains(text("make.heading")),
            format!("Help language/encoding mismatch: {tag:?}"),
        )?;
        for target in MAKE_TARGETS {
            ensure(
                help.contains(text(&format!("make.{target}"))),
                format!("Missing help entry: {tag:?}/{target}"),
            )?;
        }
        let default = capture(&mut Command::new("make"), &["--no-print-directory"], tag, 0)?;
        ensure(default.stdout == help, "make and make help differ.")?;
        let cli = capture(&mut Command::new(&executable), &["--help"], tag, 0)?.stdout;
        let expected = i18n::render(text("cli.help"), &[&version]).replace("\r\n", "\n");
        ensure(
            cli == expected.trim_end(),
            format!("CLI language/encoding mismatch: {tag:?}"),
        )?;
        let invalid = capture(
            &mut Command::new(&executable),
            &["--invalid-option"],
            tag,
            1,
        )?;
        ensure(
            invalid.stderr.contains(text("cli.unknown_argument")),
            format!("CLI diagnostic mismatch: {tag:?}"),
        )?;
        #[cfg(windows)]
        {
            installer_diagnostics(tag, &text)?;
            if windows {
                crate::window::check_launcher(&executable, tag, &text)?;
            }
        }
        let checked = if cfg!(windows) {
            "make/CLI/install/uninstall"
        } else {
            "make/CLI"
        };
        println!("PASS: language={tag:?}, {checked} language and UTF-8 checks");
    }
    #[cfg(windows)]
    installer_placeholders()?;
    let _ = windows;
    Ok(())
}

fn ensure(condition: bool, message: impl Into<String>) -> Result {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

struct Captured {
    stdout: String,
    stderr: String,
}

/// Runs with `UNBLOAT_LANG` set to `tag` (removed when empty), requiring the
/// exit code, strict UTF-8 and completion within 30 seconds.
fn capture(command: &mut Command, args: &[&str], tag: &str, expected: i32) -> Result<Captured> {
    command.args(args).current_dir(root()).stdin(Stdio::null());
    if tag.is_empty() {
        command.env_remove("UNBLOAT_LANG");
    } else {
        command.env("UNBLOAT_LANG", tag);
    }
    let description = format!("{command:?}");
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let drain = |mut stream: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).map(|_| bytes)
        })
    };
    let stdout = drain(Box::new(child.stdout.take().expect("piped stdout")));
    let stderr = drain(Box::new(child.stderr.take().expect("piped stderr")));
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(format!("Localized command timed out: {description}").into());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let decode = |output: thread::JoinHandle<std::io::Result<Vec<u8>>>| -> Result<String> {
        let bytes = output.join().map_err(|_| "output reader panicked")??;
        let text =
            String::from_utf8(bytes).map_err(|_| format!("Output is not UTF-8: {description}"))?;
        Ok(text.replace("\r\n", "\n").trim_end().to_owned())
    };
    let captured = Captured {
        stdout: decode(stdout)?,
        stderr: decode(stderr)?,
    };
    if status.code() != Some(expected) {
        return Err(format!(
            "Unexpected exit code {status} from {description}. {}",
            captured.stderr
        )
        .into());
    }
    Ok(captured)
}

#[cfg(windows)]
fn powershell() -> Result<Command> {
    let system = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    Ok(Command::new(
        std::path::PathBuf::from(system).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
    ))
}

/// The shipped installer scripts must refuse a drive root in the selected language.
#[cfg(windows)]
fn installer_diagnostics(tag: &str, text: &dyn Fn(&str) -> &'static str) -> Result {
    let root = root();
    let drive = root
        .ancestors()
        .last()
        .ok_or("repository has no drive root")?;
    let drive_root = format!("{}.", drive.display());
    for script in ["install", "uninstall"] {
        let path = root.join("scripts").join(format!("{script}.ps1"));
        let path = path.to_str().ok_or("non-UTF-8 repository path")?;
        let args = [
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            path,
            "-InstallDir",
            &drive_root,
        ];
        let output = capture(&mut powershell()?, &args, tag, 1)?;
        ensure(
            format!("{}\n{}", output.stdout, output.stderr)
                .contains(text(&format!("{script}.invalid_directory"))),
            format!("Installer diagnostic mismatch: {tag:?}/{script}"),
        )?;
    }
    Ok(())
}

/// Substitution in the PowerShell catalog loader must never interpret braces in a path.
#[cfg(windows)]
fn installer_placeholders() -> Result {
    let loader = root().join(r"scripts\i18n.ps1");
    let loader = loader
        .to_str()
        .ok_or("non-UTF-8 repository path")?
        .replace('\'', "''");
    let script = format!(
        ". '{loader}'; Get-UnbloatText 'config.read_failed' @('C:\\{{1}}\\file', 'DETAIL')"
    );
    let output = capture(
        &mut powershell()?,
        &[
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ],
        "",
        0,
    )?;
    ensure(
        output.stdout.ends_with("DETAIL"),
        "Missing placeholder value.",
    )?;
    ensure(
        output.stdout.contains(r"C:\{1}\file"),
        "Path interpreted as a placeholder.",
    )
}
