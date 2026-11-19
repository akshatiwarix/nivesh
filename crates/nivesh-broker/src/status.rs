//! The order state machine, shared by every broker and by paper trading.
//!
//! `Pending → Open → (PartiallyFilled) → Filled | Cancelled | Rejected`, with
//! two shortcuts: a market or IOC order can go `Pending → Filled` straight
//! away, and a user can cancel while `Pending`. Terminal states are final.
//! Adapters map their broker's own status strings onto this in their
//! conformance suite, so the mapping is tested, not assumed.

use crate::BrokerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderStatus {
    /// Sent, not yet acknowledged by the exchange.
    Pending,
    /// Accepted and live on the book.
    Open,
    /// Filling in parts.
    PartiallyFilled,
    /// Fully filled; terminal.
    Filled,
    /// Cancelled; terminal.
    Cancelled,
    /// Refused; terminal.
    Rejected,
}

impl OrderStatus {
    /// Whether `self → next` is a legal transition.
    pub fn can_transition_to(self, next: Self) -> bool {
        use OrderStatus::*;
        matches!(
            (self, next),
            (Pending, Open)
                | (Pending, Filled)
                | (Pending, Cancelled)
                | (Pending, Rejected)
                | (Open, PartiallyFilled)
                | (Open, Filled)
                | (Open, Cancelled)
                | (Open, Rejected)
                | (PartiallyFilled, PartiallyFilled)
                | (PartiallyFilled, Filled)
                | (PartiallyFilled, Cancelled)
                | (PartiallyFilled, Rejected)
        )
    }

    /// The checked transition, for adapters folding broker updates.
    pub fn transition(self, next: Self) -> Result<Self, BrokerError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(BrokerError::InvalidTransition {
                from: self,
                to: next,
            })
        }
    }

    /// Filled, cancelled or rejected: no update can change this any more, and
    /// an update that tries is a data error, not a state to store.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Filled | Self::Cancelled | Self::Rejected)
    }

    /// The order has live quantity on the book.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Pending | Self::Open | Self::PartiallyFilled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_happy_path_walks_through_every_state() {
        let mut s = OrderStatus::Pending;
        s = s.transition(OrderStatus::Open).unwrap();
        s = s.transition(OrderStatus::PartiallyFilled).unwrap();
        s = s.transition(OrderStatus::PartiallyFilled).unwrap();
        s = s.transition(OrderStatus::Filled).unwrap();
        assert!(s.is_terminal());
    }

    #[test]
    fn a_market_order_can_fill_immediately() {
        assert!(OrderStatus::Pending.transition(OrderStatus::Filled).is_ok());
    }

    #[test]
    fn a_pending_order_can_be_cancelled_or_rejected() {
        assert!(
            OrderStatus::Pending
                .transition(OrderStatus::Cancelled)
                .is_ok()
        );
        assert!(
            OrderStatus::Pending
                .transition(OrderStatus::Rejected)
                .is_ok()
        );
    }

    #[test]
    fn illegal_transitions_are_errors() {
        for (from, to) in [
            (OrderStatus::Filled, OrderStatus::Open),
            (OrderStatus::Cancelled, OrderStatus::Filled),
            (OrderStatus::Rejected, OrderStatus::Cancelled),
            (OrderStatus::Filled, OrderStatus::Pending),
            (OrderStatus::Open, OrderStatus::Pending),
        ] {
            let err = from.transition(to).unwrap_err();
            assert_eq!(err, BrokerError::InvalidTransition { from, to });
        }
    }

    #[test]
    fn nothing_comes_back_from_a_terminal_state() {
        for terminal in [
            OrderStatus::Filled,
            OrderStatus::Cancelled,
            OrderStatus::Rejected,
        ] {
            for next in [
                OrderStatus::Pending,
                OrderStatus::Open,
                OrderStatus::PartiallyFilled,
                OrderStatus::Filled,
                OrderStatus::Cancelled,
                OrderStatus::Rejected,
            ] {
                assert!(
                    !terminal.can_transition_to(next),
                    "{terminal:?} -> {next:?}"
                );
            }
            assert!(terminal.is_terminal());
            assert!(!terminal.is_live());
        }
    }
}
