//! Indicators against `fixtures/reference.json`, produced by independent
//! plain-Python formulas (see `fixtures/gen_reference.py`).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::Value;

/// Max relative error against the Python reference. Both sides are IEEE
/// doubles doing the same arithmetic in a different order, so the gap is a few
/// ulps; this leaves headroom without hiding a wrong formula.
pub const TOLERANCE: f64 = 1e-9;

pub struct Reference(Value);

impl Reference {
    pub fn load() -> Self {
        let text = include_str!("fixtures/reference.json");
        Self(serde_json::from_str(text).expect("reference.json is valid"))
    }

    pub fn column(&self, key: &str) -> Vec<f64> {
        self.0[key]
            .as_array()
            .unwrap_or_else(|| panic!("missing column {key}"))
            .iter()
            .map(|v| v.as_f64().unwrap_or(f64::NAN))
            .collect()
    }
}

pub fn assert_matches(name: &str, got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{name}: length");
    let mut compared = 0;
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert_eq!(
            g.is_nan(),
            w.is_nan(),
            "{name}[{i}]: warm-up differs ({g} vs {w})"
        );
        if w.is_nan() {
            continue;
        }
        compared += 1;
        let err = (g - w).abs() / w.abs().max(1.0);
        assert!(err <= TOLERANCE, "{name}[{i}]: {g} vs {w} (err {err:e})");
    }
    assert!(compared > 0, "{name}: nothing compared");
}

#[test]
fn moving_averages_match_the_reference() {
    use nivesh_indicators::{ema, sma, wma};
    let r = Reference::load();
    let c = r.column("c");
    for (period, suffix) in [(5, "5"), (20, "20")] {
        assert_matches(
            &format!("sma_{suffix}"),
            &sma(&c, period),
            &r.column(&format!("sma_{suffix}")),
        );
        assert_matches(
            &format!("ema_{suffix}"),
            &ema(&c, period),
            &r.column(&format!("ema_{suffix}")),
        );
        assert_matches(
            &format!("wma_{suffix}"),
            &wma(&c, period),
            &r.column(&format!("wma_{suffix}")),
        );
    }
}

#[test]
fn rsi_matches_the_reference() {
    use nivesh_indicators::rsi;
    let r = Reference::load();
    let c = r.column("c");
    assert_matches("rsi_14", &rsi(&c, 14), &r.column("rsi_14"));
    assert_matches("rsi_5", &rsi(&c, 5), &r.column("rsi_5"));
}

#[test]
fn macd_matches_the_reference() {
    use nivesh_indicators::macd;
    let r = Reference::load();
    let m = macd(&r.column("c"), 12, 26, 9);
    assert_matches("macd", &m.macd, &r.column("macd_line"));
    assert_matches("signal", &m.signal, &r.column("macd_signal"));
    assert_matches("hist", &m.hist, &r.column("macd_hist"));
}

#[test]
fn bollinger_matches_the_reference() {
    use nivesh_indicators::bollinger;
    let r = Reference::load();
    let b = bollinger(&r.column("c"), 20, 2.0);
    assert_matches("middle", &b.middle, &r.column("boll_mid"));
    assert_matches("upper", &b.upper, &r.column("boll_up"));
    assert_matches("lower", &b.lower, &r.column("boll_lo"));
}

#[test]
fn atr_matches_the_reference() {
    use nivesh_indicators::atr;
    let r = Reference::load();
    let out = atr(&r.column("h"), &r.column("l"), &r.column("c"), 14);
    assert_matches("atr_14", &out, &r.column("atr_14"));
}

#[test]
fn session_vwap_matches_the_reference() {
    use nivesh_indicators::{IST_OFFSET_SECS, vwap};
    let r = Reference::load();
    let t: Vec<i64> = r.column("t").iter().map(|x| *x as i64).collect();
    let out = vwap(
        &t,
        &r.column("h"),
        &r.column("l"),
        &r.column("c"),
        &r.column("v"),
        IST_OFFSET_SECS,
    );
    assert_matches("vwap", &out, &r.column("vwap"));
}
