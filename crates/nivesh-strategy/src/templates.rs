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
    /// ATR sizing: risk this fraction of equity per trade (`0.01` = 1%)
    /// with a 2×ATR(14) stop, instead of the fixed `quantity`.
    pub atr_risk: Option<f64>,
}

/// The ATR window and stop multiple the templates' ATR sizing uses.
pub const TEMPLATE_ATR_PERIOD: usize = 14;
pub const TEMPLATE_ATR_MULTIPLE: f64 = 2.0;

impl TemplateParams {
    fn sizing(&self) -> Sizing {
        match self.atr_risk {
            Some(risk) => Sizing::AtrRisk {
                risk,
                atr_period: TEMPLATE_ATR_PERIOD,
                atr_multiple: TEMPLATE_ATR_MULTIPLE,
            },
            None => Sizing::FixedQuantity(self.quantity),
        }
    }
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
            atr_risk: None,
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
    /// N-bar breakout: buy when the close breaks above the highest high of
    /// the previous N (`fast`) bars, exit when it breaks below the lowest low
    /// of the previous M (`slow`) bars.
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
            Self::Breakout => {
                "Buy a close above the last N bars' high; leave below the last M bars' low"
            }
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
                sizing: p.sizing(),
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
                sizing: p.sizing(),
                stop_loss: p.stop_loss,
                target: p.target,
                trailing_stop: p.trailing_stop,
            },
            Self::Breakout => {
                // The classic N-bar (Donchian) breakout: buy when the close
                // breaks above the previous `fast` bars' highest high, leave
                // when it breaks below the previous `slow` bars' lowest low.
                let channel = |kind: &str, n: usize| Operand::Indicator {
                    kind: kind.into(),
                    params: vec![n as f64],
                };
                RuleSet {
                    version: 1,
                    entry: Rule::Cmp {
                        left: Operand::Close,
                        cmp: Comparison::Greater,
                        right: channel("HIGHEST", p.fast),
                    },
                    exit: Rule::Cmp {
                        left: Operand::Close,
                        cmp: Comparison::Less,
                        right: channel("LOWEST", p.slow),
                    },
                    sizing: p.sizing(),
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
            atr_risk: None,
        });
        assert_eq!(set.entry.describe(), "SMA(10) crossed above SMA(40)");
        assert_eq!(set.exit.describe(), "SMA(10) crossed below SMA(40)");
        assert_eq!(set.sizing, Sizing::FixedQuantity(3));
        assert_eq!(set.stop_loss, Some(0.03));
        assert_eq!(set.target, None);
        assert_eq!(set.trailing_stop, Some(0.02));
    }

    #[test]
    fn atr_risk_switches_every_template_to_atr_sizing() {
        let params = TemplateParams {
            atr_risk: Some(0.01),
            ..TemplateParams::default()
        };
        for template in Template::all() {
            let set = template.build(&params);
            assert_eq!(
                set.sizing,
                Sizing::AtrRisk {
                    risk: 0.01,
                    atr_period: TEMPLATE_ATR_PERIOD,
                    atr_multiple: TEMPLATE_ATR_MULTIPLE,
                }
            );
            crate::validate(&set).unwrap();
        }
    }

    #[test]
    fn the_breakout_uses_the_price_channel() {
        let set = Template::Breakout.build(&TemplateParams {
            fast: 20,
            slow: 10,
            ..TemplateParams::default()
        });
        assert_eq!(set.entry.indicators(), vec![("HIGHEST".into(), vec![20.0])]);
        assert_eq!(set.exit.indicators(), vec![("LOWEST".into(), vec![10.0])]);
        crate::validate(&set).unwrap();
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
