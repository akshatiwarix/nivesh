//! The forming (latest, still-open) bar of a chart.
//!
//! History gives finished bars. The bar in progress is built here from the
//! price stream: each quote moves the last bar's close and extends its high or
//! low, and the first quote of a new interval opens the next bar. The quote's
//! own timestamp decides which bar it belongs to (not the wall clock), so a
//! delayed feed that is still reporting the previous interval's trades updates
//! that bar instead of opening a new one early.
//!
//! Volume is not tracked: public quotes report the day's cumulative volume,
//! which can't be split per bar. A forming bar keeps the volume history gave
//! it (zero for a bar that opened since), and the next history load replaces it.

use nivesh_core::bars::bucket_start;
use nivesh_core::{Candle, Interval};

#[derive(Debug, Clone)]
pub struct FormingBar {
    interval: Interval,
    bar: Option<Candle>,
}

impl FormingBar {
    /// `last` is the newest bar of the loaded history, if any.
    pub fn new(interval: Interval, last: Option<Candle>) -> Self {
        Self {
            interval,
            bar: last,
        }
    }

    /// Applies a price observed at `as_of_ms` (UTC epoch ms). Returns the bar
    /// to draw when it changed, and `None` for a price from an older interval
    /// or one that changes nothing.
    pub fn apply(&mut self, price: f64, as_of_ms: i64) -> Option<Candle> {
        if !price.is_finite() || price <= 0.0 {
            return None;
        }
        let bucket = bucket_start(self.interval, as_of_ms);
        let next = match self.bar {
            Some(bar) => {
                let current = bucket_start(self.interval, bar.t);
                if bucket < current {
                    return None;
                }
                if bucket == current {
                    Candle {
                        h: bar.h.max(price),
                        l: bar.l.min(price),
                        c: price,
                        ..bar
                    }
                } else {
                    Self::opened(bucket, price)
                }
            }
            None => Self::opened(bucket, price),
        };
        (self.bar != Some(next)).then(|| {
            self.bar = Some(next);
            next
        })
    }

    /// Like [`Self::apply`], but reports the bar that just **closed**: the
    /// previous bar, once a price from a later interval opens the next one.
    /// A strategy decides on closed bars only, so this is its feed. A bar
    /// closes when the next interval's first price arrives, not on the wall
    /// clock, so the day's last bar closes with the next session's first
    /// price.
    pub fn apply_closing(&mut self, price: f64, as_of_ms: i64) -> Option<Candle> {
        let before = self.bar;
        let after = self.apply(price, as_of_ms)?;
        before.filter(|b| bucket_start(self.interval, after.t) > bucket_start(self.interval, b.t))
    }

    fn opened(t: i64, price: f64) -> Candle {
        Candle {
            t,
            o: price,
            h: price,
            l: price,
            c: price,
            v: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-16 (Monday) 00:00 IST in UTC ms.
    const MONDAY: i64 = 1_794_767_400_000;
    const MIN: i64 = 60_000;

    fn at(h: i64, m: i64) -> i64 {
        MONDAY + (h * 60 + m) * MIN
    }

    fn bar(t: i64) -> Candle {
        Candle {
            t,
            o: 100.0,
            h: 105.0,
            l: 99.0,
            c: 101.0,
            v: 50.0,
        }
    }

    #[test]
    fn a_price_in_the_open_interval_moves_the_close_and_extends_the_range() {
        let mut f = FormingBar::new(Interval::M5, Some(bar(at(10, 0))));
        let up = f.apply(106.0, at(10, 3)).unwrap();
        assert_eq!(
            (up.o, up.h, up.l, up.c, up.v),
            (100.0, 106.0, 99.0, 106.0, 50.0)
        );
        let down = f.apply(98.0, at(10, 4)).unwrap();
        assert_eq!((down.h, down.l, down.c), (106.0, 98.0, 98.0));
        // The same price again changes nothing, so nothing is sent.
        assert_eq!(f.apply(98.0, at(10, 4)), None);
    }

    #[test]
    fn the_first_price_of_the_next_interval_opens_a_new_bar() {
        let mut f = FormingBar::new(Interval::M5, Some(bar(at(10, 0))));
        let next = f.apply(102.0, at(10, 5)).unwrap();
        assert_eq!(next.t, at(10, 5));
        assert_eq!(
            (next.o, next.h, next.l, next.c, next.v),
            (102.0, 102.0, 102.0, 102.0, 0.0)
        );
        // And it keeps building that bar.
        assert_eq!(f.apply(103.0, at(10, 6)).unwrap().h, 103.0);
    }

    #[test]
    fn a_late_price_from_an_older_interval_is_ignored() {
        let mut f = FormingBar::new(Interval::M5, Some(bar(at(10, 5))));
        assert_eq!(f.apply(200.0, at(10, 2)), None);
    }

    #[test]
    fn the_quote_time_not_the_wall_clock_picks_the_bar() {
        // A delayed feed reporting a 10:04 trade belongs to the 10:00 bar.
        let mut f = FormingBar::new(Interval::M5, Some(bar(at(10, 0))));
        assert_eq!(f.apply(101.5, at(10, 4)).unwrap().t, at(10, 0));
    }

    #[test]
    fn daily_bars_match_the_providers_open_time_for_the_day() {
        // Providers stamp daily bars 09:15 IST; a mid-session price is the same day.
        let mut f = FormingBar::new(Interval::D1, Some(bar(at(9, 15))));
        let b = f.apply(110.0, at(13, 0)).unwrap();
        assert_eq!((b.t, b.c, b.h), (at(9, 15), 110.0, 110.0));
        // Tomorrow's first price opens a new daily bar.
        let next = f.apply(111.0, at(24 + 9, 20)).unwrap();
        assert_eq!(next.t, at(24 + 9, 15));
    }

    #[test]
    fn weekly_bars_roll_over_on_monday() {
        let mut f = FormingBar::new(Interval::W1, Some(bar(at(9, 15))));
        assert_eq!(f.apply(120.0, at(4 * 24 + 11, 0)).unwrap().t, at(9, 15));
        assert_eq!(
            f.apply(121.0, at(7 * 24 + 10, 0)).unwrap().t,
            at(7 * 24 + 9, 15)
        );
    }

    #[test]
    fn the_previous_bar_closes_when_the_next_interval_opens() {
        let mut f = FormingBar::new(Interval::M5, Some(bar(at(10, 0))));
        assert_eq!(f.apply_closing(106.0, at(10, 3)), None, "still forming");
        let closed = f.apply_closing(104.0, at(10, 5)).expect("closed");
        assert_eq!((closed.t, closed.h, closed.c), (at(10, 0), 106.0, 106.0));
        assert_eq!(f.apply_closing(105.0, at(10, 6)), None);
        // A late price from the closed interval reopens nothing.
        assert_eq!(f.apply_closing(90.0, at(10, 4)), None);
        // An empty start has nothing to close.
        let mut empty = FormingBar::new(Interval::M5, None);
        assert_eq!(empty.apply_closing(50.0, at(10, 0)), None);
        assert!(empty.apply_closing(51.0, at(10, 5)).is_some());
    }

    #[test]
    fn an_empty_chart_starts_from_the_first_price_and_junk_is_ignored() {
        let mut f = FormingBar::new(Interval::M1, None);
        assert_eq!(f.apply(f64::NAN, at(10, 0)), None);
        assert_eq!(f.apply(0.0, at(10, 0)), None);
        assert_eq!(f.apply(50.0, at(10, 0)).unwrap().t, at(10, 0));
    }
}
