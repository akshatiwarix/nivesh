//! Exchanges and instrument identifiers.
//!
//! An [`InstrumentId`] is an exchange plus a trading symbol. Its canonical text
//! form is `EXCH:SYMBOL` (for example `NSE:RELIANCE`). When parsing user input
//! the exchange prefix is optional and defaults to NSE, so `RELIANCE`,
//! `nse:reliance` and `NSE:RELIANCE` all name the same instrument.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A stock exchange Nivesh can address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Exchange {
    /// National Stock Exchange of India. The default exchange.
    Nse,
    /// BSE (formerly Bombay Stock Exchange).
    Bse,
}

impl Exchange {
    /// The exchange used when user input has no `EXCH:` prefix.
    pub const DEFAULT: Exchange = Exchange::Nse;

    /// Every supported exchange, in display order.
    pub const ALL: [Exchange; 2] = [Exchange::Nse, Exchange::Bse];

    /// Upper-case code, as used in the `EXCH:SYMBOL` form.
    pub const fn code(self) -> &'static str {
        match self {
            Exchange::Nse => "NSE",
            Exchange::Bse => "BSE",
        }
    }
}

impl Default for Exchange {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Exchange {
    type Err = InstrumentIdError;

    /// Case-insensitive; surrounding whitespace is ignored.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let code = s.trim();
        Exchange::ALL
            .into_iter()
            .find(|exchange| exchange.code().eq_ignore_ascii_case(code))
            .ok_or_else(|| InstrumentIdError::UnknownExchange(code.to_owned()))
    }
}

/// Why a string is not a valid [`InstrumentId`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InstrumentIdError {
    #[error("instrument id is empty")]
    Empty,
    #[error("unknown exchange `{0}` (expected NSE or BSE)")]
    UnknownExchange(String),
    #[error("missing symbol after `{0}:`")]
    MissingSymbol(String),
    #[error("symbol is longer than {max} characters", max = InstrumentId::MAX_SYMBOL_LEN)]
    SymbolTooLong,
    #[error("symbol must start with a letter or digit")]
    InvalidStart,
    #[error("invalid character `{0}` in symbol")]
    InvalidChar(char),
}

/// An exchange-qualified trading symbol, such as `NSE:RELIANCE`.
///
/// Symbols are normalised to upper case and validated on construction, so an
/// `InstrumentId` is always well-formed. Allowed characters are ASCII letters,
/// digits and `& - _ .`, which covers NSE symbols such as `M&M` and
/// `BAJAJ-AUTO` and numeric BSE scrip codes such as `500325`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InstrumentId {
    exchange: Exchange,
    symbol: String,
}

impl InstrumentId {
    /// Longest symbol accepted. Real NSE/BSE symbols are well under this.
    pub const MAX_SYMBOL_LEN: usize = 32;

    /// Builds an id from an exchange and a symbol, normalising the symbol to
    /// upper case.
    pub fn new(exchange: Exchange, symbol: &str) -> Result<Self, InstrumentIdError> {
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return Err(InstrumentIdError::MissingSymbol(exchange.code().to_owned()));
        }
        validate_symbol(symbol)?;
        Ok(Self {
            exchange,
            symbol: symbol.to_ascii_uppercase(),
        })
    }

    /// Parses `SYMBOL` (NSE by default) or `EXCH:SYMBOL`, case-insensitively.
    pub fn parse(input: &str) -> Result<Self, InstrumentIdError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(InstrumentIdError::Empty);
        }
        match input.split_once(':') {
            Some((exchange, symbol)) => {
                let exchange: Exchange = exchange.parse()?;
                if symbol.trim().is_empty() {
                    return Err(InstrumentIdError::MissingSymbol(exchange.code().to_owned()));
                }
                Self::new(exchange, symbol)
            }
            None => Self::new(Exchange::DEFAULT, input),
        }
    }

    pub fn exchange(&self) -> Exchange {
        self.exchange
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }
}

fn validate_symbol(symbol: &str) -> Result<(), InstrumentIdError> {
    let mut chars = symbol.chars();
    // `symbol` is non-empty (checked by the caller), so `first` is Some.
    if let Some(first) = chars.next()
        && !first.is_ascii_alphanumeric()
    {
        return Err(if is_symbol_char(first) {
            InstrumentIdError::InvalidStart
        } else {
            InstrumentIdError::InvalidChar(first)
        });
    }
    if let Some(bad) = chars.find(|&c| !is_symbol_char(c)) {
        return Err(InstrumentIdError::InvalidChar(bad));
    }
    // All characters are ASCII at this point, so bytes == chars.
    if symbol.len() > InstrumentId::MAX_SYMBOL_LEN {
        return Err(InstrumentIdError::SymbolTooLong);
    }
    Ok(())
}

fn is_symbol_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '&' | '-' | '_' | '.')
}

impl fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.exchange, self.symbol)
    }
}

impl FromStr for InstrumentId {
    type Err = InstrumentIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for InstrumentId {
    type Error = InstrumentIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<InstrumentId> for String {
    fn from(id: InstrumentId) -> Self {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(exchange: Exchange, symbol: &str) -> InstrumentId {
        InstrumentId::new(exchange, symbol).unwrap()
    }

    #[test]
    fn parses_valid_ids() {
        let cases = [
            ("RELIANCE", Exchange::Nse, "RELIANCE"),
            ("reliance", Exchange::Nse, "RELIANCE"),
            ("  Reliance  ", Exchange::Nse, "RELIANCE"),
            ("NSE:RELIANCE", Exchange::Nse, "RELIANCE"),
            ("nse:reliance", Exchange::Nse, "RELIANCE"),
            ("BSE:RELIANCE", Exchange::Bse, "RELIANCE"),
            ("bse : reliance", Exchange::Bse, "RELIANCE"),
            ("BSE:500325", Exchange::Bse, "500325"),
            ("M&M", Exchange::Nse, "M&M"),
            ("bajaj-auto", Exchange::Nse, "BAJAJ-AUTO"),
            ("J&KBANK", Exchange::Nse, "J&KBANK"),
            ("MCDOWELL-N", Exchange::Nse, "MCDOWELL-N"),
            ("A_B.C", Exchange::Nse, "A_B.C"),
            ("3MINDIA", Exchange::Nse, "3MINDIA"),
        ];
        for (input, exchange, symbol) in cases {
            let parsed = InstrumentId::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(parsed.exchange(), exchange, "{input}");
            assert_eq!(parsed.symbol(), symbol, "{input}");
        }
    }

    #[test]
    fn rejects_invalid_ids() {
        use InstrumentIdError as E;
        let long = "A".repeat(InstrumentId::MAX_SYMBOL_LEN + 1);
        let cases = [
            ("", E::Empty),
            ("   ", E::Empty),
            ("BSE:", E::MissingSymbol("BSE".into())),
            ("nse:  ", E::MissingSymbol("NSE".into())),
            ("MCX:GOLD", E::UnknownExchange("MCX".into())),
            (":RELIANCE", E::UnknownExchange(String::new())),
            ("-RELIANCE", E::InvalidStart),
            ("&M", E::InvalidStart),
            ("RELI ANCE", E::InvalidChar(' ')),
            ("NSE:RELIANCE:X", E::InvalidChar(':')),
            ("RELIANCE!", E::InvalidChar('!')),
            ("₹RELIANCE", E::InvalidChar('₹')),
            ("RELIANCÉ", E::InvalidChar('É')),
            (long.as_str(), E::SymbolTooLong),
        ];
        for (input, expected) in cases {
            assert_eq!(InstrumentId::parse(input), Err(expected), "{input:?}");
        }
    }

    #[test]
    fn max_length_symbol_is_accepted() {
        let max = "A".repeat(InstrumentId::MAX_SYMBOL_LEN);
        assert!(InstrumentId::parse(&max).is_ok());
    }

    #[test]
    fn display_is_canonical_and_round_trips() {
        let cases = ["RELIANCE", "bse:tcs", "NSE:M&M"];
        for input in cases {
            let parsed = InstrumentId::parse(input).unwrap();
            let text = parsed.to_string();
            assert!(
                text.starts_with("NSE:") || text.starts_with("BSE:"),
                "{text}"
            );
            assert_eq!(text.parse::<InstrumentId>().unwrap(), parsed);
        }
        assert_eq!(id(Exchange::Bse, "tcs").to_string(), "BSE:TCS");
    }

    #[test]
    fn exchange_parsing_is_case_insensitive() {
        assert_eq!("nse".parse::<Exchange>(), Ok(Exchange::Nse));
        assert_eq!(" Bse ".parse::<Exchange>(), Ok(Exchange::Bse));
        assert!("NYSE".parse::<Exchange>().is_err());
        assert_eq!(Exchange::default(), Exchange::Nse);
    }

    #[test]
    fn serde_uses_canonical_string_form() {
        let reliance = id(Exchange::Nse, "RELIANCE");
        assert_eq!(
            serde_json::to_string(&reliance).unwrap(),
            r#""NSE:RELIANCE""#
        );
        assert_eq!(
            serde_json::from_str::<InstrumentId>(r#""bse:reliance""#).unwrap(),
            id(Exchange::Bse, "RELIANCE")
        );
        assert!(serde_json::from_str::<InstrumentId>(r#""BAD SYMBOL""#).is_err());
        assert_eq!(serde_json::to_string(&Exchange::Bse).unwrap(), r#""BSE""#);
    }
}
