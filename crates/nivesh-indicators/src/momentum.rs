//! RSI and MACD.

use crate::ma::Ema;
use crate::nan;

/// Streaming Wilder RSI.
///
/// The first average gain and loss are plain means of the first `n` changes
/// (so the first value appears on the `n + 1`-th input); after that each is
/// smoothed as `(prev * (n - 1) + x) / n`. A flat window (no gains, no
/// losses) reads 50; losses of zero with gains reads 100.
#[derive(Debug, Clone)]
pub struct Rsi {
    period: usize,
    prev: Option<f64>,
    changes: usize,
    gain: f64,
    loss: f64,
}

impl Rsi {
    pub fn new(period: usize) -> Self {
        Self {
            period: period.max(1),
            prev: None,
            changes: 0,
            gain: 0.0,
            loss: 0.0,
        }
    }

    pub fn update(&mut self, x: f64) -> Option<f64> {
        let prev = self.prev.replace(x)?;
        let change = x - prev;
        let (g, l) = if change > 0.0 {
            (change, 0.0)
        } else {
            (0.0, -change)
        };
        let n = self.period as f64;
        if self.changes < self.period {
            self.changes += 1;
            self.gain += g;
            self.loss += l;
            if self.changes < self.period {
                return None;
            }
            self.gain /= n;
            self.loss /= n;
        } else {
            self.gain = (self.gain * (n - 1.0) + g) / n;
            self.loss = (self.loss * (n - 1.0) + l) / n;
        }
        Some(rsi_from(self.gain, self.loss))
    }
}

fn rsi_from(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        if avg_gain == 0.0 { 50.0 } else { 100.0 }
    } else {
        100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
    }
}

/// `RSI(n)` with Wilder smoothing. `NaN` for the first `n` points.
pub fn rsi(values: &[f64], period: usize) -> Vec<f64> {
    let mut r = Rsi::new(period);
    values
        .iter()
        .map(|&x| r.update(x).unwrap_or_else(nan))
        .collect()
}

/// One MACD reading. Fields not yet defined are `NaN`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacdPoint {
    pub macd: f64,
    pub signal: f64,
    pub hist: f64,
}

/// MACD lines as parallel columns.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MacdSeries {
    pub macd: Vec<f64>,
    pub signal: Vec<f64>,
    pub hist: Vec<f64>,
}

/// Streaming MACD: `macd = EMA(fast) - EMA(slow)`, `signal = EMA(macd, signal)`
/// (seeded with the SMA of the first `signal` MACD values) and
/// `hist = macd - signal`.
#[derive(Debug, Clone)]
pub struct Macd {
    fast: Ema,
    slow: Ema,
    signal: Ema,
}

impl Macd {
    pub fn new(fast: usize, slow: usize, signal: usize) -> Self {
        Self {
            fast: Ema::new(fast),
            slow: Ema::new(slow),
            signal: Ema::new(signal),
        }
    }

    pub fn update(&mut self, x: f64) -> MacdPoint {
        let f = self.fast.update(x);
        let s = self.slow.update(x);
        let macd = match (f, s) {
            (Some(f), Some(s)) => Some(f - s),
            _ => None,
        };
        let signal = macd.and_then(|m| self.signal.update(m));
        MacdPoint {
            macd: macd.unwrap_or_else(nan),
            signal: signal.unwrap_or_else(nan),
            hist: match (macd, signal) {
                (Some(m), Some(s)) => m - s,
                _ => nan(),
            },
        }
    }
}

/// `MACD(fast, slow, signal)`; the usual setting is `(12, 26, 9)`.
pub fn macd(values: &[f64], fast: usize, slow: usize, signal: usize) -> MacdSeries {
    let mut m = Macd::new(fast, slow, signal);
    let mut out = MacdSeries::default();
    for &x in values {
        let p = m.update(x);
        out.macd.push(p.macd);
        out.signal.push(p.signal);
        out.hist.push(p.hist);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked RSI(14) example from StockCharts' ChartSchool, an
    /// independent published source: closes and the RSI column.
    const CLOSES: [f64; 33] = [
        44.3389, 44.0902, 44.1497, 43.6124, 44.3278, 44.8264, 45.0955, 45.4245, 45.8433, 46.0826,
        45.8931, 46.0328, 45.6140, 46.2820, 46.2820, 46.0028, 46.0328, 46.4116, 46.2222, 45.6439,
        46.2122, 46.2521, 45.7137, 46.4515, 45.7835, 45.3548, 44.0288, 44.1783, 44.2181, 44.5672,
        43.4205, 42.6628, 43.1314,
    ];
    const RSI: [f64; 19] = [
        70.53, 66.32, 66.55, 69.41, 66.36, 57.97, 62.93, 63.26, 56.06, 62.38, 54.71, 50.42, 39.99,
        41.46, 41.87, 45.46, 37.30, 33.08, 37.77,
    ];

    #[test]
    fn rsi_matches_the_published_worked_example() {
        let out = rsi(&CLOSES, 14);
        assert!(out[..14].iter().all(|x| x.is_nan()));
        for (i, want) in RSI.iter().enumerate() {
            let got = out[14 + i];
            assert!(
                (got - want).abs() < 0.01,
                "RSI[{}] = {got}, want {want}",
                14 + i
            );
        }
    }

    #[test]
    fn rsi_edge_cases() {
        let up: Vec<f64> = (0..20).map(f64::from).collect();
        assert_eq!(rsi(&up, 14)[19], 100.0);
        let down: Vec<f64> = (0..20).rev().map(f64::from).collect();
        assert_eq!(rsi(&down, 14)[19], 0.0);
        assert_eq!(rsi(&[5.0; 20], 14)[19], 50.0);
        assert!(rsi(&[1.0, 2.0], 14).iter().all(|x| x.is_nan()));
    }

    #[test]
    fn macd_warms_up_then_histogram_is_the_difference() {
        let c: Vec<f64> = (0..60).map(|i| 100.0 + f64::from(i).sin() * 5.0).collect();
        let m = macd(&c, 12, 26, 9);
        assert!(m.macd[..25].iter().all(|x| x.is_nan()));
        assert!(m.macd[25].is_finite());
        assert!(m.signal[..33].iter().all(|x| x.is_nan()));
        assert!(m.signal[33].is_finite());
        for i in 33..60 {
            assert!((m.hist[i] - (m.macd[i] - m.signal[i])).abs() < 1e-12);
        }
    }
}
