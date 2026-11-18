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
