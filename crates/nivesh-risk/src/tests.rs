use super::*;
use proptest::prelude::*;

/// 2027-01-15 13:30 IST: inside the NSE session.
const NOW: i64 = 1_800_000_000_000;

fn engine() -> RiskEngine {
    RiskEngine::new(RiskLimits::default())
}

fn buy(qty: i64) -> OrderRequest {
    OrderRequest {
        symbol: "NSE:RELIANCE".into(),
        side: Side::Buy,
        quantity: qty,
        price: None,
        product: Product::Delivery,
    }
}

fn sell(qty: i64) -> OrderRequest {
    OrderRequest {
        side: Side::Sell,
        ..buy(qty)
    }
}

fn ctx() -> RiskContext<'static> {
    RiskContext {
        ltp: 1_000.0,
        position_quantity: 0,
        symbol_exposure_paise: 0,
        open_positions: 0,
        recent_orders: &[],
        day_pnl_paise: 0,
        strategy_limits: None,
        now_ms: NOW,
    }
}

fn refused(order: &OrderRequest, ctx: &RiskContext<'_>) -> RiskCheck {
    engine().check(order, ctx).unwrap_err().check
}

#[test]
fn a_normal_order_is_approved_and_carries_the_facts() {
    let approved = engine().check(&buy(10), &ctx()).unwrap();
    assert_eq!(approved.symbol(), "NSE:RELIANCE");
    assert_eq!(approved.side(), Side::Buy);
    assert_eq!(approved.quantity(), 10);
    assert_eq!(approved.price(), Some(1_000.0), "priced at the LTP");
    assert_eq!(approved.product(), Product::Delivery);
    assert!(approved.is_valid_at(NOW + 1000));
    assert!(
        !approved.is_valid_at(NOW + 6 * 60_000),
        "expires in 5 minutes"
    );
}

#[test]
fn each_hard_limit_refuses_with_its_check() {
    assert_eq!(refused(&buy(0), &ctx()), RiskCheck::Quantity);
    assert_eq!(refused(&buy(5_000), &ctx()), RiskCheck::Quantity);
    // 600 × ₹1,000 = ₹6,00,000, over the ₹5,00,000 per-order cap.
    assert_eq!(refused(&buy(600), &ctx()), RiskCheck::OrderValue);
    let far = OrderRequest {
        price: Some(1_500.0),
        ..buy(1)
    };
    assert_eq!(refused(&far, &ctx()), RiskCheck::PriceBand);
    let near = OrderRequest {
        price: Some(900.0),
        ..buy(1)
    };
    assert!(
        engine().check(&near, &ctx()).is_ok(),
        "10% below is inside the band"
    );
    let option = OrderRequest {
        symbol: "NSE:NIFTY25NOV24000CE".into(),
        ..buy(1)
    };
    assert_eq!(refused(&option, &ctx()), RiskCheck::Segment);
    let mis = OrderRequest {
        product: Product::Intraday,
        ..buy(1)
    };
    assert_eq!(refused(&mis, &ctx()), RiskCheck::Product);
    let bad_ltp = RiskContext {
        ltp: f64::NAN,
        ..ctx()
    };
    assert_eq!(refused(&buy(1), &bad_ltp), RiskCheck::Price);
}

#[test]
fn the_account_limits_refuse_new_risk() {
    let full = RiskContext {
        open_positions: 20,
        ..ctx()
    };
    assert_eq!(refused(&buy(1), &full), RiskCheck::OpenPositions);
    // Adding to a position already held opens no new one.
    let adding = RiskContext {
        position_quantity: 5,
        ..full
    };
    assert!(engine().check(&buy(1), &adding).is_ok());

    // ₹9,99,000 held + ₹2,000 more is over the ₹10,00,000 symbol cap.
    let heavy = RiskContext {
        symbol_exposure_paise: 999_000 * 100,
        ..ctx()
    };
    assert_eq!(refused(&buy(2), &heavy), RiskCheck::SymbolExposure);

    // Realised plus MTM at −₹50,000.
    let losing = RiskContext {
        day_pnl_paise: -50_000 * 100,
        ..ctx()
    };
    assert_eq!(refused(&buy(1), &losing), RiskCheck::DailyLoss);

    // 16:00 IST: after the session.
    let late = RiskContext {
        now_ms: NOW + 150 * 60_000,
        ..ctx()
    };
    assert_eq!(refused(&buy(1), &late), RiskCheck::TradingWindow);
}

#[test]
fn an_exit_is_never_blocked_by_the_limits_on_adding_risk() {
    // Every risk-adding limit is breached at once; selling what is held
    // still goes through.
    let breached = RiskContext {
        position_quantity: 10,
        open_positions: 99,
        symbol_exposure_paise: i64::MAX / 2,
        day_pnl_paise: -1_000_000 * 100,
        now_ms: NOW + 150 * 60_000,
        ..ctx()
    };
    assert!(engine().check(&sell(10), &breached).is_ok());
    // But it cannot sell more than it holds: no short selling.
    assert_eq!(refused(&sell(11), &breached), RiskCheck::Quantity);
    assert_eq!(refused(&sell(1), &ctx()), RiskCheck::Quantity);
}

#[test]
fn the_order_rate_is_capped_per_minute_and_per_ist_day() {
    let recent: Vec<i64> = (0..10).map(|i| NOW - i * 1_000).collect();
    let busy = RiskContext {
        recent_orders: &recent,
        ..ctx()
    };
    assert_eq!(refused(&buy(1), &busy), RiskCheck::OrdersPerMinute);
    // The rate cap applies to exits too.
    let busy_exit = RiskContext {
        position_quantity: 1,
        ..busy
    };
    assert_eq!(refused(&sell(1), &busy_exit), RiskCheck::OrdersPerMinute);

    // 200 orders earlier today (IST), spaced out: the 201st is refused.
    let today: Vec<i64> = (0..200).map(|i| NOW - 60_000 - i * 60_000).collect();
    let spent = RiskContext {
        recent_orders: &today,
        ..ctx()
    };
    assert_eq!(refused(&buy(1), &spent), RiskCheck::OrdersPerDay);
    // Orders before IST midnight are yesterday's. 13:30 IST is 13.5 h
    // after midnight; orders 14 h ago don't count.
    let yesterday: Vec<i64> = (0..200).map(|i| NOW - 14 * 3_600_000 - i * 1_000).collect();
    let fresh = RiskContext {
        recent_orders: &yesterday,
        ..ctx()
    };
    assert!(engine().check(&buy(1), &fresh).is_ok());
}

#[test]
fn a_strategy_can_tighten_but_never_loosen() {
    let tight = RiskLimits {
        max_order_quantity: 5,
        ..RiskLimits::default()
    };
    let with = RiskContext {
        strategy_limits: Some(&tight),
        ..ctx()
    };
    assert_eq!(refused(&buy(6), &with), RiskCheck::Quantity);

    // "Loose" strategy limits change nothing: the global cap still holds.
    let loose = RiskLimits {
        max_order_quantity: 1_000_000,
        max_order_value_paise: i64::MAX,
        allowed_products: vec![Product::Delivery, Product::Intraday],
        trading_window: None,
        ..RiskLimits::default()
    };
    let with = RiskContext {
        strategy_limits: Some(&loose),
        ..ctx()
    };
    assert_eq!(refused(&buy(5_000), &with), RiskCheck::Quantity);
    let mis = OrderRequest {
        product: Product::Intraday,
        ..buy(1)
    };
    assert_eq!(refused(&mis, &with), RiskCheck::Product);
    let late = RiskContext {
        now_ms: NOW + 150 * 60_000,
        ..with
    };
    assert_eq!(refused(&buy(1), &late), RiskCheck::TradingWindow);
}

#[test]
fn windows_and_products_intersect() {
    let a = RiskLimits {
        trading_window: Some(TradingWindow {
            start_min: 600,
            end_min: 900,
        }),
        allowed_products: vec![Product::Delivery, Product::Intraday],
        ..RiskLimits::default()
    };
    let b = RiskLimits {
        trading_window: Some(TradingWindow {
            start_min: 1000,
            end_min: 1100,
        }),
        allowed_products: vec![Product::Intraday],
        ..RiskLimits::default()
    };
    let both = a.tightened_by(&b);
    assert_eq!(both.allowed_products, vec![Product::Intraday]);
    let w = both.trading_window.unwrap();
    assert_eq!(w.start_min, w.end_min, "no overlap: an empty window");
    assert!(both.is_within(&a) && both.is_within(&b));
}

#[test]
fn a_rejection_explains_itself() {
    let err = engine().check(&buy(600), &ctx()).unwrap_err();
    assert_eq!(err.check, RiskCheck::OrderValue);
    assert!(err.to_string().contains("₹600000"), "{err}");
}

fn limits_strategy() -> impl Strategy<Value = RiskLimits> {
    (
        (
            1..5_000i64,
            1..1_000_000_000_i64,
            1..1_000_000_000_i64,
            0..50usize,
        ),
        (0..30u32, 0..500u32, 0..10_000_000_i64, 0.0..0.5f64),
        (any::<bool>(), any::<bool>()),
        proptest::option::of((0..1440u16, 0..1440u16)),
    )
        .prop_map(
            |((q, v, e, p), (m, d, l, band), (del, intra), window)| RiskLimits {
                max_order_quantity: q,
                max_order_value_paise: v,
                max_symbol_exposure_paise: e,
                max_open_positions: p,
                max_orders_per_minute: m,
                max_orders_per_day: d,
                max_daily_loss_paise: l,
                price_band: band,
                allowed_products: [(del, Product::Delivery), (intra, Product::Intraday)]
                    .into_iter()
                    .filter_map(|(on, p)| on.then_some(p))
                    .collect(),
                trading_window: window.map(|(s, e)| TradingWindow {
                    start_min: s.min(e),
                    end_min: s.max(e),
                }),
            },
        )
}

proptest! {
    /// Whatever a strategy asks for, the limits in force are never looser
    /// than the global ones.
    #[test]
    fn strategy_limits_never_loosen_the_global_ones(
        global in limits_strategy(),
        strategy in limits_strategy(),
    ) {
        prop_assert!(global.tightened_by(&strategy).is_within(&global));
    }

    /// Anything approved with strategy limits would be approved without
    /// them, and no approval breaks a global cap.
    #[test]
    fn approvals_respect_the_global_caps(
        global in limits_strategy(),
        strategy in limits_strategy(),
        sell_side in any::<bool>(),
        qty in 1..6_000i64,
        held in 0..6_000i64,
        price in 1.0..5_000.0f64,
        exposure in 0..1_000_000_000_i64,
        open in 0..60usize,
        pnl in -20_000_000..10_000_000_i64,
        minute in 0..1440i64,
    ) {
        let order = OrderRequest {
            side: if sell_side { Side::Sell } else { Side::Buy },
            ..buy(qty)
        };
        let now = NOW - 13 * 3_600_000 - 30 * 60_000 + minute * 60_000;
        let base = RiskContext {
            ltp: price,
            position_quantity: held,
            symbol_exposure_paise: exposure,
            open_positions: open,
            day_pnl_paise: pnl,
            now_ms: now,
            ..ctx()
        };
        let engine = RiskEngine::new(global.clone());
        let with = RiskContext { strategy_limits: Some(&strategy), ..base };
        if let Ok(approved) = engine.check(&order, &with) {
            prop_assert!(engine.check(&order, &base).is_ok());
            prop_assert!(approved.quantity() <= global.max_order_quantity);
            prop_assert!(paise(price) * qty <= global.max_order_value_paise);
            prop_assert!(global.allowed_products.contains(&approved.product()));
            if order.side == Side::Sell {
                prop_assert!(held >= qty, "never short");
            }
        }
    }

    /// Selling what is held is never refused by daily loss, open
    /// positions, exposure or the trading window.
    #[test]
    fn exits_ignore_the_risk_adding_limits(
        qty in 1..1_000i64,
        pnl in i64::MIN / 4..0i64,
        open in 0..1_000usize,
        exposure in 0..i64::MAX / 4,
        minute in 0..1440i64,
    ) {
        let breached = RiskContext {
            position_quantity: qty,
            day_pnl_paise: pnl,
            open_positions: open,
            symbol_exposure_paise: exposure,
            now_ms: NOW + minute * 60_000,
            ltp: 100.0,
            ..ctx()
        };
        prop_assert!(engine().check(&sell(qty), &breached).is_ok());
    }
}
