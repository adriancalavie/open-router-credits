//! The dashboard: one compact, htop-ish credits panel.
//!
//! Color policy: layout and hierarchy come from the terminal's own foreground
//! plus attributes (`faint`, `bold`, `reversed`), never from palette slots such
//! as bright-black. Themes remap those freely — a Ghostty config can point
//! palette 8 at the background color, which makes SGR 90 text invisible.
//! Real color is reserved for meaning (balance health, errors) and dropped
//! entirely when `NO_COLOR` is set.
//!
//! Layout: every element sits on one grid. Labels start at the left edge, the
//! USD and share columns are right-aligned, and the gauge spans the content
//! width with its value at the end, so the right edge never looks ragged.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table};
use std::time::Instant;

use crate::app::App;
use crate::format::{
    bar_spans, days, left_color, masked_key, money, percent, rate, remaining_color,
};
use crate::range::Range;

const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 12;
const PANEL_WIDTH: u16 = 76;
const PANEL_HEIGHT: u16 = 12;

fn faint() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn faint_bold() -> Style {
    Style::default().add_modifier(Modifier::DIM | Modifier::BOLD)
}

/// Semantic foreground color, dropped when the user asked for no color.
fn tint(color_on: bool, value: Color) -> Style {
    if color_on {
        Style::default().fg(value)
    } else {
        Style::default()
    }
}

/// Reverse video on the terminal's own fg/bg pair: text becomes the background
/// color, fill becomes the foreground color. Maximum contrast on every theme,
/// including light ones where a colored background would wash the text out.
fn badge() -> Style {
    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let message = Paragraph::new(vec![
            Line::from(Span::styled("open-router-credits", bold())),
            Line::from(""),
            Line::from(format!(
                "terminal too small — need at least {MIN_WIDTH}x{MIN_HEIGHT}"
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(message, centered(area, area.width.min(48), 3));
        return;
    }

    let width = area.width.min(PANEL_WIDTH);
    // A window drops the "Purchased" row, so the frame shrinks with it.
    let height = if app.range == Range::All {
        PANEL_HEIGHT
    } else {
        PANEL_HEIGHT - 1
    };
    let panel = centered(area, width, height);
    let inner = panel.inner(ratatui::layout::Margin::new(1, 1));
    // Keep one column of breathing room so right-aligned values don't touch the border.
    let content = Rect {
        width: inner.width.saturating_sub(1),
        ..inner
    };

    let title = match app.range {
        Range::All => " open-router-credits ".to_string(),
        range => format!(" open-router-credits · {} ", range.label()),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(faint())
        .title(Line::from(Span::styled(title, bold())));
    frame.render_widget(block, panel);

    // Alerts live in the frame, not in a row of their own: the border carries
    // state the way a window title does.
    if let Some(credits) = app.credits {
        if credits.remaining() <= 0.0 {
            draw_badge(frame, panel, "OUT OF CREDITS");
        } else if credits.remaining() < 5.0 {
            draw_badge(frame, panel, "LOW BALANCE");
        }
    }

    let rows = Layout::vertical([
        Constraint::Length(1), // breathing room under the frame
        // Three rows for a window, four for the lifetime view: the spare line
        // is dropped rather than left as a gap under the table.
        Constraint::Length(if app.range == Range::All { 4 } else { 3 }),
        Constraint::Length(1), // spacer
        Constraint::Length(1), // gauge
        Constraint::Length(1), // spacer
        Constraint::Length(1), // footer
        Constraint::Min(1),    // error (only when present)
    ])
    .split(content);

    draw_table(frame, rows[1], app);
    draw_gauge(frame, rows[3], app);
    draw_footer(frame, rows[5], app);
    if app.error.is_some() || app.spend_error.is_some() {
        draw_error(frame, rows[6], app);
    }

    if app.show_help {
        draw_help(frame, area, app);
    }
}

fn draw_badge(frame: &mut Frame, panel: Rect, label: &str) {
    let text = format!(" {label} ");
    let width = text.chars().count() as u16;
    let rect = Rect {
        x: panel.x + panel.width.saturating_sub(width + 2),
        y: panel.y,
        width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, badge()))),
        rect,
    );
}

fn draw_table(frame: &mut Frame, area: Rect, app: &App) {
    // The lifetime view reports shares of purchases. A window cannot: that
    // denominator grows forever, so the same row carries a burn rate instead.
    let windowed = app.range != Range::All;
    let header = Row::new(vec![
        Cell::from("METRIC"),
        Cell::from(Line::from("USD").right_aligned()),
        Cell::from(Line::from(if windowed { "RATE" } else { "SHARE" }).right_aligned()),
    ])
    .style(faint_bold());

    let purchased = app.credits.map(|c| c.total_credits).unwrap_or(0.0);
    let remaining = app.credits.map(|c| c.remaining()).unwrap_or(0.0);
    let remaining_fraction = app.credits.and_then(|credits| credits.remaining_fraction());
    let remain_color = app
        .credits
        .map(|credits| remaining_color(&credits))
        .unwrap_or(Color::Yellow);

    // Share is always "of everything ever purchased", so numbers stay
    // comparable between the lifetime view and a window.
    let share_of_purchased =
        |amount: f64| (purchased > 0.0).then(|| (amount / purchased).clamp(0.0, 1.0));

    let label = |text: String| Cell::from(Line::from(Span::styled(text, faint())));
    let amount = |value: Option<f64>| {
        // `None` means the window query has not answered yet, not zero.
        let text = value.map(money).unwrap_or_else(|| "…".to_string());
        Cell::from(Line::from(Span::styled(text, bold())).right_aligned())
    };
    let share = |fraction: Option<f64>| {
        Cell::from(
            Line::from(Span::styled(
                fraction.map(percent).unwrap_or_else(|| "—".into()),
                faint(),
            ))
            .right_aligned(),
        )
    };

    let mut rows = Vec::new();
    if app.range == Range::All {
        rows.push(Row::new(vec![
            label("Purchased".into()),
            amount(Some(purchased)),
            share(share_of_purchased(purchased)),
        ]));
        rows.push(Row::new(vec![
            label("Used".into()),
            amount(app.credits.map(|c| c.total_usage)),
            share(app.credits.and_then(|c| c.used_fraction())),
        ]));
    } else {
        // A window replaces the lifetime "Used" row instead of adding to it:
        // the question changes from "how much in total" to "how much lately".
        let window = app.window_spend();
        let price = |value: Option<f64>| {
            Cell::from(
                Line::from(Span::styled(
                    value.map(rate).unwrap_or_else(|| "—".into()),
                    faint(),
                ))
                .right_aligned(),
            )
        };
        rows.push(Row::new(vec![
            label(format!("Used · {}", app.range.label())),
            amount(window),
            price(app.burn_rate()),
        ]));
        // The rest of the row keeps its plain meaning: USD, then the runway
        // that the burn rate implies.
        rows.push(Row::new(vec![
            label("Remaining".into()),
            Cell::from(
                Line::from(Span::styled(
                    money(remaining),
                    tint(app.color, remain_color).add_modifier(Modifier::BOLD),
                ))
                .right_aligned(),
            ),
            Cell::from(
                Line::from(Span::styled(
                    app.runway_days().map(days).unwrap_or_else(|| "—".into()),
                    faint(),
                ))
                .right_aligned(),
            ),
        ]));

        let table = Table::new(
            rows,
            [
                Constraint::Fill(1),
                Constraint::Length(12),
                Constraint::Length(9),
            ],
        )
        .header(header)
        .column_spacing(2);
        frame.render_widget(table, area);
        return;
    }
    rows.push(Row::new(vec![
        label("Remaining".into()),
        Cell::from(
            Line::from(Span::styled(
                money(remaining),
                tint(app.color, remain_color).add_modifier(Modifier::BOLD),
            ))
            .right_aligned(),
        ),
        share(remaining_fraction),
    ]));

    // The label column absorbs the slack, so both numeric columns stay pinned
    // to the same right edge as the gauge value below.
    let table = Table::new(
        rows,
        [
            Constraint::Fill(1),
            Constraint::Length(12),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .column_spacing(2);

    frame.render_widget(table, area);
}

fn draw_gauge(frame: &mut Frame, area: Rect, app: &App) {
    let Some(credits) = app.credits else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no credits purchased yet",
                faint(),
            ))),
            area,
        );
        return;
    };

    // Lifetime asks "how much of everything I bought is left". A window asks a
    // narrower question: of the money that moved inside it (spent + balance),
    // how much is still there. Falling back to the lifetime reading keeps the
    // line honest while the window query is in flight.
    let (fraction, note) = if app.range == Range::All {
        (credits.remaining_fraction(), " left")
    } else {
        (
            app.window_left_fraction().or(credits.remaining_fraction()),
            " left of spend",
        )
    };

    let line = match fraction {
        Some(fraction) => {
            let value = percent(fraction);
            // Two brackets, one leading space before the value, the value, and
            // the note. The bar takes whatever is left so the line fills the grid.
            let suffix = 1 + value.chars().count() + note.len();
            let bar_width = area.width.saturating_sub(2 + suffix as u16) as usize;
            let color = app.color.then(|| left_color(Some(fraction)));

            let mut spans = vec![Span::styled("[", faint())];
            spans.extend(bar_spans(fraction, bar_width, color));
            spans.push(Span::styled("]", faint()));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(value, bold()));
            spans.push(Span::styled(note, faint()));
            Line::from(spans)
        }
        None => Line::from(Span::styled("no credits purchased yet", faint())),
    };

    frame.render_widget(Paragraph::new(line), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let left = Line::from(vec![
        Span::styled(" q ", Style::default()),
        Span::styled("quit   ", faint()),
        Span::styled(" 0-7 ", Style::default()),
        Span::styled("window   ", faint()),
        Span::styled(" r ", Style::default()),
        Span::styled("refresh   ", faint()),
        Span::styled(" ? ", Style::default()),
        Span::styled("help", faint()),
    ]);

    // No key material on screen unless the user opens the help panel: a
    // permanent fingerprint ends up in screenshots and screen shares.
    let freshness = if app.in_flight && app.credits.is_none() {
        "fetching…".to_string()
    } else if app.in_flight {
        "refreshing…".to_string()
    } else {
        match app.last_updated {
            Some(at) => format!("updated {}", ago(at)),
            None => "no data".to_string(),
        }
    };
    let right = Line::from(Span::styled(
        format!("{freshness} · auto {}s ", app.refresh_secs),
        faint(),
    ))
    .right_aligned();

    frame.render_widget(Paragraph::new(left), area);
    frame.render_widget(Paragraph::new(right), area);
}

fn draw_error(frame: &mut Frame, area: Rect, app: &App) {
    // Balance errors win the single line: without a balance the panel is
    // useless, while a failed window query still leaves the balance readable.
    let (prefix, message) = match (&app.error, &app.spend_error) {
        (Some(error), _) => (" ! ", error.clone()),
        (None, Some(error)) => (" ! ", format!("spend {error}")),
        (None, None) => return,
    };
    let line = Line::from(vec![
        Span::styled(prefix, badge()),
        Span::raw(" "),
        Span::styled(message, tint(app.color, Color::Red)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let help = centered(area, area.width.min(62), 16);
    frame.render_widget(Clear, help);

    let key = |text: &'static str| Span::styled(text, Style::default());
    let text = |value: String| Span::styled(value, faint());

    let window = match app.range {
        Range::All => "all time · lifetime totals".to_string(),
        range => format!("{} · {}", range.label(), range.since(chrono::Local::now())),
    };
    let spend = if app.spend_error.is_some() {
        "unavailable".to_string()
    } else {
        match app.spend_updated {
            Some(at) => format!(
                "{} · auto {}m",
                ago(at),
                app.spend_interval().as_secs() / 60
            ),
            None => "not fetched yet".to_string(),
        }
    };

    let lines = vec![
        Line::from(Span::styled("keys", bold())),
        Line::from(""),
        Line::from(vec![key(" q / esc "), text("close the dashboard".into())]),
        Line::from(vec![key(" r       "), text("refresh now".into())]),
        Line::from(vec![key(" ?       "), text("toggle this help".into())]),
        Line::from(vec![
            key(" 0-7     "),
            text("window: 0 all · 1 session · 2 week".into()),
        ]),
        Line::from(vec![
            key("         "),
            text("3 7d · 4 month · 5 30d · 6 quarter · 7 year".into()),
        ]),
        Line::from(""),
        Line::from(vec![key("window  "), text(window)]),
        Line::from(vec![key("spend   "), text(spend)]),
        Line::from(""),
        Line::from(vec![
            key("config  "),
            text(app.config_path.display().to_string()),
        ]),
        Line::from(vec![key("key     "), text(masked_key(&app.cfg.api_key))]),
        Line::from(vec![
            key("source  "),
            text("GET /credits + POST /analytics/query".into()),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(faint())
        .padding(ratatui::widgets::Padding::new(1, 1, 1, 1));

    frame.render_widget(Paragraph::new(lines).block(block), help);
}

fn ago(at: Instant) -> String {
    let secs = at.elapsed().as_secs();
    match secs {
        0 | 1 => "just now".to_string(),
        secs if secs < 60 => format!("{secs}s ago"),
        secs if secs < 3600 => format!("{}m ago", secs / 60),
        secs => format!("{}h ago", secs / 3600),
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Credits;
    use crate::config::Config;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn app(credits: Option<Credits>) -> App {
        let mut app = App::with_range(
            Config::new("sk-or-v1-1234567890abcdef"),
            PathBuf::from("/tmp/config.toml"),
            Range::All,
        );
        app.credits = credits;
        app.in_flight = false;
        app
    }

    fn terminal(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
    }

    fn render(app: &App, width: u16, height: u16) -> String {
        let terminal = terminal(app, width, height);
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Last content column of a framed line, ignoring the final border.
    fn ends_one_column_before_border(line: &str) -> bool {
        let mut chars = line.trim_end().chars().rev();
        chars.next() == Some('│') && chars.next() == Some(' ')
    }

    /// `cargo test preview -- --ignored --nocapture` to eyeball the layout.
    #[test]
    #[ignore]
    fn preview() {
        println!(
            "{}",
            render(
                &app(Some(Credits {
                    total_credits: 100.5,
                    total_usage: 25.75,
                })),
                90,
                20
            )
        );
    }

    /// `cargo test preview_low -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_low() {
        println!(
            "{}",
            render(
                &app(Some(Credits {
                    total_credits: 20.0,
                    total_usage: 18.5,
                })),
                90,
                20
            )
        );
    }

    /// `cargo test preview_window -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_window() {
        let app = windowed_app(
            Range::Week,
            Some(spend([5.68, 8.49, 8.49, 15.73, 18.55, 36.55])),
        );
        println!("{}", render(&app, 90, 20));
    }

    /// `cargo test preview_error -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_error() {
        let mut app = app(Some(Credits {
            total_credits: 100.5,
            total_usage: 25.75,
        }));
        app.last_updated = Some(Instant::now());
        app.error = Some("network error: connection refused".into());
        println!("{}", render(&app, 90, 20));
    }

    #[test]
    fn shows_balance_rows() {
        let screen = render(
            &app(Some(Credits {
                total_credits: 100.5,
                total_usage: 25.75,
            })),
            90,
            24,
        );
        assert!(screen.contains("open-router-credits"), "{screen}");
        assert!(screen.contains("Purchased"), "{screen}");
        assert!(screen.contains("$100.50"), "{screen}");
        assert!(screen.contains("$25.75"), "{screen}");
        assert!(screen.contains("$74.75"), "{screen}");
        assert!(screen.contains("25.6%"), "{screen}");
        assert!(screen.contains("74.4% left"), "{screen}");
        assert!(screen.contains("q quit"), "{screen}");
    }

    /// The gauge value and the share column must stop on the same column, or
    /// the panel looks ragged next to the frame.
    #[test]
    fn gauge_lines_up_with_the_table() {
        let screen = render(
            &app(Some(Credits {
                total_credits: 100.5,
                total_usage: 25.75,
            })),
            90,
            24,
        );
        for needle in ["Remaining", "% left"] {
            let line = screen
                .lines()
                .find(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("missing {needle}"));
            assert!(
                ends_one_column_before_border(line),
                "{needle} line is not on the grid: {line:?}"
            );
        }
    }

    #[test]
    fn warns_on_low_balance() {
        let screen = render(
            &app(Some(Credits {
                total_credits: 20.0,
                total_usage: 18.0,
            })),
            90,
            24,
        );
        assert!(screen.contains("LOW BALANCE"), "{screen}");
    }

    #[test]
    fn warns_when_out_of_credits() {
        let screen = render(
            &app(Some(Credits {
                total_credits: 10.0,
                total_usage: 12.0,
            })),
            90,
            24,
        );
        assert!(screen.contains("OUT OF CREDITS"), "{screen}");
    }

    #[test]
    fn handles_zero_purchases() {
        let screen = render(
            &app(Some(Credits {
                total_credits: 0.0,
                total_usage: 0.0,
            })),
            90,
            24,
        );
        assert!(screen.contains("no credits purchased yet"), "{screen}");
    }

    #[test]
    fn shows_error_without_losing_layout() {
        let mut app = app(None);
        app.error = Some("network error: offline".into());
        let screen = render(&app, 90, 24);
        assert!(screen.contains("network error: offline"), "{screen}");
    }

    #[test]
    fn help_overlay_lists_keys() {
        let mut app = app(None);
        app.show_help = true;
        let screen = render(&app, 90, 24);
        assert!(screen.contains("toggle this help"), "{screen}");
    }

    fn windowed_app(range: Range, spend: Option<crate::range::Spend>) -> App {
        let mut app = app(Some(Credits {
            total_credits: 40.0,
            total_usage: 36.5,
        }));
        app.range = range;
        app.spend = spend;
        app
    }

    fn spend(figures: [f64; 6]) -> crate::range::Spend {
        let mut spend = crate::range::Spend::default();
        for (range, value) in Range::ASKED.iter().zip(figures) {
            spend.set(*range, value);
        }
        spend
    }

    /// A window replaces the lifetime "Used" row and keeps the balance intact.
    #[test]
    fn window_reports_spend_inside_the_range() {
        let app = windowed_app(
            Range::Week,
            Some(spend([5.68, 8.49, 8.49, 15.73, 18.55, 36.55])),
        );
        let screen = render(&app, 90, 24);
        assert!(
            screen.contains("open-router-credits · this week"),
            "{screen}"
        );
        assert!(screen.contains("Used · this week"), "{screen}");
        assert!(screen.contains("$5.68"), "{screen}");
        assert!(screen.contains("$3.50"), "{screen}");
        assert!(!screen.contains("Purchased"), "{screen}");
    }

    /// A window must not report a share of lifetime purchases: that
    /// denominator grows forever, so the row carries a burn rate instead.
    #[test]
    fn a_window_never_shows_a_lifetime_share() {
        let app = windowed_app(
            Range::Week,
            Some(spend([5.68, 8.49, 8.49, 15.73, 18.55, 36.55])),
        );
        let screen = render(&app, 90, 24);
        assert!(screen.contains("RATE"), "{screen}");
        assert!(!screen.contains("SHARE"), "{screen}");
        assert!(screen.contains("/d"), "{screen}");
        // 5.68 / 40.0 = 14.2%, the number this column used to print.
        assert!(!screen.contains("14.2%"), "{screen}");
    }

    /// The bar in a window measures what is left against the money that moved
    /// inside it: 3.50 remaining / (3.50 + 5.68) = 38.1%.
    #[test]
    fn the_gauge_measures_what_is_left_of_the_window() {
        let app = windowed_app(
            Range::Week,
            Some(spend([5.68, 8.49, 8.49, 15.73, 18.55, 36.55])),
        );
        let screen = render(&app, 90, 24);
        assert!(screen.contains("38.1% left of spend"), "{screen}");
        // The lifetime reading (8.8% of purchases left) must not appear.
        assert!(!screen.contains("8.8% left"), "{screen}");
    }

    #[test]
    fn the_lifetime_gauge_still_measures_purchases() {
        let app = app(Some(Credits {
            total_credits: 100.5,
            total_usage: 25.75,
        }));
        let screen = render(&app, 90, 24);
        assert!(screen.contains("74.4% left"), "{screen}");
        assert!(!screen.contains("of spend"), "{screen}");
    }

    /// Under an hour the rate is noise, so the cell stays empty rather than
    /// projecting a five-minute window across a day.
    #[test]
    fn a_window_younger_than_an_hour_has_no_rate() {
        let mut app = windowed_app(Range::Session, None);
        app.set_session_base(35.0);
        let screen = render(&app, 90, 24);
        assert!(screen.contains("$1.50"), "{screen}");
        assert!(!screen.contains("/d"), "{screen}");
    }

    /// Before the query answers, the row must read as pending, never as $0.00.
    #[test]
    fn pending_window_is_not_shown_as_zero() {
        let app = windowed_app(Range::Month, None);
        let screen = render(&app, 90, 24);
        assert!(screen.contains("Used · this month"), "{screen}");
        assert!(screen.contains('…'), "{screen}");
        assert!(!screen.contains("$0.00"), "{screen}");
    }

    /// The session window is a counter delta, so it needs no analytics data.
    #[test]
    fn session_window_uses_the_lifetime_counter() {
        let mut app = windowed_app(Range::Session, None);
        app.set_session_base(35.0);
        let screen = render(&app, 90, 24);
        assert!(screen.contains("Used · this session"), "{screen}");
        assert!(screen.contains("$1.50"), "{screen}");
    }

    #[test]
    fn a_failed_window_query_keeps_the_balance_visible() {
        let mut app = windowed_app(Range::Quarter, None);
        app.spend_error = Some("analytics query rejected: bad time range".into());
        let screen = render(&app, 90, 24);
        assert!(screen.contains("$3.50"), "{screen}");
        assert!(
            screen.contains("spend analytics query rejected"),
            "{screen}"
        );
    }

    #[test]
    fn degrades_gracefully_on_small_terminal() {
        let screen = render(&app(None), 40, 8);
        assert!(screen.contains("too small"), "{screen}");
    }

    /// Structure must never be drawn in palette slots that themes remap:
    /// bright black, white or black would vanish on custom color schemes.
    #[test]
    fn structure_avoids_unreliable_palette_slots() {
        let terminal = terminal(
            &app(Some(Credits {
                total_credits: 100.5,
                total_usage: 25.75,
            })),
            90,
            24,
        );
        let buffer = terminal.backend().buffer();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                for color in [cell.fg, cell.bg] {
                    assert!(
                        !matches!(color, Color::Black | Color::DarkGray | Color::Gray),
                        "palette slot {color:?} used at ({x},{y}) — themes remap these"
                    );
                }
            }
        }
    }

    /// `NO_COLOR` keeps the layout but drops every foreground color.
    #[test]
    fn no_color_mode_is_monochrome() {
        let mut app = app(Some(Credits {
            total_credits: 100.5,
            total_usage: 25.75,
        }));
        app.color = false;
        let terminal = terminal(&app, 90, 24);
        let buffer = terminal.backend().buffer();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                assert!(
                    matches!(cell.fg, Color::Reset),
                    "color {cell:?} survived NO_COLOR at ({x},{y})"
                );
            }
        }
    }
}
