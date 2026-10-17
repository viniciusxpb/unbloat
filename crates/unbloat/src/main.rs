#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{path::PathBuf, sync::atomic::AtomicBool};
use unbloat::{Result, archiver, config, error, scanner};
#[cfg(windows)]
mod launcher;
#[cfg(windows)]
#[allow(dead_code)] // The DLL writes this same protocol; the executable only reads it.
#[path = "../../unbloat-shell/src/selection.rs"]
mod selection;
#[cfg(windows)]
mod ui;

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
        } else if arg == "--selection-file" {
            if transport.is_some() {
                return Err(error("Transporte duplicado."));
            }
            transport = Some(PathBuf::from(
                args.next()
                    .ok_or_else(|| error("Falta o arquivo de seleção."))?,
            ));
        } else if arg == "--help" || arg == "-h" {
            println!(
                "Unbloat 0.2 — Ship the code. Not the node_modules.\n\nUso: unbloat [--] <arquivo-ou-pasta> [outros itens no mesmo diretório ...]\n     unbloat --dev-run\n\nConfiguração: config.toml ao lado do executável."
            );
            return Ok(());
        } else if arg.to_string_lossy().starts_with('-') {
            return Err(error("Argumento desconhecido. Use --help."));
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    if let Some(transport) = transport {
        if !paths.is_empty() || development {
            return Err(error("Não misture seleção interna com outros caminhos."));
        }
        #[cfg(windows)]
        {
            paths = selection::read_and_remove(&transport)?;
        }
        #[cfg(not(windows))]
        {
            let _ = transport;
            return Err(error("Transporte disponível apenas em Windows."));
        }
    }
    let config_path = config::location(development)?;
    let config = config::Config::load(&config_path)?;
    if development {
        if !paths.is_empty() {
            return Err(error("--dev-run usa apenas development.test_input."));
        }
        paths.push(
            config_path
                .parent()
                .unwrap()
                .join(&config.development.test_input),
        );
    }
    if paths.is_empty() {
        return Err(error(
            "Selecione um ou mais arquivos ou pastas, ou use --help.",
        ));
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
    status("Analisando arquivos…");
    let manifest = scanner::scan(&paths, &config.ignore, &cancel)?;
    status(&format!(
        "{} arquivos aprovados; {} entradas retornadas pelo scanner.",
        scanner::included_files(&manifest),
        manifest.examined_entries
    ));
    let path = archiver::create(&manifest, &config, &cancel, &status)?;
    println!("{}", path.display());
    Ok(())
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().len() > 1 && !std::env::args_os().any(|a| a == "--shell") {
        launcher::attach_parent_console();
    }
    if let Err(e) = run() {
        eprintln!("Unbloat: {e}");
        #[cfg(windows)]
        if std::env::args_os().len() == 1 || std::env::args_os().any(|a| a == "--shell") {
            ui::show_error(&e.to_string());
        }
        std::process::exit(1);
    }
}
