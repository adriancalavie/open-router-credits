//! Spend windows. Each window is resolved to exact local-midnight-to-now
//! timestamps and sent to the analytics endpoint as a range filter, so no UTC
//! day bucket ever decides where a window starts.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// Lifetime totals from GET /credits.
    All,
    /// Since this process started, from the live usage counter. No API call.
    Session,
    Week,
    Last7,
    Month,
    Last30,
    Quarter,
    Year,
}

impl Range {
    /// Windows that are answered by the analytics endpoint, widest last.
    pub const ASKED: [Range; 6] = [
        Range::Week,
        Range::Last7,
        Range::Month,
        Range::Last30,
        Range::Quarter,
        Range::Year,
    ];

    pub fn needs_api(self) -> bool {
        !matches!(self, Range::All | Range::Session)
    }

    pub fn label(self) -> &'static str {
        match self {
            Range::All => "all time",
            Range::Session => "this session",
            Range::Week => "this week",
            Range::Last7 => "last 7 days",
            Range::Month => "this month",
            Range::Last30 => "last 30 days",
            Range::Quarter => "this quarter",
            Range::Year => "this year",
        }
    }

    /// The digit that selects this range. Kept next to `digit` sources so the
    /// footer, the help panel and the parser cannot drift apart.
    pub fn from_key(ch: char) -> Option<Self> {
        match ch {
            '0' | 'a' => Some(Range::All),
            '1' => Some(Range::Session),
            '2' => Some(Range::Week),
            '3' => Some(Range::Last7),
            '4' => Some(Range::Month),
            '5' => Some(Range::Last30),
            '6' => Some(Range::Quarter),
            '7' => Some(Range::Year),
            _ => None,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "all" | "none" | "lifetime" => Some(Range::All),
            "session" | "s" => Some(Range::Session),
            "week" | "w" => Some(Range::Week),
            "7d" | "last7" | "last-7-days" => Some(Range::Last7),
            "month" | "m" => Some(Range::Month),
            "30d" | "last30" | "last-30-days" => Some(Range::Last30),
            "quarter" | "q" => Some(Range::Quarter),
            "year" | "y" => Some(Range::Year),
            _ => None,
        }
    }

    /// `(start, end)` in UTC for a window that starts at local midnight (or an
    /// exact local timestamp for the rolling windows). `None` for the two
    /// ranges that need no query.
    pub fn window(self, now: DateTime<Local>) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let today = now.date_naive();
        let start = match self {
            Range::All | Range::Session => return None,
            Range::Last7 => now - Duration::days(7),
            Range::Last30 => now - Duration::days(30),
            Range::Week => {
                let back = i64::from(today.weekday().num_days_from_monday());
                midnight(today - Duration::days(back))?
            }
            Range::Month => midnight(first_of_month(today))?,
            Range::Quarter => midnight(first_of_quarter(today))?,
            Range::Year => midnight(NaiveDate::from_ymd_opt(today.year(), 1, 1)?)?,
        };
        Some((start.with_timezone(&Utc), now.with_timezone(&Utc)))
    }

    /// Short description of where the window starts, for the help panel.
    pub fn since(self, now: DateTime<Local>) -> String {
        let (start, _) = match self.window(now) {
            Some(window) => window,
            None => {
                return match self {
                    Range::All => "lifetime totals".to_string(),
                    _ => "since launch".to_string(),
                };
            }
        };
        let local = start.with_timezone(&Local);
        match self {
            Range::Last7 | Range::Last30 => "rolling".to_string(),
            _ => format!("since {}", local.format("%a %-d %b")),
        }
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Spend per window, all fetched together so switching windows is instant.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spend {
    week: f64,
    last7: f64,
    month: f64,
    last30: f64,
    quarter: f64,
    year: f64,
}

impl Spend {
    pub fn get(self, range: Range) -> Option<f64> {
        match range {
            Range::Week => Some(self.week),
            Range::Last7 => Some(self.last7),
            Range::Month => Some(self.month),
            Range::Last30 => Some(self.last30),
            Range::Quarter => Some(self.quarter),
            Range::Year => Some(self.year),
            Range::All | Range::Session => None,
        }
    }

    pub fn set(&mut self, range: Range, value: f64) {
        match range {
            Range::Week => self.week = value,
            Range::Last7 => self.last7 = value,
            Range::Month => self.month = value,
            Range::Last30 => self.last30 = value,
            Range::Quarter => self.quarter = value,
            Range::Year => self.year = value,
            Range::All | Range::Session => {}
        }
    }
}

/// The timestamp format the analytics endpoint accepts: seconds are required,
/// a bare minute precision is rejected with 400.
pub fn to_iso(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Midnight local for a calendar date. `earliest()` resolves the repeated hour
/// when a DST change lands on midnight, so the window starts at the first
/// instant of that date instead of vanishing.
fn midnight(date: NaiveDate) -> Option<DateTime<Local>> {
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date)
}

fn first_of_quarter(date: NaiveDate) -> NaiveDate {
    let month = ((date.month() - 1) / 3) * 3 + 1;
    NaiveDate::from_ymd_opt(date.year(), month, 1).unwrap_or(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn now() -> DateTime<Local> {
        Local::now()
    }

    #[test]
    fn week_starts_on_monday_at_midnight() {
        let now = now();
        let (start, end) = Range::Week.window(now).unwrap();
        let start = start.with_timezone(&Local);
        assert_eq!(start.weekday(), chrono::Weekday::Mon);
        assert_eq!((start.hour(), start.minute(), start.second()), (0, 0, 0));
        assert!(start <= now);
        assert!(start > now - Duration::days(7));
        assert!(end >= start);
    }

    #[test]
    fn month_and_quarter_and_year_start_on_the_first_at_midnight() {
        let now = now();
        for (range, month_ok) in [(Range::Month, 0), (Range::Quarter, 0), (Range::Year, 1)] {
            let (start, _) = range.window(now).unwrap();
            let start = start.with_timezone(&Local);
            assert_eq!(start.hour(), 0, "{range}");
            assert_eq!(start.day(), 1, "{range}");
            if month_ok == 1 {
                assert_eq!(start.month(), 1, "{range}");
            }
            assert!(start <= now, "{range}");
        }
    }

    #[test]
    fn quarter_lands_on_a_quarter_start() {
        let (start, _) = Range::Quarter.window(now()).unwrap();
        let month = start.with_timezone(&Local).month();
        assert!(matches!(month, 1 | 4 | 7 | 10), "month {month}");
    }

    #[test]
    fn rolling_windows_are_exactly_seven_and_thirty_days() {
        let now = now();
        let (start, _) = Range::Last7.window(now).unwrap();
        assert_eq!(now - Duration::days(7), start.with_timezone(&Local));
        let (start, _) = Range::Last30.window(now).unwrap();
        assert_eq!(now - Duration::days(30), start.with_timezone(&Local));
    }

    #[test]
    fn ranges_without_a_query_have_no_window() {
        assert!(Range::All.window(now()).is_none());
        assert!(Range::Session.window(now()).is_none());
        assert!(!Range::All.needs_api());
        assert!(!Range::Session.needs_api());
        assert!(Range::Week.needs_api());
    }

    #[test]
    fn parses_cli_aliases() {
        assert_eq!(Range::parse("week"), Some(Range::Week));
        assert_eq!(Range::parse("7D"), Some(Range::Last7));
        assert_eq!(Range::parse("last-30-days"), Some(Range::Last30));
        assert_eq!(Range::parse("none"), Some(Range::All));
        assert_eq!(Range::parse("banana"), None);
    }

    #[test]
    fn every_documented_digit_selects_its_window() {
        for (key, range) in [
            ('0', Range::All),
            ('1', Range::Session),
            ('2', Range::Week),
            ('3', Range::Last7),
            ('4', Range::Month),
            ('5', Range::Last30),
            ('6', Range::Quarter),
            ('7', Range::Year),
            ('a', Range::All),
        ] {
            assert_eq!(Range::from_key(key), Some(range), "key {key}");
        }
        assert_eq!(Range::from_key('8'), None);
        assert_eq!(Range::from_key('q'), None);
    }
}
