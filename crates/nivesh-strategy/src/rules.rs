//! The strategy rule model: versioned, serialisable rules over indicator
//! references, comparisons and crosses, combinable with AND/OR/NOT.
//!
//! Rules are pure data. The evaluator (a later commit) decides them bar by
//! bar on **closed** bars; the backtester (a later commit) replays them
//! deterministically. Nothing here places orders — the model emits
//! `OrderIntent`s only.
//!
//! Versioning: `RuleSet::version` exists so a saved strategy can be
//! migrated forward instead of silently reinterpreted.

use serde::{Deserialize, Serialize};

/// A whole strategy: entry rule, exit rule, sizing and protection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    /// Forward-compatibility: bumped when the rule grammar changes.
    pub version: u32,
    /// When to be long (evaluated on each closed bar).
    pub entry: Rule,
    /// When to leave the position (evaluated while long).
    pub exit: Rule,
    /// How much to buy when the entry fires.
    pub sizing: Sizing,
    /// Stop-loss and target, as fractions of the entry price (`0.02` = 2%).
    pub stop_loss: Option<f64>,
    pub target: Option<f64>,
    /// Trailing stop as a fraction; the stop follows the high watermark.
    pub trailing_stop: Option<f64>,
}

impl Default for RuleSet {
    fn default() -> Self {
        Self {
            version: 1,
            entry: Rule::True,
            exit: Rule::False,
            sizing: Sizing::default(),
            stop_loss: None,
            target: None,
            trailing_stop: None,
        }
    }
}

/// The sizing model: how large a position the entry opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Sizing {
    /// A fixed number of shares.
    FixedQuantity(i64),
    /// A fixed notional in rupees, at the signal price.
    FixedAmount(f64),
    /// A fraction of the (paper) account equity, `0.25` = 25%.
    FractionOfEquity(f64),
    /// Volatility sizing: risk `risk` of equity (`0.01` = 1%) per trade,
    /// with the stop `atr_multiple` ATRs below the entry. The quantity is
    /// `floor(equity * risk / (atr_multiple * ATR(atr_period)))`, and that
    /// ATR stop is placed (the tighter of it and any `stop_loss` wins).
    AtrRisk {
        risk: f64,
        atr_period: usize,
        atr_multiple: f64,
    },
}

impl Default for Sizing {
    fn default() -> Self {
        Self::FixedQuantity(1)
    }
}

/// A boolean rule over indicator comparisons and crosses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Rule {
    /// Always / never (for exits that fire on entry or never).
    True,
    False,
    /// `left cmp right`, e.g. `SMA(50) > SMA(200)`.
    Cmp {
        left: Operand,
        cmp: Comparison,
        right: Operand,
    },
    /// `left` crossed above/below `right` on this bar (was not so on the
    /// previous bar).
    Cross {
        left: Operand,
        direction: CrossDirection,
        right: Operand,
    },
    And(Box<Rule>, Box<Rule>),
    Or(Box<Rule>, Box<Rule>),
    Not(Box<Rule>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossDirection {
    /// Crossed from below to above.
    Above,
    /// Crossed from above to below.
    Below,
}

/// One side of a comparison: an indicator series or a constant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operand {
    /// An indicator with its parameters, as the chart's specs spell them:
    /// `("SMA", [50])`, `("RSI", [14])`.
    Indicator { kind: String, params: Vec<f64> },
    /// A bare price, e.g. a level to compare against.
    Constant(f64),
    /// The bar's close, in case a rule wants `close > SMA(50)`.
    Close,
}

impl Operand {
    /// The canonical text of the operand, for previews and errors.
    pub fn describe(&self) -> String {
        match self {
            Self::Indicator { kind, params } => {
                if params.is_empty() {
                    kind.clone()
                } else {
                    let joined: Vec<String> = params.iter().map(|p| format!("{p}")).collect();
                    format!("{kind}({})", joined.join(", "))
                }
            }
            Self::Constant(v) => format!("{v}"),
            Self::Close => "CLOSE".to_owned(),
        }
    }

    /// Every indicator the operand references, in order.
    pub fn indicators(&self) -> Vec<(String, Vec<f64>)> {
        match self {
            Self::Indicator { kind, params } => vec![(kind.clone(), params.clone())],
            Self::Constant(_) | Self::Close => Vec::new(),
        }
    }
}

impl Rule {
    /// Every indicator the rule tree references, deduplicated, in first-seen
    /// order. The evaluator computes exactly these series.
    pub fn indicators(&self) -> Vec<(String, Vec<f64>)> {
        let mut out = Vec::new();
        self.collect_indicators(&mut out);
        out
    }

    fn collect_indicators(&self, out: &mut Vec<(String, Vec<f64>)>) {
        let push = |out: &mut Vec<(String, Vec<f64>)>, op: &Operand| {
            for (kind, params) in op.indicators() {
                if !out.iter().any(|(k, p)| k == &kind && p == &params) {
                    out.push((kind, params));
                }
            }
        };
        match self {
            Self::True | Self::False => {}
            Self::Cmp { left, right, .. } => {
                push(out, left);
                push(out, right);
            }
            Self::Cross { left, right, .. } => {
                push(out, left);
                push(out, right);
            }
            Self::And(a, b) | Self::Or(a, b) => {
                a.collect_indicators(out);
                b.collect_indicators(out);
            }
            Self::Not(a) => a.collect_indicators(out),
        }
    }

    /// A readable preview, e.g. `SMA(50) crossed above SMA(200)`.
    pub fn describe(&self) -> String {
        match self {
            Self::True => "always".to_owned(),
            Self::False => "never".to_owned(),
            Self::Cmp { left, cmp, right } => {
                let sign = match cmp {
                    Comparison::Less => "<",
                    Comparison::LessOrEqual => "<=",
                    Comparison::Greater => ">",
                    Comparison::GreaterOrEqual => ">=",
                    Comparison::Equal => "=",
                };
                format!("{} {} {}", left.describe(), sign, right.describe())
            }
            Self::Cross {
                left,
                direction,
                right,
            } => match direction {
                CrossDirection::Above => {
                    format!("{} crossed above {}", left.describe(), right.describe())
                }
                CrossDirection::Below => {
                    format!("{} crossed below {}", left.describe(), right.describe())
                }
            },
            Self::And(a, b) => format!("({}) and ({})", a.describe(), b.describe()),
            Self::Or(a, b) => format!("({}) or ({})", a.describe(), b.describe()),
            Self::Not(a) => format!("not ({})", a.describe()),
        }
    }
}

/// An intent the evaluator emits when the entry (or exit) fires on a closed
/// bar. The gateway (Part 5's later commits) turns intents into paper or
/// live orders — never this module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderIntent {
    /// NSE symbol the intent is about.
    pub symbol: String,
    pub side: IntentSide,
    /// Shares to trade (the sizing model resolved against the signal bar).
    pub quantity: i64,
    /// The signal bar's close: the price the decision was made on.
    pub signal_price: f64,
    /// Stop-loss / target / trailing fractions carried to the position.
    pub stop_loss: Option<f64>,
    pub target: Option<f64>,
    pub trailing_stop: Option<f64>,
    /// The bar's UTC ms; the gateway refuses intents that arrive late.
    pub signal_bar_t: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentSide {
    Buy,
    Sell,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sma(p: f64) -> Operand {
        Operand::Indicator {
            kind: "SMA".into(),
            params: vec![p],
        }
    }

    fn golden_cross() -> Rule {
        Rule::Cross {
            left: sma(50.0),
            direction: CrossDirection::Above,
            right: sma(200.0),
        }
    }

    #[test]
    fn rules_describe_readably() {
        assert_eq!(golden_cross().describe(), "SMA(50) crossed above SMA(200)");
        let r = Rule::And(
            Box::new(Rule::Cmp {
                left: Operand::Close,
                cmp: Comparison::Greater,
                right: sma(50.0),
            }),
            Box::new(Rule::Not(Box::new(golden_cross()))),
        );
        assert_eq!(
            r.describe(),
            "(CLOSE > SMA(50)) and (not (SMA(50) crossed above SMA(200)))"
        );
        assert_eq!(Rule::True.describe(), "always");
        assert_eq!(Operand::Constant(42.0).describe(), "42");
    }

    #[test]
    fn indicator_collection_is_deduplicated_in_order() {
        let r = Rule::And(
            Box::new(Rule::Cmp {
                left: Operand::Close,
                cmp: Comparison::Greater,
                right: sma(50.0),
            }),
            Box::new(golden_cross()),
        );
        assert_eq!(
            r.indicators(),
            vec![
                ("SMA".to_owned(), vec![50.0]),
                ("SMA".to_owned(), vec![200.0]),
            ]
        );
    }

    #[test]
    fn rules_round_trip_through_serde() {
        let r = RuleSet {
            version: 1,
            entry: golden_cross(),
            exit: Rule::Cross {
                left: sma(50.0),
                direction: CrossDirection::Below,
                right: sma(200.0),
            },
            sizing: Sizing::FractionOfEquity(0.25),
            stop_loss: Some(0.02),
            target: Some(0.05),
            trailing_stop: Some(0.015),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: RuleSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.version, 1);
    }
}
