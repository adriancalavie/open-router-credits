//! Value formatting shared by the TUI and the `--once` report.

use crate::api::Credits;
use crate::range::Range;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// Formats a USD amount: `$1,234.56`, or `$0.0031` for sub-dollar values.
pub fn money(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let abs = value.abs();
    let decimals = if abs > 0.0 && abs < 1.0 { 4 } else { 2 };
    let text = format!("{abs:.decimals$}");
    let (int_part, frac) = text.split_once('.').unwrap_or((text.as_str(), ""));
    if frac.is_empty() {
        format!("{sign}${}", group_thousands(int_part))
    } else {
        format!("{sign}${}.{frac}", group_thousands(int_part))
    }
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn percent(fraction: f64) -> String {
    format!("{:.1}%", fraction * 100.0)
}

/// Spend per day, the window-mode replacement for a share: a percentage of
/// lifetime purchases stops meaning anything once that total grows.
pub fn rate(per_day: f64) -> String {
    // Two decimals read as money; four made the column look like a measurement
    // ($0.9719/d). Only sub-cent rates earn a third digit.
    let decimals = if per_day.abs() < 0.01 { 3 } else { 2 };
    format!("${per_day:.decimals$}/d")
}

/// Days until the balance runs out at the current rate.
pub fn days(days: f64) -> String {
    match days {
        d if d >= 999.0 => "999+ d".to_string(),
        d if d >= 10.0 => format!("{d:.0} d"),
        d => format!("{d:.1} d"),
    }
}

/// Color used for a "remaining" reading: green, amber, red as the balance drops.
pub fn remaining_color(credits: &Credits) -> Color {
    match credits.used_fraction() {
        None => Color::DarkGray,
        Some(used) if used >= 0.9 => Color::Red,
        Some(used) if used >= 0.7 => Color::Yellow,
        Some(_) => Color::Green,
    }
}

/// A solid/empty block bar rendered as two spans. `color` is `None` when the
/// user opted out of color, in which case the bar uses the default foreground.
pub fn bar_spans(fraction: f64, width: usize, color: Option<Color>) -> Vec<Span<'static>> {
    let filled = (fraction.clamp(0.0, 1.0) * width as f64).round() as usize;
    let solid = match color {
        Some(color) => Style::default().fg(color),
        None => Style::default(),
    }
    .add_modifier(Modifier::BOLD);
    vec![
        Span::styled("█".repeat(filled), solid),
        Span::styled(
            "░".repeat(width.saturating_sub(filled)),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ]
}

pub fn bar_text(fraction: f64, width: usize) -> String {
    let filled = (fraction.clamp(0.0, 1.0) * width as f64).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(width.saturating_sub(filled))
    )
}

pub fn masked_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return "•".repeat(chars.len().max(1));
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// Plain-text report used by `--once`. `spend` is the amount consumed inside
/// `range`; `Range::All` and `Range::Session` read from the lifetime counters.
pub fn report(credits: &Credits, range: Range, spend: Option<f64>) -> String {
    let used = credits.used_fraction().unwrap_or(0.0);
    let remaining = credits.remaining_fraction().unwrap_or(0.0);
    let purchased = credits.total_credits;

    let mut out = String::new();
    out.push_str(&format!(
        "{:<20} {:>12}  {:>7}  {}\n",
        "METRIC",
        "USD",
        if range == Range::All { "SHARE" } else { "RATE" },
        "BAR"
    ));

    if range != Range::All {
        let window = spend.unwrap_or(0.0);
        let span = days_in(range);
        let per_day = span.filter(|d| *d > 1.0 / 24.0).map(|d| window / d);
        let runway = per_day
            .filter(|rate| *rate > 0.0)
            .map(|rate| credits.remaining() / rate);
        out.push_str(&format!(
            "{:<20} {:>12}  {:>7}\n",
            format!("Used · {}", range.label()),
            money(window),
            per_day.map(rate).unwrap_or_else(|| "—".into()),
        ));
        out.push_str(&format!(
            "{:<20} {:>12}  {:>7}  {}\n",
            "Remaining",
            money(credits.remaining()),
            runway.map(days).unwrap_or_else(|| "—".into()),
            bar_text(remaining, 24),
        ));
        return out;
    }

    let rows = [
        ("Purchased".to_string(), purchased, Some(1.0)),
        ("Used".to_string(), credits.total_usage, Some(used)),
        (
            "Remaining".to_string(),
            credits.remaining(),
            Some(remaining),
        ),
    ];

    for (label, amount, fraction) in rows {
        let bar = fraction.map(|f| bar_text(f, 24)).unwrap_or_default();
        let share = fraction.map(percent).unwrap_or_else(|| "—".into());
        out.push_str(&format!(
            "{label:<20} {:>12}  {share:>7}  {bar}\n",
            money(amount),
        ));
    }
    out
}

/// Length of a window in days, using the same local-time rules as the TUI.
fn days_in(range: Range) -> Option<f64> {
    let (start, end) = range.window(chrono::Local::now())?;
    Some((end - start).num_seconds().max(0) as f64 / 86_400.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_thousands() {
        assert_eq!(money(1234.5), "$1,234.50");
        assert_eq!(money(1000000.0), "$1,000,000.00");
        assert_eq!(money(12.0), "$12.00");
        assert_eq!(money(0.0), "$0.00");
        assert_eq!(money(-3.5), "-$3.50");
        assert_eq!(money(0.0031), "$0.0031");
    }

    #[test]
    fn formats_percent() {
        assert_eq!(percent(0.256), "25.6%");
        assert_eq!(percent(1.0), "100.0%");
    }

    #[test]
    fn masks_keys() {
        assert_eq!(masked_key("sk-or-v1-abcdef1234567890"), "sk-or-…7890");
        assert_eq!(masked_key("short"), "•••••");
    }

    #[test]
    fn bar_never_overflows() {
        assert_eq!(bar_text(2.0, 4), "████");
        assert_eq!(bar_text(-1.0, 4), "░░░░");
        assert_eq!(bar_text(0.5, 4), "██░░");
    }
}
