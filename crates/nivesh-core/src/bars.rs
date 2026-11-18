//! Candle bucketing and resampling, aligned to IST market sessions.
//!
//! NSE's day is 09:15 to 15:30 IST (375 minutes). Intraday bars are anchored
//! to the 09:15 open, so a 1-hour bar runs 09:15-10:15 (as exchanges and
//! brokers draw it, not 09:00-10:00), and the last bar of the day is cut short
//! at 15:30. Four-hour bars are therefore 09:15-13:15 and 13:15-15:30.
//! Daily, weekly (Monday-start) and monthly bars are anchored to the open of
//! their first IST day.
//!
//! Gaps are never filled: a missing bar stays missing, and a bucket with no
//! source bars does not exist. Providers' null or inconsistent rows are
//! dropped by [`normalize`].

use crate::calendar::{civil_from_days, days_from_civil, weekday};
use crate::{Candle, Interval};

const IST_OFFSET_SECS: i64 = 5 * 3600 + 30 * 60;
const DAY_SECS: i64 = 86_400;
const OPEN_SECS: i64 = 9 * 3600 + 15 * 60;
const CLOSE_SECS: i64 = 15 * 3600 + 30 * 60;
const SESSION_SECS: i64 = CLOSE_SECS - OPEN_SECS;

/// The start (UTC epoch ms) of the bar of width `interval` that contains
/// `t_ms`. Times before the open or after the close of a session belong to
/// that session's first or last bar (pre-open and closing-session prints).
pub fn bucket_start(interval: Interval, t_ms: i64) -> i64 {
    let local = t_ms.div_euclid(1000) + IST_OFFSET_SECS;
    let days = local.div_euclid(DAY_SECS);
    let start_of = |day: i64, offset: i64| (day * DAY_SECS + offset - IST_OFFSET_SECS) * 1000;
    match interval {
        Interval::D1 => start_of(days, OPEN_SECS),
        // Monday is 1 in `weekday` (Sunday 0), so Sunday counts as the last
        // day of its week.
        Interval::W1 => start_of(days - (weekday(days) + 6) % 7, OPEN_SECS),
        Interval::Mo1 => {
            let d = civil_from_days(days);
            start_of(days_from_civil(d.year, d.month, 1), OPEN_SECS)
        }
        intraday => {
            let width = intraday.seconds();
            let into = (local.rem_euclid(DAY_SECS) - OPEN_SECS).clamp(0, SESSION_SECS - 1);
            start_of(days, OPEN_SECS + into / width * width)
        }
    }
}

/// Aggregates `candles` (oldest first) into bars of `to`. The first bar of a
/// bucket gives the open, the last the close; high, low and volume combine.
/// Input in a finer interval is expected; coarser input just yields one bar
/// per input bar.
pub fn resample(candles: &[Candle], to: Interval) -> Vec<Candle> {
    let mut out: Vec<Candle> = Vec::new();
    for c in candles {
        let t = bucket_start(to, c.t);
        match out.last_mut() {
            Some(bar) if bar.t == t => {
                bar.h = bar.h.max(c.h);
                bar.l = bar.l.min(c.l);
                bar.c = c.c;
                bar.v += c.v;
            }
            _ => out.push(Candle { t, ..*c }),
        }
    }
    out
}

fn sane(c: &Candle) -> bool {
    [c.o, c.h, c.l, c.c, c.v].iter().all(|x| x.is_finite())
        && c.v >= 0.0
        && c.o > 0.0
        && c.h > 0.0
        && c.l > 0.0
        && c.c > 0.0
}

/// Cleans a provider series: drops rows with non-finite or non-positive
/// prices, negative volume, sorts by time, keeps the last of any duplicated
/// timestamp, and widens `h`/`l` to cover `o` and `c` where a provider rounded
/// them inconsistently.
pub fn normalize(mut candles: Vec<Candle>) -> Vec<Candle> {
    candles.retain(sane);
    candles.sort_by_key(|c| c.t);
    // Of equal timestamps keep the later row in the input (the sort is stable).
    candles.reverse();
    candles.dedup_by_key(|c| c.t);
    candles.reverse();
    for c in &mut candles {
        c.h = c.h.max(c.o).max(c.c).max(c.l);
        c.l = c.l.min(c.o).min(c.c).min(c.h);
    }
    candles
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-16 00:00 IST, a Monday, in UTC epoch seconds.
    const MONDAY: i64 = 1_794_767_400;

    /// UTC ms of an IST wall-clock time, `day` days after that Monday.
    fn ist(day: i64, h: i64, m: i64) -> i64 {
        (MONDAY + day * 86_400 + h * 3600 + m * 60) * 1000
    }

    fn bar(t: i64, o: f64, h: f64, l: f64, c: f64, v: f64) -> Candle {
        Candle { t, o, h, l, c, v }
    }

    #[test]
    fn intraday_buckets_anchor_to_the_0915_open() {
        let cases = [
            (Interval::M5, (0, 9, 17), (0, 9, 15)),
            (Interval::M5, (0, 9, 15), (0, 9, 15)),
            (Interval::M5, (0, 15, 29), (0, 15, 25)),
            // Closing-session print and pre-open auction fold into the edge bars.
            (Interval::M5, (0, 15, 45), (0, 15, 25)),
            (Interval::M5, (0, 9, 5), (0, 9, 15)),
            (Interval::M15, (0, 9, 44), (0, 9, 30)),
            (Interval::H1, (0, 10, 14), (0, 9, 15)),
            (Interval::H1, (0, 10, 15), (0, 10, 15)),
            (Interval::H1, (0, 15, 29), (0, 15, 15)),
            (Interval::H4, (0, 13, 14), (0, 9, 15)),
            (Interval::H4, (0, 13, 15), (0, 13, 15)),
            (Interval::H4, (0, 15, 29), (0, 13, 15)),
            // The next day starts a fresh grid.
            (Interval::H1, (1, 9, 20), (1, 9, 15)),
        ];
        for (interval, (d, h, m), (ed, eh, em)) in cases {
            assert_eq!(
                bucket_start(interval, ist(d, h, m)),
                ist(ed, eh, em),
                "{interval} at day {d} {h}:{m:02}"
            );
        }
    }

    #[test]
    fn day_week_and_month_buckets() {
        assert_eq!(bucket_start(Interval::D1, ist(2, 11, 0)), ist(2, 9, 15));
        // Wednesday and Sunday both belong to the week of Monday the 16th.
        assert_eq!(bucket_start(Interval::W1, ist(2, 11, 0)), ist(0, 9, 15));
        assert_eq!(bucket_start(Interval::W1, ist(6, 20, 0)), ist(0, 9, 15));
        assert_eq!(bucket_start(Interval::W1, ist(7, 9, 20)), ist(7, 9, 15));
        // 2026-11-30 (day 14) belongs to the month starting 2026-11-01 (day -15).
        assert_eq!(bucket_start(Interval::Mo1, ist(14, 10, 0)), ist(-15, 9, 15));
        // 2026-12-01 starts a new month.
        assert_eq!(bucket_start(Interval::Mo1, ist(15, 10, 0)), ist(15, 9, 15));
    }

    #[test]
    fn a_session_of_5m_bars_resamples_to_hourly_with_a_short_last_bar() {
        let five: Vec<Candle> = (0..75)
            .map(|i| {
                let p = 100.0 + f64::from(i);
                bar(
                    ist(0, 9, 15) + i64::from(i) * 300_000,
                    p,
                    p + 2.0,
                    p - 1.0,
                    p + 1.0,
                    10.0,
                )
            })
            .collect();
        let hourly = resample(&five, Interval::H1);
        assert_eq!(hourly.len(), 7);
        assert_eq!(hourly[0].t, ist(0, 9, 15));
        assert_eq!(hourly[6].t, ist(0, 15, 15));
        let first = hourly[0];
        assert_eq!((first.o, first.c), (100.0, 112.0));
        assert_eq!((first.h, first.l, first.v), (113.0, 99.0, 120.0));
        // 15:15-15:30 holds only three 5-minute bars.
        assert_eq!(hourly[6].v, 30.0);
        let four = resample(&five, Interval::H4);
        assert_eq!(
            four.iter().map(|b| b.t).collect::<Vec<_>>(),
            [ist(0, 9, 15), ist(0, 13, 15)]
        );
        assert_eq!(four.iter().map(|b| b.v).sum::<f64>(), 750.0);
    }

    #[test]
    fn gaps_are_left_as_gaps() {
        // Nothing traded 10:15-12:15: those hourly buckets simply don't exist.
        let sparse = [
            bar(ist(0, 9, 15), 10.0, 11.0, 9.0, 10.5, 1.0),
            bar(ist(0, 12, 20), 12.0, 13.0, 11.0, 12.5, 1.0),
        ];
        let hourly = resample(&sparse, Interval::H1);
        assert_eq!(
            hourly.iter().map(|b| b.t).collect::<Vec<_>>(),
            [ist(0, 9, 15), ist(0, 12, 15)]
        );
    }

    #[test]
    fn daily_bars_roll_up_into_weeks() {
        let days: Vec<Candle> = (0..7)
            .filter(|d| d % 7 < 5) // Mon-Fri, then the next week's Mon is day 7
            .map(|d| bar(ist(d, 9, 15), 100.0 + d as f64, 110.0 + d as f64, 90.0, 105.0, 5.0))
            .chain([bar(ist(7, 9, 15), 1.0, 2.0, 0.5, 1.5, 1.0)])
            .collect();
        let weekly = resample(&days, Interval::W1);
        assert_eq!(weekly.len(), 2);
        assert_eq!(
            (weekly[0].o, weekly[0].h, weekly[0].v),
            (100.0, 114.0, 25.0)
        );
        assert_eq!(weekly[1].t, ist(7, 9, 15));
    }

    #[test]
    fn normalize_sorts_dedupes_and_drops_junk() {
        let good = |t, c| bar(t, 10.0, 12.0, 9.0, c, 1.0);
        let out = normalize(vec![
            good(3, 11.0),
            good(1, 10.0),
            bar(2, f64::NAN, 1.0, 1.0, 1.0, 1.0),
            bar(4, 10.0, 12.0, 9.0, 0.0, 1.0),
            good(3, 11.5), // duplicate timestamp: the later row wins
            bar(5, 10.0, 9.5, 9.0, 11.0, 1.0), // high below open/close is widened
        ]);
        assert_eq!(out.iter().map(|c| c.t).collect::<Vec<_>>(), [1, 3, 5]);
        assert_eq!(out[1].c, 11.5);
        assert_eq!((out[2].h, out[2].l), (11.0, 9.0));
    }
}
