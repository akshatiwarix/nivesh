//! Paper trades like the backtest: the same closed bars driven through the
//! paper desk (runner → gateway → paper fill at the next open) produce
//! exactly the backtester's trades, for every template, with stops and
//! targets on, and with the same slippage and charges.

#[cfg(test)]
mod parity {
    use nivesh_core::{Candle, Side};
    use nivesh_exec::PaperDesk;
    use nivesh_risk::{RiskEngine, RiskLimits};
    use nivesh_strategy::{BacktestOptions, PaperRunner, Template, TemplateParams, backtest};
    use proptest::prelude::*;

    const DAY: i64 = 86_400_000;

    fn series_from(steps: &[i32]) -> Vec<Candle> {
        let mut close = 1_000.0;
        steps
            .iter()
            .enumerate()
            .map(|(i, &step)| {
                let open = close;
                close = (close + f64::from(step)).max(10.0);
                Candle {
                    t: 1_764_000_000_000 + i64::try_from(i).unwrap() * DAY,
                    o: open,
                    h: open.max(close) + 2.0,
                    l: open.min(close) - 2.0,
                    c: close,
                    v: 1_000.0,
                }
            })
            .collect()
    }

    /// (entry t, entry price, exit t, exit price, quantity, net P&L) of
    /// each closed round trip. Prices are after slippage; the P&L is net
    /// of both fills' charges.
    type Trip = (i64, f64, i64, f64, i64, f64);

    /// The paper round trips, and whether a position is still open.
    fn paper_trips(
        template: Template,
        params: &TemplateParams,
        bars: &[Candle],
    ) -> (Vec<Trip>, bool) {
        let mut desk = PaperDesk::new(RiskEngine::new(RiskLimits::default()));
        let runner = PaperRunner::new(template.build(params), "NSE:RELIANCE", 100_000.0).unwrap();
        desk.start(1, runner);
        let mut trips = Vec::new();
        let mut open: Option<(i64, f64)> = None;
        for bar in bars {
            let report = desk.on_closed_bar(1, *bar, bar.t).unwrap();
            assert!(report.refused.is_empty(), "{:?}", report.refused);
            for fill in &report.fills {
                match fill.side {
                    Side::Buy => open = Some((fill.bar_t, fill.price)),
                    Side::Sell => {
                        let (et, ep) = open.take().unwrap();
                        let pnl = report
                            .state
                            .fills
                            .iter()
                            .rev()
                            .find(|f| f.t == fill.bar_t)
                            .and_then(|f| f.pnl)
                            .unwrap();
                        trips.push((et, ep, fill.bar_t, fill.price, fill.quantity, pnl));
                    }
                }
            }
        }
        (trips, open.is_some())
    }

    fn backtest_trips(template: Template, params: &TemplateParams, bars: &[Candle]) -> Vec<Trip> {
        let mut progress = |_, _| true;
        let result = backtest(
            template.build(params),
            bars,
            BacktestOptions::default(),
            &mut progress,
        );
        result
            .trades
            .iter()
            .map(|t| {
                (
                    t.entry_t,
                    t.entry_price,
                    t.exit_t,
                    t.exit_price,
                    t.quantity,
                    t.pnl,
                )
            })
            .collect()
    }

    /// The property is only worth something if trades happen. Two regimes
    /// make every template trade: long trendless swings (MA crosses and
    /// breakouts), then sharp drops (RSI under 30). Paper and backtest
    /// still agree.
    #[test]
    fn a_two_regime_series_trades_every_template_identically() {
        let steps: Vec<i32> = (0..600)
            .map(|i| match i {
                0..300 if i % 45 < 30 => 5,
                0..300 => -10,
                _ if i % 28 < 20 => 3,
                _ => -12,
            })
            .collect();
        let bars = series_from(&steps);
        for template in Template::all() {
            let params = TemplateParams {
                stop_loss: Some(0.03),
                target: Some(0.06),
                ..TemplateParams::default()
            };
            let (paper, holding) = paper_trips(template, &params, &bars);
            let mut expected = backtest_trips(template, &params, &bars);
            if holding {
                expected.pop();
            }
            assert!(!paper.is_empty(), "{template:?} never traded");
            assert_eq!(paper, expected, "{template:?}");
        }
    }

    proptest! {
        #[test]
        fn paper_and_backtest_make_the_same_trades(
            steps in proptest::collection::vec(-25..=25i32, 80..=200),
            stop in proptest::option::of(1..8u8),
            target in proptest::option::of(1..12u8),
        ) {
            let bars = series_from(&steps);
            for template in Template::all() {
                let params = TemplateParams {
                    stop_loss: stop.map(|s| f64::from(s) / 100.0),
                    target: target.map(|t| f64::from(t) / 100.0),
                    ..TemplateParams::default()
                };
                let (paper, holding) = paper_trips(template, &params, &bars);
                let mut expected = backtest_trips(template, &params, &bars);
                if holding {
                    // The backtester force-closes an open position at the
                    // last close; a running paper strategy keeps holding it.
                    expected.pop();
                }
                prop_assert_eq!(paper, expected, "template {:?}", template);
            }
        }
    }
}
