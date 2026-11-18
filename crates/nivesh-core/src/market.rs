//! Market data value types: prices, quotes and candles.
//!
//! Money that must be exact (orders, positions, P&L) uses [`Price`], integer
//! paise. Display-only market data (quotes, candles, index levels) uses `f64`
//! rupees, which is what providers return and what charts want.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::index::IndexId;
use crate::instrument::InstrumentId;

/// An amount of money in integer paise (1/100 rupee).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Price(pub i64);

impl Price {
    pub const ZERO: Price = Price(0);

    /// Rounds a rupee amount to the nearest paisa. Non-finite input is zero.
    pub fn from_rupees(rupees: f64) -> Self {
        if !rupees.is_finite() {
            return Self::ZERO;
        }
        // Rounded and saturating: `as` on a finite f64 clamps to the i64 range.
        Price((rupees * 100.0).round() as i64)
    }

    pub const fn paise(self) -> i64 {
        self.0
    }

    pub fn rupees(self) -> f64 {
        self.0 as f64 / 100.0
    }
}

impl fmt::Display for Price {
    /// `1234.50`, `-0.05`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        write!(f, "{sign}{}.{:02}", abs / 100, abs % 100)
    }
}

/// A candle width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Interval {
    #[serde(rename = "1m")]
    M1,
    #[serde(rename = "5m")]
    M5,
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "30m")]
    M30,
    #[serde(rename = "1h")]
    H1,
    /// Not offered by any provider: built from 1h bars (see [`Interval::base`]).
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "1d")]
    D1,
    #[serde(rename = "1w")]
    W1,
    #[serde(rename = "1mo")]
    Mo1,
}

impl Interval {
    pub const ALL: [Interval; 9] = [
        Interval::M1,
        Interval::M5,
        Interval::M15,
        Interval::M30,
        Interval::H1,
        Interval::H4,
        Interval::D1,
        Interval::W1,
        Interval::Mo1,
    ];

    pub const fn code(self) -> &'static str {
        match self {
            Interval::M1 => "1m",
            Interval::M5 => "5m",
            Interval::M15 => "15m",
            Interval::M30 => "30m",
            Interval::H1 => "1h",
            Interval::H4 => "4h",
            Interval::D1 => "1d",
            Interval::W1 => "1w",
            Interval::Mo1 => "1mo",
        }
    }

    /// Nominal width in seconds (a week is 7 days, a month 30).
    pub const fn seconds(self) -> i64 {
        match self {
            Interval::M1 => 60,
            Interval::M5 => 300,
            Interval::M15 => 900,
            Interval::M30 => 1800,
            Interval::H1 => 3600,
            Interval::H4 => 14_400,
            Interval::D1 => 86_400,
            Interval::W1 => 604_800,
            Interval::Mo1 => 2_592_000,
        }
    }

    pub const fn is_intraday(self) -> bool {
        !matches!(self, Interval::D1 | Interval::W1 | Interval::Mo1)
    }

    /// The interval a provider is asked for. Providers offer 1m to 1h, 1d, 1w
    /// and 1mo; `4h` is built from their 1h bars, aligned to the IST session.
    pub const fn base(self) -> Interval {
        match self {
            Interval::H4 => Interval::H1,
            other => other,
        }
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Interval {
    type Err = String;

    /// Accepts `5m`, `5M`, `1h`, `4h`, `1d`, `D`, `W`, `1mo`, `MO`,
    /// case-insensitively. Months are `mo` because `m` means minutes.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_ascii_lowercase();
        let s = match s.as_str() {
            "d" => "1d",
            "w" => "1w",
            "h" => "1h",
            "mo" | "mn" => "1mo",
            other => other,
        };
        Interval::ALL
            .into_iter()
            .find(|i| i.code() == s)
            .ok_or_else(|| format!("unknown interval `{s}`"))
    }
}

/// A stock quote. Prices are rupees. Optional fields are `None` when the
/// provider doesn't supply them (Yahoo has no price bands, for example).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub id: InstrumentId,
    pub name: Option<String>,
    pub ltp: f64,
    pub prev_close: f64,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub volume: Option<u64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    /// Circuit limits, where the provider reports them.
    pub upper_band: Option<f64>,
    pub lower_band: Option<f64>,
}

/// Absolute and percentage change against the previous close. Percent is 0
/// when there is no usable previous close.
pub fn change_from(ltp: f64, prev_close: f64) -> (f64, f64) {
    let change = ltp - prev_close;
    let pct = if prev_close.abs() > f64::EPSILON {
        change / prev_close * 100.0
    } else {
        0.0
    };
    (change, pct)
}

impl Quote {
    pub fn change(&self) -> f64 {
        change_from(self.ltp, self.prev_close).0
    }

    pub fn change_pct(&self) -> f64 {
        change_from(self.ltp, self.prev_close).1
    }
}

/// An index level, the index analogue of [`Quote`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexQuote {
    pub index: IndexId,
    pub last: f64,
    pub prev_close: f64,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    pub advances: Option<u32>,
    pub declines: Option<u32>,
}

impl IndexQuote {
    pub fn change(&self) -> f64 {
        change_from(self.last, self.prev_close).0
    }

    pub fn change_pct(&self) -> f64 {
        change_from(self.last, self.prev_close).1
    }
}

/// Where a constituent *list* (the membership, not the prices) came from, when
/// it is not the live source's own list: a dated file bundled with the app or
/// last downloaded from the index publisher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstituentsInfo {
    /// The date the list was taken, `YYYY-MM-DD`.
    pub as_of: String,
    /// Who published it, e.g. `niftyindices.com`.
    pub source: String,
}

/// An index level plus its constituents (empty when the provider only has the
/// level, as Yahoo does).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexSnapshot {
    pub quote: IndexQuote,
    pub constituents: Vec<Quote>,
    /// `Some` when the membership came from a dated list rather than from the
    /// provider that supplied the prices. Old cache entries lack it.
    #[serde(default)]
    pub constituents_info: Option<ConstituentsInfo>,
}

/// One OHLCV bar. `t` is the bar's start, UTC epoch milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
}

/// Candles for one instrument at one interval, oldest first. A series always
/// comes from a single provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandleSeries {
    pub id: InstrumentId,
    pub interval: Interval,
    pub candles: Vec<Candle>,
}

impl CandleSeries {
    /// True when timestamps strictly increase.
    pub fn is_ordered(&self) -> bool {
        self.candles.windows(2).all(|w| w[0].t < w[1].t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Exchange;

    #[test]
    fn price_rounds_to_paise() {
        assert_eq!(Price::from_rupees(1234.5), Price(123_450));
        assert_eq!(Price::from_rupees(0.005), Price(1));
        assert_eq!(Price::from_rupees(-0.05), Price(-5));
        assert_eq!(Price::from_rupees(f64::NAN), Price::ZERO);
        assert_eq!(Price::from_rupees(f64::INFINITY), Price::ZERO);
        assert_eq!(Price(123_450).rupees(), 1234.5);
    }

    #[test]
    fn price_displays_with_two_decimals_and_sign() {
        assert_eq!(Price(123_450).to_string(), "1234.50");
        assert_eq!(Price(-5).to_string(), "-0.05");
        assert_eq!(Price(0).to_string(), "0.00");
        assert_eq!(Price(i64::MIN).to_string(), "-92233720368547758.08");
    }

    #[test]
    fn interval_parses_codes_and_shorthands() {
        for (text, want) in [
            ("5m", Interval::M5),
            ("5M", Interval::M5),
            ("1h", Interval::H1),
            ("H", Interval::H1),
            ("D", Interval::D1),
            ("1d", Interval::D1),
            ("w", Interval::W1),
            ("4H", Interval::H4),
            ("1mo", Interval::Mo1),
            ("MO", Interval::Mo1),
        ] {
            assert_eq!(text.parse::<Interval>(), Ok(want), "{text}");
        }
        assert!("7m".parse::<Interval>().is_err());
        assert!(Interval::M5.is_intraday() && !Interval::D1.is_intraday());
        assert!(!Interval::Mo1.is_intraday());
        assert_eq!(Interval::H4.base(), Interval::H1);
        assert_eq!(Interval::D1.base(), Interval::D1);
    }

    #[test]
    fn interval_serialises_as_its_code() {
        assert_eq!(serde_json::to_string(&Interval::M15).unwrap(), "\"15m\"");
        assert_eq!(
            serde_json::from_str::<Interval>("\"1d\"").unwrap(),
            Interval::D1
        );
    }

    #[test]
    fn change_handles_missing_previous_close() {
        let (c, p) = change_from(110.0, 100.0);
        assert_eq!((c, p), (10.0, 10.0));
        assert_eq!(change_from(5.0, 0.0), (5.0, 0.0));
    }

    #[test]
    fn quote_round_trips_through_json() {
        let q = Quote {
            id: InstrumentId::new(Exchange::Nse, "RELIANCE").unwrap(),
            name: Some("Reliance Industries".into()),
            ltp: 2500.0,
            prev_close: 2450.0,
            open: Some(2460.0),
            high: None,
            low: None,
            volume: Some(10),
            week52_high: None,
            week52_low: None,
            upper_band: None,
            lower_band: None,
        };
        let back: Quote = serde_json::from_str(&serde_json::to_string(&q).unwrap()).unwrap();
        assert_eq!(back, q);
        assert!((q.change_pct() - 2.0408).abs() < 1e-3);
    }

    #[test]
    fn series_order_check() {
        let c = |t| Candle {
            t,
            o: 1.0,
            h: 1.0,
            l: 1.0,
            c: 1.0,
            v: 0.0,
        };
        let id = InstrumentId::new(Exchange::Nse, "TCS").unwrap();
        let mut s = CandleSeries {
            id,
            interval: Interval::D1,
            candles: vec![c(1), c(2)],
        };
        assert!(s.is_ordered());
        s.candles.push(c(2));
        assert!(!s.is_ordered());
    }
}
