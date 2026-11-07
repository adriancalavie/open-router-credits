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
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table};
use std::time::Instant;

use crate::app::App;
use crate::format::{
    Cash, bar_spans, cash, column_decimals, days, left_color, masked_key, percent, rate_with,
    remaining_color,
};
use crate::range::Range;

const MIN_WIDTH: u16 = 16;
const PANEL_WIDTH: u16 = 76;
/// Below this the bar loses more than it explains.
const GAUGE_MIN_WIDTH: u16 = 22;

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
    let metrics: u16 = if app.range == Range::All { 3 } else { 2 };
    let min_height = 2 + metrics;
    let has_error = app.error.is_some() || app.spend_error.is_some();

    if area.width < MIN_WIDTH || area.height < min_height {
        // Two short lines, because the reason this is on screen is a terminal
        // that cannot hold much.
        let message = Paragraph::new(vec![
            Line::from(Span::styled("too small", bold())),
            Line::from(format!("need {MIN_WIDTH}x{min_height}")),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(message, centered(area, area.width.min(48), 2));
        return;
    }

    // Shed rows until the frame fits: footer, then gauge, then the table
    // header, then the blank line under the title. The metric rows are the
    // only thing that never goes.
    let width = area.width.min(PANEL_WIDTH);
    let plan = Plan {
        pad: true,
        header: true,
        // A bar narrower than a few cells reads as noise, not as a gauge.
        gauge: width >= GAUGE_MIN_WIDTH,
        footer: true,
        error: has_error,
    }
    .fit(metrics, area.height);

    let panel = centered(area, width, plan.height(metrics));

    // The title names the window when it can; the metric row then just says
    // "Used" instead of repeating it.
    let mut window_in_title = app.range != Range::All;
    let mut title = format!(" open-router-credits · {} ", app.range.label());
    if app.range == Range::All || (width as usize) <= title.chars().count() {
        title = " open-router-credits ".to_string();
        window_in_title = false;
    }
    if (width as usize) <= title.chars().count() {
        title = " credits ".to_string();
        window_in_title = false;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(faint())
        .title(Line::from(Span::styled(title.clone(), bold())));
    frame.render_widget(block, panel);

    // Alerts live in the frame, not in a row of their own: the border carries
    // state the way a window title does. It steps aside when the title and the
    // badge would collide.
    if let Some(credits) = app.credits {
        let label = if credits.remaining() <= 0.0 {
            Some("OUT OF CREDITS")
        } else if credits.remaining() < 5.0 {
            Some("LOW BALANCE")
        } else {
            None
        };
        if let Some(label) = label
            && width as usize >= title.chars().count() + label.chars().count() + 4
        {
            draw_badge(frame, panel, label);
        }
    }

    // One column of air inside the borders, so right-aligned values never touch
    // the frame.
    let content = Rect {
        x: panel.x + 1,
        y: panel.y + 1,
        width: panel.width.saturating_sub(3),
        height: 0,
    };

    let mut y = panel.y + 1;
    if plan.pad {
        y += 1;
    }
    let table_height = metrics + u16::from(plan.header);
    draw_table(
        frame,
        Rect {
            y,
            height: table_height,
            ..content
        },
        app,
        plan.header,
        window_in_title,
    );
    y += table_height;
    if plan.gauge {
        y += 1;
        draw_gauge(
            frame,
            Rect {
                y,
                height: 1,
                ..content
            },
            app,
        );
        y += 1;
    }
    if plan.footer {
        y += 1;
        draw_footer(
            frame,
            Rect {
                y,
                height: 1,
                ..content
            },
            app,
        );
        y += 1;
    }
    if plan.error {
        draw_error(
            frame,
            Rect {
                y,
                height: 1,
                ..content
            },
            app,
        );
    }

    if app.show_help {
        draw_help(frame, area, app);
    }
}

/// Which optional rows the frame can afford.
#[derive(Debug, Clone, Copy)]
struct Plan {
    pad: bool,
    header: bool,
    gauge: bool,
    footer: bool,
    error: bool,
}

impl Plan {
    /// Height of the whole frame for this set of rows, borders included.
    fn height(self, metrics: u16) -> u16 {
        2 + u16::from(self.pad)
            + metrics
            + u16::from(self.header)
            + if self.gauge { 2 } else { 0 }
            + if self.footer { 2 } else { 0 }
            + u16::from(self.error)
    }

    /// Drops the least useful row until the frame fits the terminal.
    fn fit(mut self, metrics: u16, available: u16) -> Self {
        while self.height(metrics) > available {
            self = if self.footer {
                Self {
                    footer: false,
                    ..self
                }
            } else if self.gauge {
                Self {
                    gauge: false,
                    ..self
                }
            } else if self.pad {
                // Decorative padding goes before the header: a blank line is
                // worth less than a column label.
                Self { pad: false, ..self }
            } else if self.header {
                Self {
                    header: false,
                    ..self
                }
            } else if self.error {
                Self {
                    error: false,
                    ..self
                }
            } else {
                break;
            };
        }
        self
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

fn draw_table(frame: &mut Frame, area: Rect, app: &App, show_header: bool, window_in_title: bool) {
    // The lifetime view reports shares of purchases. A window cannot: that
    // denominator grows forever, so the same row carries a burn rate instead.
    let windowed = app.range != Range::All;
    let purchased = app.credits.map(|c| c.total_credits).unwrap_or(0.0);
    let remaining = app.credits.map(|c| c.remaining()).unwrap_or(0.0);
    let remain_color = app
        .credits
        .map(|credits| remaining_color(&credits))
        .unwrap_or(Color::Yellow);
    let share_of_purchased =
        |value: f64| (purchased > 0.0).then(|| (value / purchased).clamp(0.0, 1.0));

    // One precision for the whole column, so the decimal points line up: a
    // sub-dollar value lifts its neighbours to four decimals instead of
    // leaving a ragged right edge.
    let decimals = column_decimals(
        [
            purchased,
            app.credits.map(|c| c.total_usage).unwrap_or(0.0),
            app.window_spend().unwrap_or(0.0),
            remaining,
        ],
        4,
    );

    // Everything the panel wants to show, in one budget. Each rung gives up the
    // least useful thing that still fits: the third column first (a whole
    // metric is worth more than one character), then the dollar sign (redundant
    // while the header says USD), then label length, then digits via 1.2k.
    let values = [
        purchased,
        app.credits.map(|c| c.total_usage).unwrap_or(0.0),
        app.window_spend().unwrap_or(0.0),
        remaining,
    ];
    let width_of = |style| {
        values
            .iter()
            .map(|value| cash(*value, decimals, style).chars().count())
            .max()
            .unwrap_or(0)
    };
    let (full, bare, compact) = (
        width_of(Cash::Full),
        width_of(Cash::Bare),
        width_of(Cash::Compact),
    );
    let third_width = if windowed { 9 + 2 } else { 8 + 2 };
    let cols = area.width as usize;
    let fits = |amount: usize, label: usize, third: bool| {
        amount + 2 + label + if third { third_width } else { 0 } <= cols
    };
    // Nine columns is "Remaining", the longest label; four is a readable stub.
    // With the header on screen the dollar sign is the cheapest thing to lose,
    // so it goes before any label is shortened. Without it the numbers carry the
    // unit, so labels give way first.
    let (style, show_third, label_width) = if fits(full, 9, true) {
        (Cash::Full, true, cols - full - 2 - third_width)
    } else if fits(full, 9, false) {
        (Cash::Full, false, cols - full - 2)
    } else if show_header && fits(bare, 4, false) {
        // The header carries the unit, so the sign goes before any label is
        // shortened. Whatever space is left is the label's.
        (Cash::Bare, false, cols - bare - 2)
    } else if fits(full, 4, false) {
        (Cash::Full, false, cols - full - 2)
    } else if fits(bare, 4, false) {
        (Cash::Bare, false, cols - bare - 2)
    } else {
        (Cash::Compact, false, cols.saturating_sub(compact + 2))
    };

    // `None` means the window query has not answered yet, not zero.
    let amount_text = |value: Option<f64>| {
        value
            .map(|value| cash(value, decimals, style))
            .unwrap_or_else(|| "\u{2026}".to_string())
    };
    let purchased_text = amount_text(Some(purchased));
    let lifetime_text = amount_text(app.credits.map(|c| c.total_usage));
    let window_text = amount_text(app.window_spend());
    let remaining_text = amount_text(Some(remaining));

    // The USD column is sized to the widest amount actually shown, so a panel
    // that prints "$74.75" does not reserve room for "$1,234,567.89".
    let amount_width = [
        &purchased_text,
        &lifetime_text,
        &window_text,
        &remaining_text,
    ]
    .iter()
    .map(|text| text.chars().count())
    .max()
    .unwrap_or(5)
    .clamp(5, 12) as u16;

    let used_label = if window_in_title {
        // The frame already names the window, so repeating it in the row would
        // print the same thing twice.
        elide("Used", label_width)
    } else {
        let full = format!("Used \u{b7} {}", app.range.label());
        if full.chars().count() <= label_width {
            full
        } else {
            elide(app.range.short_label(), label_width)
        }
    };

    let label = |text: String| Cell::from(Line::from(Span::styled(text, faint())));
    let amount =
        |text: &str| Cell::from(Line::from(Span::styled(text.to_string(), bold())).right_aligned());
    let note = |text: String| Cell::from(Line::from(Span::styled(text, faint())).right_aligned());
    let share =
        |fraction: Option<f64>| note(fraction.map(percent).unwrap_or_else(|| "\u{2014}".into()));
    let remain_cell = || {
        Cell::from(
            Line::from(Span::styled(
                remaining_text.clone(),
                tint(app.color, remain_color).add_modifier(Modifier::BOLD),
            ))
            .right_aligned(),
        )
    };

    let mut rows = Vec::new();
    if windowed {
        // A window replaces the lifetime "Used" row instead of adding to it:
        // the question changes from "how much in total" to "how much lately".
        let mut cells = vec![label(used_label), amount(&window_text)];
        if show_third {
            let rate_decimals = column_decimals(app.burn_rate(), 3);
            cells.push(note(
                app.burn_rate()
                    .map(|rate| rate_with(rate, rate_decimals))
                    .unwrap_or_else(|| "\u{2014}".into()),
            ));
        }
        rows.push(Row::new(cells));

        let mut cells = vec![label(elide("Remaining", label_width)), remain_cell()];
        if show_third {
            cells.push(note(
                app.runway_days()
                    .map(days)
                    .unwrap_or_else(|| "\u{2014}".into()),
            ));
        }
        rows.push(Row::new(cells));
    } else {
        let mut cells = vec![
            label(elide("Purchased", label_width)),
            amount(&purchased_text),
        ];
        if show_third {
            cells.push(share(share_of_purchased(purchased)));
        }
        rows.push(Row::new(cells));

        let mut cells = vec![label(elide("Used", label_width)), amount(&lifetime_text)];
        if show_third {
            cells.push(share(app.credits.and_then(|c| c.used_fraction())));
        }
        rows.push(Row::new(cells));

        let mut cells = vec![label(elide("Remaining", label_width)), remain_cell()];
        if show_third {
            cells.push(share(app.credits.and_then(|c| c.remaining_fraction())));
        }
        rows.push(Row::new(cells));
    }

    let mut header_cells = vec![
        Cell::from(elide("METRIC", label_width)),
        Cell::from(Line::from("USD").right_aligned()),
    ];
    if show_third {
        header_cells.push(Cell::from(
            Line::from(if windowed { "RATE" } else { "SHARE" }).right_aligned(),
        ));
    }

    // The label column absorbs the slack, so the numeric columns stay pinned to
    // the same right edge as the gauge value below.
    let mut widths = vec![Constraint::Fill(1), Constraint::Length(amount_width)];
    if show_third {
        widths.push(Constraint::Length(if windowed { 9 } else { 8 }));
    }
    let mut table = Table::new(rows, widths).column_spacing(2);
    if show_header {
        table = table.header(Row::new(header_cells).style(faint_bold()));
    }

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
    let (fraction, long_note) = if app.range == Range::All {
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
            let suffix = |note: &str| 1 + value.chars().count() + note.len();
            // "left of spend" is the full story; a narrow panel gets "left" so
            // the bar keeps a few cells to draw in.
            let note = if (2 + 4 + suffix(long_note)) as u16 <= area.width {
                long_note
            } else {
                " left"
            };
            let bar_width = area.width.saturating_sub(2 + suffix(note) as u16) as usize;
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
    // No key material on screen unless the user opens the help panel: a
    // permanent fingerprint ends up in screenshots and screen shares.
    let freshness = if app.in_flight && app.credits.is_none() {
        "fetching\u{2026}".to_string()
    } else if app.in_flight {
        "refreshing\u{2026}".to_string()
    } else {
        match app.last_updated {
            Some(at) => format!("updated {}", ago(at)),
            None => "no data".to_string(),
        }
    };
    let full = format!("{freshness} \u{b7} auto {}s ", app.refresh_secs);
    let age = format!("{freshness} ");

    // Key hints, with the most expendable last. The right side sheds first: the
    // age, then the auto-refresh, then the hints from help backwards.
    let mut hints: Vec<(&'static str, &'static str)> = vec![
        (" q ", "quit   "),
        (" 0-7 ", "window   "),
        (" r ", "refresh   "),
        (" ? ", "help"),
    ];
    let mut right = Right::Full;
    loop {
        let left_width: usize = hints.iter().map(|(key, text)| key.len() + text.len()).sum();
        let right_width = right
            .text(&full, &age)
            .map_or(0, |text| text.chars().count());
        // One column of air between the two halves.
        if left_width + right_width < area.width as usize {
            break;
        }
        right = match right {
            Right::Full => Right::Age,
            Right::Age => Right::None,
            Right::None if hints.len() > 1 => {
                hints.pop();
                Right::None
            }
            Right::None => break,
        };
    }

    let spans: Vec<Span> = hints
        .iter()
        .flat_map(|(key, text)| {
            [
                Span::styled(*key, Style::default()),
                Span::styled(*text, faint()),
            ]
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(text) = right.text(&full, &age) {
        let right = Line::from(Span::styled(text, faint())).right_aligned();
        frame.render_widget(Paragraph::new(right), area);
    }
}

/// The tail of the footer, kept as a ladder so each step can be dropped.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Right {
    Full,
    Age,
    None,
}

impl Right {
    fn text<'a>(self, full: &'a str, age: &'a str) -> Option<&'a str> {
        match self {
            Self::Full => Some(full),
            Self::Age => Some(age),
            Self::None => None,
        }
    }
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

/// Cuts `text` to `width` columns, marking the cut with an ellipsis. The table
/// widget would otherwise slice the string with no hint that it did.
fn elide(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    // No dangling space before the ellipsis: "this…", not "this …".
    let keep: String = text.chars().take(width - 1).collect();
    format!("{}…", keep.trim_end())
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

    /// `cargo test preview_sweep -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_sweep() {
        let credits = Some(Credits {
            total_credits: 100.5,
            total_usage: 25.75,
        });
        for (w, h) in [
            (90, 24),
            (44, 10),
            (33, 10),
            (32, 10),
            (31, 10),
            (22, 10),
            (21, 10),
            (20, 10),
            (16, 5),
        ] {
            println!("--- {w}x{h} ---");
            println!("{}", render(&app(credits), w, h));
        }
        let mut session = windowed_app(
            Range::Session,
            Some(spend([5.68, 8.49, 8.49, 15.73, 18.55, 36.55])),
        );
        session.set_session_base(35.0);
        for (w, h) in [(90, 24), (60, 12), (44, 8), (28, 5), (24, 9), (20, 4)] {
            println!("--- session {w}x{h} ---");
            println!("{}", render(&session, w, h));
        }
        let quarter = windowed_app(
            Range::Quarter,
            Some(spend([5.68, 8.49, 8.49, 15.73, 19.15, 36.55])),
        );
        for (w, h) in [(46, 4), (46, 5)] {
            println!("--- quarter {w}x{h} ---");
            println!("{}", render(&quarter, w, h));
        }
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
        assert!(screen.contains("this week"), "{screen}");
        assert!(screen.contains("Used "), "{screen}");
        assert!(!screen.contains("Used · this week"), "{screen}");
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
        assert!(screen.contains("Used "), "{screen}");
        assert!(screen.contains("this month"), "{screen}");
        assert!(screen.contains('…'), "{screen}");
        assert!(!screen.contains("$0.00"), "{screen}");
    }

    /// The session window is a counter delta, so it needs no analytics data.
    #[test]
    fn session_window_uses_the_lifetime_counter() {
        let mut app = windowed_app(Range::Session, None);
        app.set_session_base(35.0);
        let screen = render(&app, 90, 24);
        assert!(screen.contains("Used "), "{screen}");
        assert!(screen.contains("this session"), "{screen}");
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
        // 15 columns leaves no room for a label next to the amount.
        let screen = render(&app(None), 15, 8);
        assert!(screen.contains("too small"), "{screen}");
        // Four rows cannot hold three metrics and the border.
        let screen = render(&app(None), 60, 4);
        assert!(screen.contains("too small"), "{screen}");
    }

    /// The panel keeps reporting the numbers after everything else is gone.
    #[test]
    fn a_short_terminal_keeps_only_the_metrics() {
        let screen = render(
            &app(Some(Credits {
                total_credits: 100.5,
                total_usage: 25.75,
            })),
            60,
            5,
        );
        assert!(screen.contains("$100.50"), "{screen}");
        assert!(screen.contains("$74.75"), "{screen}");
        // No header, no bar, no keys.
        assert!(!screen.contains("METRIC"), "{screen}");
        assert!(!screen.contains("% left"), "{screen}");
        assert!(!screen.contains("quit"), "{screen}");
    }

    /// Sixteen columns is the floor: one label, one amount, one border.
    #[test]
    fn a_sliver_of_terminal_still_shows_the_numbers() {
        let screen = render(
            &app(Some(Credits {
                total_credits: 100.5,
                total_usage: 25.75,
            })),
            16,
            5,
        );
        // No header here, so the numbers keep their dollar sign.
        assert!(screen.contains("$100.50"), "{screen}");
        assert!(screen.contains("$74.75"), "{screen}");
        // The label is shortened to fit, and the cut is marked.
        assert!(screen.contains('…'), "{screen}");
    }

    /// A sub-dollar amount lifts the whole money column, so the decimal points
    /// line up instead of the column stepping in and out.
    #[test]
    fn a_sub_dollar_amount_lifts_the_whole_column() {
        let app = windowed_app(
            Range::Week,
            Some(spend([0.0197, 8.49, 8.49, 15.73, 18.55, 36.55])),
        );
        let screen = render(&app, 90, 24);
        assert!(screen.contains("$0.0197"), "{screen}");
        assert!(screen.contains("$3.5000"), "{screen}");
        assert!(screen.contains("$0.003/d"), "{screen}");

        // Each money cell is the same width and breaks at the same offset, so
        // the decimal points stack even though the bytes before them differ.
        let amounts: Vec<&str> = screen
            .lines()
            .filter_map(|line| {
                let dollar = line.find('$')?;
                let rest = &line[dollar..];
                let end = rest.find(' ').unwrap_or(rest.len());
                let cell = &rest[..end];
                (!cell.contains('/')).then_some(cell)
            })
            .collect();
        assert_eq!(amounts, ["$0.0197", "$3.5000"], "{screen}");
        assert!(
            amounts.iter().all(|cell| cell.find('.') == Some(2)),
            "{amounts:?}"
        );
    }

    /// With the header off screen the unit has to come from the numbers, so the
    /// dollar sign stays as long as it fits.
    #[test]
    fn a_hidden_header_keeps_the_dollar_sign() {
        let app = windowed_app(
            Range::Quarter,
            Some(spend([5.68, 8.49, 8.49, 15.73, 19.15, 36.55])),
        );
        let screen = render(&app, 46, 4);
        assert!(!screen.contains("METRIC"), "{screen}");
        assert!(screen.contains("$19.15"), "{screen}");
        assert!(screen.contains("$3.50"), "{screen}");
    }

    /// Past three integer digits a narrow panel trades digits for a suffix.
    #[test]
    fn a_large_balance_compacts_instead_of_widening_the_column() {
        let app = app(Some(Credits {
            total_credits: 12_345.67,
            total_usage: 100.0,
        }));
        let screen = render(&app, 16, 5);
        assert!(screen.contains("12k"), "{screen}");
        assert!(!screen.contains("12,345"), "{screen}");
    }

    /// A narrow panel drops the "Used ·" prefix before it cuts the window name.
    #[test]
    fn a_narrow_window_uses_the_short_name() {
        let mut app = windowed_app(Range::Session, None);
        app.set_session_base(35.0);
        let narrow = render(&app, 28, 5);
        assert!(narrow.contains("session"), "{narrow}");
        assert!(!narrow.contains("Used"), "{narrow}");

        // Wide: the title carries the window, the row does not repeat it.
        let wide = render(&app, 90, 24);
        assert!(wide.contains("this session"), "{wide}");
        assert!(!wide.contains("Used · this session"), "{wide}");
    }

    /// The label is measured, not left to the table widget, so a cut always
    /// shows an ellipsis instead of a half word.
    #[test]
    fn elided_text_ends_with_an_ellipsis() {
        assert_eq!(elide("this session", 12), "this session");
        assert_eq!(elide("this session", 6), "this…");
        assert_eq!(elide("this session", 1), "…");
        assert_eq!(elide("this session", 0), "");
    }

    /// No label is cut at any supported size, the short name always fits.
    #[test]
    fn every_window_label_fits_at_the_minimum_width() {
        for range in Range::ASKED.iter().chain([Range::Session].iter()) {
            let mut app =
                windowed_app(*range, Some(spend([5.68, 8.49, 8.49, 15.73, 18.55, 36.55])));
            app.set_session_base(35.0);
            let screen = render(&app, 28, 5);
            assert!(screen.contains(range.short_label()), "{range}: {screen}");
            assert!(!screen.contains('…'), "{range}: {screen}");
        }
    }

    #[test]
    fn height_drops_rows_one_at_a_time() {
        let credits = Some(Credits {
            total_credits: 100.5,
            total_usage: 25.75,
        });
        // 7 rows: header + metrics, the bar is the next thing to go.
        let screen = render(&app(credits), 90, 7);
        assert!(screen.contains("METRIC"), "{screen}");
        assert!(!screen.contains("quit"), "{screen}");
        assert!(!screen.contains("% left"), "{screen}");

        // 9 rows fits the bar but still not the keys.
        let screen = render(&app(credits), 90, 9);
        assert!(screen.contains("% left"), "{screen}");
        assert!(!screen.contains("quit"), "{screen}");

        // 11 rows is the lifetime panel complete.
        let screen = render(&app(credits), 90, 11);
        assert!(screen.contains("quit"), "{screen}");
    }

    /// Each thing leaves at its own width: the third column first, then the
    /// dollar sign, then label length.
    #[test]
    fn wide_panels_drop_columns_and_signs_one_at_a_time() {
        let credits = Some(Credits {
            total_credits: 100.5,
            total_usage: 25.75,
        });
        // 31 columns still pays for label + $100.50 + SHARE.
        let screen = render(&app(credits), 31, 12);
        assert!(screen.contains("SHARE"), "{screen}");
        assert!(screen.contains("$74.75"), "{screen}");

        // One column less and only the third column goes: the sign stays,
        // because there is plenty of room left for it.
        let screen = render(&app(credits), 30, 12);
        assert!(!screen.contains("SHARE"), "{screen}");
        assert!(screen.contains("$74.75"), "{screen}");
        assert!(screen.contains("Remaining"), "{screen}");

        // The sign only goes when it is competing with the label itself.
        let screen = render(&app(credits), 20, 12);
        assert!(!screen.contains('$'), "{screen}");
        assert!(screen.contains("74.75"), "{screen}");
        assert!(screen.contains("Remaining"), "{screen}");
    }

    /// The keys survive on a narrow terminal; the freshness text steps aside
    /// instead of printing over them.
    #[test]
    fn the_footer_drops_the_age_rather_than_collide() {
        let app = app(Some(Credits {
            total_credits: 100.5,
            total_usage: 25.75,
        }));
        assert!(render(&app, 90, 24).contains("auto 60s"));
        let narrow = render(&app, 60, 24);
        assert!(narrow.contains("0-7 window"), "{narrow}");
        assert!(!narrow.contains("auto 60s"), "{narrow}");
    }

    /// Hints go in the order help, refresh, window; quit is the last one left.
    #[test]
    fn the_footer_sheds_hints_from_the_right() {
        let app = app(Some(Credits {
            total_credits: 100.5,
            total_usage: 25.75,
        }));
        let screen = render(&app, 44, 12);
        assert!(screen.contains("0-7 window"), "{screen}");
        assert!(screen.contains("refresh"), "{screen}");
        assert!(!screen.contains("? help"), "{screen}");

        let screen = render(&app, 37, 12);
        assert!(screen.contains("0-7 window"), "{screen}");
        assert!(!screen.contains("refresh"), "{screen}");

        // At the narrowest supported width, quit and the window keys remain.
        let screen = render(&app, 30, 12);
        assert!(screen.contains("quit"), "{screen}");
        assert!(screen.contains("0-7 window"), "{screen}");
        assert!(!screen.contains("refresh"), "{screen}");
        assert!(!screen.contains("? help"), "{screen}");
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
