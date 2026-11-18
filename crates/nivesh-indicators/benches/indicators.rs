//! Indicators over 100,000 candles (about 270 sessions of 1-minute bars).
//! Run with `cargo bench -p nivesh-indicators`; budgets are in the README.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nivesh_indicators::{IST_OFFSET_SECS, atr, bollinger, ema, macd, rsi, sma, vwap, wma};

const N: usize = 100_000;
/// 375 one-minute bars from 09:15 to 15:30.
const SESSION: usize = 375;

struct Series {
    t: Vec<i64>,
    h: Vec<f64>,
    l: Vec<f64>,
    c: Vec<f64>,
    v: Vec<f64>,
}

fn series() -> Series {
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        // xorshift: deterministic, no dependency.
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    };
    let (mut t, mut h, mut l, mut c, mut v) = (vec![], vec![], vec![], vec![], vec![]);
    let mut price = 1000.0;
    // 2026-11-16 09:15 IST.
    let open = 1_794_767_400_i64 + 9 * 3600 + 15 * 60;
    for i in 0..N {
        let day = (i / SESSION) as i64;
        let minute = (i % SESSION) as i64;
        t.push((open + day * 86_400 + minute * 60) * 1000);
        let close = (price + next() * 4.0_f64).max(1.0);
        h.push(price.max(close) + next().abs());
        l.push(price.min(close) - next().abs());
        c.push(close);
        v.push(1000.0 + next().abs() * 1e5);
        price = close;
    }
    Series { t, h, l, c, v }
}

fn bench(cr: &mut Criterion) {
    let s = series();
    let mut g = cr.benchmark_group("indicators_100k");
    g.bench_function("sma_20", |b| b.iter(|| sma(black_box(&s.c), 20)));
    g.bench_function("ema_20", |b| b.iter(|| ema(black_box(&s.c), 20)));
    g.bench_function("wma_20", |b| b.iter(|| wma(black_box(&s.c), 20)));
    g.bench_function("rsi_14", |b| b.iter(|| rsi(black_box(&s.c), 14)));
    g.bench_function("macd_12_26_9", |b| {
        b.iter(|| macd(black_box(&s.c), 12, 26, 9))
    });
    g.bench_function("bollinger_20_2", |b| {
        b.iter(|| bollinger(black_box(&s.c), 20, 2.0))
    });
    g.bench_function("atr_14", |b| {
        b.iter(|| atr(black_box(&s.h), black_box(&s.l), black_box(&s.c), 14));
    });
    g.bench_function("vwap_session", |b| {
        b.iter(|| {
            vwap(
                black_box(&s.t),
                black_box(&s.h),
                black_box(&s.l),
                black_box(&s.c),
                black_box(&s.v),
                IST_OFFSET_SECS,
            )
        });
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
