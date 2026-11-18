//! Top movers computed from an index's constituent quotes.
//!
//! This is the fallback for when NSE's own gainers/losers endpoint is
//! unavailable (or doesn't cover the index): the same ranking, from the
//! constituent prices the index view already has, so it carries the same
//! provenance as them.

use nivesh_core::Quote;

use crate::{Mover, Movers};

/// A list is "most active" only if enough constituents report volume for the
/// ranking to mean something.
const MIN_VOLUME_COVERAGE: f64 = 0.5;

fn pct(q: &Quote) -> f64 {
    q.change_pct()
}

/// The `limit` biggest gainers, biggest losers and highest-volume stocks.
///
/// Unchanged stocks are in neither of the first two lists. Rows with a
/// non-finite price are ignored. Ties keep the constituents' order.
pub fn compute_movers(constituents: &[Quote], limit: usize) -> Movers {
    let usable: Vec<&Quote> = constituents
        .iter()
        .filter(|q| q.ltp.is_finite() && q.prev_close.is_finite() && q.prev_close > 0.0)
        .collect();
    let take = |mut rows: Vec<&Quote>| -> Vec<Mover> {
        rows.truncate(limit);
        rows.into_iter()
            .map(|q| Mover { quote: q.clone() })
            .collect()
    };

    let mut gainers: Vec<&Quote> = usable.iter().copied().filter(|q| pct(q) > 0.0).collect();
    gainers.sort_by(|a, b| pct(b).total_cmp(&pct(a)));
    let mut losers: Vec<&Quote> = usable.iter().copied().filter(|q| pct(q) < 0.0).collect();
    losers.sort_by(|a, b| pct(a).total_cmp(&pct(b)));

    let with_volume: Vec<&Quote> = usable
        .iter()
        .copied()
        .filter(|q| q.volume.is_some_and(|v| v > 0))
        .collect();
    let covered =
        !usable.is_empty() && with_volume.len() as f64 >= usable.len() as f64 * MIN_VOLUME_COVERAGE;
    let mut active = if covered { with_volume } else { Vec::new() };
    active.sort_by_key(|q| std::cmp::Reverse(q.volume.unwrap_or(0)));

    Movers {
        gainers: take(gainers),
        losers: take(losers),
        most_active: take(active),
    }
}

#[cfg(test)]
mod tests {
    use nivesh_core::{Exchange, InstrumentId};

    use super::*;

    fn q(sym: &str, ltp: f64, prev: f64, volume: Option<u64>) -> Quote {
        Quote {
            id: InstrumentId::new(Exchange::Nse, sym).unwrap(),
            name: None,
            ltp,
            prev_close: prev,
            open: None,
            high: None,
            low: None,
            volume,
            week52_high: None,
            week52_low: None,
            upper_band: None,
            lower_band: None,
        }
    }

    fn symbols(m: &[Mover]) -> Vec<&str> {
        m.iter().map(|m| m.quote.id.symbol()).collect()
    }

    #[test]
    fn ranks_gainers_and_losers_by_percent_and_drops_unchanged() {
        let rows = [
            q("A", 110.0, 100.0, Some(10)), // +10%
            q("B", 101.0, 100.0, Some(30)), // +1%
            q("C", 100.0, 100.0, Some(20)), // flat
            q("D", 90.0, 100.0, Some(5)),   // -10%
            q("E", 99.0, 100.0, Some(40)),  // -1%
        ];
        let m = compute_movers(&rows, 10);
        assert_eq!(symbols(&m.gainers), ["A", "B"]);
        assert_eq!(symbols(&m.losers), ["D", "E"]);
        assert_eq!(symbols(&m.most_active), ["E", "B", "C", "A", "D"]);
    }

    #[test]
    fn limits_each_list() {
        let rows: Vec<Quote> = (1..=20)
            .map(|i| q(&format!("S{i}"), 100.0 + f64::from(i), 100.0, Some(1)))
            .collect();
        let m = compute_movers(&rows, 5);
        assert_eq!(symbols(&m.gainers), ["S20", "S19", "S18", "S17", "S16"]);
        assert!(m.losers.is_empty());
        assert_eq!(m.most_active.len(), 5);
    }

    #[test]
    fn most_active_needs_volume_for_most_stocks() {
        // Only one of four stocks reports volume: not a ranking worth showing.
        let rows = [
            q("A", 101.0, 100.0, Some(10)),
            q("B", 101.0, 100.0, None),
            q("C", 101.0, 100.0, None),
            q("D", 101.0, 100.0, None),
        ];
        let m = compute_movers(&rows, 10);
        assert!(m.most_active.is_empty());
        assert_eq!(m.gainers.len(), 4, "price movers don't depend on volume");
    }

    #[test]
    fn rows_without_a_usable_price_are_ignored() {
        let rows = [
            q("A", f64::NAN, 100.0, Some(1)),
            q("B", 101.0, 0.0, Some(1)),
            q("C", 102.0, 100.0, Some(1)),
        ];
        let m = compute_movers(&rows, 10);
        assert_eq!(symbols(&m.gainers), ["C"]);
        assert_eq!(compute_movers(&[], 10), Movers::default());
    }
}
