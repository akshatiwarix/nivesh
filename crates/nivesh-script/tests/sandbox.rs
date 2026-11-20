//! The sandbox holds: runaway loops, deep recursion and growing values are
//! stopped, nothing outside the script is reachable, and a working script
//! trades through the backtester like any strategy.

#[cfg(test)]
mod sandbox {
    use std::time::{Duration, Instant};

    use nivesh_core::Candle;
    use nivesh_script::{Script, ScriptError, ScriptLimits, validate};
    use nivesh_strategy::{BacktestOptions, CostModel, Decide, IntentSide, backtest_decider};

    const DAY: i64 = 86_400_000;

    fn bar(i: i64, c: f64) -> Candle {
        Candle {
            t: 1_764_000_000_000 + i * DAY,
            o: c,
            h: c + 1.0,
            l: c - 1.0,
            c,
            v: 1_000.0,
        }
    }

    fn run(source: &str) -> (Script, Result<Option<IntentSide>, ScriptError>) {
        let mut s = Script::new(source, ScriptLimits::default()).unwrap();
        let r = s.decide(&bar(0, 100.0));
        (s, r)
    }

    #[test]
    fn an_infinite_loop_is_stopped_by_the_operation_limit() {
        let started = Instant::now();
        let (_, r) = run("fn on_bar(bar) { loop { } }");
        assert_eq!(r, Err(ScriptError::TooManyOperations(200_000)));
        assert!(started.elapsed() < Duration::from_secs(1));
        let (_, r) = run("fn on_bar(bar) { let i = 0; while true { i += 1; } }");
        assert!(matches!(r, Err(ScriptError::TooManyOperations(_))));
    }

    #[test]
    fn the_time_limit_stops_what_the_operation_budget_allows() {
        let limits = ScriptLimits {
            max_operations: 0, // unlimited operations: only the clock stops it
            max_time: Duration::from_millis(20),
            ..ScriptLimits::default()
        };
        let mut s = Script::new("fn on_bar(bar) { loop { } }", limits).unwrap();
        let started = Instant::now();
        assert_eq!(s.decide(&bar(0, 1.0)), Err(ScriptError::TooSlow(20)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn recursion_and_growing_values_hit_their_caps() {
        let (_, r) = run("fn f(n) { f(n + 1) } fn on_bar(bar) { f(0) }");
        assert!(matches!(r, Err(ScriptError::Runtime { .. })), "{r:?}");
        let (_, r) = run(r#"fn on_bar(bar) { let s = "x"; loop { s += s; } }"#);
        assert!(matches!(r, Err(ScriptError::Runtime { .. })), "{r:?}");
        // Whichever cap comes first, it is stopped.
        let (_, r) = run("fn on_bar(bar) { let a = []; loop { a.push(1); } }");
        assert!(
            matches!(
                r,
                Err(ScriptError::Runtime { .. }
                    | ScriptError::TooManyOperations(_)
                    | ScriptError::TooSlow(_))
            ),
            "{r:?}"
        );
    }

    #[test]
    fn files_and_modules_cannot_be_reached() {
        // `import` does not exist in this build: no module can be loaded.
        for source in [
            r#"import "/etc/passwd" as p; fn on_bar(bar) { }"#,
            r#"fn on_bar(bar) { import "secrets" as s; }"#,
        ] {
            assert!(
                matches!(validate(source), Err(ScriptError::Compile { .. })),
                "{source}"
            );
        }
        // eval is switched off.
        assert!(matches!(
            validate(r#"fn on_bar(bar) { eval("1 + 1") }"#),
            Err(ScriptError::Compile { .. })
        ));
    }

    #[test]
    fn no_filesystem_network_process_or_clock_function_exists() {
        for call in [
            r#"open_file("/etc/passwd")"#,
            r#"read_file("/etc/passwd")"#,
            r#"write_file("x", "y")"#,
            r#"http_get("https://example.com")"#,
            r#"fetch("https://example.com")"#,
            r#"connect("127.0.0.1:80")"#,
            r#"exec("ls")"#,
            r#"system("ls")"#,
            "timestamp()",
            "env()",
        ] {
            let (_, r) = run(&format!("fn on_bar(bar) {{ {call} }}"));
            match r {
                Err(ScriptError::Runtime { message, .. }) => {
                    assert!(message.contains("not found"), "{call}: {message}")
                }
                Err(ScriptError::Compile { .. }) => {}
                other => panic!("{call} ran: {other:?}"),
            }
        }
    }

    #[test]
    fn a_script_must_define_on_bar_and_return_a_signal() {
        assert_eq!(validate("fn tick(bar) { }"), Err(ScriptError::MissingOnBar));
        assert!(matches!(
            validate("fn on_bar(bar) { "),
            Err(ScriptError::Compile { line: 1, .. })
        ));
        let (_, r) = run("fn on_bar(bar) { 42 }");
        assert!(matches!(r, Err(ScriptError::BadReturn(_))));
        let (_, r) = run("fn on_bar(bar) { buy(0) }");
        assert!(matches!(r, Err(ScriptError::BadReturn(_))));
        let (_, r) = run("fn on_bar(bar) { buy(3) }");
        assert_eq!(r, Ok(Some(IntentSide::Buy)));
    }

    #[test]
    fn an_error_decides_nothing_and_is_kept() {
        let mut s = Script::new("fn on_bar(bar) { loop { } }", ScriptLimits::default()).unwrap();
        assert_eq!(s.feed(&bar(0, 1.0)), None);
        assert!(matches!(
            s.last_error(),
            Some(ScriptError::TooManyOperations(_))
        ));
    }

    #[test]
    fn indicators_are_unit_until_warm_and_highest_excludes_this_bar() {
        let source = r#"
            fn on_bar(bar) {
                if bar_count() == 3 && sma(3) == 101.0 && highest(2) == 102.0 && lowest(2) == 99.0 {
                    return buy(1);
                }
                if sma(50) > 0.0 { return buy(99); }
            }
        "#;
        let mut s = Script::new(source, ScriptLimits::default()).unwrap();
        assert_eq!(s.decide(&bar(0, 100.0)), Ok(None));
        assert_eq!(s.decide(&bar(1, 101.0)), Ok(None));
        assert_eq!(s.decide(&bar(2, 102.0)), Ok(Some(IntentSide::Buy)));
    }

    #[test]
    fn a_script_backtests_like_a_strategy() {
        // Long while the close is above its 3-bar average.
        let source = r#"
            fn on_bar(bar) {
                let avg = sma(3);
                if !in_position() && bar.close > avg { return buy(2); }
                if in_position() && bar.close < avg { return sell(); }
            }
        "#;
        let closes = [
            100.0, 99.0, 98.0, 101.0, 104.0, 107.0, 103.0, 99.0, 97.0, 100.0,
        ];
        let bars: Vec<Candle> = (0..).zip(closes).map(|(i, c)| bar(i, c)).collect();
        let options = BacktestOptions {
            costs: CostModel::FREE,
            ..BacktestOptions::default()
        };
        let script = Script::new(source, ScriptLimits::default()).unwrap();
        let result = backtest_decider(Box::new(script), &bars, 0, options, &mut |_, _| true);
        assert_eq!(result.trades.len(), 1);
        let t = &result.trades[0];
        // Signal on bar 3's close (101 > 99.33), filled at bar 4's open;
        // exit signal on bar 6 (103 < 104.67), filled at bar 7's open.
        assert_eq!((t.quantity, t.entry_price, t.exit_price), (2, 104.0, 99.0));

        // The same script, the same bars: the same result.
        let again = Script::new(source, ScriptLimits::default()).unwrap();
        let repeat = backtest_decider(Box::new(again), &bars, 0, options, &mut |_, _| true);
        assert_eq!(repeat.trades, result.trades);
    }
}
