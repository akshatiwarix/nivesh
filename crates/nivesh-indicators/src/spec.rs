//! Indicator specifications: the text a user types (`SMA 50`, `MACD 12 26 9`)
//! parsed, validated and computed into drawable lines. This is the one place
//! that knows which indicator draws where, so the UI only renders.

use std::fmt;
use std::str::FromStr;

use crate::{IST_OFFSET_SECS, atr, bollinger, ema, macd, rsi, sma, vwap, wma};

/// Longest period accepted. Far beyond what any chart shows, but bounded so a
/// typo can't allocate or loop absurdly.
pub const MAX_PERIOD: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Sma,
    Ema,
    Wma,
    Rsi,
    Macd,
    Boll,
    Atr,
    Vwap,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Sma,
        Kind::Ema,
        Kind::Wma,
        Kind::Rsi,
        Kind::Macd,
        Kind::Boll,
        Kind::Atr,
        Kind::Vwap,
    ];

    pub const fn code(self) -> &'static str {
        match self {
            Kind::Sma => "SMA",
            Kind::Ema => "EMA",
            Kind::Wma => "WMA",
            Kind::Rsi => "RSI",
            Kind::Macd => "MACD",
            Kind::Boll => "BOLL",
            Kind::Atr => "ATR",
            Kind::Vwap => "VWAP",
        }
    }

    /// The parameters used when none are given.
    fn defaults(self) -> &'static [f64] {
        match self {
            Kind::Sma | Kind::Ema | Kind::Wma => &[20.0],
            Kind::Rsi | Kind::Atr => &[14.0],
            Kind::Macd => &[12.0, 26.0, 9.0],
            Kind::Boll => &[20.0, 2.0],
            Kind::Vwap => &[],
        }
    }

    pub fn placement(self) -> Placement {
        match self {
            Kind::Sma | Kind::Ema | Kind::Wma | Kind::Boll | Kind::Vwap => Placement::Overlay,
            Kind::Rsi | Kind::Macd | Kind::Atr => Placement::Pane,
        }
    }

    fn from_code(code: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.code().eq_ignore_ascii_case(code))
    }
}

/// Where an indicator is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// On the price pane.
    Overlay,
    /// In its own pane under the price.
    Pane,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorSpec {
    pub kind: Kind,
    pub params: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError(pub String);

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SpecError {}

impl IndicatorSpec {
    /// The canonical text, which parses back to the same spec: `SMA 50`,
    /// `BOLL 20 2`, `VWAP`. Used as the indicator's identity and storage form.
    pub fn id(&self) -> String {
        let mut out = self.kind.code().to_owned();
        for p in &self.params {
            out.push(' ');
            out.push_str(&number(*p));
        }
        out
    }
}

fn number(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{x:.0}")
    } else {
        format!("{x}")
    }
}

impl FromStr for IndicatorSpec {
    type Err = SpecError;

    /// `SMA 50`, `rsi`, `MACD 12 26 9`, `BOLL 20 2.5`. Missing parameters take
    /// the usual defaults; periods are whole numbers from 1 to 500.
    fn from_str(s: &str) -> Result<Self, SpecError> {
        let mut words = s.split_whitespace();
        let name = words
            .next()
            .ok_or_else(|| SpecError("name an indicator, e.g. SMA 50".into()))?;
        let kind = Kind::from_code(name).ok_or_else(|| {
            SpecError(format!(
                "unknown indicator `{name}` (use {})",
                Kind::ALL.map(Kind::code).join(", ")
            ))
        })?;
        let mut params = Vec::new();
        for w in words {
            let v: f64 = w
                .parse()
                .map_err(|_| SpecError(format!("`{w}` is not a number")))?;
            params.push(v);
        }
        let defaults = kind.defaults();
        if params.len() > defaults.len() {
            return Err(SpecError(format!(
                "{} takes at most {} parameter(s)",
                kind.code(),
                defaults.len()
            )));
        }
        // Fill the rest from the defaults, so `MACD 8` means `MACD 8 26 9`.
        params.extend_from_slice(&defaults[params.len()..]);
        validate(kind, &params)?;
        Ok(Self { kind, params })
    }
}

fn validate(kind: Kind, params: &[f64]) -> Result<(), SpecError> {
    let period_ok = |p: f64| p.fract() == 0.0 && (1.0..=f64::from(MAX_PERIOD)).contains(&p);
    let periods = match kind {
        Kind::Boll => &params[..1],
        _ => params,
    };
    if let Some(bad) = periods.iter().find(|p| !period_ok(**p)) {
        return Err(SpecError(format!(
            "periods are whole numbers from 1 to {MAX_PERIOD}, not {}",
            number(*bad)
        )));
    }
    if kind == Kind::Macd && params[0] >= params[1] {
        return Err(SpecError(
            "MACD needs a fast period shorter than the slow one".into(),
        ));
    }
    if kind == Kind::Boll && !(params[1] > 0.0 && params[1] <= 10.0) {
        return Err(SpecError(
            "Bollinger width must be above 0 and at most 10".into(),
        ));
    }
    Ok(())
}

/// Candle columns an indicator is computed from. All the same length.
#[derive(Debug, Clone, Copy)]
pub struct Ohlcv<'a> {
    /// Bar start, UTC epoch milliseconds.
    pub t: &'a [i64],
    pub h: &'a [f64],
    pub l: &'a [f64],
    pub c: &'a [f64],
    pub v: &'a [f64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    Line,
    Histogram,
}

/// One drawable series, aligned to the candles. `NaN` means "no value".
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub name: String,
    pub values: Vec<f64>,
    pub style: LineStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub id: String,
    pub kind: Kind,
    pub placement: Placement,
    pub lines: Vec<Line>,
    /// Fixed value range of the pane (RSI is always 0 to 100).
    pub bounds: Option<(f64, f64)>,
    /// Reference levels to draw (RSI 30 and 70).
    pub guides: Vec<f64>,
}

fn line(name: &str, values: Vec<f64>) -> Line {
    Line {
        name: name.to_owned(),
        values,
        style: LineStyle::Line,
    }
}

/// Computes `spec` over `data`. Cheap to call again on every bar update.
pub fn compute(spec: &IndicatorSpec, data: &Ohlcv<'_>) -> Output {
    let p = &spec.params;
    // Periods were validated to be whole numbers in 1..=500.
    let period = |i: usize| p.get(i).copied().unwrap_or(1.0) as usize;
    let mut bounds = None;
    let mut guides = Vec::new();
    let lines = match spec.kind {
        Kind::Sma => vec![line(&spec.id(), sma(data.c, period(0)))],
        Kind::Ema => vec![line(&spec.id(), ema(data.c, period(0)))],
        Kind::Wma => vec![line(&spec.id(), wma(data.c, period(0)))],
        Kind::Rsi => {
            bounds = Some((0.0, 100.0));
            guides = vec![30.0, 70.0];
            vec![line(&spec.id(), rsi(data.c, period(0)))]
        }
        Kind::Macd => {
            let m = macd(data.c, period(0), period(1), period(2));
            vec![
                Line {
                    name: "Hist".into(),
                    values: m.hist,
                    style: LineStyle::Histogram,
                },
                line("MACD", m.macd),
                line("Signal", m.signal),
            ]
        }
        Kind::Boll => {
            let b = bollinger(data.c, period(0), p.get(1).copied().unwrap_or(2.0));
            vec![
                line("Upper", b.upper),
                line(&spec.id(), b.middle),
                line("Lower", b.lower),
            ]
        }
        Kind::Atr => vec![line(&spec.id(), atr(data.h, data.l, data.c, period(0)))],
        Kind::Vwap => vec![line(
            "VWAP",
            vwap(data.t, data.h, data.l, data.c, data.v, IST_OFFSET_SECS),
        )],
    };
    Output {
        id: spec.id(),
        kind: spec.kind,
        placement: spec.kind.placement(),
        lines,
        bounds,
        guides,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(s: &str) -> IndicatorSpec {
        s.parse().unwrap()
    }

    #[test]
    fn parses_with_defaults_and_round_trips_through_its_id() {
        for (text, id) in [
            ("sma 50", "SMA 50"),
            ("SMA", "SMA 20"),
            ("rsi", "RSI 14"),
            ("MACD", "MACD 12 26 9"),
            ("macd 8", "MACD 8 26 9"),
            ("BOLL 20 2.5", "BOLL 20 2.5"),
            ("vwap", "VWAP"),
        ] {
            let s = spec(text);
            assert_eq!(s.id(), id);
            assert_eq!(spec(&s.id()), s, "{id} must parse back");
        }
    }

    #[test]
    fn rejects_bad_input_with_a_reason() {
        for (text, needle) in [
            ("", "name an indicator"),
            ("FOO 5", "unknown indicator"),
            ("SMA x", "not a number"),
            ("SMA 0", "1 to 500"),
            ("SMA 2.5", "whole numbers"),
            ("SMA 501", "1 to 500"),
            ("SMA 5 6", "at most 1"),
            ("MACD 26 12", "fast period"),
            ("BOLL 20 0", "width"),
            ("VWAP 5", "at most 0"),
            ("RSI NaN", "whole numbers"),
        ] {
            let err = text.parse::<IndicatorSpec>().unwrap_err().to_string();
            assert!(err.contains(needle), "{text:?}: {err}");
        }
    }

    fn data(n: usize) -> (Vec<i64>, Vec<f64>) {
        let t = (0..n as i64).map(|i| i * 300_000).collect();
        let c = (0..n).map(|i| 100.0 + (i as f64).sin() * 5.0).collect();
        (t, c)
    }

    #[test]
    fn computes_lines_aligned_to_the_candles_in_the_right_pane() {
        let (t, c) = data(60);
        let h: Vec<f64> = c.iter().map(|x| x + 1.0).collect();
        let l: Vec<f64> = c.iter().map(|x| x - 1.0).collect();
        let v = vec![10.0; 60];
        let d = Ohlcv {
            t: &t,
            h: &h,
            l: &l,
            c: &c,
            v: &v,
        };
        for text in [
            "SMA 5", "EMA 5", "WMA 5", "RSI", "MACD", "BOLL", "ATR", "VWAP",
        ] {
            let s = spec(text);
            let out = compute(&s, &d);
            assert_eq!(out.id, s.id());
            assert!(!out.lines.is_empty());
            assert!(out.lines.iter().all(|l| l.values.len() == 60), "{text}");
            assert!(
                out.lines
                    .iter()
                    .any(|l| l.values.iter().any(|x| x.is_finite())),
                "{text} should have values by bar 60"
            );
        }
        assert_eq!(compute(&spec("RSI"), &d).bounds, Some((0.0, 100.0)));
        assert_eq!(compute(&spec("RSI"), &d).guides, [30.0, 70.0]);
        assert_eq!(compute(&spec("SMA 5"), &d).placement, Placement::Overlay);
        assert_eq!(compute(&spec("MACD"), &d).placement, Placement::Pane);
        assert_eq!(
            compute(&spec("MACD"), &d).lines[0].style,
            LineStyle::Histogram
        );
    }

    #[test]
    fn the_sma_line_matches_the_batch_function() {
        let (t, c) = data(30);
        let v = vec![1.0; 30];
        let d = Ohlcv {
            t: &t,
            h: &c,
            l: &c,
            c: &c,
            v: &v,
        };
        let out = compute(&spec("SMA 7"), &d);
        let want = sma(&c, 7);
        for (a, b) in out.lines[0].values.iter().zip(&want) {
            assert!(a.to_bits() == b.to_bits() || (a - b).abs() < 1e-12);
        }
    }
}
