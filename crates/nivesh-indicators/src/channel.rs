//! Price channels: the highest high and lowest low of the previous `n` bars.
//!
//! The current bar is **not** in its own window, so `close > HIGHEST(n)`
//! reads "the close broke above the last `n` bars' high", the classic N-bar
//! breakout (Donchian). The first value appears once `n` earlier bars exist:
//! batch output is `NaN` for the first `n` points, streaming returns `None`.

use std::collections::VecDeque;

use crate::nan;

/// Which extreme the channel follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    High,
    Low,
}

/// Streaming rolling extreme over the previous `n` values.
#[derive(Debug, Clone)]
struct Extreme {
    side: Side,
    period: usize,
    window: VecDeque<f64>,
}

impl Extreme {
    fn new(side: Side, period: usize) -> Self {
        let period = period.max(1);
        Self {
            side,
            period,
            window: VecDeque::with_capacity(period + 1),
        }
    }

    /// The extreme of the previous `n` values, then `value` joins the window.
    fn update(&mut self, value: f64) -> Option<f64> {
        let out = (self.window.len() == self.period).then(|| {
            let it = self.window.iter().copied();
            match self.side {
                Side::High => it.fold(f64::NEG_INFINITY, f64::max),
                Side::Low => it.fold(f64::INFINITY, f64::min),
            }
        });
        self.window.push_back(value);
        if self.window.len() > self.period {
            self.window.pop_front();
        }
        out
    }
}

/// Streaming highest high of the previous `n` bars.
#[derive(Debug, Clone)]
pub struct Highest(Extreme);

impl Highest {
    pub fn new(period: usize) -> Self {
        Self(Extreme::new(Side::High, period))
    }

    pub fn update(&mut self, high: f64) -> Option<f64> {
        self.0.update(high)
    }
}

/// Streaming lowest low of the previous `n` bars.
#[derive(Debug, Clone)]
pub struct Lowest(Extreme);

impl Lowest {
    pub fn new(period: usize) -> Self {
        Self(Extreme::new(Side::Low, period))
    }

    pub fn update(&mut self, low: f64) -> Option<f64> {
        self.0.update(low)
    }
}

fn batch(values: &[f64], mut stream: impl FnMut(f64) -> Option<f64>) -> Vec<f64> {
    values
        .iter()
        .map(|v| stream(*v).unwrap_or_else(nan))
        .collect()
}

/// `HIGHEST(n)` over a high column.
pub fn highest(high: &[f64], period: usize) -> Vec<f64> {
    let mut s = Highest::new(period);
    batch(high, |v| s.update(v))
}

/// `LOWEST(n)` over a low column.
pub fn lowest(low: &[f64], period: usize) -> Vec<f64> {
    let mut s = Lowest::new(period);
    batch(low, |v| s.update(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_excludes_the_current_bar() {
        let highs = [1.0, 5.0, 3.0, 2.0, 4.0, 9.0];
        let out = highest(&highs, 3);
        assert!(out[..3].iter().all(|v| v.is_nan()), "warm-up");
        // Bar 3 sees bars 0..3: max(1, 5, 3) = 5; bar 5 sees 2..5: max(3, 2, 4).
        assert_eq!(&out[3..], &[5.0, 5.0, 4.0]);
        let lows = [5.0, 1.0, 3.0, 4.0, 2.0];
        assert_eq!(&lowest(&lows, 2)[2..], &[1.0, 1.0, 3.0]);
    }

    #[test]
    fn streaming_and_batch_agree() {
        let data: Vec<f64> = (0..200).map(|i| ((i * 37) % 101) as f64).collect();
        let batch = highest(&data, 20);
        let mut s = Highest::new(20);
        for (i, v) in data.iter().enumerate() {
            match s.update(*v) {
                Some(x) => assert_eq!(x, batch[i]),
                None => assert!(batch[i].is_nan()),
            }
        }
    }
}
