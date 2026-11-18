//! The catalogue of indices Nivesh knows about.
//!
//! Indices are a closed set because each one needs a provider mapping (NSE
//! name, Yahoo ticker) and a place on the MKT dashboard.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A supported index. Serialised as its [`code`](IndexId::code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum IndexId {
    Nifty50,
    Sensex,
    NiftyBank,
    NiftyIt,
    NiftyAuto,
    NiftyPharma,
    NiftyFmcg,
    NiftyMetal,
    NiftyRealty,
    NiftyEnergy,
    NiftyPsuBank,
    NiftyFinService,
    NiftyMedia,
}

/// Static facts about one index.
#[derive(Debug, Clone, Copy)]
pub struct IndexInfo {
    pub id: IndexId,
    /// Stable machine code, e.g. `NIFTY50`.
    pub code: &'static str,
    /// Display name, e.g. `NIFTY 50`.
    pub title: &'static str,
    /// The name NSE's APIs use, or `None` for indices NSE doesn't publish
    /// (SENSEX is a BSE index).
    pub nse_name: Option<&'static str>,
    /// Upper-case names a user may type. The first is canonical.
    pub aliases: &'static [&'static str],
    /// Part of the broad-market row on the dashboard (as opposed to sectors).
    pub headline: bool,
}

const fn info(
    id: IndexId,
    code: &'static str,
    title: &'static str,
    nse_name: Option<&'static str>,
    aliases: &'static [&'static str],
    headline: bool,
) -> IndexInfo {
    IndexInfo {
        id,
        code,
        title,
        nse_name,
        aliases,
        headline,
    }
}

/// Every index, headline indices first, in dashboard order.
pub const CATALOGUE: [IndexInfo; 13] = [
    info(
        IndexId::Nifty50,
        "NIFTY50",
        "NIFTY 50",
        Some("NIFTY 50"),
        &["NIFTY", "NIFTY50", "NSEI"],
        true,
    ),
    info(
        IndexId::Sensex,
        "SENSEX",
        "SENSEX",
        None,
        &["SENSEX", "BSESN"],
        true,
    ),
    info(
        IndexId::NiftyBank,
        "BANKNIFTY",
        "NIFTY BANK",
        Some("NIFTY BANK"),
        &["BANKNIFTY", "NIFTYBANK", "NSEBANK"],
        true,
    ),
    info(
        IndexId::NiftyIt,
        "NIFTYIT",
        "NIFTY IT",
        Some("NIFTY IT"),
        &["NIFTYIT", "CNXIT"],
        false,
    ),
    info(
        IndexId::NiftyAuto,
        "NIFTYAUTO",
        "NIFTY AUTO",
        Some("NIFTY AUTO"),
        &["NIFTYAUTO", "CNXAUTO"],
        false,
    ),
    info(
        IndexId::NiftyPharma,
        "NIFTYPHARMA",
        "NIFTY PHARMA",
        Some("NIFTY PHARMA"),
        &["NIFTYPHARMA", "CNXPHARMA"],
        false,
    ),
    info(
        IndexId::NiftyFmcg,
        "NIFTYFMCG",
        "NIFTY FMCG",
        Some("NIFTY FMCG"),
        &["NIFTYFMCG", "CNXFMCG"],
        false,
    ),
    info(
        IndexId::NiftyMetal,
        "NIFTYMETAL",
        "NIFTY METAL",
        Some("NIFTY METAL"),
        &["NIFTYMETAL", "CNXMETAL"],
        false,
    ),
    info(
        IndexId::NiftyRealty,
        "NIFTYREALTY",
        "NIFTY REALTY",
        Some("NIFTY REALTY"),
        &["NIFTYREALTY", "CNXREALTY"],
        false,
    ),
    info(
        IndexId::NiftyEnergy,
        "NIFTYENERGY",
        "NIFTY ENERGY",
        Some("NIFTY ENERGY"),
        &["NIFTYENERGY", "CNXENERGY"],
        false,
    ),
    info(
        IndexId::NiftyPsuBank,
        "NIFTYPSUBANK",
        "NIFTY PSU BANK",
        Some("NIFTY PSU BANK"),
        &["NIFTYPSUBANK", "PSUBANK", "CNXPSUBANK"],
        false,
    ),
    info(
        IndexId::NiftyFinService,
        "NIFTYFINSERVICE",
        "NIFTY FIN SERVICE",
        Some("NIFTY FINANCIAL SERVICES"),
        &["NIFTYFINSERVICE", "FINNIFTY", "NIFTYFIN"],
        false,
    ),
    info(
        IndexId::NiftyMedia,
        "NIFTYMEDIA",
        "NIFTY MEDIA",
        Some("NIFTY MEDIA"),
        &["NIFTYMEDIA", "CNXMEDIA"],
        false,
    ),
];

impl IndexId {
    /// All indices in dashboard order.
    pub fn all() -> impl Iterator<Item = IndexId> {
        CATALOGUE.iter().map(|i| i.id)
    }

    pub fn info(self) -> &'static IndexInfo {
        // Every variant has exactly one catalogue entry (checked by a test),
        // so the fallback to the first entry is unreachable.
        CATALOGUE
            .iter()
            .find(|i| i.id == self)
            .unwrap_or(&CATALOGUE[0])
    }

    pub fn code(self) -> &'static str {
        self.info().code
    }

    pub fn title(self) -> &'static str {
        self.info().title
    }

    pub fn nse_name(self) -> Option<&'static str> {
        self.info().nse_name
    }

    /// Looks an index up by code, alias, or NSE name, ignoring case and
    /// spaces: `nifty 50`, `NIFTY50` and `nsei` all find NIFTY 50.
    pub fn find(name: &str) -> Option<IndexId> {
        let key: String = name
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        CATALOGUE
            .iter()
            .find(|i| {
                i.code == key
                    || i.aliases.contains(&key.as_str())
                    || i.nse_name.is_some_and(|n| n.replace(' ', "") == key)
                    || i.title.replace(' ', "") == key
            })
            .map(|i| i.id)
    }
}

impl fmt::Display for IndexId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

impl FromStr for IndexId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        IndexId::find(s).ok_or_else(|| format!("unknown index `{s}`"))
    }
}

impl TryFrom<String> for IndexId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<IndexId> for String {
    fn from(id: IndexId) -> Self {
        id.code().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_one_catalogue_entry() {
        let mut ids: Vec<_> = IndexId::all().collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CATALOGUE.len());
        for id in IndexId::all() {
            assert_eq!(id.info().id, id);
        }
    }

    #[test]
    fn catalogue_has_headline_and_at_least_ten_sector_indices() {
        let headline = CATALOGUE.iter().filter(|i| i.headline).count();
        assert_eq!(headline, 3);
        assert!(CATALOGUE.len() - headline >= 10);
    }

    #[test]
    fn aliases_are_unique_and_upper_case() {
        let mut seen = std::collections::HashSet::new();
        for info in &CATALOGUE {
            for alias in info.aliases {
                assert_eq!(*alias, alias.to_ascii_uppercase());
                assert!(seen.insert(*alias), "duplicate alias {alias}");
            }
        }
    }

    #[test]
    fn find_accepts_codes_aliases_and_nse_names() {
        for text in ["nifty", "NIFTY 50", "nifty50", "NSEI"] {
            assert_eq!(IndexId::find(text), Some(IndexId::Nifty50), "{text}");
        }
        assert_eq!(
            IndexId::find("NIFTY FINANCIAL SERVICES"),
            Some(IndexId::NiftyFinService)
        );
        assert_eq!(IndexId::find("Nifty Bank"), Some(IndexId::NiftyBank));
        assert_eq!(IndexId::find("banknifty"), Some(IndexId::NiftyBank));
        assert_eq!(IndexId::find("RELIANCE"), None);
    }

    #[test]
    fn serialises_as_code() {
        assert_eq!(
            serde_json::to_string(&IndexId::NiftyBank).unwrap(),
            "\"BANKNIFTY\""
        );
        assert_eq!(
            serde_json::from_str::<IndexId>("\"NIFTYIT\"").unwrap(),
            IndexId::NiftyIt
        );
        assert!(serde_json::from_str::<IndexId>("\"NOPE\"").is_err());
    }

    #[test]
    fn sensex_has_no_nse_name() {
        assert_eq!(IndexId::Sensex.nse_name(), None);
    }
}
