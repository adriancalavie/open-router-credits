//! Value formatting shared by the TUI and the `--once` report.

use crate::api::Credits;
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

/// Plain-text report used by `--once`.
pub fn report(credits: &Credits) -> String {
    let used = credits.used_fraction().unwrap_or(0.0);
    let remaining = credits.remaining_fraction().unwrap_or(0.0);
    let rows = [
        ("Purchased", credits.total_credits, 1.0),
        ("Used", credits.total_usage, used),
        ("Remaining", credits.remaining(), remaining),
    ];
    let mut out = String::new();
    out.push_str(&format!(
        "{:<11} {:>12}  {:>7}  {}\n",
        "METRIC", "USD", "SHARE", "BAR"
    ));
    for (label, amount, fraction) in rows {
        out.push_str(&format!(
            "{:<11} {:>12}  {:>7}  {}\n",
            label,
            money(amount),
            percent(fraction),
            bar_text(fraction, 24)
        ));
    }
    out
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
