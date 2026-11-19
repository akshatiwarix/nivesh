//! The pre-trade risk engine.
//!
//! **One gate, enforced by types:** an [`ApprovedOrder`] cannot be
//! constructed outside this crate (its fields are private, it cannot be
//! deserialised, and only [`RiskEngine::check`] mints one), so nothing
//! downstream can trade without the engine's decision.
//!
//! The limits are PLAN.md §11.4's hard limits: per-order quantity and value,
//! per-symbol exposure, open positions, orders per minute and per day, daily
//! loss (realised plus MTM), a price band around the LTP, allowed products
//! and segments (cash equity only), and a trading window. Money is integer
//! paise. A strategy may carry its own limits, which can only make the
//! global ones tighter ([`RiskLimits::tightened_by`]).
//!
//! Orders that **reduce** a position (selling shares the strategy holds) are
//! not subject to the limits that guard against adding risk: open
//! positions, symbol exposure, daily loss and the trading window. A strategy
//! must always be able to get out. Quantity, value, rate, segment and price
//! band still apply to them.
//!
//! Everything is pure: no I/O and no clock of its own. The caller supplies
//! `now_ms`, so the engine is deterministic and property-testable.

use nivesh_core::Side;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The current UTC time in milliseconds; tests inject a fixed clock.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// IST is UTC+05:30. Days, and the trading window, are Indian.
const IST_OFFSET_MS: i64 = 19_800_000;
const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

/// Rupees to paise, rounded to the nearest paisa.
pub fn paise(rupees: f64) -> i64 {
    (rupees * 100.0).round() as i64
}

/// Which margin product an order uses. Cash equity only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Product {
    /// Delivery (CNC): held across sessions.
    Delivery,
    /// Intraday (MIS): squared off the same day.
    Intraday,
}

/// The part of the IST day new risk may be added in, as minutes after IST
/// midnight, `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradingWindow {
    pub start_min: u16,
    pub end_min: u16,
}

impl TradingWindow {
    /// NSE's cash session: 09:15 to 15:30 IST.
    pub const NSE_SESSION: Self = Self {
        start_min: 9 * 60 + 15,
        end_min: 15 * 60 + 30,
    };

    pub fn contains(&self, now_ms: i64) -> bool {
        let minute = (now_ms + IST_OFFSET_MS).rem_euclid(DAY_MS) / MINUTE_MS;
        (i64::from(self.start_min)..i64::from(self.end_min)).contains(&minute)
    }

    /// The overlap of two windows (empty when they don't overlap).
    fn intersect(self, other: Self) -> Self {
        let start_min = self.start_min.max(other.start_min);
        Self {
            start_min,
            end_min: self.end_min.min(other.end_min).max(start_min),
        }
    }

    /// An empty window allows nothing, so it is within any window.
    fn is_within(&self, outer: &Self) -> bool {
        self.start_min >= self.end_min
            || (self.start_min >= outer.start_min && self.end_min <= outer.end_min)
    }
}

/// The limits every order is checked against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskLimits {
    /// Maximum shares in one order.
    pub max_order_quantity: i64,
    /// Maximum notional of one order, paise.
    pub max_order_value_paise: i64,
    /// Maximum notional held in one symbol after the order, paise.
    pub max_symbol_exposure_paise: i64,
    /// Maximum open positions.
    pub max_open_positions: usize,
    /// Orders per rolling minute.
    pub max_orders_per_minute: u32,
    /// Orders per IST day.
    pub max_orders_per_day: u32,
    /// The loss (realised plus MTM, paise, a positive number) at which new
    /// risk is refused. The kill switch auto-engages on it too.
    pub max_daily_loss_paise: i64,
    /// A limit price must sit within this fraction of the LTP (`0.1` = 10%).
    pub price_band: f64,
    /// The products orders may use.
    pub allowed_products: Vec<Product>,
    /// When new risk may be added; `None` means at any time (simulations
    /// over synthetic clocks).
    pub trading_window: Option<TradingWindow>,
}

impl Default for RiskLimits {
    /// Conservative defaults for a small retail account.
    fn default() -> Self {
        Self {
            max_order_quantity: 1_000,
            max_order_value_paise: 500_000 * 100, // ₹5,00,000
            max_symbol_exposure_paise: 1_000_000 * 100, // ₹10,00,000
            max_open_positions: 20,
            max_orders_per_minute: 10,
            max_orders_per_day: 200,
            max_daily_loss_paise: 50_000 * 100, // ₹50,000
            price_band: 0.10,
            allowed_products: vec![Product::Delivery],
            trading_window: Some(TradingWindow::NSE_SESSION),
        }
    }
}

impl RiskLimits {
    /// The limits in force when a strategy's own `strategy` limits apply on
    /// top of these: every number is the smaller, products the common ones,
    /// the window the overlap. The result is never looser than `self`.
    pub fn tightened_by(&self, strategy: &RiskLimits) -> RiskLimits {
        RiskLimits {
            max_order_quantity: self.max_order_quantity.min(strategy.max_order_quantity),
            max_order_value_paise: self
                .max_order_value_paise
                .min(strategy.max_order_value_paise),
            max_symbol_exposure_paise: self
                .max_symbol_exposure_paise
                .min(strategy.max_symbol_exposure_paise),
            max_open_positions: self.max_open_positions.min(strategy.max_open_positions),
            max_orders_per_minute: self
                .max_orders_per_minute
                .min(strategy.max_orders_per_minute),
            max_orders_per_day: self.max_orders_per_day.min(strategy.max_orders_per_day),
            max_daily_loss_paise: self.max_daily_loss_paise.min(strategy.max_daily_loss_paise),
            price_band: if strategy.price_band.is_finite() {
                self.price_band.min(strategy.price_band)
            } else {
                self.price_band
            },
            allowed_products: self
                .allowed_products
                .iter()
                .copied()
                .filter(|p| strategy.allowed_products.contains(p))
                .collect(),
            trading_window: match (self.trading_window, strategy.trading_window) {
                (Some(a), Some(b)) => Some(a.intersect(b)),
                (a, b) => a.or(b),
            },
        }
    }

    /// Whether these limits are nowhere looser than `outer`. Loosening a
    /// limit is a deliberate, confirmed and audited act; this tells the
    /// caller which kind of change it is looking at.
    pub fn is_within(&self, outer: &RiskLimits) -> bool {
        self.max_order_quantity <= outer.max_order_quantity
            && self.max_order_value_paise <= outer.max_order_value_paise
            && self.max_symbol_exposure_paise <= outer.max_symbol_exposure_paise
            && self.max_open_positions <= outer.max_open_positions
            && self.max_orders_per_minute <= outer.max_orders_per_minute
            && self.max_orders_per_day <= outer.max_orders_per_day
            && self.max_daily_loss_paise <= outer.max_daily_loss_paise
            && self.price_band <= outer.price_band
            && self
                .allowed_products
                .iter()
                .all(|p| outer.allowed_products.contains(p))
            && match (self.trading_window, outer.trading_window) {
                (Some(inner), Some(outer)) => inner.is_within(&outer),
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => true,
            }
    }
}

/// Which hard limit refused an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskCheck {
    Quantity,
    OrderValue,
    Price,
    PriceBand,
    Segment,
    Product,
    TradingWindow,
    OpenPositions,
    SymbolExposure,
    DailyLoss,
    OrdersPerMinute,
    OrdersPerDay,
}

/// A refusal: which check, and why in plain language (for the audit log
/// and the UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{reason}")]
pub struct RiskRejection {
    pub check: RiskCheck,
    pub reason: String,
}

/// What is being asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRequest {
    /// The instrument, `NSE:RELIANCE` shaped.
    pub symbol: String,
    pub side: Side,
    pub quantity: i64,
    /// The limit price, rupees; `None` prices it at the LTP.
    pub price: Option<f64>,
    pub product: Product,
}

/// What the engine knows when it checks an order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskContext<'a> {
    /// The last traded price, rupees.
    pub ltp: f64,
    /// Shares the strategy holds in this symbol (long-only: zero or more).
    pub position_quantity: i64,
    /// Notional the account holds in this symbol now, paise.
    pub symbol_exposure_paise: i64,
    /// Positions open across the account.
    pub open_positions: usize,
    /// Timestamps (ms) of recent approvals, any order.
    pub recent_orders: &'a [i64],
    /// Today's P&L, realised plus MTM, paise (negative is a loss).
    pub day_pnl_paise: i64,
    /// The strategy's own limits, if it has any (they can only tighten).
    pub strategy_limits: Option<&'a RiskLimits>,
    /// Now, UTC ms.
    pub now_ms: i64,
}

/// The risk engine: the global limits and pure checks.
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
    /// The approved price, rupees (the limit, or the LTP it was checked at).
    price: Option<f64>,
    product: Product,
    /// The engine's decision id, linking it to the audit log.
    decision_id: u64,
    /// When the approval dies (5 minutes out).
    expires_at_ms: i64,
}

impl ApprovedOrder {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn quantity(&self) -> i64 {
        self.quantity
    }

    pub fn price(&self) -> Option<f64> {
        self.price
    }

    pub fn product(&self) -> Product {
        self.product
    }

    pub fn decision_id(&self) -> u64 {
        self.decision_id
    }

    pub fn is_valid_at(&self, now_ms: i64) -> bool {
        now_ms <= self.expires_at_ms
    }

    /// How long the approval is valid for, from minting.
    pub const VALIDITY: Duration = Duration::from_secs(5 * 60);
}

impl fmt::Display for Product {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Product::Delivery => "delivery",
            Product::Intraday => "intraday",
        })
    }
}

fn rupees(paise: i64) -> String {
    format!("₹{:.0}", paise as f64 / 100.0)
}

impl RiskEngine {
    pub fn new(limits: RiskLimits) -> Self {
        Self { limits }
    }

    /// Checks the order against every hard limit (the global ones, tightened
    /// by the strategy's) and, on a pass, mints the approval.
    pub fn check(
        &self,
        order: &OrderRequest,
        ctx: &RiskContext<'_>,
    ) -> Result<ApprovedOrder, RiskRejection> {
        let tightened;
        let limits = match ctx.strategy_limits {
            Some(strategy) => {
                tightened = self.limits.tightened_by(strategy);
                &tightened
            }
            None => &self.limits,
        };
        let reject = |check: RiskCheck, reason: String| Err(RiskRejection { check, reason });
        let quantity = order.quantity;
        // Long-only: a sell reduces when the strategy holds that many shares.
        let reduces = order.side == Side::Sell && ctx.position_quantity >= quantity;

        // Segment and product.
        if !is_cash_equity(&order.symbol) {
            return reject(
                RiskCheck::Segment,
                format!("{} is not a cash-equity symbol", order.symbol),
            );
        }
        if !limits.allowed_products.contains(&order.product) {
            return reject(
                RiskCheck::Product,
                format!("{} orders are not allowed", order.product),
            );
        }
        if order.side == Side::Sell && !reduces {
            return reject(
                RiskCheck::Quantity,
                format!(
                    "selling {quantity} needs that many shares held; {} are (no short selling)",
                    ctx.position_quantity
                ),
            );
        }

        // Quantity, price and value.
        if quantity <= 0 {
            return reject(RiskCheck::Quantity, "quantity must be positive".into());
        }
        if quantity > limits.max_order_quantity {
            return reject(
                RiskCheck::Quantity,
                format!(
                    "quantity {quantity} exceeds the per-order cap {}",
                    limits.max_order_quantity
                ),
            );
        }
        let price = order.price.unwrap_or(ctx.ltp);
        if !price.is_finite() || price <= 0.0 || !ctx.ltp.is_finite() || ctx.ltp <= 0.0 {
            return reject(
                RiskCheck::Price,
                "order price and LTP must be positive".into(),
            );
        }
        if (price - ctx.ltp).abs() / ctx.ltp > limits.price_band {
            return reject(
                RiskCheck::PriceBand,
                format!(
                    "price ₹{price:.2} is outside the {:.0}% band around LTP ₹{:.2}",
                    limits.price_band * 100.0,
                    ctx.ltp
                ),
            );
        }
        let value = paise(price).saturating_mul(quantity);
        if value > limits.max_order_value_paise {
            return reject(
                RiskCheck::OrderValue,
                format!(
                    "order value {} exceeds the per-order cap {}",
                    rupees(value),
                    rupees(limits.max_order_value_paise)
                ),
            );
        }

        // Limits on adding risk.
        if !reduces {
            if let Some(window) = limits.trading_window
                && !window.contains(ctx.now_ms)
            {
                return reject(
                    RiskCheck::TradingWindow,
                    format!(
                        "outside the trading window {:02}:{:02}–{:02}:{:02} IST",
                        window.start_min / 60,
                        window.start_min % 60,
                        window.end_min / 60,
                        window.end_min % 60
                    ),
                );
            }
            if ctx.day_pnl_paise <= -limits.max_daily_loss_paise {
                return reject(
                    RiskCheck::DailyLoss,
                    format!(
                        "today's loss ({}) has reached the limit ({})",
                        rupees(-ctx.day_pnl_paise),
                        rupees(limits.max_daily_loss_paise)
                    ),
                );
            }
            if ctx.position_quantity == 0 && ctx.open_positions >= limits.max_open_positions {
                return reject(
                    RiskCheck::OpenPositions,
                    format!(
                        "{} positions are open, the maximum",
                        limits.max_open_positions
                    ),
                );
            }
            let exposure = ctx.symbol_exposure_paise.saturating_add(value);
            if exposure > limits.max_symbol_exposure_paise {
                return reject(
                    RiskCheck::SymbolExposure,
                    format!(
                        "exposure to {} would be {}, over the cap {}",
                        order.symbol,
                        rupees(exposure),
                        rupees(limits.max_symbol_exposure_paise)
                    ),
                );
            }
        }

        // Order rate: per rolling minute and per IST day, for every order.
        let minute_ago = ctx.now_ms - MINUTE_MS;
        let in_window = |from: i64| {
            ctx.recent_orders
                .iter()
                .filter(|t| **t > from && **t <= ctx.now_ms)
                .count()
        };
        if in_window(minute_ago) >= limits.max_orders_per_minute as usize {
            return reject(
                RiskCheck::OrdersPerMinute,
                format!(
                    "the per-minute order cap ({}) is reached; wait a moment",
                    limits.max_orders_per_minute
                ),
            );
        }
        let day_start = ist_day_start(ctx.now_ms);
        if in_window(day_start - 1) >= limits.max_orders_per_day as usize {
            return reject(
                RiskCheck::OrdersPerDay,
                format!(
                    "the per-day order cap ({}) is reached",
                    limits.max_orders_per_day
                ),
            );
        }

        Ok(ApprovedOrder {
            symbol: order.symbol.clone(),
            side: order.side,
            quantity,
            price: Some(price),
            product: order.product,
            decision_id: decision_id(ctx.now_ms, &order.symbol, quantity),
            expires_at_ms: ctx.now_ms
                + i64::try_from(ApprovedOrder::VALIDITY.as_millis()).unwrap_or(i64::MAX),
        })
    }
}

/// UTC ms of the IST midnight that starts `now_ms`'s IST day.
fn ist_day_start(now_ms: i64) -> i64 {
    let ist = now_ms + IST_OFFSET_MS;
    ist - ist.rem_euclid(DAY_MS) - IST_OFFSET_MS
}

/// `NSE:SYMBOL` / `BSE:SYMBOL` / bare symbols. Cash-equity symbols are
/// letters plus `&`, `-` and `_`; digits (strikes, expiry codes) mark
/// derivatives.
fn is_cash_equity(symbol: &str) -> bool {
    let bare = symbol.split(':').next_back().unwrap_or(symbol);
    !bare.is_empty()
        && bare
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '&' || c == '-' || c == '_')
}

/// A deterministic decision id from the order's facts: time, symbol and
/// quantity.
fn decision_id(now_ms: i64, symbol: &str, quantity: i64) -> u64 {
    let mut hash = u64::from_ne_bytes(now_ms.to_ne_bytes());
    for b in symbol.as_bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(*b));
    }
    hash ^ u64::from_ne_bytes(quantity.to_ne_bytes())
}

#[cfg(test)]
mod tests;
