//! open-router-credits — a compact htop-style view of your OpenRouter balance.

mod api;
mod app;
mod config;
mod format;
mod setup;
mod ui;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use config::{Config, LoadError};

const HELP: &str = "\
open-router-credits — live OpenRouter credit balance in a compact TUI

USAGE:
    open-router-credits [OPTIONS]

OPTIONS:
    --setup           run the setup wizard (asks for a new key)
    --once            print the balance once and exit, no dashboard
    --config <PATH>   use a different config file
    -h, --help        show this help
    -V, --version     show the version

CONFIG:
    ~/.config/open-router-credits/config.toml (mode 0600)

    A management key is required: https://openrouter.ai/settings/management-keys
    Inference keys are rejected with 403 by GET /api/v1/credits.

KEYS:
    q / esc  quit        r  refresh now        ?  help

ENV:
    OPENROUTER_API_BASE   override the API base URL (default https://openrouter.ai/api/v1)
";

#[derive(Default)]
struct Args {
    setup: bool,
    once: bool,
    help: bool,
    version: bool,
    config: Option<PathBuf>,
}

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = Args::default();
        let mut raw = raw.peekable();
        while let Some(arg) = raw.next() {
            match arg.as_str() {
                "--setup" => args.setup = true,
                "--once" => args.once = true,
                "-h" | "--help" => args.help = true,
                "-V" | "--version" => args.version = true,
                "--config" => {
                    let path = raw
                        .next()
                        .ok_or_else(|| "--config needs a path".to_string())?;
                    args.config = Some(PathBuf::from(path));
                }
                other if other.starts_with("--config=") => {
                    let path = other.trim_start_matches("--config=");
                    if path.is_empty() {
                        return Err("--config needs a path".to_string());
                    }
                    args.config = Some(PathBuf::from(path));
                }
                other => return Err(format!("unknown argument `{other}` (try --help)")),
            }
        }
        Ok(args)
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse(std::env::args().skip(1)).map_err(invalid_input)?;

    if args.help {
        print!("{HELP}");
        return Ok(());
    }
    if args.version {
        println!("open-router-credits {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let config_path = args.config.clone().unwrap_or_else(Config::default_path);
    let existing = match Config::load(&config_path) {
        Ok(config) => Some(config),
        Err(LoadError::NotFound) => None,
        Err(err) => return Err(err.into()),
    };

    let config = match (existing, args.setup) {
        (Some(config), false) => config,
        (existing, _) => {
            if !std::io::stdin().is_terminal() {
                return Err(invalid_input(format!(
                    "no config at {} — run `open-router-credits --setup` in a terminal",
                    config_path.display()
                ))
                .into());
            }
            setup::run(&config_path, existing.as_ref())?
        }
    };

    if args.once {
        let agent = api::agent();
        let credits = api::fetch_credits(&agent, &config.api_base(), &config.api_key)?;
        print!("{}", format::report(&credits));
        return Ok(());
    }

    if !std::io::stdout().is_terminal() {
        return Err(
            invalid_input("the dashboard needs a terminal — use --once for plain output").into(),
        );
    }

    app::run(config, config_path)?;
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}
