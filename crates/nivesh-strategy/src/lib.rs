//! Rule-based strategies for Nivesh: the rule model and its evaluator live
//! here; templates, the backtester and the paper runner join in later
//! commits. Strategies emit `OrderIntent`s and never place orders.

pub mod evaluate;
pub mod rules;

pub use evaluate::Evaluator;
pub use rules::{
    Comparison, CrossDirection, IntentSide, Operand, OrderIntent, Rule, RuleSet, Sizing,
};
