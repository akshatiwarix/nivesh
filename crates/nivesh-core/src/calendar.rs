//! The NSE equity market calendar and session status, in IST.
//!
//! Pure and clock-free: callers pass the instant as Unix seconds (UTC), and
//! the result is computed in IST (a fixed UTC+05:30, no daylight saving), so
//! no timezone database is needed. Times are stored as UTC elsewhere and only
//! converted for display.
//!
//! Sessions (NSE equity segment, IST):
//!
//! | Time          | Session   |
//! |---------------|-----------|
//! | 09:00 - 09:15 | `PreOpen` |
//! | 09:15 - 15:30 | `Normal`  |
//! | 15:30 - 15:40 | `Closed`  |
//! | 15:40 - 16:00 | `Closing` |
//! | otherwise     | `Closed`  |
//!
//! Weekends and listed holidays are `Closed` all day. Special sessions
//! (Muhurat trading, weekend special sessions) are not modelled.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const IST_OFFSET_SECS: i64 = 5 * 3600 + 30 * 60;
const DAY_SECS: i64 = 86_400;

const PRE_OPEN: i64 = 9 * 3600;
const NORMAL_OPEN: i64 = 9 * 3600 + 15 * 60;
const NORMAL_CLOSE: i64 = 15 * 3600 + 30 * 60;
const CLOSING_OPEN: i64 = 15 * 3600 + 40 * 60;
const CLOSING_CLOSE: i64 = 16 * 3600;
/// Seconds-of-day at which the session changes on a trading day.
const BOUNDARIES: [i64; 5] = [
    PRE_OPEN,
    NORMAL_OPEN,
    NORMAL_CLOSE,
    CLOSING_OPEN,
    CLOSING_CLOSE,
];

/// How far ahead to look for the next trading day. Far longer than any real
/// run of holidays.
const MAX_LOOKAHEAD_DAYS: i64 = 30;

/// Holiday data bundled with the app: `(year, file contents)`.
const BUNDLED: &[(i32, &str)] = &[(2026, include_str!("../data/nse_holidays_2026.txt"))];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Session {
    PreOpen,
    Normal,
    Closing,
    Closed,
}

/// A calendar date in IST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// Days since 1970-01-01. Returns `None` for a date that doesn't exist.
    fn to_days(self) -> Option<i64> {
        if !(1..=12).contains(&self.month) || self.day == 0 {
            return None;
        }
        let days = days_from_civil(self.year, self.month, self.day);
        (civil_from_days(days) == self).then_some(days)
    }
}

/// Howard Hinnant's `days_from_civil`.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> Date {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    // The ranges are guaranteed by the algorithm: m in 1..=12, d in 1..=31,
    // and years stay far inside i32 for any i64 seconds a caller can pass.
    Date {
        year: i32::try_from(y).unwrap_or(i32::MAX),
        month: u32::try_from(m).unwrap_or(1),
        day: u32::try_from(d).unwrap_or(1),
    }
}

/// 0 = Sunday ... 6 = Saturday. 1970-01-01 was a Thursday.
fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

fn is_weekend(days: i64) -> bool {
    matches!(weekday(days), 0 | 6)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CalendarError {
    #[error("line {line}: expected `YYYY-MM-DD Name`")]
    BadLine { line: usize },
    #[error("line {line}: `{date}` is not a valid date")]
    BadDate { line: usize, date: String },
    #[error("line {line}: {date} is not in {year}")]
    WrongYear {
        line: usize,
        date: String,
        year: i32,
    },
}

/// Holidays for the years the app has data for.
#[derive(Debug, Clone, Default)]
pub struct MarketCalendar {
    /// Days since epoch -> holiday name.
    holidays: BTreeMap<i64, String>,
    /// Years for which holidays are known.
    years: BTreeSet<i32>,
}

/// What the market is doing at an instant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarketStatus {
    pub session: Session,
    /// Name of the holiday if the IST date is a listed trading holiday.
    pub holiday: Option<String>,
    /// Unix seconds (UTC) of the next session change.
    pub next_change_utc: i64,
    /// False when there is no holiday data for this year. Weekday hours are
    /// still applied, but a holiday would be missed, so show this to the user.
    pub calendar_known: bool,
}

impl MarketCalendar {
    /// The holiday data compiled into the binary.
    pub fn bundled() -> Self {
        let mut calendar = Self::default();
        for (year, text) in BUNDLED {
            // A bad bundled file is a bug caught by `bundled_data_parses_and_is_sane`.
            // If one ever slips through, `add_year` leaves the calendar untouched
            // and statuses report `calendar_known: false` instead of crashing.
            let _ = calendar.add_year(*year, text);
        }
        calendar
    }

    /// Adds one year of holidays from `YYYY-MM-DD Name` lines.
    pub fn add_year(&mut self, year: i32, text: &str) -> Result<(), CalendarError> {
        let mut parsed = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            let (date, name) = raw
                .split_once(char::is_whitespace)
                .ok_or(CalendarError::BadLine { line })?;
            let name = name.trim();
            if name.is_empty() {
                return Err(CalendarError::BadLine { line });
            }
            let bad_date = || CalendarError::BadDate {
                line,
                date: date.to_owned(),
            };
            let mut parts = date.split('-');
            let (Some(y), Some(m), Some(d), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Err(bad_date());
            };
            let civil = Date {
                year: y.parse().map_err(|_| bad_date())?,
                month: m.parse().map_err(|_| bad_date())?,
                day: d.parse().map_err(|_| bad_date())?,
            };
            let days = civil.to_days().ok_or_else(bad_date)?;
            if civil.year != year {
                return Err(CalendarError::WrongYear {
                    line,
                    date: date.to_owned(),
                    year,
                });
            }
            parsed.push((days, name.to_owned()));
        }
        // Only commit once the whole file parsed.
        self.holidays.extend(parsed);
        self.years.insert(year);
        Ok(())
    }

    /// Holiday name for an IST date, if it is a listed holiday.
    pub fn holiday(&self, date: Date) -> Option<&str> {
        let days = date.to_days()?;
        self.holidays.get(&days).map(String::as_str)
    }

    fn is_trading_day(&self, days: i64) -> bool {
        !is_weekend(days) && !self.holidays.contains_key(&days)
    }

    /// Market status at `utc_secs` (Unix seconds, UTC).
    pub fn status_at(&self, utc_secs: i64) -> MarketStatus {
        let ist = utc_secs.saturating_add(IST_OFFSET_SECS);
        let days = ist.div_euclid(DAY_SECS);
        let sod = ist.rem_euclid(DAY_SECS);
        let trading = self.is_trading_day(days);

        let session = if !trading {
            Session::Closed
        } else if (PRE_OPEN..NORMAL_OPEN).contains(&sod) {
            Session::PreOpen
        } else if (NORMAL_OPEN..NORMAL_CLOSE).contains(&sod) {
            Session::Normal
        } else if (CLOSING_OPEN..CLOSING_CLOSE).contains(&sod) {
            Session::Closing
        } else {
            Session::Closed
        };

        let next_ist = trading
            .then(|| BOUNDARIES.iter().find(|&&b| b > sod))
            .flatten()
            .map(|b| days * DAY_SECS + b)
            .unwrap_or_else(|| self.next_open_after(days));

        MarketStatus {
            session,
            holiday: self.holidays.get(&days).cloned(),
            next_change_utc: next_ist - IST_OFFSET_SECS,
            calendar_known: self.years.contains(&civil_from_days(days).year),
        }
    }

    /// IST seconds of the pre-open of the first trading day after `days`.
    fn next_open_after(&self, days: i64) -> i64 {
        let next = (days + 1..=days + MAX_LOOKAHEAD_DAYS)
            .find(|&d| self.is_trading_day(d))
            .unwrap_or(days + 1);
        next * DAY_SECS + PRE_OPEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix seconds for an IST wall-clock time.
    fn ist(y: i32, mo: u32, d: u32, h: i64, mi: i64, s: i64) -> i64 {
        days_from_civil(y, mo, d) * DAY_SECS + h * 3600 + mi * 60 + s - IST_OFFSET_SECS
    }

    fn cal() -> MarketCalendar {
        MarketCalendar::bundled()
    }

    #[test]
    fn civil_conversion_round_trips() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday(0), 4); // Thursday
        for days in [-800_000, -1, 0, 59, 60, 11_016, 20_000, 2_000_000] {
            let d = civil_from_days(days);
            assert_eq!(d.to_days(), Some(days), "{d:?}");
        }
        // 2026-11-18 is a Wednesday.
        assert_eq!(weekday(days_from_civil(2026, 11, 18)), 3);
        // Leap day handling.
        assert!(
            Date {
                year: 2028,
                month: 2,
                day: 29
            }
            .to_days()
            .is_some()
        );
        assert!(
            Date {
                year: 2026,
                month: 2,
                day: 29
            }
            .to_days()
            .is_none()
        );
    }

    #[test]
    fn session_boundaries_on_a_trading_day() {
        // Wednesday 2026-11-18.
        let cases = [
            ((8, 59, 59), Session::Closed),
            ((9, 0, 0), Session::PreOpen),
            ((9, 14, 59), Session::PreOpen),
            ((9, 15, 0), Session::Normal),
            ((15, 29, 59), Session::Normal),
            ((15, 30, 0), Session::Closed),
            ((15, 39, 59), Session::Closed),
            ((15, 40, 0), Session::Closing),
            ((15, 59, 59), Session::Closing),
            ((16, 0, 0), Session::Closed),
            ((23, 59, 59), Session::Closed),
            ((0, 0, 0), Session::Closed),
        ];
        for ((h, m, s), expected) in cases {
            let status = cal().status_at(ist(2026, 11, 18, h, m, s));
            assert_eq!(status.session, expected, "{h:02}:{m:02}:{s:02}");
            assert!(status.calendar_known);
            assert_eq!(status.holiday, None);
        }
    }

    #[test]
    fn next_change_points_at_the_next_boundary() {
        let c = cal();
        let at = |h, m, s| c.status_at(ist(2026, 11, 18, h, m, s)).next_change_utc;
        assert_eq!(at(8, 0, 0), ist(2026, 11, 18, 9, 0, 0));
        assert_eq!(at(9, 0, 0), ist(2026, 11, 18, 9, 15, 0));
        assert_eq!(at(12, 0, 0), ist(2026, 11, 18, 15, 30, 0));
        assert_eq!(at(15, 35, 0), ist(2026, 11, 18, 15, 40, 0));
        assert_eq!(at(15, 45, 0), ist(2026, 11, 18, 16, 0, 0));
        // After close: tomorrow's pre-open.
        assert_eq!(at(16, 0, 0), ist(2026, 11, 19, 9, 0, 0));
    }

    #[test]
    fn weekends_are_closed_and_next_open_skips_them() {
        let c = cal();
        // Saturday 2026-11-21 noon.
        let sat = c.status_at(ist(2026, 11, 21, 12, 0, 0));
        assert_eq!(sat.session, Session::Closed);
        assert_eq!(sat.holiday, None);
        assert_eq!(sat.next_change_utc, ist(2026, 11, 23, 9, 0, 0));
        // Friday after close -> Monday.
        let fri = c.status_at(ist(2026, 11, 20, 17, 0, 0));
        assert_eq!(fri.next_change_utc, ist(2026, 11, 23, 9, 0, 0));
    }

    #[test]
    fn holidays_are_closed_all_day_and_named() {
        let c = cal();
        // Republic Day 2026-01-26, a Monday.
        let s = c.status_at(ist(2026, 1, 26, 10, 0, 0));
        assert_eq!(s.session, Session::Closed);
        assert_eq!(s.holiday.as_deref(), Some("Republic Day"));
        assert_eq!(s.next_change_utc, ist(2026, 1, 27, 9, 0, 0));
        // The day before is a normal trading day (Friday 23rd is, Sunday 25th isn't).
        assert_eq!(
            c.status_at(ist(2026, 1, 23, 10, 0, 0)).session,
            Session::Normal
        );
    }

    #[test]
    fn consecutive_closures_chain() {
        // Thursday 2026-04-02 is a trading day; Fri 3 Apr is Good Friday, then a weekend.
        let after = cal().status_at(ist(2026, 4, 2, 17, 0, 0));
        assert_eq!(after.next_change_utc, ist(2026, 4, 6, 9, 0, 0));
    }

    #[test]
    fn utc_and_ist_dates_differ_around_midnight() {
        let c = cal();
        // 2026-01-25 20:00 UTC = 2026-01-26 01:30 IST: already the holiday.
        let utc = days_from_civil(2026, 1, 25) * DAY_SECS + 20 * 3600;
        assert_eq!(c.status_at(utc).holiday.as_deref(), Some("Republic Day"));
        // 2026-11-18 03:45 UTC = 09:15 IST sharp.
        let utc = days_from_civil(2026, 11, 18) * DAY_SECS + 3 * 3600 + 45 * 60;
        assert_eq!(c.status_at(utc).session, Session::Normal);
        assert_eq!(c.status_at(utc - 1).session, Session::PreOpen);
    }

    #[test]
    fn years_without_data_apply_hours_but_flag_it() {
        let s = cal().status_at(ist(2031, 3, 5, 10, 0, 0)); // a Wednesday
        assert_eq!(s.session, Session::Normal);
        assert!(!s.calendar_known);
    }

    #[test]
    fn bundled_data_parses_and_is_sane() {
        let mut c = MarketCalendar::default();
        for (year, text) in BUNDLED {
            c.add_year(*year, text).expect("bundled file parses");
            assert!(c.years.contains(year));
        }
        assert!(c.holidays.len() >= 10);
        // Weekend holidays are omitted, so every listed day is a weekday.
        for &days in c.holidays.keys() {
            assert!(
                !is_weekend(days),
                "{:?} is a weekend",
                civil_from_days(days)
            );
        }
        // And the bundled() constructor kept them all.
        assert_eq!(MarketCalendar::bundled().holidays.len(), c.holidays.len());
    }

    #[test]
    fn parse_errors_are_reported_and_leave_the_calendar_unchanged() {
        let mut c = MarketCalendar::default();
        for (text, expected) in [
            ("2026-01-26", CalendarError::BadLine { line: 1 }),
            (
                "2026-13-01 X",
                CalendarError::BadDate {
                    line: 1,
                    date: "2026-13-01".into(),
                },
            ),
            (
                "# c\n2026-02-30 X",
                CalendarError::BadDate {
                    line: 2,
                    date: "2026-02-30".into(),
                },
            ),
            (
                "2026-01-01 Ok\n2025-12-25 X",
                CalendarError::WrongYear {
                    line: 2,
                    date: "2025-12-25".into(),
                    year: 2026,
                },
            ),
        ] {
            assert_eq!(c.add_year(2026, text), Err(expected));
        }
        assert!(c.holidays.is_empty() && c.years.is_empty());
    }
}
