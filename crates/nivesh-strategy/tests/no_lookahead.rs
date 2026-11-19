//! The no-lookahead property test, per PLAN Part 5: perturbing future bars
//! never changes past decisions — for every template, not just the one
//! the unit test uses.

use nivesh_core::Candle;
use nivesh_strategy::{BacktestOptions, Template, TemplateParams, backtest};
use proptest::prelude::*;

fn series_from(closes: &[i32]) -> Vec<Candle> {
    closes
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let t = 1_764_000_000_000 + i64::try_from(i).unwrap() * 86_400_000;
            let close = f64::from(c) + 1_000.0;
            Candle {
                t,
                o: close,
                h: close + 1.0,
                l: close - 1.0,
                c: close,
                v: 1_000.0,
            }
        })
        .collect()
}

fn early_trades(
    result: &nivesh_strategy::BacktestResult,
    cut: usize,
) -> Vec<nivesh_strategy::BacktestTrade> {
    result
        .trades
        .iter()
        .filter(|t| t.exit_index < cut)
        .cloned()
        .collect()
}

proptest! {
    /// Perturbing bars after the cut must not change any trade that
    /// finished before the cut.
    #[test]
    fn perturbing_future_bars_never_changes_past_trades(
        closes in proptest::collection::vec(0..200i32, 120..=200),
        perturb_from in 60..100usize,
        bump in 0..3_000i32,
    ) {
        let base = series_from(&closes);
        let mut perturbed = base.clone();
        for bar in perturbed.iter_mut().skip(perturb_from) {
            bar.c += f64::from(bump);
            bar.o += f64::from(bump);
            bar.h += f64::from(bump);
            bar.l += f64::from(bump);
        }
        for template in Template::all() {
            let set = template.build(&TemplateParams::default());
            let mut progress = |_, _| true;
            let a = backtest(set.clone(), &base, BacktestOptions::default(), &mut progress);
            let b = backtest(set, &perturbed, BacktestOptions::default(), &mut progress);
            prop_assert_eq!(
                early_trades(&a, perturb_from),
                early_trades(&b, perturb_from)
            );
        }
    }
}
