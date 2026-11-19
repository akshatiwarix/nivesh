//! Rule-based strategies for Nivesh: the rule model and its evaluator live
//! here; templates, the backtester and the paper runner join in later
//! commits. Strategies emit `OrderIntent`s and never place orders.

pub mod backtest;
pub mod costs;
pub mod evaluate;
pub mod paper;
pub mod rules;
pub mod templates;
pub mod walk_forward;

pub use backtest::{
    BacktestOptions, BacktestResult, BacktestTrade, CurvePoint, backtest, backtest_range,
};
pub use costs::{ChargeSchedule, Charges, CostModel};
pub use evaluate::{Evaluator, validate};
pub use paper::{PaperFill, PaperPosition, PaperRunner, PaperState};
pub use rules::{
    Comparison, CrossDirection, IntentSide, Operand, OrderIntent, Rule, RuleSet, Sizing,
};
pub use templates::{TEMPLATE_ATR_MULTIPLE, TEMPLATE_ATR_PERIOD, Template, TemplateParams};
pub use walk_forward::{
    WalkForward, WalkForwardOptions, WindowResult, walk_forward, window_bounds,
};
