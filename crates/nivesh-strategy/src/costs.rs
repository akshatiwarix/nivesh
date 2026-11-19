//! The cost model: statutory charges, brokerage and slippage, applied to
//! every simulated fill. Backtests and paper fills use the same model, so a
//! strategy is always judged net of costs.
//!
//! The schedule is data, not code: [`ChargeSchedule`] holds the rates and
//! the defaults approximate Zerodha's published equity charges
//! (zerodha.com/charges, checked November 2026). Rates change; the values
//! here are approximations for simulation, not a broker's contract note.
//!
//! Charges are computed in integer paise. Each component is rounded to the
//! nearest paisa on its own, the way a contract note itemises them.

use nivesh_core::Side;
use serde::{Deserialize, Serialize};

/// The rates and caps of one product's charges. Rates are fractions of
/// turnover (`0.001` = 0.1%).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChargeSchedule {
    /// Brokerage per executed order, as a fraction of its turnover...
    pub brokerage_rate: f64,
    /// ...capped at this many paise per order.
    pub brokerage_cap_paise: i64,
    /// Securities transaction tax on the buy and the sell side.
    pub stt_buy_rate: f64,
    pub stt_sell_rate: f64,
    /// Exchange transaction charges, both sides.
    pub exchange_rate: f64,
    /// SEBI turnover fee, both sides.
    pub sebi_rate: f64,
    /// Stamp duty, buy side only.
    pub stamp_buy_rate: f64,
    /// GST on brokerage, exchange charges and the SEBI fee.
    pub gst_rate: f64,
    /// Depository charge per sell order (GST included), paise.
    pub dp_per_sell_paise: i64,
}

impl ChargeSchedule {
    /// Zerodha equity delivery (CNC): no brokerage, STT 0.1% both sides,
    /// stamp 0.015% on buys, DP ₹15.34 per sell.
    pub const ZERODHA_DELIVERY: Self = Self {
        brokerage_rate: 0.0,
        brokerage_cap_paise: 0,
        stt_buy_rate: 0.001,
        stt_sell_rate: 0.001,
        exchange_rate: 0.000_030_7,
        sebi_rate: 0.000_001,
        stamp_buy_rate: 0.000_15,
        gst_rate: 0.18,
        dp_per_sell_paise: 1_534,
    };

    /// Zerodha equity intraday (MIS): 0.03% or ₹20 per order, whichever is
    /// lower; STT 0.025% on sells; stamp 0.003% on buys; no DP charge.
    pub const ZERODHA_INTRADAY: Self = Self {
        brokerage_rate: 0.000_3,
        brokerage_cap_paise: 2_000,
        stt_buy_rate: 0.0,
        stt_sell_rate: 0.000_25,
        exchange_rate: 0.000_030_7,
        sebi_rate: 0.000_001,
        stamp_buy_rate: 0.000_03,
        gst_rate: 0.18,
        dp_per_sell_paise: 0,
    };

    /// No charges at all, for tests that isolate the strategy logic.
    pub const NONE: Self = Self {
        brokerage_rate: 0.0,
        brokerage_cap_paise: 0,
        stt_buy_rate: 0.0,
        stt_sell_rate: 0.0,
        exchange_rate: 0.0,
        sebi_rate: 0.0,
        stamp_buy_rate: 0.0,
        gst_rate: 0.0,
        dp_per_sell_paise: 0,
    };
}

/// One fill's charges, itemised, in paise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Charges {
    pub brokerage: i64,
    pub stt: i64,
    pub exchange: i64,
    pub sebi: i64,
    pub stamp: i64,
    pub gst: i64,
    pub dp: i64,
}

impl Charges {
    pub fn total_paise(&self) -> i64 {
        self.brokerage + self.stt + self.exchange + self.sebi + self.stamp + self.gst + self.dp
    }

    /// The total in rupees, for P&L arithmetic.
    pub fn total_rupees(&self) -> f64 {
        self.total_paise() as f64 / 100.0
    }
}

/// Charges plus slippage: what a simulated fill costs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostModel {
    pub schedule: ChargeSchedule,
    /// Adverse slippage as a fraction of the price (`0.0005` = 5 bps):
    /// buys fill this much higher, sells this much lower.
    pub slippage: f64,
}

impl Default for CostModel {
    /// Delivery charges (strategies hold across sessions) and 5 bps of
    /// slippage.
    fn default() -> Self {
        Self {
            schedule: ChargeSchedule::ZERODHA_DELIVERY,
            slippage: 0.000_5,
        }
    }
}

impl CostModel {
    /// No charges and no slippage.
    pub const FREE: Self = Self {
        schedule: ChargeSchedule::NONE,
        slippage: 0.0,
    };

    /// The price a fill at `price` actually gets, after slippage (rupees).
    pub fn fill_price(&self, side: Side, price: f64) -> f64 {
        match side {
            Side::Buy => price * (1.0 + self.slippage),
            Side::Sell => price * (1.0 - self.slippage),
        }
    }

    /// The charges on one fill of `quantity` shares at `price` rupees.
    pub fn charges(&self, side: Side, quantity: i64, price: f64) -> Charges {
        let s = &self.schedule;
        let turnover = (price * 100.0).round() * quantity as f64; // paise
        let pct = |rate: f64| (turnover * rate).round() as i64;
        let brokerage = pct(s.brokerage_rate).min(s.brokerage_cap_paise);
        let exchange = pct(s.exchange_rate);
        let sebi = pct(s.sebi_rate);
        let (stt, stamp, dp) = match side {
            Side::Buy => (pct(s.stt_buy_rate), pct(s.stamp_buy_rate), 0),
            Side::Sell => (pct(s.stt_sell_rate), 0, s.dp_per_sell_paise),
        };
        let gst = ((brokerage + exchange + sebi) as f64 * s.gst_rate).round() as i64;
        Charges {
            brokerage,
            stt,
            exchange,
            sebi,
            stamp,
            gst,
            dp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_charges_match_a_hand_computed_round_trip() {
        // 100 shares at ₹1,000: turnover ₹1,00,000 = 1,00,00,000 paise.
        let model = CostModel {
            slippage: 0.0,
            ..CostModel::default()
        };
        let buy = model.charges(Side::Buy, 100, 1_000.0);
        assert_eq!(
            buy,
            Charges {
                brokerage: 0,
                stt: 10_000,   // 0.1%       = ₹100
                exchange: 307, // 0.00307%   = ₹3.07
                sebi: 10,      // ₹10/crore  = ₹0.10
                stamp: 1_500,  // 0.015%     = ₹15
                gst: 57,       // 18% of ₹3.17 = ₹0.5706
                dp: 0,
            }
        );
        assert_eq!(buy.total_paise(), 11_874);
        let sell = model.charges(Side::Sell, 100, 1_000.0);
        assert_eq!(
            (sell.stt, sell.stamp, sell.dp, sell.total_paise()),
            (10_000, 0, 1_534, 11_908)
        );
    }

    #[test]
    fn intraday_brokerage_is_capped_at_twenty_rupees() {
        let model = CostModel {
            schedule: ChargeSchedule::ZERODHA_INTRADAY,
            slippage: 0.0,
        };
        // ₹1,00,000 turnover: 0.03% = ₹30, capped at ₹20.
        let big = model.charges(Side::Sell, 100, 1_000.0);
        assert_eq!(big.brokerage, 2_000);
        assert_eq!(big.stt, 2_500); // 0.025% on the sell side
        assert_eq!(big.dp, 0);
        // GST: 18% of (₹20 + ₹3.07 + ₹0.10) = ₹4.1706.
        assert_eq!(big.gst, 417);
        // ₹10,000 turnover: 0.03% = ₹3, under the cap.
        let small = model.charges(Side::Buy, 10, 1_000.0);
        assert_eq!((small.brokerage, small.stt, small.stamp), (300, 0, 30));
    }

    #[test]
    fn slippage_always_hurts() {
        let model = CostModel::default();
        assert!(model.fill_price(Side::Buy, 100.0) > 100.0);
        assert!(model.fill_price(Side::Sell, 100.0) < 100.0);
        assert_eq!(CostModel::FREE.fill_price(Side::Buy, 100.0), 100.0);
        assert_eq!(
            CostModel::FREE.charges(Side::Sell, 10, 100.0).total_paise(),
            0
        );
    }

    #[test]
    fn a_schedule_round_trips_as_data() {
        let json = serde_json::to_string(&CostModel::default()).unwrap();
        let back: CostModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CostModel::default());
    }
}
