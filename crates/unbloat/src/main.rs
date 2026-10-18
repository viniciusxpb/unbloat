#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use unbloat::i18n;

use std::{path::PathBuf, sync::atomic::AtomicBool};
use unbloat::{Result, archiver, config, error, scanner};
#[cfg(windows)]
mod launcher;
#[cfg(windows)]
#[allow(dead_code)] // The DLL writes this same protocol; the executable only reads it.
#[path = "../../unbloat-shell/src/selection.rs"]
mod selection;
#[cfg(windows)]
mod setup;
#[cfg(windows)]
mod ui;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Setup {
    Install,
    Uninstall,
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1).peekable();
    #[cfg(windows)]
    if args.peek().is_none() {
        return launcher::run();
    }
    let mut shell = false;
    let mut development = false;
    let mut paths = Vec::new();
    let mut transport = None;
    let mut literal = false;
    let mut setup = None;
    let mut install_dir = None;
    while let Some(arg) = args.next() {
        if literal {
            paths.push(PathBuf::from(arg));
            continue;
        }
        if arg == "--" {
            literal = true;
        } else if arg == "--shell" {
            shell = true;
        } else if arg == "--dev-run" {
            development = true;
        } else if arg == "--install" || arg == "--uninstall" {
            if setup.is_some() {
                return Err(error(i18n::text("cli.setup_options")));
            }
            setup = Some(if arg == "--install" {
                Setup::Install
            } else {
                Setup::Uninstall
            });
        } else if arg == "--install-dir" {
            install_dir = Some(PathBuf::from(
                args.next()
                    .ok_or_else(|| error(i18n::text("cli.setup_options")))?,
            ));
        } else if arg == "--selection-file" {
            if transport.is_some() {
                return Err(error(i18n::text("cli.duplicate_transport")));
            }
            transport = Some(PathBuf::from(
                args.next()
                    .ok_or_else(|| error(i18n::text("cli.missing_selection")))?,
            ));
        } else if arg == "--help" || arg == "-h" {
            println!(
                "{}",
                i18n::message("cli.help", &[&env!("CARGO_PKG_VERSION")])
            );
            return Ok(());
        } else if arg.to_string_lossy().starts_with('-') {
            return Err(error(i18n::text("cli.unknown_argument")));
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    if let Some(setup) = setup {
        if shell || development || transport.is_some() || !paths.is_empty() {
            return Err(error(i18n::text("cli.setup_options")));
        }
        return run_setup(setup, install_dir);
    }
    if install_dir.is_some() {
        return Err(error(i18n::text("cli.setup_options")));
    }
    if let Some(transport) = transport {
        if !paths.is_empty() || development {
            return Err(error(i18n::text("cli.mixed_transport")));
        }
        #[cfg(windows)]
        {
            paths = selection::read_and_remove(&transport)?;
        }
        #[cfg(not(windows))]
        {
            let _ = transport;
            return Err(error(i18n::text("cli.windows_only")));
        }
    }
    let config_path = config::location(development)?;
    let config = config::Config::load(&config_path)?;
    if development {
        if !paths.is_empty() {
            return Err(error(i18n::text("cli.development_paths")));
        }
        paths.push(
            config_path
                .parent()
                .unwrap()
                .join(&config.development.test_input),
        );
    }
    if paths.is_empty() {
        return Err(error(i18n::text("cli.no_selection")));
    }
    #[cfg(windows)]
    if shell {
        ui::run(paths, config)?;
        return Ok(());
    }
    let _ = shell;
    let cancel = AtomicBool::new(false);
    let status = |message: &str| {
        if config.logging.enabled {
            eprintln!("{message}");
        }
    };
    status(i18n::text("progress.scanning"));
    let manifest = scanner::scan(&paths, &config.ignore, &cancel)?;
    status(&i18n::message(
        "cli.scan_count",
        &[
            &scanner::included_files(&manifest),
            &manifest.examined_entries,
        ],
    ));
    let path = archiver::create(&manifest, &config, &cancel, &status)?;
    println!("{}", unbloat::display_path(&path));
    Ok(())
}

/// `--install` / `--uninstall`: the same per-user setup as the launcher buttons.
fn run_setup(action: Setup, dir: Option<PathBuf>) -> Result<()> {
    #[cfg(windows)]
    {
        let dir = match dir {
            Some(dir) => dir,
            None => setup::default_dir()?,
        };
        match action {
            Setup::Install => {
                let dir = setup::install_dir(&dir)?;
                let dir = setup::install(&setup::Package::current()?, &dir)?;
                println!(
                    "{}",
                    i18n::message("install.complete", &[&unbloat::display_path(&dir)])
                );
                println!("{}", i18n::text("install.usage"));
            }
            Setup::Uninstall => match setup::uninstall(&dir)? {
                setup::Uninstalled::Removed => println!("{}", i18n::text("uninstall.complete")),
                setup::Uninstalled::AlreadyRemoved => {
                    println!("{}", i18n::text("uninstall.already_removed"))
                }
            },
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (action, dir);
        Err(error(i18n::text("cli.windows_only")))
    }
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().len() > 1 && !std::env::args_os().any(|a| a == "--shell") {
        launcher::attach_parent_console();
    }
    if let Err(e) = run() {
        eprintln!("Unbloat: {e}");
        #[cfg(windows)]
        if (std::env::args_os().len() == 1 || std::env::args_os().any(|a| a == "--shell"))
            && e.downcast_ref::<ui::Reported>().is_none()
        {
            ui::show_error(&e.to_string());
        }
        std::process::exit(1);
    }
}
