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
use crate::format::{bar_spans, masked_key, money, percent, remaining_color};

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
    let panel = centered(area, width, PANEL_HEIGHT);
    let inner = panel.inner(ratatui::layout::Margin::new(1, 1));
    // Keep one column of breathing room so right-aligned values don't touch the border.
    let content = Rect {
        width: inner.width.saturating_sub(1),
        ..inner
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(faint())
        .title(Line::from(Span::styled(" open-router-credits ", bold())));
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
        Constraint::Length(4), // table: header + three rows
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
    if app.error.is_some() {
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
    let header = Row::new(vec![
        Cell::from("METRIC"),
        Cell::from(Line::from("USD").right_aligned()),
        Cell::from(Line::from("SHARE").right_aligned()),
    ])
    .style(faint_bold());

    let (purchased, used, remaining, used_fraction) = match app.credits {
        Some(credits) => (
            credits.total_credits,
            credits.total_usage,
            credits.remaining(),
            credits.used_fraction(),
        ),
        None => (0.0, 0.0, 0.0, None),
    };
    let remaining_fraction = app.credits.and_then(|credits| credits.remaining_fraction());
    let remain_color = app
        .credits
        .map(|credits| remaining_color(&credits))
        .unwrap_or(Color::Yellow);

    let label = |text: &'static str| Cell::from(Line::from(Span::styled(text, faint())));
    let amount =
        |value: f64| Cell::from(Line::from(Span::styled(money(value), bold())).right_aligned());
    let share = |fraction: Option<f64>| {
        Cell::from(
            Line::from(Span::styled(
                fraction.map(percent).unwrap_or_else(|| "—".into()),
                faint(),
            ))
            .right_aligned(),
        )
    };

    let rows = vec![
        Row::new(vec![
            label("Purchased"),
            amount(purchased),
            share(if purchased > 0.0 { Some(1.0) } else { None }),
        ]),
        Row::new(vec![label("Used"), amount(used), share(used_fraction)]),
        Row::new(vec![
            label("Remaining"),
            Cell::from(
                Line::from(Span::styled(
                    money(remaining),
                    tint(app.color, remain_color).add_modifier(Modifier::BOLD),
                ))
                .right_aligned(),
            ),
            share(remaining_fraction),
        ]),
    ];

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
    let line = match app.credits.and_then(|credits| {
        credits
            .remaining_fraction()
            .map(|fraction| (credits, fraction))
    }) {
        Some((credits, fraction)) => {
            let value = percent(fraction);
            // Two brackets, one leading space before the value, the value, and
            // " left". The bar takes whatever is left so the line fills the grid.
            let suffix = 1 + value.chars().count() + " left".len();
            let bar_width = area.width.saturating_sub(2 + suffix as u16) as usize;
            let color = app.color.then(|| remaining_color(&credits));

            let mut spans = vec![Span::styled("[", faint())];
            spans.extend(bar_spans(fraction, bar_width, color));
            spans.push(Span::styled("]", faint()));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(value, bold()));
            spans.push(Span::styled(" left", faint()));
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
    let Some(error) = &app.error else {
        return;
    };
    let line = Line::from(vec![
        Span::styled(" ! ", badge()),
        Span::raw(" "),
        Span::styled(error.clone(), tint(app.color, Color::Red)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let help = centered(area, area.width.min(58), 10);
    frame.render_widget(Clear, help);

    let key = |text: &'static str| Span::styled(text, Style::default());
    let text = |value: String| Span::styled(value, faint());

    let lines = vec![
        Line::from(Span::styled("keys", bold())),
        Line::from(""),
        Line::from(vec![key(" q / esc "), text("close the dashboard".into())]),
        Line::from(vec![key(" r       "), text("refresh now".into())]),
        Line::from(vec![key(" ?       "), text("toggle this help".into())]),
        Line::from(""),
        Line::from(vec![
            key("config  "),
            text(app.config_path.display().to_string()),
        ]),
        Line::from(vec![key("key     "), text(masked_key(&app.cfg.api_key))]),
        Line::from(vec![
            key("source  "),
            text("GET /api/v1/credits (management key)".into()),
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
        let mut app = App::new(
            Config::new("sk-or-v1-1234567890abcdef"),
            PathBuf::from("/tmp/config.toml"),
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
