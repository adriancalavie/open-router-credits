//! Value formatting shared by the TUI and the `--once` report.

use crate::api::Credits;
use crate::range::Range;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// Formats a USD amount with an explicit number of decimals.
pub fn money_fixed(value: f64, decimals: usize) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let text = format!("{:.*}", decimals, value.abs());
    let (int_part, frac) = text.split_once('.').unwrap_or((text.as_str(), ""));
    if frac.is_empty() {
        format!("{sign}${}", group_thousands(int_part))
    } else {
        format!("{sign}${}.{frac}", group_thousands(int_part))
    }
}

/// Formats a USD amount: `$1,234.56`.
pub fn money(value: f64) -> String {
    money_fixed(value, 2)
}

/// Decimals for a whole column, chosen from its smallest value.
///
/// Mixed precisions in one column is the thing that looks broken: right
/// alignment only lines the decimal points up when every cell has the same
/// digits after the point. So one value under a dollar lifts the whole column
/// to `sub_dollar` decimals, zero-padding its neighbours.
pub fn column_decimals(values: impl IntoIterator<Item = f64>, sub_dollar: usize) -> usize {
    let smallest = values
        .into_iter()
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(f64::abs)
        .fold(f64::INFINITY, f64::min);
    if smallest < 1.0 { sub_dollar } else { 2 }
}

/// How much of the money notation fits on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cash {
    /// `$1,234.56`
    Full,
    /// `1,234.56`: the column header already says USD, so the sign is the
    /// first thing to spend when the panel is narrow.
    Bare,
    /// `1.2k`: three integer digits is the floor, larger amounts trade digits
    /// for a suffix.
    Compact,
}

pub fn cash(value: f64, decimals: usize, style: Cash) -> String {
    match style {
        Cash::Full => money_fixed(value, decimals),
        Cash::Bare => money_fixed(value, decimals).replacen('$', "", 1),
        Cash::Compact => compact(value, decimals),
    }
}

/// Keeps the integer part to three digits: `1.2k`, `12k`, `123k`, `1.2M`.
fn compact(value: f64, decimals: usize) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let abs = value.abs();
    let (scaled, unit) = if abs >= 1_000_000.0 {
        (abs / 1_000_000.0, "M")
    } else if abs >= 1_000.0 {
        (abs / 1_000.0, "k")
    } else {
        // Under a thousand the plain number already fits the floor.
        return format!("{sign}{abs:.decimals$}");
    };
    if scaled < 10.0 {
        format!("{sign}{scaled:.1}{unit}")
    } else {
        format!("{sign}{scaled:.0}{unit}")
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
pub fn rate_with(per_day: f64, decimals: usize) -> String {
    format!("{}/d", money_fixed(per_day, decimals))
}

pub fn rate(per_day: f64) -> String {
    rate_with(per_day, 2)
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
    left_color(credits.remaining_fraction())
}

/// Same rule, stated as what is left instead of what is spent: one threshold
/// table serves the lifetime bar, the window bar and overdraw alike.
pub fn left_color(remaining: Option<f64>) -> Color {
    match remaining {
        None => Color::DarkGray,
        Some(left) if left <= 0.10 => Color::Red,
        Some(left) if left <= 0.30 => Color::Yellow,
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

    let decimals = column_decimals(
        [
            credits.total_credits,
            credits.total_usage,
            credits.remaining(),
            spend.unwrap_or(0.0),
        ],
        4,
    );

    if range != Range::All {
        let window = spend.unwrap_or(0.0);
        let span = days_in(range);
        let per_day = span.filter(|d| *d > 1.0 / 24.0).map(|d| window / d);
        let runway = per_day
            .filter(|rate| *rate > 0.0)
            .map(|rate| credits.remaining() / rate);
        // The bar answers the same question as the TUI: of the money that moved
        // inside this window, how much is still there.
        let moved = window + credits.remaining().max(0.0);
        let left = (moved > 0.0).then(|| credits.remaining().max(0.0) / moved);
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
            left.map(|f| bar_text(f, 24)).unwrap_or_default(),
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
            money_fixed(amount, decimals),
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
    }

    /// Two decimals everywhere: a rate like $0.0197 prints as $0.02, and
    /// anything under half a cent is $0.00 rather than a long tail.
    #[test]
    fn always_two_decimals() {
        assert_eq!(money(0.0197), "$0.02");
        assert_eq!(money(0.0149), "$0.01");
        assert_eq!(money(0.0031), "$0.00");
        assert_eq!(rate(0.0197), "$0.02/d");
        assert_eq!(rate(36.0 / 311.0), "$0.12/d");
    }

    #[test]
    fn money_fixed_keeps_thousands() {
        assert_eq!(money_fixed(1234.5, 4), "$1,234.5000");
        assert_eq!(money_fixed(-0.0031, 4), "-$0.0031");
    }

    /// One value under a dollar lifts the whole column, so every cell ends at
    /// the same decimal point.
    #[test]
    fn column_decimals_follow_the_smallest_value() {
        assert_eq!(column_decimals([100.5, 25.75, 0.0], 4), 2);
        assert_eq!(column_decimals([5.68, 0.0197], 4), 4);
        assert_eq!(column_decimals([], 4), 2);
        assert_eq!(column_decimals([0.0, 0.0], 4), 2);
        assert_eq!(column_decimals([12.0], 3), 2);
        assert_eq!(column_decimals([0.97], 3), 3);
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
