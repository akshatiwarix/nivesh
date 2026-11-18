//! Session-aligned resampling of 100,000 one-minute candles.
//! Run with `cargo bench -p nivesh-core`; budgets are in the README.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nivesh_core::bars::{normalize, resample};
use nivesh_core::{Candle, Interval};

const N: usize = 100_000;
const SESSION: usize = 375;

fn candles() -> Vec<Candle> {
    // 2026-11-16 09:15 IST, one-minute bars, session after session (weekends
    // included: the bucketing does not care).
    let open = (1_794_767_400_i64 + 9 * 3600 + 15 * 60) * 1000;
    (0..N)
        .map(|i| {
            let day = (i / SESSION) as i64;
            let minute = (i % SESSION) as i64;
            let p = 1000.0 + (i as f64 * 0.37).sin() * 20.0;
            Candle {
                t: open + day * 86_400_000 + minute * 60_000,
                o: p,
                h: p + 1.0,
                l: p - 1.0,
                c: p + 0.5,
                v: 100.0,
            }
        })
        .collect()
}

fn bench(cr: &mut Criterion) {
    let c = candles();
    let mut g = cr.benchmark_group("resample_100k_1m");
    for to in [
        Interval::M5,
        Interval::H1,
        Interval::H4,
        Interval::D1,
        Interval::W1,
        Interval::Mo1,
    ] {
        g.bench_function(to.code(), |b| b.iter(|| resample(black_box(&c), to)));
    }
    // What every provider series goes through before it is served.
    g.bench_function("normalize", |b| b.iter(|| normalize(black_box(c.clone()))));
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
