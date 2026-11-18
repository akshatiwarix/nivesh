//! Simple, exponential and weighted moving averages.

use std::collections::VecDeque;

use crate::nan;

/// Streaming simple moving average over the last `period` values.
#[derive(Debug, Clone)]
pub struct Sma {
    period: usize,
    window: VecDeque<f64>,
    sum: f64,
}

impl Sma {
    /// A period of 0 is treated as 1.
    pub fn new(period: usize) -> Self {
        let period = period.max(1);
        Self {
            period,
            window: VecDeque::with_capacity(period),
            sum: 0.0,
        }
    }

    pub fn update(&mut self, x: f64) -> Option<f64> {
        self.window.push_back(x);
        self.sum += x;
        if self.window.len() > self.period
            && let Some(old) = self.window.pop_front()
        {
            self.sum -= old;
        }
        (self.window.len() == self.period).then(|| self.sum / self.period as f64)
    }
}

/// `SMA(n)`: the mean of the last `n` values. `NaN` for the first `n - 1`.
pub fn sma(values: &[f64], period: usize) -> Vec<f64> {
    let mut s = Sma::new(period);
    values
        .iter()
        .map(|&x| s.update(x).unwrap_or_else(nan))
        .collect()
}

/// Streaming exponential moving average, `alpha = 2 / (n + 1)`, seeded with
/// the SMA of the first `n` values.
#[derive(Debug, Clone)]
pub struct Ema {
    period: usize,
    alpha: f64,
    seen: usize,
    seed_sum: f64,
    value: f64,
}

impl Ema {
    pub fn new(period: usize) -> Self {
        let period = period.max(1);
        Self {
            period,
            alpha: 2.0 / (period as f64 + 1.0),
            seen: 0,
            seed_sum: 0.0,
            value: f64::NAN,
        }
    }

    pub fn update(&mut self, x: f64) -> Option<f64> {
        if self.seen < self.period {
            self.seen += 1;
            self.seed_sum += x;
            if self.seen == self.period {
                self.value = self.seed_sum / self.period as f64;
                return Some(self.value);
            }
            return None;
        }
        self.value += self.alpha * (x - self.value);
        Some(self.value)
    }
}

/// `EMA(n)`, seeded with the SMA of the first `n` values. `NaN` before that.
pub fn ema(values: &[f64], period: usize) -> Vec<f64> {
    let mut e = Ema::new(period);
    values
        .iter()
        .map(|&x| e.update(x).unwrap_or_else(nan))
        .collect()
}

/// Streaming linearly weighted moving average: the newest value has weight
/// `n`, the oldest weight 1. Updates in O(1).
#[derive(Debug, Clone)]
pub struct Wma {
    period: usize,
    window: VecDeque<f64>,
    /// Sum of the window, and of value * weight.
    sum: f64,
    weighted: f64,
}

impl Wma {
    pub fn new(period: usize) -> Self {
        let period = period.max(1);
        Self {
            period,
            window: VecDeque::with_capacity(period),
            sum: 0.0,
            weighted: 0.0,
        }
    }

    pub fn update(&mut self, x: f64) -> Option<f64> {
        if self.window.len() == self.period {
            // Every weight drops by one (the oldest value's becomes zero and
            // it leaves), and the new value takes the top weight.
            let oldest = self.window.pop_front().unwrap_or_default();
            self.weighted += self.period as f64 * x - self.sum;
            self.sum += x - oldest;
        } else {
            self.sum += x;
            self.weighted += x * (self.window.len() + 1) as f64;
        }
        self.window.push_back(x);
        (self.window.len() == self.period).then(|| self.weighted / Self::divisor(self.period))
    }

    fn divisor(n: usize) -> f64 {
        (n * (n + 1)) as f64 / 2.0
    }
}

/// `WMA(n)`: weights 1..=n, newest heaviest. `NaN` for the first `n - 1`.
pub fn wma(values: &[f64], period: usize) -> Vec<f64> {
    let mut w = Wma::new(period);
    values
        .iter()
        .map(|&x| w.update(x).unwrap_or_else(nan))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn sma_matches_hand_computation() {
        let out = sma(&[1.0, 2.0, 3.0, 4.0, 5.0], 3);
        assert!(out[0].is_nan() && out[1].is_nan());
        assert_eq!(&out[2..], &[2.0, 3.0, 4.0]);
    }

    #[test]
    fn ema_is_seeded_with_the_sma() {
        // alpha = 0.5 for period 3: seed (1+2+3)/3 = 2, then 2 + .5*(4-2) = 3.
        let out = ema(&[1.0, 2.0, 3.0, 4.0, 6.0], 3);
        assert!(out[1].is_nan());
        assert_eq!(out[2], 2.0);
        assert_eq!(out[3], 3.0);
        assert_eq!(out[4], 4.5);
    }

    #[test]
    fn wma_weights_the_newest_heaviest() {
        // (1*1 + 2*2 + 3*3) / 6 = 14/6; then (2*1 + 3*2 + 4*3) / 6 = 20/6.
        let out = wma(&[1.0, 2.0, 3.0, 4.0], 3);
        assert!(close(out[2], 14.0 / 6.0));
        assert!(close(out[3], 20.0 / 6.0));
    }

    #[test]
    fn period_one_is_the_input_and_zero_is_one() {
        let v = [3.0, 1.0, 4.0];
        assert_eq!(sma(&v, 1), v);
        assert_eq!(ema(&v, 1), v);
        assert_eq!(wma(&v, 1), v);
        assert_eq!(sma(&v, 0), v);
    }

    #[test]
    fn shorter_than_period_is_all_nan() {
        for out in [
            sma(&[1.0, 2.0], 5),
            ema(&[1.0, 2.0], 5),
            wma(&[1.0, 2.0], 5),
        ] {
            assert!(out.iter().all(|x| x.is_nan()));
        }
        assert!(sma(&[], 3).is_empty());
    }

    #[test]
    fn streaming_returns_none_during_warm_up() {
        let mut s = Sma::new(2);
        assert_eq!(s.update(1.0), None);
        assert_eq!(s.update(3.0), Some(2.0));
        assert_eq!(s.update(5.0), Some(4.0));
    }
}
