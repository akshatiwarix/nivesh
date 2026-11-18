//! Session-anchored VWAP.

use crate::nan;

/// India Standard Time, UTC+05:30. NSE and BSE sessions are IST days.
pub const IST_OFFSET_SECS: i64 = 19_800;

/// Streaming VWAP anchored to a session: cumulative `typical price * volume`
/// over cumulative volume since the session began, where typical price is
/// `(h + l + c) / 3`. Call [`reset`](Vwap::reset) at each new session, or use
/// [`vwap`] to derive sessions from timestamps.
#[derive(Debug, Clone, Default)]
pub struct Vwap {
    pv: f64,
    volume: f64,
}

impl Vwap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `None` until some volume has traded in the session.
    pub fn update(&mut self, high: f64, low: f64, close: f64, volume: f64) -> Option<f64> {
        self.pv += (high + low + close) / 3.0 * volume;
        self.volume += volume;
        (self.volume > 0.0).then(|| self.pv / self.volume)
    }
}

/// VWAP per bar, restarting at each calendar day in the zone `utc_offset_secs`
/// east of UTC (use [`IST_OFFSET_SECS`]). `t` is UTC epoch milliseconds. On
/// daily or longer bars every bar is its own session, so VWAP is just the
/// typical price; the chart only offers it for intraday intervals.
pub fn vwap(
    t: &[i64],
    high: &[f64],
    low: &[f64],
    close: &[f64],
    volume: &[f64],
    utc_offset_secs: i64,
) -> Vec<f64> {
    let mut v = Vwap::new();
    let mut day = None;
    let n = t
        .len()
        .min(high.len())
        .min(low.len())
        .min(close.len())
        .min(volume.len());
    (0..n)
        .map(|i| {
            let this_day = (t[i] / 1000 + utc_offset_secs).div_euclid(86_400);
            if day != Some(this_day) {
                v.reset();
                day = Some(this_day);
            }
            v.update(high[i], low[i], close[i], volume[i])
                .unwrap_or_else(nan)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vwap_restarts_each_ist_day() {
        const H: i64 = 3_600_000;
        // 00:00 and 01:00 UTC are 05:30 and 06:30 IST, the same IST day.
        // 19:00 UTC is 00:30 IST the next day: a new session.
        let t = [0, H, 19 * H, 20 * H];
        let px = [10.0, 20.0, 100.0, 200.0];
        let vol = [1.0, 3.0, 1.0, 1.0];
        let out = vwap(&t, &px, &px, &px, &vol, IST_OFFSET_SECS);
        assert_eq!(out[0], 10.0);
        assert_eq!(out[1], (10.0 + 60.0) / 4.0);
        assert_eq!(out[2], 100.0);
        assert_eq!(out[3], 150.0);
    }

    #[test]
    fn no_volume_means_no_value() {
        let out = vwap(&[0], &[1.0], &[1.0], &[1.0], &[0.0], 0);
        assert!(out[0].is_nan());
    }
}
