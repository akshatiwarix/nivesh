//! What the backtester and the paper runner drive: anything that decides
//! on closed bars. The rule [`Evaluator`](crate::Evaluator) is one; a
//! sandboxed script (`nivesh-script`) is another. Both trade through the
//! same fill model and, when paper or live, through the same gateway.

use nivesh_core::Candle;

use crate::rules::OrderIntent;

pub trait Decide: Send {
    /// Feeds one **closed** bar; returns the intent it decided on, if any.
    /// It may assume an intent fills; [`Self::sync_position`] corrects it.
    fn feed(&mut self, bar: &Candle) -> Option<OrderIntent>;

    /// Aligns the decider's view of the position with the account's:
    /// `entry` is the open position's entry price, `None` when flat.
    fn sync_position(&mut self, entry: Option<f64>);
}

impl Decide for crate::Evaluator {
    fn feed(&mut self, bar: &Candle) -> Option<OrderIntent> {
        crate::Evaluator::feed(self, bar)
    }

    fn sync_position(&mut self, entry: Option<f64>) {
        crate::Evaluator::sync_position(self, entry);
    }
}
