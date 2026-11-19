//! The pre-trade risk engine.
//!
//! **One gate, enforced by types:** an [`ApprovedOrder`] cannot be
//! constructed outside this crate (its fields are private and only
//! [`RiskEngine::check`] mints one), so nothing downstream can trade
//! without the engine's decision. Part 6 adds the full limit model on the
//! same skeleton; the basic checks here cover quantity, value, segment
//! and price sanity, plus the order-rate guard.
//!
//! Everything is pure: no I/O, no clock — the caller supplies `now_ms` so
//! the engine is deterministic and property-testable.

use nivesh_core::Side;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The current UTC time in milliseconds; tests inject a fixed clock.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The limits every order is checked against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskLimits {
    /// Maximum shares in one order.
    pub max_order_quantity: i64,
    /// Maximum notional (rupees) in one order.
    pub max_order_value: f64,
    /// Maximum open positions the account may hold.
    pub max_open_positions: usize,
    /// Orders per rolling minute.
    pub max_orders_per_minute: u32,
    /// Orders per calendar day.
    pub max_orders_per_day: u32,
    /// The daily loss (realised + MTM, rupees, negative is worse) that must
    /// stop trading. Part 6 wires the auto-kill-switch; the gateway checks
    /// this from Part 5.
    pub max_daily_loss: f64,
    /// A limit price must sit within this fraction of the LTP
    /// (`0.1` = within 10%).
    pub price_band: f64,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_order_quantity: 1_000,
            max_order_value: 500_000.0,
            max_open_positions: 20,
            max_orders_per_minute: 10,
            max_orders_per_day: 200,
            max_daily_loss: 50_000.0,
            price_band: 0.10,
        }
    }
}

/// One rejected or approved decision, for the audit trail.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskDecision {
    pub approved: bool,
    /// Why (when rejected), in plain language.
    pub reason: Option<String>,
}

/// What the engine knows when it checks an order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskContext<'a> {
    /// The instrument, `NSE:RELIANCE` shaped.
    pub symbol: &'a str,
    /// The last traded price, rupees.
    pub ltp: f64,
    /// How many positions are open right now.
    pub open_positions: usize,
    /// Timestamps (ms) of recent order approvals, newest first.
    pub recent_orders: &'a [i64],
    /// Today's realised P&L so far, rupees (negative is a loss).
    pub day_pnl: f64,
    /// Now, ms.
    pub now_ms: i64,
}

/// The risk engine: immutable limits, pure checks.
#[derive(Debug, Clone)]
pub struct RiskEngine {
    pub limits: RiskLimits,
}

/// The order the engine approved. Constructing one outside this crate is
/// impossible: the fields are private and [`RiskEngine::check`] is the only
/// minter. It carries the decision's id and a short expiry, so a stale
/// approval cannot be replayed.
///
/// It is `Serialize` (for the audit log) but deliberately **not**
/// `Deserialize`: decoding one from bytes would mint an approval without a
/// check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApprovedOrder {
    symbol: String,
    side: Side,
    quantity: i64,
    /// The limit price the caller asked for, rupees.
    price: Option<f64>,
    /// The engine's decision id, linking it to the audit log.
    decision_id: u64,
    /// When the approval dies (5 minutes out).
    expires_at_ms: i64,
}

impl ApprovedOrder {
    /// The instrument.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Buy or sell.
    pub fn side(&self) -> Side {
        self.side
    }

    /// Shares.
    pub fn quantity(&self) -> i64 {
        self.quantity
    }

    /// The approved limit price, if any.
    pub fn price(&self) -> Option<f64> {
        self.price
    }

    /// The decision id for the audit log.
    pub fn decision_id(&self) -> u64 {
        self.decision_id
    }

    /// Whether the approval is still valid at `now_ms`.
    pub fn is_valid_at(&self, now_ms: i64) -> bool {
        now_ms <= self.expires_at_ms
    }

    /// How long the approval is valid for, from minting.
    pub const VALIDITY: Duration = Duration::from_secs(5 * 60);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RiskError {
    #[error("the order breaks a hard limit")]
    Rejected,
}

impl RiskEngine {
    pub fn new(limits: RiskLimits) -> Self {
        Self { limits }
    }

    /// Checks the order against every hard limit and, on a pass, mints the
    /// approval. Pure: no I/O, no clock of its own.
    pub fn check(
        &self,
        symbol: &str,
        side: Side,
        quantity: i64,
        price: Option<f64>,
        ctx: &RiskContext<'_>,
    ) -> Result<ApprovedOrder, RiskError> {
        let reject = |reason: String| -> RiskError {
            let _ = reason;
            RiskError::Rejected
        };

        // Quantity and value.
        if quantity <= 0 {
            return Err(reject("quantity must be positive".into()));
        }
        if quantity > self.limits.max_order_quantity {
            return Err(reject(format!(
                "quantity {quantity} exceeds the per-order cap {}",
                self.limits.max_order_quantity
            )));
        }
        let price = price.unwrap_or(ctx.ltp);
        if !price.is_finite() || price <= 0.0 || !ctx.ltp.is_finite() || ctx.ltp <= 0.0 {
            return Err(reject("order price and LTP must be positive".into()));
        }
        let value = price * quantity as f64;
        if value > self.limits.max_order_value {
            return Err(reject(format!(
                "order value ₹{value:.0} exceeds the per-order cap ₹{:.0}",
                self.limits.max_order_value
            )));
        }
        // Price band: a limit price far from the LTP is a fat finger.
        if (price - ctx.ltp).abs() / ctx.ltp > self.limits.price_band {
            return Err(reject(format!(
                "price ₹{price:.2} is outside the {:.0}% band around LTP ₹{:.2}",
                self.limits.price_band * 100.0,
                ctx.ltp
            )));
        }
        // Open positions.
        if ctx.open_positions >= self.limits.max_open_positions {
            return Err(reject(
                "the maximum number of open positions is reached".into(),
            ));
        }
        // Rate: orders per minute (rolling) and per day.
        let minute_ago = ctx.now_ms - 60_000;
        let per_minute = ctx
            .recent_orders
            .iter()
            .filter(|t| **t >= minute_ago && **t <= ctx.now_ms)
            .count();
        if per_minute >= self.limits.max_orders_per_minute as usize {
            return Err(reject(format!(
                "the per-minute order cap ({}) is reached; wait a moment",
                self.limits.max_orders_per_minute
            )));
        }
        let day_start = ctx.now_ms - (ctx.now_ms % 86_400_000);
        let per_day = ctx
            .recent_orders
            .iter()
            .filter(|t| **t >= day_start && **t <= ctx.now_ms)
            .count();
        if per_day >= self.limits.max_orders_per_day as usize {
            return Err(reject(format!(
                "the per-day order cap ({}) is reached",
                self.limits.max_orders_per_day
            )));
        }
        // Daily loss.
        if ctx.day_pnl <= -self.limits.max_daily_loss {
            return Err(reject(format!(
                "the daily loss (₹{:.0}) has reached the limit (₹{:.0})",
                -ctx.day_pnl, self.limits.max_daily_loss
            )));
        }
        // Cash-only segments: the symbol must be an NSE/BSE equity shape.
        if !symbol_starts_valid(symbol) {
            return Err(reject("only cash-equity symbols are allowed".into()));
        }

        let decision_id = decision_counter(ctx.now_ms, symbol, quantity);
        Ok(ApprovedOrder {
            symbol: symbol.to_owned(),
            side,
            quantity,
            price: Some(price),
            decision_id,
            expires_at_ms: ctx.now_ms
                + i64::try_from(ApprovedOrder::VALIDITY.as_millis()).unwrap_or(i64::MAX),
        })
    }
}

/// `NSE:SYMBOL` / `BSE:SYMBOL` / bare symbols; F&O instrument shapes
/// (month codes) are refused.
fn symbol_starts_valid(symbol: &str) -> bool {
    // Cash-equity symbols are letters plus &, - and _; digits inside the
    // body (strike prices, expiry codes) mark derivatives.
    let bare = symbol.split(':').next_back().unwrap_or(symbol);
    !bare.is_empty()
        && bare
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '&' || c == '-' || c == '_')
}

/// A deterministic decision id from the order's facts: time, symbol and
/// quantity. Collisions in practice would need the same order twice in the
/// same millisecond.
fn decision_counter(now_ms: i64, symbol: &str, quantity: i64) -> u64 {
    let mut hash = u64::from_ne_bytes(now_ms.to_ne_bytes());
    for b in symbol.as_bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(*b));
    }
    hash ^ u64::from_ne_bytes(quantity.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> RiskEngine {
        RiskEngine::new(RiskLimits::default())
    }

    fn ctx(now_ms: i64) -> RiskContext<'static> {
        RiskContext {
            symbol: "NSE:RELIANCE",
            ltp: 1_000.0,
            open_positions: 0,
            recent_orders: &[],
            day_pnl: 0.0,
            now_ms,
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn a_normal_order_is_approved_and_carries_the_facts() {
        let approved = engine()
            .check("NSE:RELIANCE", Side::Buy, 10, Some(1_000.0), &ctx(NOW))
            .unwrap();
        assert_eq!(approved.symbol(), "NSE:RELIANCE");
        assert_eq!(approved.side(), Side::Buy);
        assert_eq!(approved.quantity(), 10);
        assert_eq!(approved.price(), Some(1_000.0));
        assert!(approved.is_valid_at(NOW + 1000));
        assert!(
            !approved.is_valid_at(NOW + 6 * 60_000),
            "expires in 5 minutes"
        );
    }

    #[test]
    fn a_sell_carries_its_side() {
        let approved = engine()
            .check("NSE:RELIANCE", Side::Sell, 10, None, &ctx(NOW))
            .unwrap();
        assert_eq!(approved.side(), Side::Sell);
        assert_eq!(approved.price(), Some(1_000.0), "priced at the LTP");
    }

    #[test]
    fn hard_limits_reject() {
        let e = engine();
        assert!(
            e.check("NSE:REL", Side::Buy, 0, None, &ctx(NOW)).is_err(),
            "zero qty"
        );
        assert!(
            e.check("NSE:REL", Side::Buy, 5_000, None, &ctx(NOW))
                .is_err(),
            "over the quantity cap"
        );
        assert!(
            e.check("NSE:REL", Side::Buy, 1_000, Some(1_000.0), &ctx(NOW))
                .is_err(),
            "value 1M over the ₹500k cap"
        );
        assert!(
            e.check("NSE:REL", Side::Buy, 100, Some(1_500.0), &ctx(NOW))
                .is_err(),
            "50% above LTP is outside the 10% band"
        );
        assert!(
            e.check("NSE:REL", Side::Buy, 100, Some(900.0), &ctx(NOW))
                .is_ok(),
            "10% below LTP is inside the band"
        );
    }

    #[test]
    fn the_rate_and_position_limits_reject() {
        let e = engine();
        // Ten orders in the last minute: the 11th is refused.
        let recent: Vec<i64> = (0..10).map(|i| NOW - i64::from(i) * 1_000).collect();
        let busy = RiskContext {
            recent_orders: &recent,
            ..ctx(NOW)
        };
        assert!(e.check("NSE:REL", Side::Buy, 1, None, &busy).is_err());
        // Outside the minute window they no longer count.
        let old: Vec<i64> = recent.iter().map(|t| t - 120_000).collect();
        let calm = RiskContext {
            recent_orders: &old,
            ..ctx(NOW)
        };
        assert!(e.check("NSE:REL", Side::Buy, 1, None, &calm).is_ok());

        // Twenty open positions: the 21st is refused.
        let full = RiskContext {
            open_positions: 20,
            ..ctx(NOW)
        };
        assert!(e.check("NSE:REL", Side::Buy, 1, None, &full).is_err());

        // A deep daily loss refuses everything new.
        let losing = RiskContext {
            day_pnl: -50_000.0,
            ..ctx(NOW)
        };
        assert!(e.check("NSE:REL", Side::Buy, 1, None, &losing).is_err());
    }

    #[test]
    fn non_equity_shapes_are_refused() {
        let e = engine();
        assert!(
            e.check("NSE:NIFTY25NOV24000CE", Side::Buy, 1, None, &ctx(NOW))
                .is_err()
        );
        assert!(
            e.check("NSE:RELIANCE", Side::Buy, 1, None, &ctx(NOW))
                .is_ok()
        );
    }
}
