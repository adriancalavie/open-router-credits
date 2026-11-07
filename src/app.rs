//! Dashboard state and event loop. Network calls run on a worker thread so the
//! UI keeps drawing (and stays responsive) while a request is in flight.
//!
//! Two data streams, with different rhythms: the balance is cheap and changes
//! every request, the spend windows are rollups that move slowly, so they are
//! fetched on a longer interval and only when a window is selected.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::api::{self, ApiError, Credits};
use crate::config::Config;
use crate::range::{Range, Spend, to_iso};
use crate::ui;

/// Spend windows are daily rollups upstream; polling them every minute would
/// only add queries without showing new money.
const SPEND_REFRESH_SECS: u64 = 300;

pub struct App {
    pub cfg: Config,
    pub config_path: PathBuf,
    pub credits: Option<Credits>,
    pub error: Option<String>,
    pub last_updated: Option<Instant>,
    pub in_flight: bool,
    pub show_help: bool,
    pub refresh_secs: u64,
    /// Which window the table reports. `Range::All` is the lifetime view.
    pub range: Range,
    pub spend: Option<Spend>,
    pub spend_error: Option<String>,
    pub spend_updated: Option<Instant>,
    pub in_spend_flight: bool,
    /// False when `NO_COLOR` is set: the UI then uses attributes only.
    pub color: bool,
    session_base: Option<f64>,
    /// When the panel started: the origin of the session window.
    started_at: Instant,
    agent: ureq::Agent,
    should_quit: bool,
    last_request: Option<Instant>,
    last_spend_request: Option<Instant>,
    tx: Sender<Result<Credits, ApiError>>,
    rx: Receiver<Result<Credits, ApiError>>,
    spend_tx: Sender<Result<Spend, ApiError>>,
    spend_rx: Receiver<Result<Spend, ApiError>>,
}

impl App {
    pub fn with_range(cfg: Config, config_path: PathBuf, range: Range) -> Self {
        let (tx, rx) = mpsc::channel();
        let (spend_tx, spend_rx) = mpsc::channel();
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
            range,
            spend: None,
            spend_error: None,
            spend_updated: None,
            in_spend_flight: false,
            color: std::env::var_os("NO_COLOR").is_none(),
            session_base: None,
            started_at: Instant::now(),
            agent: api::agent(),
            should_quit: false,
            // `None` means "due now": the first frame triggers a fetch.
            last_request: None,
            last_spend_request: None,
            tx,
            rx,
            spend_tx,
            spend_rx,
        }
    }

    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(self.refresh_secs)
    }

    pub fn spend_interval(&self) -> Duration {
        Duration::from_secs(SPEND_REFRESH_SECS)
    }

    /// Credits burned inside the selected window, if it is known yet.
    /// `Range::All` has no window: the lifetime figure is already in `credits`.
    pub fn window_spend(&self) -> Option<f64> {
        match self.range {
            Range::All => None,
            // The live lifetime counter gives the session window for free, and
            // it is exact: no rollup involves a rounding step.
            Range::Session => {
                let start = self.session_base?;
                let now = self.credits?.total_usage;
                Some((now - start).max(0.0))
            }
            range => self.spend.and_then(|spend| spend.get(range)),
        }
    }

    /// How many days the selected window has covered so far. `None` for the
    /// lifetime view, which has no start.
    pub fn window_days(&self) -> Option<f64> {
        let seconds = match self.range {
            Range::All => return None,
            Range::Session => self.started_at.elapsed().as_secs_f64(),
            range => {
                let (start, end) = range.window(chrono::Local::now())?;
                (end - start).num_seconds().max(0) as f64
            }
        };
        Some(seconds / 86_400.0)
    }

    /// Average spend per day across the window. `None` until the window has run
    /// for an hour: dividing by five minutes turns one request into a silly
    /// projection.
    pub fn burn_rate(&self) -> Option<f64> {
        let days = self.window_days()?;
        if days * 86_400.0 < 3_600.0 {
            return None;
        }
        self.window_spend().map(|spend| spend / days)
    }

    /// Days of balance left if the window's average rate holds.
    pub fn runway_days(&self) -> Option<f64> {
        let rate = self.burn_rate()?;
        if rate <= 0.0 {
            return None;
        }
        self.credits
            .map(|credits| (credits.remaining() / rate).max(0.0))
    }

    /// What is left as a share of the money that moved during the window:
    /// everything spent inside it plus what still sits in the account.
    /// `None` while the window query is still in flight.
    /// This includes any top-up made inside the window, because the balance
    /// does; the API has no way to tell the two apart.
    pub fn window_left_fraction(&self) -> Option<f64> {
        let spend = self.window_spend()?;
        let remaining = self.credits?.remaining().max(0.0);
        let moved = spend + remaining;
        (moved > 0.0).then(|| remaining / moved)
    }

    /// Seeds the session baseline without waiting for a first fetch. Only used
    /// by tests; in the app the baseline comes from the first `GET /credits`.
    #[cfg(test)]
    pub fn set_session_base(&mut self, usage: f64) {
        self.session_base = Some(usage);
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
        self.drain_spend();

        let due = match self.last_request {
            Some(at) => at.elapsed() >= self.refresh_interval(),
            None => true,
        };
        if due {
            self.request();
        }

        // No window selected means no analytics traffic at all.
        if self.range.needs_api() && !self.in_spend_flight {
            let due = match self.last_spend_request {
                Some(at) => at.elapsed() >= self.spend_interval(),
                None => true,
            };
            if due {
                self.request_spend();
            }
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
            KeyCode::Char('r') => {
                self.request();
                if self.range.needs_api() {
                    self.last_spend_request = None;
                }
            }
            KeyCode::Char(ch) => {
                if let Some(range) = Range::from_key(ch) {
                    self.select_range(range);
                }
            }
            _ => {}
        }
    }

    /// Switching windows reuses a fresh result and only re-queries when the
    /// cached one has aged out, so cycling through the keys is instant.
    fn select_range(&mut self, range: Range) {
        self.range = range;
        let stale = match self.spend_updated {
            Some(at) => at.elapsed() >= self.spend_interval(),
            None => true,
        };
        if range.needs_api() && stale {
            self.last_spend_request = None;
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

    /// Fetches every window in one background job: six small queries, one round
    /// trip each, so switching windows never waits on the network.
    fn request_spend(&mut self) {
        self.last_spend_request = Some(Instant::now());
        if self.in_spend_flight {
            return;
        }
        self.in_spend_flight = true;

        let tx = self.spend_tx.clone();
        let agent = self.agent.clone();
        let base = self.cfg.api_base();
        let key = self.cfg.api_key.clone();
        thread::spawn(move || {
            let _ = tx.send(fetch_windows(&agent, &base, &key, chrono::Local::now()));
        });
    }

    fn drain_results(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            self.in_flight = false;
            match result {
                Ok(credits) => {
                    // First successful read of the lifetime counter is the
                    // session baseline. Anything spent before launch counts as
                    // history, not as this session.
                    self.session_base.get_or_insert(credits.total_usage);
                    self.credits = Some(credits);
                    self.error = None;
                    self.last_updated = Some(Instant::now());
                }
                Err(err) => self.error = Some(err.to_string()),
            }
        }
    }

    fn drain_spend(&mut self) {
        while let Ok(result) = self.spend_rx.try_recv() {
            self.in_spend_flight = false;
            match result {
                Ok(spend) => {
                    self.spend = Some(spend);
                    self.spend_error = None;
                    self.spend_updated = Some(Instant::now());
                }
                // A failed window query must not wipe the balance view; it gets
                // its own line so the two streams stay independent.
                Err(err) => self.spend_error = Some(err.to_string()),
            }
        }
    }
}

fn fetch_windows(
    agent: &ureq::Agent,
    base: &str,
    key: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Result<Spend, ApiError> {
    let mut spend = Spend::default();
    for range in Range::ASKED {
        let (start, end) = range.window(now).expect("every asked range has a window");
        let value = api::fetch_spend(agent, base, key, &to_iso(start), &to_iso(end))?;
        spend.set(range, value);
    }
    Ok(spend)
}

pub fn run(cfg: Config, config_path: PathBuf, range: Range) -> std::io::Result<()> {
    let mut app = App::with_range(cfg, config_path, range);
    ratatui::run(|terminal| app.run(terminal))
}
