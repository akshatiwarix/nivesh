//! The Kite ticker's binary parser, keyed by packet length straight from the
//! docs (`kite.trade/docs/connect/v3/websocket`): **8** bytes is an LTP
//! packet, **44** a quote, **184** full (with depth), and **28**/**32** the
//! index packets. All int32 prices are paise on the wire; the parser turns
//! them into f64 by dividing by 100. Timestamps are epoch seconds.
//!
//! Packets carry the broker's `instrument_token`; the stream resolves it to
//! an instrument through the token map and drops unknown tokens. Market
//! depth (the `full` mode tail) is read in Part 6 for risk; Nivesh's quotes
//! need the level fields only.
//!
//! The parser never panics: arbitrary bytes yield packets or an error, which
//! the property tests pin down.

use nivesh_broker::BrokerError;

/// One parsed packet, still keyed by the broker's token.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTick {
    pub instrument_token: i64,
    pub ltp: f64,
    pub volume: Option<u64>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    /// Epoch seconds on the wire; ms out.
    pub exchange_ts_ms: Option<i64>,
}

/// Splits one binary message into its packets and parses each. The first two
/// bytes count the packets; every packet is a two-byte length followed by
/// that many bytes.
pub fn parse_message(frame: &[u8]) -> Result<Vec<RawTick>, BrokerError> {
    if frame.len() < 2 {
        return Err(BrokerError::Upstream(
            "tick frame shorter than its header".into(),
        ));
    }
    let count = u16::from_le_bytes([frame[0], frame[1]]) as usize;
    let mut ticks = Vec::with_capacity(count.min(1024));
    let mut cursor = 2usize;
    for _ in 0..count {
        if cursor + 2 > frame.len() {
            return Err(BrokerError::Upstream(
                "tick packet header out of bounds".into(),
            ));
        }
        let length = u16::from_le_bytes([frame[cursor], frame[cursor + 1]]) as usize;
        cursor += 2;
        if cursor + length > frame.len() {
            return Err(BrokerError::Upstream("tick packet out of bounds".into()));
        }
        if let Some(tick) = parse_packet(&frame[cursor..cursor + length])? {
            ticks.push(tick);
        }
        cursor += length;
    }
    Ok(ticks)
}

/// Parses one packet by its length.
fn parse_packet(packet: &[u8]) -> Result<Option<RawTick>, BrokerError> {
    let int32 = |offset: usize| -> Option<i32> {
        let bytes = packet.get(offset..offset + 4)?;
        Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let price = |offset: usize| -> Option<f64> { int32(offset).map(|v| f64::from(v) / 100.0) };

    let token = int32(0).ok_or_else(|| BrokerError::Upstream("packet without a token".into()))?;
    let mut tick = RawTick {
        instrument_token: i64::from(token),
        ltp: 0.0,
        volume: None,
        open: None,
        high: None,
        low: None,
        close: None,
        exchange_ts_ms: None,
    };

    match packet.len() {
        // LTP mode: token + last traded price.
        8 => {
            tick.ltp = price(4).unwrap_or(0.0);
            Ok(Some(tick))
        }
        // Quote mode, per the docs' field table.
        44 => {
            tick.ltp = price(4).unwrap_or(0.0);
            tick.volume = int32(16).map(|v| v.max(0) as u64);
            tick.open = price(28);
            tick.high = price(32);
            tick.low = price(36);
            tick.close = price(40);
            Ok(Some(tick))
        }
        // Full mode: the quote fields, then the OI block, the exchange
        // timestamp at 60..64 and market depth from 64 (ignored here).
        184 => {
            tick.ltp = price(4).unwrap_or(0.0);
            tick.volume = int32(16).map(|v| v.max(0) as u64);
            tick.open = price(28);
            tick.high = price(32);
            tick.low = price(36);
            tick.close = price(40);
            tick.exchange_ts_ms = int32(60).map(|v| i64::from(v) * 1000);
            Ok(Some(tick))
        }
        // Index packets: fewer, differently ordered fields.
        28 | 32 => {
            tick.ltp = price(4).unwrap_or(0.0);
            tick.high = price(8);
            tick.low = price(12);
            tick.open = price(16);
            tick.close = price(20);
            if packet.len() == 32 {
                tick.exchange_ts_ms = int32(28).map(|v| i64::from(v) * 1000);
            }
            Ok(Some(tick))
        }
        other => Err(BrokerError::Upstream(format!(
            "unknown tick packet length {other}"
        ))),
    }
}

/// Builds one message of little-endian packets, for tests.
#[cfg(test)]
pub(crate) fn encode_message(ints_per_packet: &[&[i32]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(ints_per_packet.len() as u16).to_le_bytes());
    for ints in ints_per_packet {
        let packet: Vec<u8> = ints.iter().flat_map(|v| v.to_le_bytes()).collect();
        out.extend_from_slice(&(packet.len() as u16).to_le_bytes());
        out.extend_from_slice(&packet);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multi_packet_message_parses_whole() {
        // One 8-byte LTP packet and one 28-byte index packet.
        let frame = encode_message(&[
            &[738561, 137_560],
            &[
                16, 2_551_435, 2_556_000, 2_548_000, 2_550_000, 2_549_000, 2_300,
            ],
        ]);
        let ticks = parse_message(&frame).unwrap();
        assert_eq!(ticks.len(), 2);
        assert_eq!(ticks[0].instrument_token, 738561);
        assert_eq!(ticks[0].ltp, 1375.6);
        assert_eq!(ticks[1].ltp, 25514.35);
        assert_eq!(ticks[1].high, Some(25560.0));
        assert_eq!(ticks[1].close, Some(25490.0));
    }

    #[test]
    fn a_quote_packet_carries_the_day_fields() {
        // 44 bytes = 11 int32 fields.
        let frame = encode_message(&[&[
            738561,  // token
            137_560, // ltp
            5,       // ltq
            137_100, // avg price
            123_456, // volume
            10,      // buy qty
            20,      // sell qty
            136_000, // open
            138_000, // high
            135_500, // low
            135_000, // close
        ]]);
        let ticks = parse_message(&frame).unwrap();
        assert_eq!(ticks.len(), 1);
        let t = &ticks[0];
        assert_eq!(t.ltp, 1375.6);
        assert_eq!(t.volume, Some(123_456));
        assert_eq!(t.open, Some(1360.0));
        assert_eq!(t.high, Some(1380.0));
        assert_eq!(t.low, Some(1355.0));
        assert_eq!(t.close, Some(1350.0));
    }

    #[test]
    fn a_full_packet_carries_the_exchange_timestamp() {
        // 184 bytes = 46 int32 fields; field 15 is the exchange timestamp at
        // offset 60..64.
        let mut fields = vec![0i32; 46];
        fields[0] = 738561;
        fields[1] = 137_560;
        fields[15] = 1_763_567_200;
        let frame = encode_message(&[&fields]);
        let ticks = parse_message(&frame).unwrap();
        assert_eq!(ticks.len(), 1);
        assert_eq!(ticks[0].ltp, 1375.6);
        assert_eq!(ticks[0].exchange_ts_ms, Some(1_763_567_200_000));
    }

    #[test]
    fn truncated_frames_are_errors_not_panics() {
        assert!(parse_message(&[]).is_err());
        assert!(parse_message(&[1, 0]).is_err(), "count with no packets");
        assert!(parse_message(&[1, 0, 10, 0, 1, 2]).is_err(), "short packet");
        assert!(
            parse_message(&[1, 0, 8, 0, 1, 2, 3]).is_err(),
            "truncated ltp"
        );
        assert!(
            parse_message(&[2, 0, 8, 0, 1, 2, 3, 4]).is_err(),
            "second packet missing"
        );
    }

    #[test]
    fn an_unknown_packet_length_is_an_error() {
        let frame = encode_message(&[&[1, 2, 3]]);
        assert!(parse_message(&frame).is_err());
    }

    proptest::proptest! {
        /// PLAN: the Kite tick parser never panics on arbitrary bytes.
        #[test]
        fn never_panics_on_arbitrary_bytes(frame in proptest::prelude::any::<Vec<u8>>()) {
            // Any outcome is fine; a panic is the failure.
            let _ = parse_message(&frame);
        }
    }
}
