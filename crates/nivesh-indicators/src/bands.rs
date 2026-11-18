//! Bollinger bands and ATR.

use std::collections::VecDeque;

use crate::nan;

/// One Bollinger reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BollingerPoint {
    pub middle: f64,
    pub upper: f64,
    pub lower: f64,
}

/// Bollinger bands as parallel columns.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BollingerSeries {
    pub middle: Vec<f64>,
    pub upper: Vec<f64>,
    pub lower: Vec<f64>,
}

/// Streaming Bollinger bands: `SMA(n) ± k * σ`, where σ is the population
/// standard deviation of the same window.
///
/// The variance is recomputed from the window on every update (O(n)) rather
/// than from running sums, which lose precision on large prices.
#[derive(Debug, Clone)]
pub struct Bollinger {
    period: usize,
    k: f64,
    window: VecDeque<f64>,
}

impl Bollinger {
    pub fn new(period: usize, k: f64) -> Self {
        let period = period.max(1);
        Self {
            period,
            k,
            window: VecDeque::with_capacity(period + 1),
        }
    }

    pub fn update(&mut self, x: f64) -> Option<BollingerPoint> {
        self.window.push_back(x);
        if self.window.len() > self.period {
            self.window.pop_front();
        }
        if self.window.len() < self.period {
            return None;
        }
        let n = self.period as f64;
        let mean = self.window.iter().sum::<f64>() / n;
        let var = self.window.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        let sd = var.sqrt();
        Some(BollingerPoint {
            middle: mean,
            upper: mean + self.k * sd,
            lower: mean - self.k * sd,
        })
    }
}

/// `BOLL(n, k)`; the usual setting is `(20, 2)`.
pub fn bollinger(values: &[f64], period: usize, k: f64) -> BollingerSeries {
    let mut b = Bollinger::new(period, k);
    let mut out = BollingerSeries::default();
    for &x in values {
        let p = b.update(x);
        out.middle.push(p.map_or_else(nan, |p| p.middle));
        out.upper.push(p.map_or_else(nan, |p| p.upper));
        out.lower.push(p.map_or_else(nan, |p| p.lower));
    }
    out
}

/// Streaming Wilder ATR.
///
/// True range is `max(h - l, |h - prev_close|, |l - prev_close|)`; the first
/// bar has no previous close, so its range is `h - l`. The first ATR is the
/// mean of the first `n` true ranges, then `(prev * (n - 1) + tr) / n`.
#[derive(Debug, Clone)]
pub struct Atr {
    period: usize,
    prev_close: Option<f64>,
    seen: usize,
    sum: f64,
    value: f64,
}

impl Atr {
    pub fn new(period: usize) -> Self {
        Self {
            period: period.max(1),
            prev_close: None,
            seen: 0,
            sum: 0.0,
            value: f64::NAN,
        }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        let tr = match self.prev_close.replace(close) {
            Some(pc) => (high - low).max((high - pc).abs()).max((low - pc).abs()),
            None => high - low,
        };
        let n = self.period as f64;
        if self.seen < self.period {
            self.seen += 1;
            self.sum += tr;
            if self.seen < self.period {
                return None;
            }
            self.value = self.sum / n;
        } else {
            self.value = (self.value * (n - 1.0) + tr) / n;
        }
        Some(self.value)
    }
}

/// `ATR(n)` over parallel high, low and close columns (the shortest wins).
pub fn atr(high: &[f64], low: &[f64], close: &[f64], period: usize) -> Vec<f64> {
    let mut a = Atr::new(period);
    high.iter()
        .zip(low)
        .zip(close)
        .map(|((&h, &l), &c)| a.update(h, l, c).unwrap_or_else(nan))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bollinger_of_a_constant_collapses_to_the_mean() {
        let b = bollinger(&[10.0; 6], 3, 2.0);
        assert!(b.middle[1].is_nan());
        assert_eq!((b.middle[5], b.upper[5], b.lower[5]), (10.0, 10.0, 10.0));
    }

    #[test]
    fn bollinger_uses_the_population_deviation() {
        // Window 2, 4, 6: mean 4, population variance 8/3.
        let b = bollinger(&[2.0, 4.0, 6.0], 3, 1.0);
        let sd = (8.0f64 / 3.0).sqrt();
        assert!((b.upper[2] - (4.0 + sd)).abs() < 1e-12);
        assert!((b.lower[2] - (4.0 - sd)).abs() < 1e-12);
    }

    #[test]
    fn atr_handles_gaps_through_the_previous_close() {
        // Bar 2 gaps up: its true range is high - prev_close, not high - low.
        let out = atr(
            &[11.0, 20.0, 21.0],
            &[9.0, 18.0, 19.0],
            &[10.0, 19.0, 20.0],
            2,
        );
        assert!(out[0].is_nan());
        // TR = [2, max(2, 10, 8) = 10]; seed = 6; then (6 * 1 + max(2, 2, 0) = 2) / 2 = 4.
        assert_eq!(out[1], 6.0);
        assert_eq!(out[2], 4.0);
    }
}
