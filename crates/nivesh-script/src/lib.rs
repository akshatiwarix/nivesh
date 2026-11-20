//! Strategy scripts, sandboxed (PLAN.md §10 Part 6 item 11).
//!
//! A script is Rhai source that defines `fn on_bar(bar)`. It is called on
//! every **closed** bar and returns `buy(qty)`, `sell()` or nothing:
//!
//! ```text
//! fn on_bar(bar) {
//!     if !in_position() && bar.close > highest(20) { return buy(10); }
//!     if in_position() && bar.close < lowest(10) { return sell(); }
//! }
//! ```
//!
//! `bar` has `t` (UTC ms), `open`, `high`, `low`, `close` and `volume`.
//! The functions it may call:
//!
//! - `sma(n)`, `ema(n)`, `rsi(n)`, `atr(n)`: as of this bar, `()` until
//!   warm (a comparison with `()` is false, so nothing fires early);
//! - `highest(n)` / `lowest(n)`: the highest high / lowest low of the `n`
//!   bars **before** this one, so `bar.close > highest(20)` is a breakout;
//! - `in_position()`, `entry_price()`, `bar_count()`;
//! - `buy(qty)` (whole shares) and `sell()` (the whole position).
//!
//! The sandbox:
//!
//! - no `import` (modules are compiled out), so no file is ever read;
//!   there is no filesystem, network, process or clock function at all,
//!   and `eval` is disabled;
//! - every call has an operation budget and a wall-clock budget, and call
//!   depth, expression depth, string, array and map sizes are capped (the
//!   memory limit: a script can only grow its values this far);
//! - `print` and `debug` go nowhere.
//!
//! A script that errors decides nothing on that bar (fail closed) and the
//! error is kept for the UI. Scripts emit intents like any strategy, so
//! they backtest through the same fill model and trade through the same
//! gateway: paper by default, live only when armed.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use nivesh_core::Candle;
use nivesh_strategy::{Decide, IntentSide, OrderIntent};
use rhai::{AST, Dynamic, Engine, EvalAltResult, Map, Position, Scope};

/// Bars of history a script can look back over.
pub const HISTORY: usize = 2_000;

/// The sandbox's hard limits, per `on_bar` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptLimits {
    pub max_operations: u64,
    pub max_time: Duration,
    pub max_call_levels: usize,
    pub max_expr_depth: usize,
    pub max_string_size: usize,
    pub max_array_size: usize,
    pub max_map_size: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            max_operations: 200_000,
            max_time: Duration::from_millis(50),
            max_call_levels: 32,
            max_expr_depth: 64,
            max_string_size: 4_096,
            max_array_size: 10_000,
            max_map_size: 1_000,
        }
    }
}

/// Why a script was refused or failed. `line` is 1-based, when known.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("line {line}: {message}")]
    Compile { line: usize, message: String },
    #[error("the script must define fn on_bar(bar)")]
    MissingOnBar,
    #[error("line {line}: {message}")]
    Runtime { line: usize, message: String },
    #[error("the script stopped: it used more than its {0} operations")]
    TooManyOperations(u64),
    #[error("the script stopped: it ran longer than {0} ms")]
    TooSlow(u128),
    #[error("on_bar returned {0}; return buy(qty), sell() or nothing")]
    BadReturn(String),
}

fn line(pos: Position) -> usize {
    pos.line().unwrap_or(0)
}

/// What the script's functions read: recent bars and the position.
#[derive(Debug, Default)]
struct World {
    bars: Vec<Candle>,
    in_position: bool,
    entry_price: Option<f64>,
}

impl World {
    fn closes(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.c).collect()
    }
}

fn lock(world: &Mutex<World>) -> MutexGuard<'_, World> {
    world.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The last value of a batch indicator, `()` when not warm.
fn last(values: &[f64]) -> Dynamic {
    match values.last() {
        Some(v) if v.is_finite() => Dynamic::from_float(*v),
        _ => Dynamic::UNIT,
    }
}

fn period(n: i64) -> Option<usize> {
    usize::try_from(n)
        .ok()
        .filter(|n| (1..=HISTORY).contains(n))
}

/// The marker `buy` and `sell` return.
fn signal(side: &str, qty: i64) -> Map {
    let mut m = Map::new();
    m.insert("side".into(), side.into());
    m.insert("qty".into(), qty.into());
    m
}

fn sandboxed(
    limits: ScriptLimits,
    world: &Arc<Mutex<World>>,
    deadline: &Arc<Mutex<Instant>>,
) -> Engine {
    let mut engine = Engine::new();
    engine
        .set_max_operations(limits.max_operations)
        .set_max_call_levels(limits.max_call_levels)
        .set_max_expr_depths(limits.max_expr_depth, limits.max_expr_depth)
        .set_max_string_size(limits.max_string_size)
        .set_max_array_size(limits.max_array_size)
        .set_max_map_size(limits.max_map_size)
        .set_max_variables(256)
        .set_max_functions(64);
    engine.disable_symbol("eval");
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    {
        let deadline = Arc::clone(deadline);
        engine.on_progress(move |ops| {
            if ops % 512 == 0
                && Instant::now() > *deadline.lock().unwrap_or_else(PoisonError::into_inner)
            {
                Some(Dynamic::from("time"))
            } else {
                None
            }
        });
    }

    let w = Arc::clone(world);
    engine.register_fn("sma", move |n: i64| {
        let Some(n) = period(n) else {
            return Dynamic::UNIT;
        };
        last(&nivesh_indicators::sma(&lock(&w).closes(), n))
    });
    let w = Arc::clone(world);
    engine.register_fn("ema", move |n: i64| {
        let Some(n) = period(n) else {
            return Dynamic::UNIT;
        };
        last(&nivesh_indicators::ema(&lock(&w).closes(), n))
    });
    let w = Arc::clone(world);
    engine.register_fn("rsi", move |n: i64| {
        let Some(n) = period(n) else {
            return Dynamic::UNIT;
        };
        last(&nivesh_indicators::rsi(&lock(&w).closes(), n))
    });
    let w = Arc::clone(world);
    engine.register_fn("atr", move |n: i64| {
        let Some(n) = period(n) else {
            return Dynamic::UNIT;
        };
        let world = lock(&w);
        let (h, l, c): (Vec<f64>, Vec<f64>, Vec<f64>) = (
            world.bars.iter().map(|b| b.h).collect(),
            world.bars.iter().map(|b| b.l).collect(),
            world.closes(),
        );
        last(&nivesh_indicators::atr(&h, &l, &c, n))
    });
    let w = Arc::clone(world);
    engine.register_fn("highest", move |n: i64| {
        let Some(n) = period(n) else {
            return Dynamic::UNIT;
        };
        let world = lock(&w);
        let before = world.bars.len().saturating_sub(1);
        if before < n {
            return Dynamic::UNIT;
        }
        let max = world.bars[before - n..before]
            .iter()
            .map(|b| b.h)
            .fold(f64::MIN, f64::max);
        Dynamic::from_float(max)
    });
    let w = Arc::clone(world);
    engine.register_fn("lowest", move |n: i64| {
        let Some(n) = period(n) else {
            return Dynamic::UNIT;
        };
        let world = lock(&w);
        let before = world.bars.len().saturating_sub(1);
        if before < n {
            return Dynamic::UNIT;
        }
        let min = world.bars[before - n..before]
            .iter()
            .map(|b| b.l)
            .fold(f64::MAX, f64::min);
        Dynamic::from_float(min)
    });
    let w = Arc::clone(world);
    engine.register_fn("in_position", move || lock(&w).in_position);
    let w = Arc::clone(world);
    engine.register_fn("entry_price", move || {
        lock(&w)
            .entry_price
            .map_or(Dynamic::UNIT, Dynamic::from_float)
    });
    let w = Arc::clone(world);
    engine.register_fn("bar_count", move || {
        i64::try_from(lock(&w).bars.len()).unwrap_or(i64::MAX)
    });
    engine.register_fn("buy", |qty: i64| signal("buy", qty));
    engine.register_fn("sell", || signal("sell", 0));
    engine
}

/// Compiles `source` in the sandbox and checks it defines `on_bar(bar)`.
pub fn validate(source: &str) -> Result<(), ScriptError> {
    Script::new(source, ScriptLimits::default()).map(drop)
}

/// A compiled script, ready to decide on bars.
pub struct Script {
    engine: Engine,
    ast: AST,
    world: Arc<Mutex<World>>,
    deadline: Arc<Mutex<Instant>>,
    limits: ScriptLimits,
    symbol: String,
    last_error: Option<ScriptError>,
}

impl Script {
    pub fn new(source: &str, limits: ScriptLimits) -> Result<Self, ScriptError> {
        let world = Arc::new(Mutex::new(World::default()));
        let deadline = Arc::new(Mutex::new(Instant::now()));
        let engine = sandboxed(limits, &world, &deadline);
        let ast = engine.compile(source).map_err(|e| ScriptError::Compile {
            line: line(e.position()),
            message: e.err_type().to_string(),
        })?;
        if !ast
            .iter_functions()
            .any(|f| f.name == "on_bar" && f.params.len() == 1)
        {
            return Err(ScriptError::MissingOnBar);
        }
        Ok(Self {
            engine,
            ast,
            world,
            deadline,
            limits,
            symbol: String::new(),
            last_error: None,
        })
    }

    /// The symbol intents are about.
    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = symbol.into();
        self
    }

    /// The error of the last bar, if it failed.
    pub fn last_error(&self) -> Option<&ScriptError> {
        self.last_error.as_ref()
    }

    /// Runs `on_bar` on a closed bar: the signal, or why it failed.
    pub fn decide(&mut self, bar: &Candle) -> Result<Option<IntentSide>, ScriptError> {
        self.decide_qty(bar).map(|s| s.map(|(side, _)| side))
    }

    fn decide_qty(&mut self, bar: &Candle) -> Result<Option<(IntentSide, i64)>, ScriptError> {
        {
            let mut world = lock(&self.world);
            world.bars.push(*bar);
            if world.bars.len() > HISTORY {
                world.bars.remove(0);
            }
        }
        *self.deadline.lock().unwrap_or_else(PoisonError::into_inner) =
            Instant::now() + self.limits.max_time;
        let mut b = Map::new();
        b.insert("t".into(), bar.t.into());
        b.insert("open".into(), Dynamic::from_float(bar.o));
        b.insert("high".into(), Dynamic::from_float(bar.h));
        b.insert("low".into(), Dynamic::from_float(bar.l));
        b.insert("close".into(), Dynamic::from_float(bar.c));
        b.insert("volume".into(), Dynamic::from_float(bar.v));
        let result: Result<Dynamic, Box<EvalAltResult>> = self.engine.call_fn(
            &mut Scope::new(),
            &self.ast,
            "on_bar",
            (Dynamic::from_map(b),),
        );
        let value = result.map_err(|e| self.runtime(*e))?;
        if value.is_unit() {
            return Ok(None);
        }
        let shown = value.to_string();
        let map = value
            .try_cast::<Map>()
            .ok_or_else(|| ScriptError::BadReturn(shown.clone()))?;
        let side = map.get("side").and_then(|s| s.clone().into_string().ok());
        let qty = map.get("qty").and_then(|q| q.as_int().ok()).unwrap_or(0);
        match side.as_deref() {
            Some("buy") if qty > 0 => Ok(Some((IntentSide::Buy, qty))),
            Some("buy") => Err(ScriptError::BadReturn(format!(
                "buy({qty}): buy at least 1 share"
            ))),
            Some("sell") => Ok(Some((IntentSide::Sell, 0))),
            _ => Err(ScriptError::BadReturn(shown)),
        }
    }

    fn runtime(&self, e: EvalAltResult) -> ScriptError {
        match e {
            EvalAltResult::ErrorTooManyOperations(_) => {
                ScriptError::TooManyOperations(self.limits.max_operations)
            }
            EvalAltResult::ErrorTerminated(..) => {
                ScriptError::TooSlow(self.limits.max_time.as_millis())
            }
            other => ScriptError::Runtime {
                line: line(other.position()),
                message: other.to_string(),
            },
        }
    }
}

impl Decide for Script {
    fn feed(&mut self, bar: &Candle) -> Option<OrderIntent> {
        match self.decide_qty(bar) {
            Ok(None) => {
                self.last_error = None;
                None
            }
            Ok(Some((side, quantity))) => {
                self.last_error = None;
                let mut world = lock(&self.world);
                match side {
                    IntentSide::Buy => {
                        world.in_position = true;
                        world.entry_price = Some(bar.c);
                    }
                    IntentSide::Sell => {
                        world.in_position = false;
                        world.entry_price = None;
                    }
                }
                Some(OrderIntent {
                    symbol: self.symbol.clone(),
                    side,
                    quantity,
                    signal_price: bar.c,
                    stop_loss: None,
                    target: None,
                    trailing_stop: None,
                    signal_bar_t: bar.t,
                })
            }
            Err(err) => {
                // Fail closed: nothing on this bar.
                self.last_error = Some(err);
                None
            }
        }
    }

    fn sync_position(&mut self, entry: Option<f64>) {
        let mut world = lock(&self.world);
        world.in_position = entry.is_some();
        world.entry_price = entry;
    }
}
