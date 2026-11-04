//! First-run setup wizard. Plain terminal prompts, no alternate screen, so the
//! output stays readable if anything goes wrong.

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::style::Stylize;
use crossterm::{execute, terminal};
use std::io::{self, IsTerminal, Write};
use std::path::Path;

use crate::api::{self, ApiError, MANAGEMENT_KEYS_URL};
use crate::config::Config;
use crate::format::{masked_key, money};

pub fn run(path: &Path, existing: Option<&Config>) -> io::Result<Config> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "setup needs an interactive terminal",
        ));
    }

    println!();
    println!("{}", "open-router-credits · setup".bold().cyan());
    println!("{}", "─".repeat(46).dim());
    println!();
    println!(
        "Credits are read with a {} key; inference keys get a 403.",
        "management".bold()
    );
    println!("Create one here (set an expiration):");
    println!("  {}", MANAGEMENT_KEYS_URL.cyan().underlined());
    println!();

    let mut api_key = match existing {
        Some(config) => {
            let keep = prompt_yes_no(
                &format!("Keep the existing key ({})?", masked_key(&config.api_key)),
                true,
            )?;
            if keep {
                config.api_key.clone()
            } else {
                ask_for_key()?
            }
        }
        None => ask_for_key()?,
    };

    let agent = api::agent();
    let base = existing.map(Config::api_base).unwrap_or_else(|| {
        std::env::var("OPENROUTER_API_BASE").unwrap_or_else(|_| api::DEFAULT_API_BASE.to_string())
    });

    loop {
        print!("Checking key… ");
        io::stdout().flush()?;
        match api::fetch_credits(&agent, &base, &api_key) {
            Ok(credits) => {
                println!("{}", "ok".green().bold());
                println!(
                    "  {} purchased · {} used · {} remaining",
                    money(credits.total_credits),
                    money(credits.total_usage),
                    money(credits.remaining()).green()
                );
                break;
            }
            Err(err) => {
                println!("{}", "failed".red().bold());
                println!("  {err}");
                if matches!(err, ApiError::Unauthorized | ApiError::Forbidden) {
                    println!("  Keys are created at {}", MANAGEMENT_KEYS_URL.cyan());
                }
                if !prompt_yes_no("Try a different key?", true)? {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "setup cancelled",
                    ));
                }
                api_key = ask_for_key()?;
            }
        }
    }

    let default_refresh = existing.map(|config| config.refresh_secs).unwrap_or(60);
    let refresh_secs = prompt_refresh(default_refresh)?;

    let mut config = Config::new(api_key);
    config.refresh_secs = refresh_secs;
    config.api_base = existing.and_then(|config| config.api_base.clone());
    config.save(path)?;

    println!();
    println!("Saved to {}", path.display().to_string().bold());
    println!("{}", "  (mode 0600 — it holds a management key)".dim());
    println!();
    Ok(config)
}

fn ask_for_key() -> io::Result<String> {
    loop {
        println!("Paste the management key and press Enter (input is hidden):");
        match prompt_secret("  key › ")? {
            Some(key) if !key.is_empty() => return Ok(key),
            Some(_) => println!("{}", "  nothing entered, try again".yellow()),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "setup cancelled",
                ));
            }
        }
    }
}

fn prompt_secret(label: &str) -> io::Result<Option<String>> {
    print!("{label}");
    io::stdout().flush()?;
    terminal::enable_raw_mode()?;
    let _ = execute!(io::stdout(), EnableBracketedPaste);

    let result = read_hidden_line();

    let _ = execute!(io::stdout(), DisableBracketedPaste);
    terminal::disable_raw_mode()?;
    println!();
    result
}

fn read_hidden_line() -> io::Result<Option<String>> {
    let mut buffer = String::new();
    loop {
        match event::read()? {
            Event::Key(KeyEvent {
                code,
                kind: KeyEventKind::Press,
                modifiers,
                ..
            }) => match code {
                KeyCode::Enter => return Ok(Some(buffer.trim().to_string())),
                KeyCode::Esc => return Ok(None),
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "setup cancelled",
                    ));
                }
                KeyCode::Backspace => {
                    if buffer.pop().is_some() {
                        print!("\u{8} \u{8}");
                        io::stdout().flush()?;
                    }
                }
                KeyCode::Char(ch) => {
                    buffer.push(ch);
                    print!("•");
                    io::stdout().flush()?;
                }
                _ => {}
            },
            Event::Paste(text) => {
                buffer.push_str(&text);
                print!("{}", "•".repeat(text.chars().count()));
                io::stdout().flush()?;
            }
            _ => {}
        }
    }
}

fn prompt_refresh(default: u64) -> io::Result<u64> {
    loop {
        print!("Refresh every how many seconds? [{}] ", default);
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "setup cancelled",
            ));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(default);
        }
        match trimmed.parse::<u64>() {
            Ok(secs) if (5..=3600).contains(&secs) => return Ok(secs),
            _ => println!("{}", "  enter a number between 5 and 3600".yellow()),
        }
    }
}

fn prompt_yes_no(question: &str, default_yes: bool) -> io::Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        print!("{question} {hint} ");
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "setup cancelled",
            ));
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default_yes),
            "y" | "yes" => return Ok(true),
            "n" | "no" | "q" | "quit" => return Ok(false),
            _ => println!("{}", "  please answer y or n".yellow()),
        }
    }
}
