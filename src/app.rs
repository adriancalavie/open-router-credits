//! Dashboard state and event loop. Network calls run on a worker thread so the
//! UI keeps drawing (and stays responsive) while a request is in flight.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::api::{self, ApiError, Credits};
use crate::config::Config;
use crate::ui;

pub struct App {
    pub cfg: Config,
    pub config_path: PathBuf,
    pub credits: Option<Credits>,
    pub error: Option<String>,
    pub last_updated: Option<Instant>,
    pub in_flight: bool,
    pub show_help: bool,
    pub refresh_secs: u64,
    /// False when `NO_COLOR` is set: the UI then uses attributes only.
    pub color: bool,
    agent: ureq::Agent,
    should_quit: bool,
    last_request: Option<Instant>,
    tx: Sender<Result<Credits, ApiError>>,
    rx: Receiver<Result<Credits, ApiError>>,
}

impl App {
    pub fn new(cfg: Config, config_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let refresh_secs = cfg.refresh_secs.clamp(5, 3600);
        Self {
            cfg,
            config_path,
            credits: None,
            error: None,
            last_updated: None,
            in_flight: false,
            show_help: false,
            refresh_secs,
            color: std::env::var_os("NO_COLOR").is_none(),
            agent: api::agent(),
            should_quit: false,
            // `None` means "due now": the first frame triggers a fetch.
            last_request: None,
            tx,
            rx,
        }
    }

    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(self.refresh_secs)
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            terminal.draw(|frame| ui::draw(frame, self))?;
            self.pump();
            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    fn pump(&mut self) {
        if event::poll(Duration::from_millis(200)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            self.on_key(key);
        }

        self.drain_results();

        let due = match self.last_request {
            Some(at) => at.elapsed() >= self.refresh_interval(),
            None => true,
        };
        if due {
            self.request();
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('q') => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Esc => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('?') | KeyCode::Char('h') => self.show_help = !self.show_help,
            KeyCode::Char('r') => self.request(),
            _ => {}
        }
    }

    fn request(&mut self) {
        self.last_request = Some(Instant::now());
        if self.in_flight {
            return;
        }
        self.in_flight = true;

        let tx = self.tx.clone();
        let agent = self.agent.clone();
        let base = self.cfg.api_base();
        let key = self.cfg.api_key.clone();
        thread::spawn(move || {
            let result = api::fetch_credits(&agent, &base, &key);
            // Receiver is gone only when the app has already exited.
            let _ = tx.send(result);
        });
    }

    fn drain_results(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            self.in_flight = false;
            match result {
                Ok(credits) => {
                    self.credits = Some(credits);
                    self.error = None;
                    self.last_updated = Some(Instant::now());
                }
                Err(err) => self.error = Some(err.to_string()),
            }
        }
    }
}

pub fn run(cfg: Config, config_path: PathBuf) -> std::io::Result<()> {
    let mut app = App::new(cfg, config_path);
    ratatui::run(|terminal| app.run(terminal))
}
