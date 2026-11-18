//! Where a market datum came from and how fresh it is.
//!
//! Every value that crosses from a provider to the UI is a [`Sourced<T>`]. The
//! UI renders the [`Freshness`] label through one shared badge, so public data
//! can never be shown as live.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A data provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Nse,
    Yahoo,
    Rss,
    /// Broker-backed data (Part 4 onward).
    Kite,
}

impl ProviderId {
    pub const fn code(self) -> &'static str {
        match self {
            ProviderId::Nse => "nse",
            ProviderId::Yahoo => "yahoo",
            ProviderId::Rss => "rss",
            ProviderId::Kite => "kite",
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// How timely a provider's data is by nature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FeedKind {
    /// Broker real-time feed. Never public data.
    Live,
    /// Public, unofficial or delayed data.
    Delayed,
    EndOfDay,
}

/// What the UI shows next to a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Freshness {
    Live,
    Delayed,
    Eod,
    Stale,
}

impl Freshness {
    pub const fn label(self) -> &'static str {
        match self {
            Freshness::Live => "LIVE",
            Freshness::Delayed => "DELAYED",
            Freshness::Eod => "EOD",
            Freshness::Stale => "STALE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub provider: ProviderId,
    pub feed: FeedKind,
    /// Timestamp of the data itself, as reported by the source.
    pub as_of: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
    /// Set by the cache when it serves data past its TTL because refreshing
    /// failed (offline, blocked). Such data is always labelled STALE.
    #[serde(default)]
    pub stale: bool,
}

impl Provenance {
    pub fn new(
        provider: ProviderId,
        feed: FeedKind,
        as_of: DateTime<Utc>,
        fetched_at: DateTime<Utc>,
    ) -> Self {
        Self {
            provider,
            feed,
            as_of,
            fetched_at,
            stale: false,
        }
    }

    /// The label to show. Stale wins over the feed kind.
    pub fn freshness(&self) -> Freshness {
        if self.stale {
            return Freshness::Stale;
        }
        match self.feed {
            FeedKind::Live => Freshness::Live,
            FeedKind::Delayed => Freshness::Delayed,
            FeedKind::EndOfDay => Freshness::Eod,
        }
    }

    /// The same provenance, flagged as served from stale cache.
    pub fn into_stale(mut self) -> Self {
        self.stale = true;
        self
    }
}

/// A value together with its provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sourced<T> {
    pub data: T,
    pub provenance: Provenance,
}

impl<T> Sourced<T> {
    pub fn new(data: T, provenance: Provenance) -> Self {
        Self { data, provenance }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Sourced<U> {
        Sourced {
            data: f(self.data),
            provenance: self.provenance,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn prov(feed: FeedKind) -> Provenance {
        let t = Utc.with_ymd_and_hms(2026, 11, 18, 10, 0, 0).unwrap();
        Provenance::new(ProviderId::Yahoo, feed, t, t)
    }

    #[test]
    fn public_feeds_are_never_labelled_live() {
        assert_eq!(prov(FeedKind::Delayed).freshness(), Freshness::Delayed);
        assert_eq!(prov(FeedKind::EndOfDay).freshness(), Freshness::Eod);
        assert_eq!(prov(FeedKind::Live).freshness(), Freshness::Live);
    }

    #[test]
    fn stale_overrides_the_feed_label() {
        for feed in [FeedKind::Live, FeedKind::Delayed, FeedKind::EndOfDay] {
            assert_eq!(prov(feed).into_stale().freshness(), Freshness::Stale);
        }
        assert_eq!(Freshness::Stale.label(), "STALE");
    }

    #[test]
    fn old_json_without_stale_flag_still_parses() {
        let json = r#"{"provider":"nse","feed":"Delayed","as_of":"2026-11-18T10:00:00Z","fetched_at":"2026-11-18T10:00:01Z"}"#;
        let p: Provenance = serde_json::from_str(json).unwrap();
        assert!(!p.stale);
        assert_eq!(p.provider, ProviderId::Nse);
    }

    #[test]
    fn map_keeps_provenance() {
        let s = Sourced::new(2, prov(FeedKind::Delayed)).map(|n| n * 2);
        assert_eq!(s.data, 4);
        assert_eq!(s.provenance.provider, ProviderId::Yahoo);
    }
}
