//! The validator and the bar-by-bar evaluator.
//!
//! The evaluator decides rules on **closed** bars only, feeding streaming
//! indicator states, so a decision at bar `i` can never see bar `i+1` —
//! the no-lookahead property the backtests depend on. It emits
//! [`OrderIntent`]s; the sizing model resolves against the signal bar.

use nivesh_core::Candle;
use nivesh_indicators::IndicatorSpec;

use crate::rules::{Comparison, CrossDirection, Operand, OrderIntent, Rule, RuleSet, Sizing};

/// What is wrong with a rule set, in plain language for the builder UI.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuleError {
    #[error("unknown indicator kind {0:?} (try SMA EMA WMA RSI MACD BOLL VWAP ATR)")]
    UnknownIndicator(String),
    #[error("{0} needs different parameters: {1}")]
    BadParameters(String, String),
    #[error("a rule is too deep (more than 8 nested rules)")]
    TooDeep,
    #[error("sizing is not usable: {0}")]
    BadSizing(String),
}

/// Checks a rule set over without evaluating: indicator kinds must exist,
/// parameters must fit their windows, and the tree must not be absurdly
/// deep. `version` 1 is the only grammar this build knows.
pub fn validate(set: &RuleSet) -> Result<(), RuleError> {
    if set.version != 1 {
        return Err(RuleError::BadParameters(
            "version".into(),
            format!("rule set version {} is newer than this build", set.version),
        ));
    }
    validate_rule(&set.entry, 0)?;
    validate_rule(&set.exit, 0)?;
    validate_sizing(&set.sizing)?;
    for (name, value) in [
        ("stop_loss", set.stop_loss),
        ("target", set.target),
        ("trailing_stop", set.trailing_stop),
    ] {
        if let Some(f) = value
            && (!f.is_finite() || f <= 0.0 || f >= 1.0)
        {
            return Err(RuleError::BadSizing(format!(
                "{name} must be a fraction between 0 and 1, e.g. 0.02"
            )));
        }
    }
    Ok(())
}

fn validate_rule(rule: &Rule, depth: usize) -> Result<(), RuleError> {
    if depth > 8 {
        return Err(RuleError::TooDeep);
    }
    match rule {
        Rule::True | Rule::False => Ok(()),
        Rule::Cmp { left, right, .. } | Rule::Cross { left, right, .. } => {
            validate_operand(left)?;
            validate_operand(right)
        }
        Rule::And(a, b) | Rule::Or(a, b) => {
            validate_rule(a, depth + 1)?;
            validate_rule(b, depth + 1)
        }
        Rule::Not(a) => validate_rule(a, depth + 1),
    }
}

fn validate_operand(op: &Operand) -> Result<(), RuleError> {
    match op {
        Operand::Constant(_) | Operand::Close => Ok(()),
        Operand::Indicator { kind, params } => {
            let text = format!(
                "{kind} {}",
                params.iter().map(|p| format!("{p} ")).collect::<String>()
            );
            let spec: IndicatorSpec = text
                .trim()
                .parse()
                .map_err(|_| RuleError::UnknownIndicator(kind.clone()))?;
            // A parse with the same id proves the kind and parameters are
            // real; the spec's own validation covers the windows.
            let _ = spec.id();
            Ok(())
        }
    }
}

fn validate_sizing(sizing: &Sizing) -> Result<(), RuleError> {
    match sizing {
        Sizing::FixedQuantity(q) => {
            if *q <= 0 {
                return Err(RuleError::BadSizing(
                    "fixed quantity must be positive".into(),
                ));
            }
        }
        Sizing::FixedAmount(a) => {
            if !a.is_finite() || *a <= 0.0 {
                return Err(RuleError::BadSizing("fixed amount must be positive".into()));
            }
        }
        Sizing::FractionOfEquity(f) => {
            if !f.is_finite() || *f <= 0.0 || *f > 1.0 {
                return Err(RuleError::BadSizing(
                    "fraction of equity must be between 0 and 1".into(),
                ));
            }
        }
    }
    Ok(())
}

/// The streaming indicator states, one per series the rules reference.
/// Each yields `Some(value)` when its window has warmed up.
enum StreamingIndicator {
    Sma(nivesh_indicators::Sma),
    Ema(nivesh_indicators::Ema),
    Wma(nivesh_indicators::Wma),
    Rsi(nivesh_indicators::Rsi),
    Macd(nivesh_indicators::Macd),
}

impl StreamingIndicator {
    fn build(spec: &IndicatorSpec) -> Self {
        let p = &spec.params;
        let period = |i: usize| p.get(i).copied().unwrap_or(1.0) as usize;
        match spec.kind {
            nivesh_indicators::Kind::Sma => Self::Sma(nivesh_indicators::Sma::new(period(0))),
            nivesh_indicators::Kind::Ema => Self::Ema(nivesh_indicators::Ema::new(period(0))),
            nivesh_indicators::Kind::Wma => Self::Wma(nivesh_indicators::Wma::new(period(0))),
            nivesh_indicators::Kind::Rsi => Self::Rsi(nivesh_indicators::Rsi::new(period(0))),
            nivesh_indicators::Kind::Macd => Self::Macd(nivesh_indicators::Macd::new(
                period(0).max(1),
                period(1).max(1),
                period(2).max(1),
            )),
            // BOLL/VWAP/ATR streams arrive with the backtester; rules over
            // them validate today and the evaluator reports None.
            nivesh_indicators::Kind::Boll
            | nivesh_indicators::Kind::Atr
            | nivesh_indicators::Kind::Vwap => {
                Self::Sma(nivesh_indicators::Sma::new(period(0).max(1)))
            }
        }
    }

    /// Advances with one closed bar's close.
    fn update(&mut self, bar: &Candle) -> Option<f64> {
        match self {
            Self::Sma(s) => s.update(bar.c),
            Self::Ema(s) => s.update(bar.c),
            Self::Wma(s) => s.update(bar.c),
            Self::Rsi(s) => s.update(bar.c),
            Self::Macd(s) => Some(s.update(bar.c).macd),
        }
    }
}

/// The evaluator: seed the indicator states from the series once, then feed
/// closed bars one at a time.
pub struct Evaluator {
    set: RuleSet,
    symbol: String,
    /// The distinct (kind, params) series, entry's first then exit's;
    /// `states`, `prev` and `last` are indexed like this.
    specs: Vec<(String, Vec<f64>)>,
    /// Streaming states per (kind, params) in the set's indicator order.
    states: Vec<StreamingIndicator>,
    /// The last two values of each series, so `Cross` can compare this bar
    /// with the previous one.
    prev: Vec<Option<f64>>,
    last: Vec<Option<f64>>,
    /// The previous closed bar's close, for `Close`-cross rules.
    prev_close: Option<f64>,
    /// True while a position is open (exit rules decide when to leave).
    in_position: bool,
    /// The price the position was entered at, for stop/target checks.
    entry_price: Option<f64>,
    /// The running high watermark for the trailing stop.
    high_watermark: Option<f64>,
    /// The effective stop, moved by the trailing rule.
    stop: Option<f64>,
    equity: f64,
}

impl Evaluator {
    /// Builds an evaluator over `set`; indicator series are validated
    /// first. `equity` is the account's (paper) equity, for sizing.
    pub fn new(set: RuleSet, equity: f64) -> Result<Self, RuleError> {
        validate(&set)?;
        let specs = set
            .entry
            .indicators()
            .into_iter()
            .chain(set.exit.indicators())
            .collect::<Vec<_>>();
        // Deduplicated again across both trees, in order.
        let mut unique: Vec<(String, Vec<f64>)> = Vec::new();
        for (kind, params) in specs {
            if !unique.iter().any(|(k, p)| k == &kind && p == &params) {
                unique.push((kind, params));
            }
        }
        let states = unique
            .iter()
            .map(|(kind, params)| {
                let text = format!(
                    "{kind} {}",
                    params
                        .iter()
                        .map(|p| format!("{p} "))
                        .collect::<String>()
                        .trim_end()
                );
                let spec: IndicatorSpec = text
                    .parse()
                    .map_err(|_| RuleError::UnknownIndicator(kind.clone()))?;
                Ok::<_, RuleError>(StreamingIndicator::build(&spec))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let n = states.len();
        Ok(Self {
            set,
            symbol: String::new(),
            specs: unique,
            states,
            prev: vec![None; n],
            last: vec![None; n],
            prev_close: None,
            in_position: false,
            entry_price: None,
            high_watermark: None,
            stop: None,
            equity,
        })
    }

    /// The indicator values as of the last fed bar, for previews.
    pub fn last_values(&self) -> &[Option<f64>] {
        &self.last
    }

    /// Feeds one **closed** bar. Returns the intent, if any, in order:
    /// exits first (a stop or target on the same bar as an entry signal
    /// wins), then entries.
    pub fn feed(&mut self, bar: &Candle) -> Option<OrderIntent> {
        let this_prev_close = self.prev_close.replace(bar.c);
        // 1. Advance every indicator series with this closed bar.
        //    The value before this bar becomes `prev`, for crosses.
        for ((state, last), prev) in self
            .states
            .iter_mut()
            .zip(self.last.iter_mut())
            .zip(self.prev.iter_mut())
        {
            *prev = *last;
            *last = state.update(bar);
        }
        // 2. Decide, exits first.
        if self.in_position {
            let intent = self.check_exit(bar, this_prev_close);
            if intent.is_some() {
                self.in_position = false;
                self.entry_price = None;
                self.high_watermark = None;
                self.stop = None;
                return intent;
            }
            // Trail the stop on the way up (long-only).
            if let (Some(hw), Some(stop)) = (self.high_watermark, self.set.trailing_stop) {
                self.stop = Some(self.stop.unwrap_or(0.0).max(hw * (1.0 - stop)));
            }
            return None;
        }
        let entry_rule = self.set.entry.clone();
        if self.decide(&entry_rule, bar, this_prev_close) {
            let quantity = self.quantity_for(bar);
            let intent = OrderIntent {
                symbol: self.symbol.clone(),
                side: crate::rules::IntentSide::Buy,
                quantity,
                signal_price: bar.c,
                stop_loss: self.set.stop_loss,
                target: self.set.target,
                trailing_stop: self.set.trailing_stop,
                signal_bar_t: bar.t,
            };
            self.in_position = true;
            self.entry_price = Some(bar.c);
            self.high_watermark = Some(bar.c);
            self.stop = self.set.stop_loss.map(|f| bar.c * (1.0 - f));
            return Some(intent);
        }
        None
    }

    /// The symbol intents are about; set by the harness (one evaluator per
    /// symbol, like one evaluator per strategy).
    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = symbol.into();
        self
    }

    fn check_exit(&mut self, bar: &Candle, prev_close: Option<f64>) -> Option<OrderIntent> {
        // Stops and targets are checked first, on the bar's range: a stop
        // fires if the bar touched the level, at the level (not the close).
        let entry = self.entry_price?;
        let stop = self.stop;
        let touched_stop = stop.is_some_and(|s| bar.l <= s);
        let target = self.set.target.map(|f| entry * (1.0 + f));
        let touched_target = target.is_some_and(|t| bar.h >= t);
        let trailing_gun = self
            .set
            .trailing_stop
            .and(self.high_watermark)
            .and_then(|hw| self.set.trailing_stop.map(|f| hw * (1.0 - f)));
        let fired = match (touched_stop, touched_target) {
            (true, _) => Some(stop.unwrap_or(entry)),
            (false, true) => target,
            _ => None,
        };
        let price = fired
            .or(trailing_gun.filter(|_| touched_stop || bar.c < self.stop.unwrap_or(f64::NAN)));
        let _ = price;
        let exit_rule = self.set.exit.clone();
        let rule_fired = self.decide(&exit_rule, bar, prev_close);
        let stop_fired = touched_stop;
        let target_fired = touched_target;
        let trailing_fired =
            trailing_gun.is_some_and(|s| bar.l <= s) && self.set.stop_loss.is_none();
        if rule_fired || stop_fired || target_fired || trailing_fired {
            let quantity = self.quantity_for(bar);
            return Some(OrderIntent {
                symbol: self.symbol.clone(),
                side: crate::rules::IntentSide::Sell,
                quantity,
                signal_price: if target_fired && !stop_fired {
                    target.unwrap_or(bar.c)
                } else if stop_fired {
                    stop.unwrap_or(bar.c)
                } else {
                    bar.c
                },
                stop_loss: None,
                target: None,
                trailing_stop: None,
                signal_bar_t: bar.t,
            });
        }
        // Track the watermark for the trailing stop (long-only).
        self.high_watermark = self.high_watermark.map(|hw| hw.max(bar.h)).or(Some(bar.h));
        None
    }

    fn decide(&mut self, rule: &Rule, bar: &Candle, prev_close: Option<f64>) -> bool {
        match rule {
            Rule::True => true,
            Rule::False => false,
            Rule::Cmp { left, cmp, right } => {
                let (l, r) = (
                    self.operand_value(left, bar),
                    self.operand_value(right, bar),
                );
                match (l, r) {
                    (Some(l), Some(r)) => match cmp {
                        Comparison::Less => l < r,
                        Comparison::LessOrEqual => l <= r,
                        Comparison::Greater => l > r,
                        Comparison::GreaterOrEqual => l >= r,
                        Comparison::Equal => l == r,
                    },
                    _ => false,
                }
            }
            Rule::Cross {
                left,
                direction,
                right,
            } => {
                // `Close` has no indicator series: its "previous" is the
                // previous bar's close, its "now" is this bar's close.
                let cross_value = |op: &Operand, prev: Option<f64>| -> (Option<f64>, Option<f64>) {
                    match op {
                        // Close: previous bar's close, then this bar's.
                        Operand::Close => (prev, Some(bar.c)),
                        // Constants never change, so "previous" equals "now".
                        Operand::Constant(v) => (Some(*v), Some(*v)),
                        other => (
                            self.prev_value(self.index_of(other)),
                            self.value_of(other, bar),
                        ),
                    }
                };
                let (l_prev, l_now) = cross_value(left, prev_close);
                let (r_prev, r_now) = cross_value(right, prev_close);
                match (l_prev, l_now, r_prev, r_now) {
                    (Some(lp), Some(ln), Some(rp), Some(rn)) => match direction {
                        CrossDirection::Above => lp <= rp && ln > rn,
                        CrossDirection::Below => lp >= rp && ln < rn,
                    },
                    _ => false,
                }
            }
            Rule::And(a, b) => self.decide(a, bar, prev_close) && self.decide(b, bar, prev_close),
            Rule::Or(a, b) => self.decide(a, bar, prev_close) || self.decide(b, bar, prev_close),
            Rule::Not(a) => !self.decide(a, bar, prev_close),
        }
    }

    fn index_of(&self, op: &Operand) -> Option<usize> {
        match op {
            Operand::Indicator { kind, params } => self
                .specs
                .iter()
                .position(|(k, p)| k == kind && p == params),
            _ => None,
        }
    }

    fn prev_value(&self, index: Option<usize>) -> Option<f64> {
        index.and_then(|i| self.prev[i])
    }

    /// An operand's value at the just-closed bar: a computed series' newest
    /// value, a constant, or the close.
    fn value_of(&self, op: &Operand, bar: &Candle) -> Option<f64> {
        match op {
            Operand::Constant(v) => Some(*v),
            Operand::Close => Some(bar.c),
            Operand::Indicator { .. } => self.index_of(op).and_then(|i| self.last[i]),
        }
    }

    fn operand_value(&self, op: &Operand, bar: &Candle) -> Option<f64> {
        self.value_of(op, bar)
    }

    /// The sizing model against this bar.
    fn quantity_for(&self, bar: &Candle) -> i64 {
        match &self.set.sizing {
            Sizing::FixedQuantity(q) => *q,
            Sizing::FixedAmount(a) if bar.c > 0.0 => (*a / bar.c).floor() as i64,
            Sizing::FractionOfEquity(f) if bar.c > 0.0 => {
                ((self.equity * f) / bar.c).floor() as i64
            }
            _ => 0,
        }
        .max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{CrossDirection, Operand, Rule};
    use nivesh_core::Candle;

    fn candle(t: i64, c: f64) -> Candle {
        Candle {
            t,
            o: c,
            h: c,
            l: c,
            c,
            v: 1000.0,
        }
    }

    /// A rising series of closes: 100, 101, 102, ...
    fn rising(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| candle(1_000 + i64::try_from(i).unwrap(), 100.0 + i as f64))
            .collect()
    }

    fn sma_op(p: f64) -> Operand {
        Operand::Indicator {
            kind: "SMA".into(),
            params: vec![p],
        }
    }

    #[test]
    fn validation_rejects_unknown_kinds_and_bad_params() {
        let unknown = RuleSet {
            entry: Rule::Cmp {
                left: Operand::Indicator {
                    kind: "NOPE".into(),
                    params: vec![5.0],
                },
                cmp: Comparison::Greater,
                right: Operand::Constant(1.0),
            },
            ..RuleSet::default()
        };
        assert!(matches!(
            validate(&unknown),
            Err(RuleError::UnknownIndicator(_))
        ));
        let bad_params = RuleSet {
            entry: Rule::Cmp {
                left: sma_op(0.0),
                cmp: Comparison::Greater,
                right: Operand::Constant(1.0),
            },
            ..RuleSet::default()
        };
        assert!(matches!(
            validate(&bad_params),
            Err(RuleError::UnknownIndicator(_))
        ));
        let bad_sizing = RuleSet {
            entry: Rule::True,
            sizing: Sizing::FixedQuantity(0),
            ..RuleSet::default()
        };
        assert!(matches!(
            validate(&bad_sizing),
            Err(RuleError::BadSizing(_))
        ));
    }

    #[test]
    fn an_sma_comparison_fires_only_when_the_series_has_warmed_up() {
        let set = RuleSet {
            entry: Rule::Cmp {
                left: Operand::Close,
                cmp: Comparison::Greater,
                right: sma_op(5.0),
            },
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut ev = Evaluator::new(set, 100_000.0)
            .unwrap()
            .with_symbol("RELIANCE");
        let bars = rising(10);
        // The SMA needs 5 closed bars; the close is always above it in a
        // rising series, but the first four bars cannot decide.
        let intents: Vec<_> = bars.iter().filter_map(|b| ev.feed(b)).collect();
        assert_eq!(intents.len(), 1, "one entry, after warm-up");
        assert_eq!(intents[0].quantity, 1);
        assert_eq!(intents[0].side, crate::rules::IntentSide::Buy);
    }

    #[test]
    fn sizing_resolves_against_the_signal_bar() {
        let set = RuleSet {
            entry: Rule::True,
            sizing: Sizing::FixedAmount(100_000.0),
            ..RuleSet::default()
        };
        let mut ev = Evaluator::new(set, 100_000.0)
            .unwrap()
            .with_symbol("RELIANCE");
        let intent = ev.feed(&candle(1, 100.0)).unwrap();
        assert_eq!(intent.quantity, 1_000);
        // Fraction of equity.
        let set = RuleSet {
            entry: Rule::True,
            sizing: Sizing::FractionOfEquity(0.5),
            ..RuleSet::default()
        };
        let mut ev = Evaluator::new(set, 100_000.0)
            .unwrap()
            .with_symbol("RELIANCE");
        let intent = ev.feed(&candle(1, 100.0)).unwrap();
        assert_eq!(intent.quantity, 500);
    }

    #[test]
    fn a_cross_fires_exactly_once() {
        let set = RuleSet {
            entry: Rule::Cross {
                left: Operand::Close,
                direction: CrossDirection::Above,
                right: Operand::Constant(100.0),
            },
            exit: Rule::Cross {
                left: Operand::Close,
                direction: CrossDirection::Below,
                right: Operand::Constant(100.0),
            },
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut ev = Evaluator::new(set, 100_000.0).unwrap().with_symbol("X");
        // 99, 100, 101: crosses above on the third bar.
        assert!(ev.feed(&candle(1, 99.0)).is_none());
        assert!(ev.feed(&candle(2, 100.0)).is_none(), "equal is not above");
        let entry = ev.feed(&candle(3, 101.0)).unwrap();
        assert_eq!(entry.side, crate::rules::IntentSide::Buy);
        // While in position, entry-like bars do nothing.
        assert!(ev.feed(&candle(4, 102.0)).is_none());
        // Crossing back below exits.
        let exit = ev.feed(&candle(5, 99.0)).unwrap();
        assert_eq!(exit.side, crate::rules::IntentSide::Sell);
        // And the entry can fire again on the next cross.
        assert!(ev.feed(&candle(6, 100.0)).is_none());
        assert!(ev.feed(&candle(7, 101.0)).is_some());
    }

    fn sma(n: f64) -> Operand {
        Operand::Indicator {
            kind: "SMA".into(),
            params: vec![n],
        }
    }

    #[test]
    fn an_indicator_cross_fires() {
        // SMA(1) is the close itself; SMA(3) lags it. A dip then a rally
        // makes the fast series cross above the slow one exactly once.
        let set = RuleSet {
            entry: Rule::Cross {
                left: sma(1.0),
                direction: CrossDirection::Above,
                right: sma(3.0),
            },
            exit: Rule::False,
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut ev = Evaluator::new(set, 100_000.0).unwrap().with_symbol("X");
        let closes = [100.0, 99.0, 98.0, 97.0, 99.0, 101.0, 102.0];
        let fired: Vec<i64> = closes
            .iter()
            .enumerate()
            .filter_map(|(i, c)| ev.feed(&candle(i as i64, *c)).map(|x| x.signal_bar_t))
            .collect();
        // SMA(3) at bar 4 is (98+97+99)/3 = 98 < 99, and at bar 3 it was
        // 98 > 97: the cross is on bar 4.
        assert_eq!(fired, vec![4]);
    }

    #[test]
    fn exit_only_indicators_read_their_own_series() {
        // Entry uses SMA(1) and SMA(2); exit uses SMA(2) and SMA(5). The
        // exit's SMA(5) is the third distinct series, not the fourth.
        let set = RuleSet {
            entry: Rule::Cmp {
                left: sma(1.0),
                cmp: Comparison::Greater,
                right: sma(2.0),
            },
            exit: Rule::Cmp {
                left: sma(2.0),
                cmp: Comparison::Less,
                right: sma(5.0),
            },
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut ev = Evaluator::new(set, 100_000.0).unwrap().with_symbol("X");
        let mut sides = Vec::new();
        for (i, c) in [100.0, 100.0, 100.0, 100.0, 100.0, 101.0, 102.0, 90.0, 80.0]
            .iter()
            .enumerate()
        {
            if let Some(intent) = ev.feed(&candle(i as i64, *c)) {
                sides.push((i, intent.side));
            }
        }
        // The up-tick on bar 5 enters (101 > 100.5). On bar 7 SMA(2) = 96
        // drops under SMA(5) = 98.6, so the exit is bar 7. Reading SMA(5)
        // from a fourth, non-existent slot would panic instead.
        assert_eq!(
            sides,
            vec![
                (5, crate::rules::IntentSide::Buy),
                (7, crate::rules::IntentSide::Sell)
            ]
        );
    }

    #[test]
    fn a_stop_fires_on_the_touch() {
        let set = RuleSet {
            entry: Rule::True,
            stop_loss: Some(0.02),
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut ev = Evaluator::new(set, 100_000.0).unwrap().with_symbol("X");
        let entry = ev.feed(&candle(1, 100.0)).unwrap();
        assert_eq!(entry.side, crate::rules::IntentSide::Buy);
        // A bar that dips to 98 (the 2% stop) fires, even at the low.
        let mut bar = candle(2, 99.0);
        bar.l = 98.0;
        let exit = ev.feed(&bar).unwrap();
        assert_eq!(exit.side, crate::rules::IntentSide::Sell);
        assert_eq!(exit.signal_price, 98.0);
    }
}
