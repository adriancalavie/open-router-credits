//! open-router-credits — a compact htop-style view of your OpenRouter balance.

mod api;
mod app;
mod config;
mod format;
mod range;
mod setup;
mod ui;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use config::{Config, LoadError};
use range::Range;

const HELP: &str = "\
open-router-credits — live OpenRouter credit balance in a compact TUI

USAGE:
    open-router-credits [OPTIONS]

OPTIONS:
    --setup           run the setup wizard (asks for a new key)
    --once            print the balance once and exit, no dashboard
    --range <RANGE>   report one window instead of lifetime totals:
                      session, week, last7, month, last30, quarter, year, all
    --config <PATH>   use a different config file
    -h, --help        show this help
    -V, --version     show the version

CONFIG:
    ~/.config/open-router-credits/config.toml (mode 0600)

    A management key is required: https://openrouter.ai/settings/management-keys
    Inference keys are rejected with 403 by both endpoints.

WINDOWS:
    0  all time            lifetime totals from GET /credits
    1  this session        since this dashboard started, no query needed
    2  this week           since Monday 00:00 local
    3  last 7 days         rolling
    4  this month          since the 1st, local
    5  last 30 days        rolling
    6  this quarter        since the quarter's first day, local
    7  this year           since 1 January, local

    Windows come from POST /analytics/query with exact start/end instants, so a
    window starts at local midnight, not at a UTC day boundary. One query per
    window, refreshed every 5 minutes. 367 days is the server's limit, which
    covers \"this year\".

    In a window the RATE column replaces SHARE: $/day for the spend row, days
    of balance left at that rate for the remaining row. A share of everything
    ever purchased says less the longer you use the account.

    The bar follows the view: lifetime it is credit left of purchases, in a
    window it is credit left of the money that moved inside the window
    (remaining / (spent + remaining)).

KEYS:
    q / esc  quit         r  refresh now          ?  help
    0-7      switch the reported window

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
    range: Option<Range>,
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
                "--range" => {
                    let value = raw
                        .next()
                        .ok_or_else(|| "--range needs a value (try --help)".to_string())?;
                    args.range = Some(parse_range(&value)?);
                }
                other if other.starts_with("--range=") => {
                    args.range = Some(parse_range(other.trim_start_matches("--range="))?);
                }
                other => return Err(format!("unknown argument `{other}` (try --help)")),
            }
        }
        Ok(args)
    }
}

fn parse_range(value: &str) -> Result<Range, String> {
    Range::parse(value).ok_or_else(|| {
        format!("unknown range `{value}` — use session, week, last7, month, last30, quarter, year or all")
    })
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
        let range = args.range.unwrap_or(Range::All);
        let credits = api::fetch_credits(&agent, &config.api_base(), &config.api_key)?;
        let spend = match range.window(chrono::Local::now()) {
            Some((start, end)) => Some(api::fetch_spend(
                &agent,
                &config.api_base(),
                &config.api_key,
                &range::to_iso(start),
                &range::to_iso(end),
            )?),
            // A session that has not run yet has spent nothing.
            None if range == Range::Session => Some(0.0),
            None => None,
        };
        print!("{}", format::report(&credits, range, spend));
        return Ok(());
    }

    if !std::io::stdout().is_terminal() {
        return Err(
            invalid_input("the dashboard needs a terminal — use --once for plain output").into(),
        );
    }

    app::run(config, config_path, args.range.unwrap_or(Range::All))?;
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}
