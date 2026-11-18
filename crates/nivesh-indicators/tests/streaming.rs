//! Property tests: feeding a series through the streaming form one value at a
//! time gives the batch result, and batch results agree with naive formulas.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use nivesh_indicators::{
    Atr, Bollinger, Ema, Macd, Rsi, Sma, Vwap, Wma, atr, bollinger, ema, macd, rsi, sma, wma,
};
use proptest::prelude::*;

/// Prices in a realistic range. Relative tolerance, since values are large.
fn close(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

fn all_close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| close(*x, *y))
}

fn series() -> impl Strategy<Value = Vec<f64>> {
    prop::collection::vec(1.0f64..50_000.0, 0..300)
}

fn bars() -> impl Strategy<Value = Vec<(f64, f64, f64, f64)>> {
    // (low, range, close position, volume) -> consistent OHLC
    prop::collection::vec(
        (1.0f64..50_000.0, 0.0f64..500.0, 0.0f64..1.0, 0.0f64..1e6),
        0..300,
    )
}

proptest! {
    #[test]
    fn sma_streaming_equals_batch_and_the_naive_mean(v in series(), n in 1usize..40) {
        let batch = sma(&v, n);
        let mut s = Sma::new(n);
        let streamed: Vec<f64> = v.iter().map(|&x| s.update(x).unwrap_or(f64::NAN)).collect();
        prop_assert!(all_close(&batch, &streamed));
        let naive: Vec<f64> = (0..v.len())
            .map(|i| if i + 1 < n { f64::NAN } else { v[i + 1 - n..=i].iter().sum::<f64>() / n as f64 })
            .collect();
        prop_assert!(all_close(&batch, &naive));
    }

    #[test]
    fn ema_streaming_equals_batch(v in series(), n in 1usize..40) {
        let mut e = Ema::new(n);
        let streamed: Vec<f64> = v.iter().map(|&x| e.update(x).unwrap_or(f64::NAN)).collect();
        prop_assert!(all_close(&ema(&v, n), &streamed));
    }

    #[test]
    fn wma_streaming_equals_the_naive_definition(v in series(), n in 1usize..40) {
        let mut w = Wma::new(n);
        let streamed: Vec<f64> = v.iter().map(|&x| w.update(x).unwrap_or(f64::NAN)).collect();
        let d = (n * (n + 1)) as f64 / 2.0;
        let naive: Vec<f64> = (0..v.len())
            .map(|i| {
                if i + 1 < n { return f64::NAN; }
                v[i + 1 - n..=i].iter().enumerate().map(|(k, x)| (k + 1) as f64 * x).sum::<f64>() / d
            })
            .collect();
        prop_assert!(all_close(&streamed, &naive));
        prop_assert!(all_close(&wma(&v, n), &naive));
    }

    #[test]
    fn rsi_streaming_equals_batch_and_stays_in_range(v in series(), n in 1usize..30) {
        let mut r = Rsi::new(n);
        let streamed: Vec<f64> = v.iter().map(|&x| r.update(x).unwrap_or(f64::NAN)).collect();
        let batch = rsi(&v, n);
        prop_assert!(all_close(&batch, &streamed));
        prop_assert!(batch.iter().all(|x| x.is_nan() || (0.0..=100.0).contains(x)));
    }

    #[test]
    fn macd_streaming_equals_batch(v in series(), f in 2usize..15, extra in 1usize..20, g in 2usize..12) {
        let slow = f + extra;
        let mut m = Macd::new(f, slow, g);
        let (mut line, mut sig, mut hist) = (vec![], vec![], vec![]);
        for &x in &v {
            let p = m.update(x);
            line.push(p.macd); sig.push(p.signal); hist.push(p.hist);
        }
        let b = macd(&v, f, slow, g);
        prop_assert!(all_close(&b.macd, &line));
        prop_assert!(all_close(&b.signal, &sig));
        prop_assert!(all_close(&b.hist, &hist));
    }

    #[test]
    fn bollinger_streaming_equals_batch_and_bands_nest(v in series(), n in 1usize..40, k in 0.0f64..4.0) {
        let mut b = Bollinger::new(n, k);
        let batch = bollinger(&v, n, k);
        for (i, &x) in v.iter().enumerate() {
            match b.update(x) {
                Some(p) => {
                    prop_assert!(close(batch.middle[i], p.middle));
                    prop_assert!(close(batch.upper[i], p.upper));
                    prop_assert!(close(batch.lower[i], p.lower));
                    prop_assert!(p.lower <= p.middle && p.middle <= p.upper);
                }
                None => prop_assert!(batch.middle[i].is_nan()),
            }
        }
    }

    #[test]
    fn atr_and_vwap_streaming_equal_batch(b in bars(), n in 1usize..30) {
        let high: Vec<f64> = b.iter().map(|(l, r, _, _)| l + r).collect();
        let low: Vec<f64> = b.iter().map(|(l, ..)| *l).collect();
        let close_: Vec<f64> = b.iter().map(|(l, r, p, _)| l + r * p).collect();
        let vol: Vec<f64> = b.iter().map(|(.., v)| *v).collect();

        let mut a = Atr::new(n);
        let streamed: Vec<f64> = (0..b.len())
            .map(|i| a.update(high[i], low[i], close_[i]).unwrap_or(f64::NAN))
            .collect();
        let batch = atr(&high, &low, &close_, n);
        prop_assert!(all_close(&batch, &streamed));
        prop_assert!(batch.iter().all(|x| x.is_nan() || *x >= 0.0));

        // One session: VWAP stays inside the day's price range.
        let t: Vec<i64> = (0..b.len() as i64).map(|i| i * 60_000).collect();
        let vw = nivesh_indicators::vwap(&t, &high, &low, &close_, &vol, 0);
        let mut s = Vwap::new();
        for i in 0..b.len() {
            let want = s.update(high[i], low[i], close_[i], vol[i]).unwrap_or(f64::NAN);
            prop_assert!(close(vw[i], want));
            if want.is_finite() {
                let lo = low[..=i].iter().copied().fold(f64::INFINITY, f64::min);
                let hi = high[..=i].iter().copied().fold(f64::NEG_INFINITY, f64::max);
                prop_assert!(want >= lo - 1e-6 && want <= hi + 1e-6);
            }
        }
    }
}
