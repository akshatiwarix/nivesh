//! Rule-based strategies for Nivesh: the rule model lives here today; the
//! validator, evaluator, templates, backtester and paper runner join in
//! later commits. Strategies emit `OrderIntent`s and never place orders.

pub mod rules;

pub use rules::{
    Comparison, CrossDirection, IntentSide, Operand, OrderIntent, Rule, RuleSet, Sizing,
};
