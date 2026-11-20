//! What a saved strategy is: rules (from a template) or a sandboxed
//! script. Both become a `Decide`r, so the backtester, the paper desk and
//! (when armed) the live route treat them alike.

use nivesh_script::{Script, ScriptLimits};
use nivesh_store::StrategyRow;
use nivesh_strategy::{Decide, Evaluator, PaperRunner, RuleSet};
use serde::{Deserialize, Serialize};

use crate::ipc::IpcError;

/// The `template` column of a script strategy.
pub const SCRIPT_TEMPLATE: &str = "Script";

/// How a script is stored in the `json` column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptJson {
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrategyBody {
    Rules(Box<RuleSet>),
    Script(String),
}

impl StrategyBody {
    pub fn from_row(row: &StrategyRow) -> Result<Self, IpcError> {
        let unreadable = |e: serde_json::Error| {
            IpcError::internal(format!("stored strategy {} is unreadable: {e}", row.id))
        };
        if row.template == SCRIPT_TEMPLATE {
            let s: ScriptJson = serde_json::from_str(&row.json).map_err(unreadable)?;
            Ok(Self::Script(s.source))
        } else {
            Ok(Self::Rules(
                serde_json::from_str(&row.json).map_err(unreadable)?,
            ))
        }
    }

    /// A one-line preview for the strategy list.
    pub fn describe(&self) -> String {
        match self {
            Self::Rules(set) => set.entry.describe(),
            Self::Script(source) => format!("script, {} lines", source.lines().count()),
        }
    }

    /// The rules, for what only rule strategies support (walk-forward).
    pub fn rules(&self) -> Option<&RuleSet> {
        match self {
            Self::Rules(set) => Some(set),
            Self::Script(_) => None,
        }
    }

    /// A fresh decider over `symbol`.
    pub fn decider(&self, symbol: &str, equity: f64) -> Result<Box<dyn Decide>, IpcError> {
        match self {
            Self::Rules(set) => Ok(Box::new(
                Evaluator::new((**set).clone(), equity)
                    .map_err(|e| IpcError::invalid_argument(e.to_string()))?
                    .with_symbol(symbol),
            )),
            Self::Script(source) => Ok(Box::new(
                Script::new(source, ScriptLimits::default())
                    .map_err(|e| IpcError::invalid_argument(e.to_string()))?
                    .with_symbol(symbol),
            )),
        }
    }

    /// A paper runner over `symbol`.
    pub fn runner(&self, symbol: &str, equity: f64) -> Result<PaperRunner, IpcError> {
        match self {
            Self::Rules(set) => PaperRunner::new((**set).clone(), symbol, equity)
                .map_err(|e| IpcError::invalid_argument(e.to_string())),
            Self::Script(_) => Ok(PaperRunner::with_decider(
                self.decider(symbol, equity)?,
                symbol,
                equity,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(template: &str, json: &str) -> StrategyRow {
        StrategyRow {
            id: 1,
            name: "T".into(),
            template: template.into(),
            json: json.into(),
            created_at: 0,
        }
    }

    #[test]
    fn rows_read_as_rules_or_scripts() {
        let rules = serde_json::to_string(&RuleSet::default()).unwrap();
        assert!(matches!(
            StrategyBody::from_row(&row("MA Crossover", &rules)).unwrap(),
            StrategyBody::Rules(_)
        ));
        let script = serde_json::to_string(&ScriptJson {
            source: "fn on_bar(bar) { }\n".into(),
        })
        .unwrap();
        let body = StrategyBody::from_row(&row(SCRIPT_TEMPLATE, &script)).unwrap();
        assert_eq!(body.describe(), "script, 1 lines");
        assert!(body.rules().is_none());
        assert!(body.runner("NSE:TCS", 1.0).is_ok());
        assert!(StrategyBody::from_row(&row(SCRIPT_TEMPLATE, "{}")).is_err());
    }

    #[test]
    fn a_script_that_no_longer_compiles_cannot_run() {
        let body = StrategyBody::Script("fn tick() {}".into());
        assert!(body.runner("NSE:TCS", 1.0).is_err());
    }
}
