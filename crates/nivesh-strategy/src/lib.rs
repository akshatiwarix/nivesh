//! Rule-based strategies for Nivesh: the rule model and its evaluator live
//! here; templates, the backtester and the paper runner join in later
//! commits. Strategies emit `OrderIntent`s and never place orders.

pub mod backtest;
pub mod evaluate;
pub mod rules;
pub mod templates;

pub use backtest::{BacktestOptions, BacktestResult, BacktestTrade, backtest};
pub use evaluate::Evaluator;
pub use rules::{
    Comparison, CrossDirection, IntentSide, Operand, OrderIntent, Rule, RuleSet, Sizing,
};
pub use templates::Template;
