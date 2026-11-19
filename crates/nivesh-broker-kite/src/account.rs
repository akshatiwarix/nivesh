//! Mapping Kite's account payloads onto `nivesh_broker`'s domain types.
//!
//! Every mapping is total and tested against scrubbed fixture recordings:
//! money arrives as rupees in JSON and leaves as integer paise, quantities
//! are whole shares, and Kite's own order statuses map onto the shared state
//! machine through one table (which the conformance suite also pins).

use serde_json::Value;

use nivesh_broker::{
    BrokerError, BrokerOrderId, Funds, Holding, Order, OrderKind, OrderStatus, Positions, Product,
    Side, Trade,
};
use nivesh_core::{Exchange, InstrumentId};

/// Reads an integer from a JSON number or numeric string, rounding floats.
fn as_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_f64().map(|f| f.round() as i64),
        Value::String(s) => s.trim().parse::<f64>().ok().map(|f| f.round() as i64),
        _ => None,
    }
}

/// Rupees to paise.
fn to_paise(rupees: Option<f64>) -> Option<i64> {
    rupees.map(|r| (r * 100.0).round() as i64)
}

fn as_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// `tradingsymbol` + `exchange` → `InstrumentId`; unknown exchanges and
/// unusable symbols make the row invalid, and invalid rows are skipped.
fn instrument(row: &Value) -> Option<InstrumentId> {
    let symbol = row.get("tradingsymbol")?.as_str()?;
    let exchange = row.get("exchange")?.as_str()?;
    let exchange: Exchange = exchange.parse().ok()?;
    InstrumentId::new(exchange, symbol).ok()
}

fn side(row: &Value) -> Option<Side> {
    match row
        .get("transaction_type")?
        .as_str()?
        .to_ascii_uppercase()
        .as_str()
    {
        "BUY" => Some(Side::Buy),
        "SELL" => Some(Side::Sell),
        _ => None,
    }
}

fn order_kind(row: &Value) -> Option<OrderKind> {
    match row
        .get("order_type")?
        .as_str()?
        .to_ascii_uppercase()
        .as_str()
    {
        "MARKET" => Some(OrderKind::Market),
        "LIMIT" => Some(OrderKind::Limit),
        "SL" => Some(OrderKind::StopLoss),
        "SL-M" | "SLM" => Some(OrderKind::StopLossMarket),
        _ => None,
    }
}

fn product(row: &Value) -> Option<Product> {
    match row.get("product")?.as_str()?.to_ascii_uppercase().as_str() {
        "CNC" => Some(Product::Cnc),
        "MIS" => Some(Product::Mis),
        "NRML" => Some(Product::Nrml),
        _ => None,
    }
}

/// Kite's order status strings onto the shared state machine. `TRIGGER
/// PENDING` is a live stop order waiting for its trigger, so it sits in
/// `Open`; `PUT ORDER RECEIVED` has not been acknowledged yet.
pub fn parse_order_status(kite: &str) -> Result<OrderStatus, BrokerError> {
    match kite.trim().to_ascii_uppercase().as_str() {
        "PUT ORDER RECEIVED" | "PUT ORDER REQ RECEIVED" | "VALIDATION PENDING" => {
            Ok(OrderStatus::Pending)
        }
        "OPEN" | "TRIGGER PENDING" | "MODIFY PENDING" | "MODIFY VALIDATION PENDING" => {
            Ok(OrderStatus::Open)
        }
        "COMPLETE" => Ok(OrderStatus::Filled),
        "CANCELLED" => Ok(OrderStatus::Cancelled),
        "REJECTED" => Ok(OrderStatus::Rejected),
        other => Err(BrokerError::Upstream(format!(
            "unknown order status {other:?}"
        ))),
    }
}

/// One IST timestamp as Kite writes it (`2026-11-19T12:34:56+0530`, or the
/// space-separated older form), into UTC.
fn parse_ist_timestamp(text: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    let ist = chrono::FixedOffset::east_opt(5 * 3600 + 30 * 60)?;
    let text = text.trim();
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(parsed.with_timezone(&chrono::Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return Some(
                ist.from_local_datetime(&naive)
                    .single()?
                    .with_timezone(&chrono::Utc),
            );
        }
    }
    None
}

/// One row of `/portfolio/holdings`.
fn holding(row: &Value) -> Option<Holding> {
    Some(Holding {
        id: instrument(row)?,
        quantity: as_i64(row.get("quantity"))?,
        average_price: to_paise(as_f64(row.get("average_price")))?,
        last_price: to_paise(as_f64(row.get("last_price"))),
        pnl: to_paise(as_f64(row.get("pnl"))),
    })
}

/// `/portfolio/holdings`: an array; unusable rows are skipped, an unusable
/// payload is a parse error.
pub fn parse_holdings(data: &Value) -> Result<Vec<Holding>, BrokerError> {
    let rows = data
        .as_array()
        .ok_or_else(|| BrokerError::Upstream("holdings payload is not an array".into()))?;
    Ok(rows.iter().filter_map(holding).collect())
}

fn position(row: &Value) -> Option<nivesh_broker::Position> {
    let net_quantity = as_i64(row.get("quantity"))?;
    let day = as_i64(row.get("day_buy_quantity")).unwrap_or(0)
        - as_i64(row.get("day_sell_quantity")).unwrap_or(0);
    Some(nivesh_broker::Position {
        id: instrument(row)?,
        net_quantity,
        day_quantity: day,
        average_price: to_paise(as_f64(row.get("average_price")))?,
        last_price: to_paise(as_f64(row.get("last_price"))),
        pnl: to_paise(as_f64(row.get("m2m")).or_else(|| as_f64(row.get("pnl")))),
        realised: to_paise(as_f64(row.get("realised"))),
    })
}

/// `/portfolio/positions`: `{net: [...], day: [...]}`.
pub fn parse_positions(data: &Value) -> Result<Positions, BrokerError> {
    let net = data
        .get("net")
        .and_then(Value::as_array)
        .ok_or_else(|| BrokerError::Upstream("positions payload has no net list".into()))?;
    Ok(Positions {
        net: net.iter().filter_map(position).collect(),
        day: data
            .get("day")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(position).collect())
            .unwrap_or_default(),
    })
}

pub(crate) fn order(row: &Value) -> Option<Order> {
    Some(Order {
        id: BrokerOrderId(row.get("order_id")?.as_str()?.to_owned()),
        id_instrument: instrument(row)?,
        side: side(row)?,
        kind: order_kind(row)?,
        product: product(row)?,
        quantity: as_i64(row.get("quantity"))?,
        filled_quantity: as_i64(row.get("filled_quantity")).unwrap_or(0),
        price: to_paise(as_f64(row.get("price"))).filter(|p| *p > 0),
        average_price: to_paise(as_f64(row.get("average_price"))).filter(|p| *p > 0),
        status: parse_order_status(row.get("status")?.as_str()?).ok()?,
        placed_at: row
            .get("order_timestamp")
            .and_then(Value::as_str)
            .and_then(parse_ist_timestamp),
        tag: row
            .get("tag")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned),
    })
}

/// `/orders`: an array.
pub fn parse_orders(data: &Value) -> Result<Vec<Order>, BrokerError> {
    let rows = data
        .as_array()
        .ok_or_else(|| BrokerError::Upstream("orders payload is not an array".into()))?;
    Ok(rows.iter().filter_map(order).collect())
}

fn trade(row: &Value) -> Option<Trade> {
    let quantity = as_i64(row.get("quantity"))?;
    if quantity <= 0 {
        // A fill of zero shares is junk, not a trade.
        return None;
    }
    Some(Trade {
        order_id: BrokerOrderId(row.get("order_id")?.as_str()?.to_owned()),
        id_instrument: instrument(row)?,
        side: side(row)?,
        quantity,
        price: to_paise(as_f64(row.get("price")).or_else(|| as_f64(row.get("average_price"))))?,
        traded_at: row
            .get("fill_timestamp")
            .or_else(|| row.get("exchange_timestamp"))
            .and_then(Value::as_str)
            .and_then(parse_ist_timestamp)?,
    })
}

/// `/trades`: an array.
pub fn parse_trades(data: &Value) -> Result<Vec<Trade>, BrokerError> {
    let rows = data
        .as_array()
        .ok_or_else(|| BrokerError::Upstream("trades payload is not an array".into()))?;
    Ok(rows.iter().filter_map(trade).collect())
}

/// `/user/margins`, equity segment only for now (cash-only Nivesh):
/// available is cash + live balance + collateral; used is the utilised
/// debits; total is the net.
pub fn parse_funds(data: &Value) -> Result<Funds, BrokerError> {
    let equity = data
        .get("equity")
        .ok_or_else(|| BrokerError::Upstream("margins payload has no equity".into()))?;
    let available = &equity["available"];
    let used = &equity["utilised"];
    let cash = as_f64(available.get("cash")).unwrap_or(0.0);
    let live = as_f64(available.get("live_balance")).unwrap_or(0.0);
    let collateral = as_f64(available.get("collateral")).unwrap_or(0.0);
    let debits = as_f64(used.get("debits")).unwrap_or(0.0);
    let net = as_f64(equity.get("net")).unwrap_or(cash + live + collateral - debits);
    let available_paise = to_paise(Some(cash + live + collateral)).unwrap_or(0);
    let used_paise = to_paise(Some(debits)).unwrap_or(0);
    Ok(Funds {
        available: available_paise,
        used: used_paise,
        total: to_paise(Some(net)).unwrap_or(available_paise - used_paise),
    })
}
