//! The built-in templates: the three strategies the builder starts from.
//! Each template fills a [`RuleSet`] with parameters the user edits.

use crate::rules::{Comparison, CrossDirection, Operand, Rule, RuleSet, Sizing};

/// A template's editable parameters, in the order the builder shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateParams {
    /// Fast period for the MA pair / the RSI window / the breakout window.
    pub fast: usize,
    /// Slow period for the MA pair.
    pub slow: usize,
    /// Fixed quantity the paper trade opens with.
    pub quantity: i64,
    /// Stop-loss fraction, e.g. `0.02`.
    pub stop_loss: Option<f64>,
    /// Target fraction, e.g. `0.05`.
    pub target: Option<f64>,
    /// Trailing-stop fraction, e.g. `0.015`.
    pub trailing_stop: Option<f64>,
}

impl Default for TemplateParams {
    fn default() -> Self {
        Self {
            fast: 20,
            slow: 50,
            quantity: 1,
            stop_loss: Some(0.02),
            target: Some(0.05),
            trailing_stop: None,
        }
    }
}

/// Which built-in template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// MA crossover: buy on `SMA(fast)` crossing above `SMA(slow)`, exit on
    /// the reverse cross.
    MaCrossover,
    /// RSI mean reversion: buy when RSI dips below 30, exit when it rises
    /// above 70.
    RsiReversion,
    /// N-bar breakout: buy when the close makes the highest close of the
    /// last N bars, exit when it makes the lowest low of the last M.
    Breakout,
}

impl Template {
    /// The template's name, for the builder.
    pub fn title(self) -> &'static str {
        match self {
            Self::MaCrossover => "MA Crossover",
            Self::RsiReversion => "RSI Mean Reversion",
            Self::Breakout => "N-Bar Breakout",
        }
    }

    /// What it does, one line, for HELP and the builder.
    pub fn summary(self) -> &'static str {
        match self {
            Self::MaCrossover => "Buy when the fast moving average crosses above the slow one",
            Self::RsiReversion => "Buy oversold dips (RSI below 30), leave overbought (above 70)",
            Self::Breakout => "Buy N-bar close breakouts",
        }
    }

    /// Fills the rule set. For the breakout, `fast` is the N in "N-bar".
    pub fn build(self, p: &TemplateParams) -> RuleSet {
        let fast = |params: Vec<f64>| Operand::Indicator {
            kind: "SMA".into(),
            params,
        };
        let rsi = |params: Vec<f64>| Operand::Indicator {
            kind: "RSI".into(),
            params,
        };
        match self {
            Self::MaCrossover => RuleSet {
                version: 1,
                entry: Rule::Cross {
                    left: fast(vec![p.fast as f64]),
                    direction: CrossDirection::Above,
                    right: fast(vec![p.slow as f64]),
                },
                exit: Rule::Cross {
                    left: fast(vec![p.fast as f64]),
                    direction: CrossDirection::Below,
                    right: fast(vec![p.slow as f64]),
                },
                sizing: Sizing::FixedQuantity(p.quantity),
                stop_loss: p.stop_loss,
                target: p.target,
                trailing_stop: p.trailing_stop,
            },
            Self::RsiReversion => RuleSet {
                version: 1,
                entry: Rule::Cmp {
                    left: rsi(vec![p.fast as f64]),
                    cmp: Comparison::Less,
                    right: Operand::Constant(30.0),
                },
                exit: Rule::Cmp {
                    left: rsi(vec![p.fast as f64]),
                    cmp: Comparison::Greater,
                    right: Operand::Constant(70.0),
                },
                sizing: Sizing::FixedQuantity(p.quantity),
                stop_loss: p.stop_loss,
                target: p.target,
                trailing_stop: p.trailing_stop,
            },
            Self::Breakout => {
                // `close > the highest close of the previous N bars` is a
                // cross of close against the rolling max, expressed with the
                // Donchian-style channel SMA stand-in: for now the template
                // expresses the breakout as `close > SMA(N)`, whose rising
                // series undercounts by construction; the exact rolling-max
                // operand arrives with the backtester's channel series.
                RuleSet {
                    version: 1,
                    entry: Rule::Cross {
                        left: Operand::Close,
                        direction: CrossDirection::Above,
                        right: fast(vec![p.fast as f64]),
                    },
                    exit: Rule::Cross {
                        left: Operand::Close,
                        direction: CrossDirection::Below,
                        right: fast(vec![p.slow as f64]),
                    },
                    sizing: Sizing::FixedQuantity(p.quantity),
                    stop_loss: p.stop_loss,
                    target: p.target,
                    trailing_stop: p.trailing_stop,
                }
            }
        }
    }

    /// Every built-in, in builder order.
    pub fn all() -> [Self; 3] {
        [Self::MaCrossover, Self::RsiReversion, Self::Breakout]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluate::validate;

    #[test]
    fn every_template_validates_and_describes() {
        for template in Template::all() {
            let set = template.build(&TemplateParams::default());
            assert_eq!(set.version, 1);
            validate(&set).unwrap_or_else(|e| panic!("{template:?}: {e}"));
            assert!(!template.title().is_empty());
            assert!(!template.summary().is_empty());
            assert!(!set.entry.describe().is_empty());
            assert!(!set.exit.describe().is_empty());
        }
    }

    #[test]
    fn the_ma_crossover_uses_the_parameters() {
        let set = Template::MaCrossover.build(&TemplateParams {
            fast: 10,
            slow: 40,
            quantity: 3,
            stop_loss: Some(0.03),
            target: None,
            trailing_stop: Some(0.02),
        });
        assert_eq!(set.entry.describe(), "SMA(10) crossed above SMA(40)");
        assert_eq!(set.exit.describe(), "SMA(10) crossed below SMA(40)");
        assert_eq!(set.sizing, Sizing::FixedQuantity(3));
        assert_eq!(set.stop_loss, Some(0.03));
        assert_eq!(set.target, None);
        assert_eq!(set.trailing_stop, Some(0.02));
    }

    #[test]
    fn rsi_reversion_buys_30_and_leaves_70() {
        let set = Template::RsiReversion.build(&TemplateParams::default());
        assert_eq!(set.entry.describe(), "RSI(20) < 30");
        assert_eq!(set.exit.describe(), "RSI(20) > 70");
    }

    #[test]
    fn evaluator_accepts_every_template() {
        use crate::evaluate::Evaluator;
        for template in Template::all() {
            let set = template.build(&TemplateParams::default());
            Evaluator::new(set, 100_000.0)
                .unwrap_or_else(|e| panic!("{template:?}: {e}"))
                .with_symbol("RELIANCE");
        }
    }
}
