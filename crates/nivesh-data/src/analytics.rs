//! Pure market analytics: breadth counts, share of stocks above their moving
//! averages, and index returns over fixed periods.

use nivesh_core::bars::{day_number, days_in_month, ist_day_number, ymd};
use nivesh_core::{Candle, Quote};
use nivesh_indicators::sma;
use serde::{Deserialize, Serialize};

/// Advances, declines and unchanged among a set of stocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Breadth {
    pub advances: u32,
    pub declines: u32,
    pub unchanged: u32,
}

impl Breadth {
    pub fn total(&self) -> u32 {
        self.advances + self.declines + self.unchanged
    }
}

/// Counts by the sign of the change against the previous close. A quote with no
/// usable previous close can't be classified and is left out of the counts.
pub fn breadth(quotes: &[Quote]) -> Breadth {
    let mut b = Breadth::default();
    for q in quotes {
        if !(q.ltp.is_finite() && q.prev_close.is_finite() && q.prev_close > 0.0) {
            continue;
        }
        // Compare in paise so float noise can't make a flat stock "advance".
        let change = (q.ltp * 100.0).round() - (q.prev_close * 100.0).round();
        if change > 0.0 {
            b.advances += 1;
        } else if change < 0.0 {
            b.declines += 1;
        } else {
            b.unchanged += 1;
        }
    }
    b
}

/// How many stocks closed above their `period`-day simple moving average.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AboveMa {
    pub period: u32,
    /// Stocks with at least `period` closes, i.e. those that could be measured.
    pub measured: u32,
    pub above: u32,
}

impl AboveMa {
    /// Percent of the measured stocks above the average, if any were measured.
    pub fn percent(&self) -> Option<f64> {
        (self.measured > 0).then(|| f64::from(self.above) / f64::from(self.measured) * 100.0)
    }
}

/// For each period, how many of the `closes` series (oldest first) end above
/// their SMA. A series shorter than the period isn't measured for it.
pub fn above_moving_averages(closes: &[Vec<f64>], periods: &[u32]) -> Vec<AboveMa> {
    periods
        .iter()
        .map(|&period| {
            let mut out = AboveMa {
                period,
                measured: 0,
                above: 0,
            };
            for series in closes {
                let Some(last) = series.last().copied().filter(|c| c.is_finite()) else {
                    continue;
                };
                let Some(avg) = sma(series, period as usize).last().copied() else {
                    continue;
                };
                if !avg.is_finite() {
                    continue;
                }
                out.measured += 1;
                if last > avg {
                    out.above += 1;
                }
            }
            out
        })
        .collect()
}

/// Percent change of an index over the usual look-backs. `None` where the
/// history doesn't reach back far enough.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Returns {
    pub d1: Option<f64>,
    pub w1: Option<f64>,
    pub m1: Option<f64>,
    pub ytd: Option<f64>,
}

fn pct(now: f64, then: f64) -> Option<f64> {
    (now.is_finite() && then.is_finite() && then > 0.0).then(|| (now / then - 1.0) * 100.0)
}

/// Returns from daily candles (oldest first, the last one is "now"):
///
/// * `d1`: against the previous bar's close,
/// * `w1`: against the close on or before 7 days earlier,
/// * `m1`: against the close on or before the same date a calendar month
///   earlier (the month's length clamps the day: 31 Mar looks back to 28 Feb),
/// * `ytd`: against the last close of the previous calendar year.
///
/// All dates are IST dates.
pub fn period_returns(daily: &[Candle]) -> Returns {
    let Some(last) = daily.last() else {
        return Returns::default();
    };
    let day = ist_day_number(last.t);
    let (year, month, dom) = ymd(day);
    let close_on_or_before = |target: i64| {
        daily
            .iter()
            .rev()
            .find(|c| ist_day_number(c.t) <= target)
            .map(|c| c.c)
    };

    let (prev_year, prev_month) = if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    };
    let month_ago = day_number(
        prev_year,
        prev_month,
        dom.min(days_in_month(prev_year, prev_month)),
    );
    let year_end = day_number(year, 1, 1) - 1;

    Returns {
        d1: daily
            .len()
            .checked_sub(2)
            .and_then(|i| pct(last.c, daily[i].c)),
        w1: close_on_or_before(day - 7).and_then(|then| pct(last.c, then)),
        m1: close_on_or_before(month_ago).and_then(|then| pct(last.c, then)),
        ytd: close_on_or_before(year_end).and_then(|then| pct(last.c, then)),
    }
}

#[cfg(test)]
mod tests {
    use nivesh_core::{Exchange, InstrumentId};

    use super::*;

    fn q(ltp: f64, prev: f64) -> Quote {
        Quote {
            id: InstrumentId::new(Exchange::Nse, "X").unwrap(),
            name: None,
            ltp,
            prev_close: prev,
            open: None,
            high: None,
            low: None,
            volume: None,
            week52_high: None,
            week52_low: None,
            upper_band: None,
            lower_band: None,
        }
    }

    #[test]
    fn breadth_counts_by_sign_in_paise_and_skips_unusable_rows() {
        let b = breadth(&[
            q(101.0, 100.0),
            q(100.004, 100.0), // under half a paisa: unchanged
            q(99.0, 100.0),
            q(100.0, 100.0),
            q(5.0, 0.0),
            q(f64::NAN, 1.0),
        ]);
        assert_eq!((b.advances, b.declines, b.unchanged), (1, 1, 2));
        assert_eq!(b.total(), 4);
    }

    #[test]
    fn above_moving_average_counts_only_measurable_series() {
        let rising: Vec<f64> = (1..=30).map(f64::from).collect(); // last 30 > SMA
        let falling: Vec<f64> = (1..=30).rev().map(f64::from).collect(); // last 1 < SMA
        let short = vec![5.0; 4];
        let out = above_moving_averages(&[rising, falling, short], &[5, 20, 50]);
        assert_eq!((out[0].measured, out[0].above), (2, 1));
        assert_eq!(out[0].percent(), Some(50.0));
        assert_eq!((out[1].measured, out[1].above), (2, 1));
        assert_eq!((out[2].measured, out[2].above), (0, 0));
        assert_eq!(out[2].percent(), None);
    }

    /// A daily candle at the IST open of 2026-`month`-`day` closing at `c`.
    fn bar(year: i32, month: u32, day: u32, c: f64) -> Candle {
        let open_ms = (day_number(year, month, day) * 86_400 + 9 * 3600 + 15 * 60 - 19_800) * 1000;
        Candle {
            t: open_ms,
            o: c,
            h: c,
            l: c,
            c,
            v: 1.0,
        }
    }

    #[test]
    fn returns_use_calendar_look_backs() {
        let daily = [
            bar(2025, 12, 30, 100.0),
            bar(2025, 12, 31, 200.0), // last close of the previous year
            bar(2026, 10, 16, 250.0),
            bar(2026, 11, 6, 300.0), // on or before 7 days before the 13th
            bar(2026, 11, 12, 320.0),
            bar(2026, 11, 13, 330.0),
        ];
        let r = period_returns(&daily);
        assert!((r.d1.unwrap() - (330.0 / 320.0 - 1.0) * 100.0).abs() < 1e-9);
        // 7 days before 13 Nov is 6 Nov.
        assert!((r.w1.unwrap() - 10.0).abs() < 1e-9);
        // A month before 13 Nov is 13 Oct. The newest bar on or before it is
        // 31 Dec 2025 (200); the 16 Oct bar is too late.
        assert!((r.m1.unwrap() - 65.0).abs() < 1e-9);
        assert!((r.ytd.unwrap() - 65.0).abs() < 1e-9);
    }

    #[test]
    fn a_month_look_back_clamps_the_day() {
        // 31 Mar looks back to 28 Feb (2026 is not a leap year).
        let daily = [
            bar(2026, 2, 27, 90.0),
            bar(2026, 2, 28, 100.0),
            bar(2026, 3, 31, 110.0),
        ];
        assert!((period_returns(&daily).m1.unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn short_histories_leave_returns_unset() {
        assert_eq!(period_returns(&[]), Returns::default());
        let one = [bar(2026, 11, 13, 100.0)];
        assert_eq!(period_returns(&one), Returns::default());
        // A year-old listing has no previous-year close, so no YTD.
        let young = [bar(2026, 3, 1, 100.0), bar(2026, 3, 2, 101.0)];
        let r = period_returns(&young);
        assert!(r.d1.is_some() && r.ytd.is_none());
    }
}
