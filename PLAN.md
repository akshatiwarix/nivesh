# Nivesh: Build Plan

Nivesh (निवेश, "investment") is a keyboard-driven, Bloomberg Terminal-style desktop
app for Indian equity markets (NSE/BSE), built with Tauri 2, a Rust core and a thin,
fast web UI.

This document is the source of truth for architecture and delivery. It implements
the product decisions agreed with Akshat. Scope changes go through Akshat. If an
implementation has to deviate, update this file in the same PR and say so in the PR
description.

**Contents**

1. [Overview](#1-overview)
2. [Architecture](#2-architecture)
3. [Delivery workflow](#3-delivery-workflow)
4. [Part 1: Foundation](#part-1-foundation)
5. [Part 2: Browse-mode data](#part-2-browse-mode-data)
6. [Part 3: Charts & market views](#part-3-charts--market-views)
7. [Part 4: Broker connection](#part-4-broker-connection)
8. [Part 5: Strategies](#part-5-strategies)
9. [Part 6: Live trading & safety](#part-6-live-trading--safety)
10. [Data-source limits](#10-data-source-limits)
11. [Trading safety principles](#11-trading-safety-principles)
12. [Later / out of scope](#12-later--out-of-scope)

---

## 1. Overview

### What it is

A dense, dark, amber-on-black, monospace, multi-panel terminal for Indian markets:

- **Browse mode (no login):** indices, quotes, charts, movers, breadth, sectors,
  news and watchlists from free public sources (NSE public endpoints, Yahoo
  Finance, RSS). This data is unofficial and delayed. Every value is labelled
  **DELAYED** and shows its source and as-of time.
- **Connected mode (broker login):** Zerodha Kite Connect first, then Upstox,
  Angel One and Dhan through the same adapter trait. Adds live ticks, holdings,
  positions, orders and funds.
- **Strategies:** rule-based building blocks (MA crossover, RSI, breakout,
  stop-loss/target), backtesting and paper trading. Paper trading is the default.
  Live trading is an explicit opt-in per strategy, behind hard risk limits, a kill
  switch and a tamper-evident audit log. Scripting comes last.

### How it feels

- A command bar is always one keystroke away. Typing `RELIANCE` then Enter opens
  the quote, `NIFTY` opens the index, and `HELP` lists every command.
- Function-key navigation and multi-panel layouts. Every view works without a
  mouse.
- Instant response. Snappiness is a feature and has explicit budgets (§2.9).

### Platforms

macOS first (primary target, `.dmg`). Linux must build and run (CI plus the dev
box). Windows comes later, so platform-specific code stays behind traits.

### Guiding principles

1. **Snappy first.** Fine-grained UI updates, no heavy dependencies, and all heavy
   work off the UI thread and in Rust.
2. **Rust does the work, the UI renders.** Data access, caching, indicators,
   strategies, risk and broker I/O all live in Rust crates that can be tested
   without the app.
3. **Honest data.** Every number says where it came from and how old it is. Public
   data is never labelled LIVE.
4. **Safe by default.** Paper trading is the default. The system fails closed, and
   no code path skips the risk gate.
5. **Local-first and private.** There is no server and no telemetry. Data lives in
   local SQLite and secrets live in the OS keychain.
6. **Pluggable.** Data providers and brokers sit behind traits, with fallback
   between providers.

### Non-goals (for now)

Licensed real-time exchange feeds, F&O/options trading, multi-user or cloud sync,
mobile, and investment advice.

---

## 2. Architecture

```
┌──────────────────────────── Nivesh (Tauri 2 app) ────────────────────────────┐
│ WebView: SolidJS UI                                                          │
│   command bar · panels · views · keymap        (renders only, no network)    │
│        │ invoke(command)                 ▲ Channel<T> streams · events       │
├────────┼─────────────────────────────────┼───────────────────────────────────┤
│ src-tauri (nivesh-app): thin command handlers, AppState, event bridge        │
├──────────────────────────────────────────────────────────────────────────────┤
│ nivesh-data        nivesh-indicators   nivesh-strategy     nivesh-exec       │
│  providers, chain,  pure math           rules, backtest,    OrderGateway:    │
│  cache, QuoteHub                        paper runner ─────► risk, kill switch│
│        │                                                    audit, routing   │
│        │           nivesh-store (SQLite)   nivesh-secrets (Keychain)   │     │
│        ▼                                                               ▼     │
│  NSE · Yahoo · RSS (HTTPS)                    nivesh-broker(-kite …) adapters│
└──────────────────────────────────────────────────────────────────────────────┘
```

### 2.1 Repository layout

```
nivesh/
├── Cargo.toml              # workspace root: members = crates/*, src-tauri
├── Cargo.lock              # committed (this is an application)
├── rust-toolchain.toml     # pinned stable toolchain, bumped deliberately
├── rustfmt.toml, clippy.toml
├── package.json            # frontend + @tauri-apps/cli (pnpm)
├── pnpm-lock.yaml
├── index.html, vite.config.ts, tsconfig.json, biome.json
├── ui/                     # SolidJS frontend source
│   ├── main.tsx
│   ├── app/                # shell, layout, panels, focus management
│   ├── commands/           # command parser + registry (single source for HELP)
│   ├── keys/               # keymap
│   ├── views/              # one folder per view: quote/, index/, chart/, …
│   ├── components/         # Panel, DataTable, Badge, … (no UI kit)
│   ├── ipc/                # typed invoke/Channel wrappers
│   │   └── bindings/       # generated by ts-rs, committed, checked in CI
│   ├── styles/             # tokens.css, base.css
│   └── fonts/              # bundled monospace font (no network fonts)
├── src-tauri/              # Tauri 2 app crate (nivesh-app)
│   ├── tauri.conf.json
│   ├── capabilities/       # minimal permissions
│   ├── icons/
│   └── src/                # main.rs, state.rs, events.rs, commands/*.rs
├── crates/                 # Rust library crates (see 2.2)
│   └── <crate>/tests/fixtures/   # recorded, scrubbed API responses
├── .github/workflows/      # ci.yml (checks + bundles)
├── PLAN.md, CLAUDE.md, README.md
```

### 2.2 Cargo workspace crates

| Crate | Responsibility | Depends on | Part |
|---|---|---|---|
| `nivesh-core` | Domain types: `Exchange`, `InstrumentId`, `Instrument`, `Price` (integer paise), `Qty`, `Quote`, `Candle`/`CandleSeries`, `Interval`, `IndexId`, `Provenance`/`FeedKind`/`Sourced<T>`, IST market calendar and session status. No I/O. | – | 1 |
| `nivesh-store` | SQLite (`rusqlite`, bundled, WAL) plus versioned migrations. Holds settings, layout, watchlists, instrument master, candle cache, strategies, backtests, paper ledger and audit log. Runs on a dedicated blocking thread. | core | 1 |
| `nivesh-data` | `DataProvider` trait, shared HTTP client (rate limits, backoff, cookies), providers (NSE, Yahoo, RSS, plus a broker-backed provider in Part 4), the `MarketData` facade (chain, fallback, cache, coalescing) and the `QuoteHub` subscription scheduler. | core, store | 2 |
| `nivesh-indicators` | Pure `f64` indicator math, batch and streaming (SMA, EMA, WMA, RSI, MACD, Bollinger, ATR, VWAP, Donchian). No I/O, no async. | core | 3 |
| `nivesh-secrets` | `SecretStore` trait. Keychain backend via `keyring` (macOS Keychain, later Windows Credential Manager, Linux Secret Service). In-memory backend for tests. `secrecy` wrappers so secrets can't be printed. | – | 4 |
| `nivesh-broker` | Broker traits (`BrokerAuth`, `BrokerAccount`, `MarketStream`, then `BrokerExecution` in Part 5), account and order domain types, order state machine, conformance test harness, fill model and `PaperBroker`. | core, risk (from Part 5) | 4 |
| `nivesh-broker-kite` | Zerodha Kite Connect adapter: login, REST, instrument dump, WebSocket ticker. Order execution comes in Part 6 behind the `live-trading` Cargo feature. | broker, secrets | 4 |
| `nivesh-strategy` | Rule model, validator, evaluator, templates, backtester, metrics and paper-trading runner. Emits `OrderIntent`s and never places orders itself. | core, indicators, broker (fill model) | 5 |
| `nivesh-risk` | Pure pre-trade risk engine: limits model, checks and the `ApprovedOrder` token. | core | 5 (basic), 6 (full) |
| `nivesh-exec` | `OrderGateway`, the **only** path from intents to brokers. Handles paper/live routing, risk enforcement, kill switch, reconciliation and audit writes. | broker, risk, store, strategy | 5, 6 |
| `nivesh-script` | Sandboxed Rhai scripting for strategies. | strategy, indicators | 6 |
| `src-tauri` (`nivesh-app`) | Tauri commands and events, `AppState` wiring, window and plugins. No business logic. | all | 1 |

**Workspace conventions**

- `[workspace.package]`: edition 2024, `publish = false`.
- `[workspace.dependencies]`: one version of each dependency.
- `[workspace.lints]`: `unsafe_code = "forbid"`. Clippy denies `dbg_macro` and
  `todo` and warns on `unwrap_used`/`expect_used` outside tests (with
  `-D warnings` in CI, and `allow-unwrap-in-tests` in `clippy.toml`).
- Release profile: `lto = "thin"`, `codegen-units = 1`.
- Errors: `thiserror` in libraries, `anyhow` only in `src-tauri`.
- Time: stored and compared in UTC, displayed in IST. IST is a fixed +05:30 with
  no DST, so `chrono::FixedOffset` is enough and no tz database is needed.
- Money: order, position and P&L amounts use integer paise (`Price(i64)`) because
  they must be exact and tick sizes can be ₹0.01. Indicators and display-only
  market data use `f64`.

**Main third-party crates** (kept deliberately small): `tokio`, `serde`/`serde_json`,
`thiserror`, `anyhow`, `tracing`, `chrono`, `reqwest` (rustls, cookies, gzip/brotli),
`governor` (rate limiting), `moka` (TTL cache), `rusqlite` and `rusqlite_migration`,
`ts-rs`, `keyring`, `secrecy`, `tokio-tungstenite` (rustls), `feed-rs`, `rhai`,
`sha2`. For tests and benches: `proptest`, `insta`, `wiremock`, `criterion`.

### 2.3 Tauri 2 app

- **Window:** a single main window, dark background, minimum 1280×800, with
  geometry persisted.
- **Single instance:** `tauri-plugin-single-instance`. Two instances would fight
  over the database and, later, could double-send orders.
- **Security:** strict CSP (`default-src 'self'`; no remote scripts, frames or
  images). **The webview makes no network requests**: all HTTP and WebSocket
  traffic happens in Rust.
- **Capabilities:** `src-tauri/capabilities/main.json` grants only our commands
  plus a minimal core set.
- **Plugins** are added only when needed:
  - `opener` (http/https links only, opened in the system browser)
  - `global-shortcut` (optional OS-level kill-switch shortcut in Part 6)
- **Runtime:** commands run on Tauri's tokio runtime. CPU-heavy work (backtests,
  instrument indexing, large resampling) goes to `spawn_blocking` or a worker
  pool, never onto async workers or the main thread.
- **AppState:** a managed `Arc<Services>` built at startup. It holds `Store`,
  `MarketData`, `QuoteHub`, `BrokerManager`, `StrategyRuntime` and
  `OrderGateway`. Command handlers stay short: parse arguments, call a service,
  map the error.
- **Bundles:**
  - macOS `.dmg`: aarch64 in PR CI, universal for releases.
  - Linux `.deb` and `.AppImage`, built on Ubuntu 22.04 with
    `libwebkit2gtk-4.1-dev`.
  - Windows later.
  - Builds stay unsigned until signing is set up (Part 6); the macOS app is
    ad-hoc signed (`signingIdentity: "-"`) so Apple Silicon will launch it.
    Signing credentials
    live only in GitHub Actions secrets.

### 2.4 Frontend: SolidJS + TypeScript + Vite

**Decision: SolidJS 1.x with strict TypeScript, built by Vite, linted and
formatted by Biome, tested with Vitest.**

| Option | Update model | Runtime (gz, approx.) | Verdict |
|---|---|---|---|
| **SolidJS** | Compiled, fine-grained signals, no virtual DOM. A tick updates exactly the bound text node. | ~7 KB | **Chosen** |
| Svelte 5 | Compiled, fine-grained (runes) | small | Close second. Needs `.svelte` preprocessing and fits typed stream primitives less naturally. |
| React | Virtual DOM diff and reconciliation on every update. High tick rates need memo discipline. | ~45 KB | Rejected: per-update overhead and the pull of a large dependency ecosystem |
| Vanilla TS | Manual DOM | 0 | Rejected: we would re-implement reactivity, components and cleanup, which slows feature work |

Why Solid fits a ticking terminal:

- **Fine-grained reactivity.** A streamed quote calls a signal setter and only the
  DOM nodes bound to that signal change. There are no component re-renders, no
  diffing and no memoisation tuning. Dense tables of moving prices stay cheap.
- **Components run once.** Performance is predictable and there are no
  hook-ordering pitfalls.
- **Leak-free subscriptions.** `onCleanup` ties IPC subscriptions to component
  lifetime. A `createQuote(id)` primitive subscribes on mount and unsubscribes on
  unmount, so closed panels stop polling.
- **Small and boring.** Tiny runtime, plain TSX, one Vite plugin
  (`vite-plugin-solid`), mature 1.x API.

**Dependency diet** (enforced in review):

- No UI kits, no CSS frameworks, no CSS-in-JS runtime. Use plain CSS with custom
  properties.
- No Monaco, moment or lodash.
- Charts use TradingView `lightweight-charts`: Apache-2.0, canvas-based, about
  45 KB gzipped. It is lazy-loaded on the first chart view, and its attribution is
  kept as its licence requires.
- Table virtualisation is hand-rolled because it is small.
- Any new runtime dependency needs a justification and its gzipped size in the PR
  description.

**Rendering rules:**

- Coalesce streaming updates per `requestAnimationFrame`.
- Virtualise any list longer than about 100 rows.
- Use fixed row heights and `font-variant-numeric: tabular-nums` so numbers don't
  shift layout.
- Apply `contain` on panels.
- No blur, shadows or animations. WebKitGTK on Linux is noticeably slower than
  WKWebView, so performance is checked on both.
- Use canvas for heatmaps and sparklines.

**Look:**

- Black background with amber/orange accents.
- Green/red for up/down, always paired with ▲/▼ for colour-blind users.
- A bundled OFL monospace font.
- Dense 12–13 px text with uppercase labels.

**Interaction model:**

- **Type anywhere.** Printable keys go to the command bar. Enter means GO, Esc
  clears and then goes back, ↑/↓ walks history, Tab autocompletes. View-local
  shortcuts use non-letter keys (arrows, `+`/`-`, `[`/`]`, Delete) or modifiers,
  so they never collide with typing.
- **Grammar:** `[EXCH:]TARGET [FUNCTION] [ARGS…]`, case-insensitive. Index
  aliases take precedence over stock symbols, and NSE is the default exchange.
- **Function keys:** F1–F10, with Ctrl+1…Ctrl+0 as equivalents because Mac
  keyboards need Fn for F-keys. F11 and F12 are avoided because macOS reserves
  them.
- **Panels:** layouts of 1, 2 or 4 panels. Each panel has its own view stack, and
  Tab/Shift+Tab cycles focus.
- **Keymap:** table-driven and rendered into HELP, so it can't drift and can be
  remapped later.

| Input | Opens | Part |
|---|---|---|
| `RELIANCE` ⏎ | Quote page (NSE) | 1 (placeholder), 2 |
| `BSE:RELIANCE` ⏎ | BSE quote | 2 |
| `NIFTY`, `BANKNIFTY`, `SENSEX` ⏎ | Index view with constituents | 1 (placeholder), 2 |
| `MKT` / F2 | Index dashboard | 2 |
| `WL`, `WL ADD TCS` / F3 | Watchlists | 2 |
| `RELIANCE CH 5M` / F4 | Candlestick chart | 3 |
| `MOV` / F5, `SEC` / F6, `BRD`, `NEWS` / F7 | Movers, sectors, breadth, news | 3 |
| `LOGIN`, `PORT` / F8, `POS`, `ORD` / F9, `FUNDS` | Broker views | 4 |
| `STRAT` / F10, `BT`, `PAPER` | Strategies, backtests, paper trading | 5 |
| `RISK`, `AUDIT`, `KILL` | Safety | 6 |
| `HELP`, `?`, F1 | Help and command list | 1 |

### 2.5 Data-provider layer (`nivesh-data`)

```rust
/// Where a value came from and how fresh it is. Attached to every market datum.
pub struct Provenance {
    pub provider: ProviderId,       // "nse" | "yahoo" | "kite" | …
    pub feed: FeedKind,             // Live | Delayed | EndOfDay
    pub as_of: DateTime<Utc>,       // timestamp of the data itself
    pub fetched_at: DateTime<Utc>,
}
pub struct Sourced<T> { pub data: T, pub provenance: Provenance }

#[async_trait]
pub trait DataProvider: Send + Sync + 'static {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;   // QUOTE | HISTORY | INDEX | MOVERS | BREADTH | NEWS | INSTRUMENTS
    fn feed_kind(&self) -> FeedKind;
    // Every method has a default impl returning Err(DataError::Unsupported).
    async fn quotes(&self, ids: &[InstrumentId]) -> Result<Vec<Sourced<Quote>>, DataError>;
    async fn history(&self, req: &HistoryRequest) -> Result<Sourced<CandleSeries>, DataError>;
    async fn index_snapshot(&self, index: IndexId) -> Result<Sourced<IndexSnapshot>, DataError>;
    async fn movers(&self, req: &MoversRequest) -> Result<Sourced<Movers>, DataError>;
    async fn news(&self, req: &NewsRequest) -> Result<Vec<NewsItem>, DataError>;
    async fn instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>, DataError>;
}

pub enum DataError {
    Unsupported,
    RateLimited { retry_after: Option<Duration> },
    Blocked,          // e.g. 403 after a session refresh
    Unauthorized,
    NotFound,
    Timeout,
    Upstream(String),
    Parse(String),
}
```

**`MarketData` facade.** The rest of the app only ever talks to this; it never
calls providers directly.

- **Routing.** Providers are tried in a configurable priority order per
  capability:

  | Capability | Default order |
  |---|---|
  | Quotes | Kite (when connected) → NSE → Yahoo |
  | History | Kite (when connected and entitled) → Yahoo → NSE |
  | Index snapshot and constituents | NSE → Yahoo (index level only) |
  | Movers and breadth | NSE → computed locally from constituents |
  | Instruments and search | Local instrument master (refreshed daily) |
  | News | RSS feeds plus NSE announcements |

- **Fallback.** Moves to the next provider on `Unsupported`, `Timeout`,
  `RateLimited`, `Blocked`, `Upstream` and `Parse`. `NotFound` also tries the
  next provider, and only becomes `NotFound` when every provider says so.
- **Health.** A per-provider circuit breaker opens after N consecutive failures,
  waits out a cool-down, then sends a half-open probe. Health is pushed to the
  status bar as a provider-status event.
- **Coalescing.** Identical in-flight requests share one upstream call
  (singleflight).
- **Caching.** An in-memory `moka` TTL cache sits in front of a SQLite candle
  cache. Indicative TTLs:

  | Data | TTL |
  |---|---|
  | Quotes | 5–15 s |
  | Index snapshots | 15 s |
  | Movers | 60 s |
  | Intraday history | 1 min |
  | Daily history | until the next session close |
  | Instrument master | 24 h |
  | News | 5 min |

  Stale entries are served immediately while a refresh runs in the background
  (stale-while-revalidate). When offline, cached data is returned flagged
  **STALE**.
- **Series consistency.** A single candle series always comes from one provider.
  Providers differ in adjustment and timestamps, so a fallback re-fetches the
  whole series rather than splicing two sources together.
- **QuoteHub.** UI subscriptions are reference-counted per instrument.
  - The poller only fetches instruments some open view is showing.
  - It batches requests where a provider allows it.
  - It slows down outside market hours and pauses when the window is hidden.
  - When a broker is connected, the hub moves those instruments to the broker
    WebSocket and the provenance becomes LIVE.

**Freshness labels** (rendered by one shared badge component):

| Label | Meaning |
|---|---|
| `LIVE` | Broker real-time feed only |
| `DELAYED` | Public sources. Always shown with the provider and as-of time. |
| `EOD` | End-of-day data |
| `STALE` | Older than a per-kind threshold, or served offline from cache |
| `PAPER` | Simulated trading state, not real positions or orders |

### 2.6 Broker adapter layer (`nivesh-broker`)

The adapter is split by capability, so read-only access, which is all Part 4
needs, can't accidentally grow order placement:

```rust
#[async_trait] pub trait BrokerAuth: Send + Sync {
    fn login_url(&self, state: &LoginState) -> Url;
    async fn complete_login(&self, callback: LoginCallback) -> Result<SessionInfo, BrokerError>;
    async fn logout(&self) -> Result<(), BrokerError>;
}
#[async_trait] pub trait BrokerAccount: Send + Sync {
    fn broker_id(&self) -> BrokerId;
    async fn session(&self) -> Result<SessionStatus, BrokerError>;
    async fn holdings(&self) -> Result<Vec<Holding>, BrokerError>;
    async fn positions(&self) -> Result<Positions, BrokerError>;
    async fn orders(&self) -> Result<Vec<Order>, BrokerError>;
    async fn trades(&self) -> Result<Vec<Trade>, BrokerError>;
    async fn funds(&self) -> Result<Funds, BrokerError>;
}
#[async_trait] pub trait MarketStream: Send + Sync {
    async fn subscribe(&self, ids: &[InstrumentId], mode: TickMode) -> Result<TickSubscription, BrokerError>;
}
// Added in Part 5. Implemented by PaperBroker (Part 5) and Kite (Part 6, feature `live-trading`).
#[async_trait] pub trait BrokerExecution: BrokerAccount {
    async fn place(&self, order: ApprovedOrder) -> Result<OrderAck, BrokerError>;
    async fn modify(&self, change: ApprovedModification) -> Result<OrderAck, BrokerError>;
    async fn cancel(&self, id: &BrokerOrderId) -> Result<OrderAck, BrokerError>; // reduces risk: no approval needed
}
```

- **`ApprovedOrder`** is defined in `nivesh-risk`. Its fields are private and only
  `RiskEngine::check` can construct one, so the compiler makes the risk gate
  non-optional.
  - It carries the risk decision ID, which links it to the audit log.
  - It has a short expiry, so a stale approval can't be replayed.
- **Only `nivesh-exec::OrderGateway` holds `BrokerExecution` objects.** Strategies
  emit `OrderIntent`s. Tauri commands and the UI never get a broker handle that
  can trade.
- **Idempotency.** Every order carries a client tag (strategy ID plus a
  sequence). An ambiguous failure, such as a timeout after sending, triggers
  reconciliation against the broker order book. The gateway never blindly
  retries placement.
- **Order state machine.** It lives in `nivesh-broker`:
  `Pending → Open → (PartiallyFilled) → Filled | Cancelled | Rejected`. Transition
  rules are tested and broker statuses map onto it.
- **Adding a broker** (Upstox, Angel One, Dhan) means implementing the same traits
  and passing the shared conformance suite in `nivesh-broker::conformance`, which
  runs against a mock server.
- **Credentials** come from `nivesh-secrets` and are never returned over IPC.

### 2.7 IPC design

- **Commands** handle request/response: async `#[tauri::command]` returning
  `Result<T, IpcError>`. Rust names are `snake_case`, wrapped by `camelCase`
  functions in `ui/ipc/`. **Components never call `invoke` directly.**
- **Channels** (`tauri::ipc::Channel<T>`) carry per-subscription streams:
  - quote and tick batches
  - the forming candle
  - backtest progress
  - paper/live strategy state

  They are ordered and cheaper than global events. The pattern is
  `subscribe_quotes(ids, channel) -> SubscriptionId` and `unsubscribe(id)`. The
  backend drops a subscription when its channel closes.
- **Events** carry low-frequency, app-wide broadcasts named `domain:event`:
  - `provider:status`
  - `broker:session`
  - `market:session`
  - `killswitch:changed`
  - `risk:alert`
- **Typed bindings.** IPC types derive `ts_rs::TS` and are exported to
  `ui/ipc/bindings/`. `pnpm gen:bindings` regenerates them, and CI fails if the
  committed bindings are stale. We chose `ts-rs` because it is stable; we can
  revisit `tauri-specta` once it is.
- **Errors.** `IpcError { code: ErrorCode, message: String, retryable: bool }`.
  Codes are stable enums so the UI can react, for example by showing a re-login
  prompt.
- **Payloads.**
  - Display data (quotes, candles) uses `f64` rupees.
  - Order and position payloads use integer paise.
  - Candle series are columnar (`{t:[], o:[], h:[], l:[], c:[], v:[], provenance}`),
    not arrays of objects. This keeps JSON small and maps straight onto chart
    APIs.
  - Payloads over about 1 MB use `tauri::ipc::Response` raw bytes.
- **Back-pressure.** The backend coalesces ticks per instrument and sends at most
  about 10 batches per second per subscription (configurable). The UI applies
  each batch in one animation frame.
- **Safety boundary.** The Rust side never trusts the UI for safety. Risk checks,
  kill-switch state and live arming are all enforced in Rust. Arming live,
  loosening limits and resetting the kill switch need a typed confirmation phrase
  that Rust validates.

### 2.8 Persistence, secrets, logging

- **SQLite.** A single database in the Tauri app data directory, in WAL mode.
  There is one writer thread, and migrations are versioned and tested on every
  change.
- **Secrets.** Kite API secret, access tokens and similar values live only in the
  OS keychain. They never go into SQLite, config files, logs, IPC payloads, crash
  output or test fixtures. `secrecy::SecretString` keeps them out of
  `Debug`/`Display`.
- **Diagnostics log.** `tracing` writes rolling files in the app log directory,
  and also to stderr in dev. URLs are logged without query strings that could
  carry tokens.
- **Audit log** (Part 6). Separate from diagnostics: append-only and hash-chained
  in SQLite, exportable.
- **No telemetry.**

### 2.9 Performance budgets

| Metric | Budget |
|---|---|
| Cold start to interactive shell (release) | < 1.0 s on Apple Silicon, < 2.0 s on Linux |
| Keystroke to command-bar suggestions | < 16 ms (local search index) |
| View switch with cached data | < 50 ms. Show a placeholder only if it takes longer than 150 ms. |
| Tick to paint | < 50 ms, batched per animation frame |
| First render of a 5,000-candle chart | < 100 ms. Pan and zoom at 60 fps. |
| Initial JS bundle (gz) | < 150 KB excluding the lazy-loaded chart library |
| Idle CPU (window hidden or market closed) | ≈ 0–1 % |
| Typical memory (RSS) | < 300 MB |

- Rust benchmarks (`criterion`) cover indicators, resampling and backtests.
- Every UI PR is checked against the bundle size budget.
- A regression must be called out in the PR.

### 2.10 Testing strategy

- **Unit tests** in every crate. Pure logic (core, indicators, risk, strategy
  evaluator) gets the most coverage.
- **Fixture-based parser tests.** Every provider and broker response shape is
  recorded once, scrubbed of tokens and account identifiers, and stored under
  `tests/fixtures/`.
- **Property tests** (`proptest`):
  - batch indicators equal streaming indicators
  - risk limits are never exceeded
  - the Kite tick parser never panics on arbitrary bytes
  - backtests never look ahead
- **Golden tests** (`insta`) for backtest results and metrics.
- **Integration tests** against `wiremock` and local WebSocket mock servers, for
  NSE/Yahoo session handling and Kite login, REST and ticker flows.
- **Frontend tests:** Vitest for the command parser, keymap, stores and IPC
  wrappers, plus light component tests.
- **No network in CI.** Live smoke tests are marked `#[ignore]` and run manually
  (`cargo test -p nivesh-data -- --ignored`). They never place orders.

---

## 3. Delivery workflow

- There are six parts. Each part is one branch (`part-<n>-<slug>`) and one PR into
  `main`, titled `Part <n>: <name>`.
- A PR has 10–15 meaningful commits. They are not padded and not squashed, and
  each one builds and passes tests on its own, so the history stays bisectable.
- The PR description contains:
  - a summary
  - the commit list
  - this plan's acceptance criteria as a ticked checklist with evidence
  - how to test
  - screenshots for UI changes
- Merge with a **merge commit** once CI is green.
- **Check-ins with Akshat** happen after Part 2 is merged and after Part 4 is
  merged. Work stops for review and feedback before the next part starts.
- The commit lists below are the intent. Small reordering or merging is fine if it
  keeps commits meaningful, and the PR description notes any change.

---

## Part 1: Foundation

**Branch:** `part-1-foundation`

### Goals

- A buildable Cargo workspace plus a Tauri 2 app with a SolidJS UI, running on
  macOS and Linux.
- The terminal shell: theme, multi-panel layout, command bar, view routing,
  keyboard navigation and help.
- A typed IPC foundation and the SQLite store, both used from Part 2 onward.
- CI that runs fmt, clippy, tests and frontend checks, builds a macOS `.dmg`
  artifact and runs a Linux bundle build.
- No network access yet.

### Commits

1. `chore: set up cargo workspace and nivesh-core crate`
   - Root `Cargo.toml` with workspace dependencies, lints and release profile.
   - `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`.
   - `nivesh-core` with `Exchange` and `InstrumentId` parsing (`RELIANCE`,
     `BSE:RELIANCE`), plus unit tests.
2. `chore(ui): scaffold solid, vite and typescript frontend`
   - pnpm `package.json`, Vite config, strict `tsconfig`, Biome, Vitest,
     `ui/main.tsx`.
   - Scripts: `lint`, `typecheck`, `test`, `build`.
3. `feat(app): add tauri 2 app crate`
   - `src-tauri` wired to the Vite dev server and to `dist`.
   - Single main window, strict CSP, minimal capabilities, single-instance plugin,
     `tracing` setup, placeholder icons.
4. `feat(app): add typed ipc foundation`
   - `IpcError` and an `app_info` command.
   - `ts-rs` export to `ui/ipc/bindings`, a `ui/ipc` wrapper and
     `pnpm gen:bindings`.
   - A test that fails when bindings are stale.
5. `feat(ui): add terminal theme and base components`
   - Design tokens: black, amber, up/down colours, density.
   - Bundled monospace font.
   - `Panel`, `DataTable` (fixed row height), `Badge`, `KeyHint`.
6. `feat(ui): add multi-panel shell layout`
   - Header with IST clock and a market-status slot.
   - 1/2/4-panel grid, status bar, focused-panel highlight, layout switching.
7. `feat(ui): add command parser and registry`
   - Pure TypeScript grammar (`[EXCH:]TARGET [FUNCTION] [ARGS]`, aliases).
   - The registry is the single source for dispatch, HELP and autocomplete.
   - Table-driven tests.
8. `feat(ui): add command bar with history and autocomplete`
   - Type-anywhere capture, Enter means GO, Esc clears and goes back.
   - ↑/↓ history, Tab completion, inline error feedback.
9. `feat(ui): add per-panel view stack and placeholder views`
   - A command opens its view in the focused panel, and Esc goes back.
   - `RELIANCE` opens a placeholder quote view and `NIFTY` a placeholder index
     view.
10. `feat(ui): add keymap, function keys and help view`
    - Table-driven keymap: F1–F10 plus Ctrl+digit fallbacks, Tab panel cycling,
      Alt+1/2/4 to switch layout.
    - Doesn't hijack keys while an input is focused.
    - `HELP`, `?` and F1 render the registry and keymap.
    - Unit tests.
11. `feat(core): add ist market calendar and session status`
    - Pre-open, normal, closing and closed sessions.
    - Bundled holiday data file for the current year.
    - `market_status` command and a header indicator, polled every 15 s (the
      `market:session` event replaces polling in Part 2). Holiday data is
      flagged `calendar_known: false` for years without a bundled file.
    - Boundary tests.
12. `feat(store): add sqlite store with migrations`
    - `nivesh-store` with bundled rusqlite, WAL mode and a migration runner.
    - Settings table, persisted layout, command history and window state.
      Window geometry is restored only if it is still reachable on a connected
      monitor.
13. `ci: add rust and frontend checks`
    - Ubuntu and macOS matrix.
    - `cargo fmt --check`, `clippy -D warnings`, `cargo test`, Biome, `tsc`,
      Vitest, binding freshness.
    - Rust and pnpm caching.
14. `ci: build macos dmg and linux bundles as artifacts`
    - `tauri build` on macOS produces an aarch64 `.dmg`.
    - Ubuntu 22.04 produces `.deb` and `.AppImage` (with webkit2gtk-4.1 system
      dependencies).
    - Uploads artifacts. Builds are unsigned for now.
15. `docs: add readme with setup and keyboard reference`
    - Prerequisites per OS, dev and build commands.
    - How to open an unsigned `.dmg`.

### Acceptance criteria

- [ ] `pnpm install && pnpm tauri dev` opens the terminal shell on macOS and Linux.
- [ ] Typing `RELIANCE` then Enter opens the placeholder quote view in the focused
      panel, `NIFTY` opens the placeholder index view, Esc goes back, and
      `HELP`/F1 lists every registered command and key.
- [ ] Every interaction works without a mouse, and focus is always visible.
- [ ] Layout, command history and window geometry persist across restarts.
- [ ] CI is green on the PR (fmt, clippy `-D warnings`, Rust tests, Biome, `tsc`,
      Vitest, bindings fresh). The macOS `.dmg` and Linux `.deb`/`.AppImage`
      artifacts can be downloaded.
- [ ] Cold-start time and bundle size are measured and recorded in the PR against
      §2.9.
- [ ] The app makes no network requests. Capabilities and CSP are minimal.

---

## Part 2: Browse-mode data

**Branch:** `part-2-browse-data`

### Goals

- Real market data with no login:
  - a data-provider layer with fallback, rate limiting and caching
  - an index dashboard (NIFTY 50, SENSEX, BANK NIFTY, sector indices)
  - local instrument search
  - quote pages
  - persisted watchlists
- Clear DELAYED and STALE labelling on everything.

### Commits

1. `feat(core): add market data types and provenance`
   - `Quote`, `Candle`/`CandleSeries`, `Interval` and `Price` (paise).
   - `IndexId` catalogue: NIFTY 50, SENSEX, NIFTY BANK and sector indices
     (IT, AUTO, PHARMA, FMCG, METAL, REALTY, ENERGY, PSU BANK, FIN SERVICE, MEDIA).
   - `FeedKind`, `Provenance`, `Sourced<T>`.
2. `feat(data): add data provider trait and error taxonomy`
   - Capabilities, with methods defaulting to `Unsupported`.
   - The `DataError` taxonomy.
   - A scripted mock provider for tests.
3. `feat(data): add shared http client with per-host rate limits`
   - reqwest with rustls, a cookie jar and `governor` token buckets per host.
   - Timeouts, jittered backoff for idempotent GETs, `Retry-After` handling.
   - Redacted request logging.
   - Tests use a mock clock and wiremock.
4. `feat(data): add yahoo finance provider`
   - Chart endpoint for quotes and history.
   - `.NS`/`.BO` and index ticker mapping as a data table.
   - Provider interval and range limits exposed as typed errors.
   - Fixture parser tests and an `#[ignore]` live smoke test.
5. `feat(data): add nse public endpoints provider`
   - Session bootstrap: cookie warm-up and browser-like headers.
   - Re-bootstraps once on 401/403, then backs off and reports `Blocked`.
   - Quotes, all-indices, index constituents, market status.
   - Conservative rate limits. Fixtures plus an ignored smoke test.
6. `feat(data): add provider chain with fallback and health tracking`
   - The `MarketData` facade with per-capability priorities.
   - Circuit breaker, singleflight, the single-provider-per-series rule.
   - `provider:status` events.
7. `feat(data): add tiered caching with stale-while-revalidate`
   - `moka` TTLs per data kind and a SQLite candle cache in `nivesh-store`.
   - Offline reads return cached data flagged STALE.
8. `feat(data): add instrument master and local symbol search`
   - NSE and BSE equity lists refreshed daily into SQLite, with a bundled seed for
     the first offline run.
   - In-memory ranked index: exact symbol, then prefix, then fuzzy matching on
     symbol and name.
9. `feat(data): add quote hub with ref-counted subscriptions`
   - Polls only instruments a view is showing, batched.
   - Adaptive intervals for market open or closed, and paused while the window is
     hidden.
10. `feat(app): expose market data over ipc`
    - `search_instruments`, `get_quote`, `index_snapshot`,
      `subscribe_quotes`/`unsubscribe` (over a Channel), `market_status`.
    - Updated bindings.
11. `feat(ui): add index dashboard and index view`
    - `MKT`/F2: a grid of the major and sector indices.
    - `NIFTY` and similar: the index header plus a constituents table that
      updates in place.
12. `feat(ui): add live symbol search to the command bar`
    - Debounced backend search with keyboard selection and exchange tags.
13. `feat(ui): add quote page`
    - LTP, change and %, OHLC, previous close, volume and 52-week range.
    - Price bands where available.
    - Source and as-of time.
14. `feat(ui): add persisted watchlists`
    - Store tables and IPC for multiple named lists.
    - `WL`/F3 view with keyboard add, remove and reorder.
    - `WL ADD <SYM>` and `WL RM <SYM>`.
15. `feat(ui): add delayed-data badges and staleness indicators`
    - A shared provenance badge that `DataPanel` requires: DELAYED or STALE, with
      as-of time and source.
    - Provider health shown in the status bar.

### Acceptance criteria

- [ ] With no login, `MKT` shows NIFTY 50, SENSEX, NIFTY BANK and at least 8
      sector indices. `NIFTY` shows its constituents updating in place.
- [ ] `RELIANCE` then Enter shows a populated quote page labelled DELAYED, with
      source and as-of time.
- [ ] Typing `relia` suggests RELIANCE within 50 ms, served from the local index.
- [ ] Forcing NSE to fail (via a test hook or simulated blocking) makes the app
      fall back to Yahoo automatically. The UI shows the source change, and no
      series mixes providers.
- [ ] Configured per-host rate limits are never exceeded. Unit tests prove this
      with a mock clock. Polling stops for views that are not visible and pauses
      while the window is hidden.
- [ ] Watchlists, including multiple lists, survive restart and are fully
      keyboard-operable.
- [ ] Offline start works: cached data shows as STALE, nothing crashes, and errors
      appear inline.
- [ ] Every provider parser has fixture tests, and CI makes no network calls.
- [ ] **Check-in with Akshat after merge.**

---

## Part 3: Charts & market views

**Branch:** `part-3-charts-markets`

### Goals

- Fast candlestick charts with indicators computed in Rust.
- Market views: top gainers and losers, most active, market breadth, sectors and
  news.
- Performance budgets enforced for charts and large tables.

### Commits

1. `feat(indicators): add moving averages with streaming state`
   - SMA, EMA and WMA, each with a batch and an incremental API.
   - Tests against reference values.
2. `feat(indicators): add rsi, macd, bollinger bands, atr and vwap`
   - Wilder smoothing and session-anchored VWAP.
   - Reference vectors and a property test that batch equals streaming.
3. `feat(data): add history requests with resampling and session alignment`
   - Intervals from 1m to 1M.
   - Resampling for intervals a provider doesn't offer, aligned to IST market
     sessions.
   - Gap handling and typed out-of-range errors.
4. `feat(app): serve columnar candle series and stream the forming bar`
   - `get_history` returns columnar arrays.
   - The forming last candle comes from QuoteHub over a Channel.
5. `feat(ui): add candlestick chart view`
   - Lazy-loaded `lightweight-charts` in the terminal theme.
   - Volume pane, crosshair OHLCV readout and provenance badge.
6. `feat(ui): add chart keyboard controls and intervals`
   - `RELIANCE CH 5M` and range arguments.
   - `+`/`-` zoom, ←/→ pan, Home/End, interval hotkeys.
7. `feat(ui): add indicator overlays and sub-panes`
   - `ADD SMA 50`, `ADD RSI 14`, `RM RSI`.
   - Values come from the Rust `compute_indicators` command.
   - Overlays persist per symbol.
8. `feat(data): add top gainers, losers and most active`
   - NSE endpoints, with a fallback computed from index constituent quotes.
9. `feat(ui): add market movers view`
   - `MOV`/F5, by index universe.
   - Enter jumps to the quote, and a modifier key opens it in another panel.
10. `feat(data): add market breadth and sector performance`
    - Advances, declines and unchanged per index.
    - % of stocks above their 20/50/200-day moving average for NIFTY 50 and 100,
      from cached daily candles fetched at a limited rate.
    - Sector returns over 1D, 1W, 1M and YTD.
11. `feat(ui): add breadth and sectors views with heatmap`
    - `BRD`, and `SEC`/F6 with a canvas heatmap of constituents by % change.
12. `feat(data): add news from rss feeds and exchange announcements`
    - `feed-rs` with a configurable feed list.
    - Deduplication and symbol tagging via the instrument master. Cached.
13. `feat(ui): add news panel`
    - `NEWS`/F7 and `RELIANCE NEWS`.
    - Headlines and sources only. Links open in the system browser through the
      opener allowlist, and remote HTML is never rendered.
14. `perf(ui): virtualize large tables and batch tick rendering`
    - rAF coalescing and virtualised rows.
    - Measured against §2.9.
15. `perf(indicators): add criterion benchmarks for indicators and resampling`
    - Benchmarks at 100k candles, with budgets recorded in the README.

### Acceptance criteria

- [ ] `RELIANCE CH` renders a daily chart from cache within the §2.9 budget.
      Pan and zoom stay at 60 fps with 5,000 or more candles on macOS, and are
      checked on Linux.
- [ ] Intervals 1m, 5m, 15m, 1h, 1D and 1W work within provider limits. When a
      range isn't supported, the UI explains why instead of showing an empty
      chart.
- [ ] SMA, EMA, WMA, RSI, MACD, Bollinger, ATR and VWAP match the reference
      datasets within a documented tolerance. Batch and streaming results agree.
- [ ] Movers, breadth, sectors and news can all be opened by command and by
      function key. All are labelled DELAYED or STALE, and all work without a
      mouse.
- [ ] News never renders remote content in the webview, and only http/https links
      open.
- [ ] The bundle budget still holds, and the chart library loads only on first
      chart open.

---

## Part 4: Broker connection

**Branch:** `part-4-broker-kite`

### Goals

- The broker adapter traits.
- A complete Zerodha Kite Connect integration: login, holdings, positions,
  orders, trades and funds.
- Live ticks over WebSocket, with secrets in the macOS Keychain.
- Read-only: **this part has no order-placement capability.**

### Commits

1. `feat(secrets): add secret store with keychain backend`
   - `SecretStore` trait with a `keyring` backend and an in-memory test backend.
   - `secrecy` types and redaction tests.
2. `feat(broker): add broker traits and account domain types`
   - `BrokerAuth`, `BrokerAccount` and `MarketStream`.
   - Holding, Position, Order, Trade and Funds types.
   - The order state machine, `BrokerError`, and the conformance harness.
3. `feat(broker-kite): add kite connect rest client`
   - Version and auth headers, response envelope parsing, error mapping.
   - Rate limits per endpoint class.
   - Tests against scrubbed wiremock fixtures.
4. `feat(broker-kite): add login flow with loopback redirect`
   - Builds the login URL with a random `state` echoed through redirect params.
   - A one-shot `127.0.0.1` listener receives the redirect.
   - Exchanges the `request_token` with the SHA-256 checksum.
   - Stores the token in the Keychain with its expiry. Supports logout.
5. `feat(broker-kite): fetch holdings, positions, orders, trades and funds`
   - Maps Kite responses onto domain types.
   - Passes the conformance suite.
6. `feat(broker-kite): add instrument dump and token mapping`
   - Daily instrument CSV into SQLite.
   - `instrument_token` ↔ `InstrumentId` mapping.
7. `feat(broker-kite): add websocket ticker`
   - Binary packet parser for ltp, quote, full and index packets, plus order
     update messages.
   - Heartbeat, and reconnect with backoff and resubscribe.
   - Property and fuzz tests on the parser.
8. `feat(app): add broker manager and session ipc`
   - `broker_setup` (credentials go Rust → Keychain and are never returned),
     `broker_login`, `broker_logout`, `broker_status`.
   - `broker:session` events and token-expiry handling.
9. `feat(data): add broker-backed live provider`
   - Kite quotes and entitled history at top priority.
   - QuoteHub switches to the ticker when connected, and provenance becomes LIVE.
   - Automatically drops back to DELAYED on disconnect.
10. `feat(ui): add broker setup, login and connection status`
    - `LOGIN` and `LOGOUT`, and a masked credential form.
    - Status bar shows LIVE or DELAYED and the session expiry.
11. `feat(ui): add holdings and positions views`
    - `PORT`/F8 and `POS`, with live mark-to-market (MTM) from ticks.
    - Day and overall P&L, and totals.
12. `feat(ui): add orders, trades and funds views`
    - `ORD`/F9, `TRD`, `FUNDS`, all read-only.
13. `test(broker-kite): add end-to-end tests against mock kite server`
    - Login, fetch and ticker flows against a local WebSocket server.
    - Reconnect and token expiry.
    - Checks that secrets never reach logs, SQLite or IPC payloads.
14. `docs: document kite connect setup and data entitlements`
    - Developer app, redirect URL, daily login, subscriptions and limits.

### Acceptance criteria

- [ ] The API key and secret are entered once and stored only in the Keychain.
      Automated tests prove they are absent from SQLite, logs, config and IPC
      responses.
- [ ] `LOGIN` opens the system browser and completes Kite login, and the status
      bar shows the connected state. An expired token (the daily reset) produces
      a clear re-login prompt, not errors.
- [ ] Holdings, positions, orders, trades and funds match Kite web for Akshat's
      account (checked manually by Akshat).
- [ ] Ticks for visible instruments reach the screen within 100 ms of receipt.
      The badge switches to LIVE only for broker-sourced data, and back to
      DELAYED on disconnect.
- [ ] The WebSocket reconnects and resubscribes automatically after a network
      drop.
- [ ] The Kite adapter doesn't implement `BrokerExecution`, and no IPC command
      can place, modify or cancel an order.
- [ ] CI runs only against mock servers.
- [ ] **Check-in with Akshat after merge.**

---

## Part 5: Strategies

**Branch:** `part-5-strategies`

### Goals

- A rule-based strategy builder: MA crossover, RSI, breakout, stop-loss and
  target.
- A deterministic backtester on historical data.
- Paper trading through the same order gateway that live trading will use in
  Part 6.
- Everything paper: no path to a real broker exists in this part.

### Commits

1. `feat(strategy): add rule model`
   - Versioned, serde-serialisable rules: indicator references, comparisons,
     crosses above/below, AND/OR/NOT.
   - Entry and exit rules, and sizing (fixed quantity, fixed ₹ or % of equity).
   - Stop-loss, target and trailing stop.
2. `feat(strategy): add rule validation and bar-by-bar evaluator`
   - Warm-up and lookback checks.
   - Decides on closed bars only, deterministically, using streaming indicators.
3. `feat(strategy): add built-in templates`
   - MA crossover, RSI mean-reversion, N-bar breakout. Each has stop-loss and
     target parameters.
4. `feat(risk): add risk crate with basic checks and approved-order token`
   - `RiskEngine::check` covers per-order quantity and value and allowed
     segment.
   - `ApprovedOrder`/`ApprovedModification` with private constructors and expiry.
5. `feat(broker): add execution trait, fill model and paper broker`
   - `BrokerExecution` takes an `ApprovedOrder`.
   - The fill model fills at the next bar open or LTP ± slippage, and handles
     limit-touch.
   - The charges model is data-driven, with defaults approximating Zerodha
     equity charges.
   - Market hours are enforced. `PaperBroker` keeps a ledger.
6. `feat(strategy): add backtesting engine`
   - Event-driven over a `CandleSeries`, using the same evaluator and fill model.
   - Deterministic, with progress callbacks and cancellation.
7. `feat(strategy): add backtest metrics and trade log`
   - Total return, CAGR, max drawdown, Sharpe, Sortino, win rate, profit factor
     and exposure.
   - Equity curve and per-trade log.
8. `feat(store): persist strategies, backtest runs and paper ledger`
   - Migrations and repository APIs.
9. `feat(exec): add order gateway with paper routing`
   - The single entry point: intent → risk check → `ApprovedOrder` →
     `PaperBroker`.
   - Records each decision.
   - Live routing does not exist yet.
10. `feat(strategy): add paper trading runner`
    - Evaluates on bar close from QuoteHub data and sends intents to the gateway.
    - Running state persists and resumes after restart.
11. `feat(app): add strategy, backtest and paper ipc`
    - CRUD commands, `run_backtest` with Channel progress and cancellation, and
      paper start and stop.
12. `feat(ui): add keyboard-driven rule builder`
    - `STRAT`/F10: pick a template, then edit parameters and conditions.
    - Inline validation and a readable rule preview.
13. `feat(ui): add backtest results view`
    - `BT`: metrics table, equity and drawdown curves, trade list, and trade
      markers on the candlestick chart.
14. `feat(ui): add paper trading dashboard`
    - `PAPER`: running strategies with paper orders, positions and P&L.
    - Start and stop, with a PAPER watermark on every panel.
15. `test(strategy): add golden backtests and no-lookahead property tests`
    - Fixed datasets with `insta` snapshots for each template.
    - A property test: perturbing future bars never changes past decisions.
    - Documents backtest assumptions and limits.

### Acceptance criteria

- [ ] Using only the keyboard, a user can create MA-crossover, RSI and breakout
      strategies with stop-loss and target, save them, and backtest them on
      5 years of daily data for one symbol in under 1 s (excluding download).
- [ ] Backtests are deterministic: the same inputs give byte-identical results.
      The no-lookahead property test passes.
- [ ] Metrics match hand-computed fixtures.
- [ ] A paper strategy runs during market hours, produces paper orders and fills
      through `OrderGateway`, and survives an app restart. Every paper surface is
      labelled PAPER. Paper trading on DELAYED data is flagged as approximate.
- [ ] No code path can reach a real broker: Kite has no `BrokerExecution` impl
      and the gateway routes only to paper.

---

## Part 6: Live trading & safety

**Branch:** `part-6-live-safety`

### Goals

- Live trading as an explicit opt-in per strategy, behind:
  - full hard risk limits
  - a persistent kill switch
  - reconciliation
  - a tamper-evident audit log
- Sandboxed scripting.
- Polish: onboarding, settings, startup and bundle.

### Commits

1. `feat(risk): add full risk limit model`
   - Global limits plus per-strategy limits, which can only be tighter than
     global.
   - Per-order quantity and value, per-symbol exposure, maximum open positions.
   - Orders per minute and per day, daily loss (realised plus MTM).
   - Limit price within a band around LTP, allowed products and segments,
     trading window.
   - Pure functions with property tests.
2. `feat(store): add hash-chained append-only audit log`
   - SHA-256 chain, a verification routine, and CSV/JSONL export.
   - The audit write completes before an order is sent.
3. `feat(exec): add kill switch`
   - Engaging it:
     - blocks all new orders
     - cancels open Nivesh-tagged orders
     - disarms all live strategies
   - State persists across restart.
   - Auto-triggers: daily-loss breach, a storm of rejections, broker disconnect
     while live with open orders, stale data, clock drift.
   - Reset needs confirmation and a cool-down.
4. `feat(broker-kite): implement order placement, modification and cancellation`
   - Behind the `live-trading` Cargo feature.
   - Tag-based idempotency, market protection on market orders.
   - Never retries blindly. Order updates arrive by WebSocket and polling.
5. `feat(exec): add order and position reconciliation`
   - Runs at startup, on reconnect and periodically.
   - Unknown or mismatched state pauses the strategy and raises a `risk:alert`.
6. `feat(exec): add per-strategy live arming`
   - A global "live trading enabled" master switch, off by default.
   - Arming preconditions:
     - broker connected
     - LIVE data
     - limits configured
     - typed confirmation
   - Arming expires at session end.
   - Rejects any intent derived from DELAYED data.
7. `feat(app): add risk, kill switch and audit ipc`
   - A confirmation challenge for arming, loosening limits and resetting the kill
     switch.
   - `killswitch:changed` and `risk:alert` events.
8. `feat(ui): add risk limits view`
   - `RISK` shows current usage against limits.
   - Tightening applies immediately. Loosening needs confirmation and is audited.
9. `feat(ui): add kill switch control and live indicators`
   - An always-visible status-bar control, a dedicated shortcut (proposed
     Ctrl+Shift+K) and the `KILL` command.
   - An optional OS-level global shortcut.
   - A red LIVE banner on armed strategies.
10. `feat(ui): add audit log viewer`
    - `AUDIT` with filters, chain verification status and export.
11. `feat(script): add sandboxed rhai strategy scripting`
    - A limited API: bars, indicators and emitting intents.
    - Operation, time and memory limits, with no filesystem or network access.
    - Scripts can be backtested and are paper-only until armed like any
      strategy.
12. `feat(ui): add script editor and runner`
    - A lightweight textarea editor with line numbers and an error gutter (no
      Monaco).
    - Validate, backtest and paper-run.
13. `test(exec): add end-to-end safety scenarios`
    - Against a mock Kite server:
      - a limit breach is rejected and audited
      - a daily-loss breach auto-engages the kill switch
      - the kill switch cancels open orders and survives restart
      - DELAYED-derived intents are rejected
      - an ambiguous placement is reconciled, not retried
      - scripts hit their limits
14. `perf(ui): polish onboarding, settings and startup`
    - First-run onboarding explains DELAYED data, the paper default and the
      disclaimer.
    - A settings view.
    - Startup and bundle profiling against §2.9.
15. `docs: add trading safety guide and release checklist`
    - Operator guide for limits, arming and kill switch.
    - Release checklist, including signing and notarisation steps for when a
      Developer ID is available.

### Acceptance criteria

- [ ] On a fresh install, after every restart, and for every new strategy,
      trading is paper and disarmed. Going live needs the global master switch,
      a per-strategy arming with typed confirmation, and a connected broker
      streaming LIVE data. Arming expires at session end.
- [ ] Every live order passes `RiskEngine` (enforced by the type system) and is
      audited before it is sent. Audit chain verification passes, and tests show
      it detects a modified or deleted row.
- [ ] One keystroke engages the kill switch. Against the mock, open orders are
      cancelled within 1 s. The kill switch stays engaged across restart until
      reset with confirmation.
- [ ] A daily-loss breach automatically engages the kill switch (tested).
      Intents derived from DELAYED data are rejected (tested).
- [ ] An infinite loop in a script is stopped by the operation limit. Scripts
      cannot touch the filesystem or network (tested).
- [ ] A supervised live smoke test with the smallest possible size is done
      **manually by Akshat**. Agents and CI never do it.
- [ ] §2.9 budgets still hold, and all six parts' features work by keyboard
      alone.

---

## 10. Data-source limits

Free public data is what makes browse mode possible. These sources are also
fragile and unofficial. The design assumes they will break, throttle or change
without notice.

### NSE public endpoints (`www.nseindia.com/api/...`)

- **Not a supported API.** These are undocumented JSON endpoints behind the NSE
  website: equity quote, all-indices, index constituents, gainers/losers,
  market status, holiday master, corporate announcements. Instrument lists and
  bhavcopies come from NSE archive hosts. Paths and response shapes change
  without notice, so every endpoint is wrapped by a parser with fixtures and
  typed `Parse` errors.
- **Browser-like headers and session cookies are needed.** Requests without a
  normal `User-Agent`, `Accept`, `Accept-Language` and `Referer`, and without the
  cookies set by first loading an NSE page, usually get 401 or 403.
  - The provider warms up a session (cookie jar) and refreshes it when cookies
    expire, which happens within minutes.
  - On 401/403 it re-bootstraps **once**, then backs off and reports `Blocked`.
- **Rate limiting and blocking.** NSE throttles aggressively and may block IPs.
  Datacenter IPs such as CI runners are often blocked outright, so CI never calls
  NSE.
  - Defaults: about 1 request per second sustained per host with a small burst.
  - Quote polling no faster than every 5–15 s, and only for visible instruments.
  - Heavy caching, and no bulk crawling.
- **Terms of use.** NSE's website terms restrict automated access and
  redistribution. Nivesh uses these endpoints only for personal, low-volume
  viewing on the user's own machine, and never redistributes the data.
  **We do not try to defeat blocking.** That means no proxy or IP rotation, no
  fingerprint spoofing beyond ordinary headers, and no CAPTCHA solving. When NSE
  blocks, Nivesh falls back to another provider and says so.
- **Freshness.** NSE website data may look close to real time, but it is
  unofficial and has no timeliness guarantee. It is always labelled **DELAYED**
  with its as-of time.

### Yahoo Finance (`query1/query2.finance.yahoo.com`)

- **Unofficial, with no SLA.** We use the chart endpoint (`/v8/finance/chart/…`)
  for quotes and history. The quote endpoint now needs a cookie and crumb, so we
  avoid it. Response shapes and access rules change, and HTTP 429 is common.
- **Tickers:**
  - NSE stocks use the `.NS` suffix (`RELIANCE.NS`).
  - BSE stocks use `.BO` (`RELIANCE.BO`, or the scrip code as in `500325.BO`).
  - Indices use symbols such as `^NSEI` (NIFTY 50), `^BSESN` (SENSEX),
    `^NSEBANK` (NIFTY BANK) and `^CNXIT`.
  - The mapping is a data table checked by the live smoke test, because some
    sector tickers are missing or unreliable.
- **Delayed and approximate.** Yahoo marks Indian quotes as delayed. Treat them
  as **DELAYED**, with possible gaps, late corrections and adjustment
  differences.
- **Intraday history is limited** (approximate; verify during implementation):
  - 1-minute bars cover only the last few weeks, with a short window per request.
  - 2–90-minute bars cover about 60 days.
  - Hourly bars cover about 2 years.
  - Daily history goes back many years.
  - Requests outside these limits return a typed error that the UI explains.
- **Adjustment.** Yahoo daily data is split/dividend-adjusted and NSE data
  generally isn't. This is one reason a series never mixes providers.

### BSE and news

- BSE endpoints (`api.bseindia.com`) have the same unofficial nature, header
  requirements and fragility. They are lower priority, and Yahoo `.BO` covers BSE
  quotes.
- News comes from publisher RSS feeds and exchange announcements. We show
  headline, source, time and link only, never scraped article bodies, and links
  open in the system browser. Feed lists are configurable, so a broken or
  disallowed feed can be removed without a code change.

### Kite Connect (Part 4 onward)

- An official API, but it needs a Kite Connect developer app with an API key and
  secret.
  - Market data and historical candles may need a paid subscription or add-on.
  - The access token expires daily (around 6 AM IST the next day), so the user
    logs in every day.
- Documented limits include:
  - per-endpoint requests per second (quote and historical are stricter)
  - per-minute and per-day order caps
  - a cap on modifications per order
  - WebSocket caps on instruments per connection and connections per API key

  The adapter's rate limits must be set from the current docs at implementation
  time, not from memory.
- **Regulation.** SEBI's 2025 framework for retail algorithmic trading changed
  how API and algo orders work through brokers. It covers things like static-IP
  registration for API order placement, orders-per-second thresholds above which
  an algo must be exchange-registered, and order tagging. **Before Part 6 enables
  live orders, verify Zerodha's current requirements.** Nivesh's default order
  rate stays far below any unregistered-algo threshold.

### Design consequences

- Every datum carries `Provenance`, and the UI always shows DELAYED, LIVE, EOD,
  STALE or PAPER.
- Fallback chains, circuit breakers, caching and conservative rate limits are
  built in from Part 2.
- Live trading (Part 6) never uses public data. Only broker real-time data can
  feed a live strategy.
- Provider breakage is expected maintenance work. Fixtures make a parser fix a
  small, testable change.

---

## 11. Trading safety principles

These rules are non-negotiable. Any change that touches `nivesh-risk`,
`nivesh-exec` or any `BrokerExecution` implementation must uphold them and include
tests for the failure paths.

1. **Paper by default.** Fresh installs, new or imported strategies, scripts and
   every app restart start in paper mode. Nothing ever silently becomes live.
2. **Live is deliberate, per strategy, and expires.** Going live needs all of:
   - the global master switch, which is off by default
   - per-strategy arming with a typed confirmation that Rust validates
   - a connected broker streaming LIVE data
   - configured limits

   Arming expires at the end of the trading session and has to be redone each
   day.
3. **One gate, enforced by types.** Only `OrderGateway` holds a broker that can
   trade, and `BrokerExecution::place` only accepts an `ApprovedOrder` minted by
   `RiskEngine`. No flags, env vars, debug builds or IPC commands may bypass the
   risk check, the kill switch or the audit log.
4. **Hard limits** are checked before every order:
   - per-order quantity and value
   - per-symbol exposure and maximum open positions
   - orders per minute and per day
   - daily loss (realised plus MTM)
   - price band around LTP
   - allowed products and segments (cash equity only at first; no F&O)
   - trading window

   Per-strategy limits can only be tighter than global ones. Tightening applies
   at once, while loosening needs confirmation and is audited.
5. **Making things safer is one keystroke. Making them riskier is deliberate.**
   Engaging the kill switch, cancelling orders and disarming happen immediately.
   Arming, loosening limits and resetting the kill switch need confirmation.
6. **Kill switch.** It is always visible and reachable in one keystroke.
   - Engaging it blocks all new orders, cancels open Nivesh-tagged orders and
     disarms every live strategy.
   - It persists across restarts.
   - It auto-engages on a daily-loss breach, a run of rejections, broker
     disconnect while live, stale data or clock drift.
   - Squaring off positions is a separate, explicit, confirmed action, because
     flattening is itself a trade.
7. **Fail closed.** Any uncertainty stops new orders for the affected strategy:
   stale data, a dropped connection, an unknown order state, a reconciliation
   mismatch, or an unexpected error. Order placement is never blindly retried.
   Ambiguous outcomes are reconciled through client order tags first.
8. **Live strategies only trade on live data.** Intents derived from DELAYED,
   STALE or EOD data are rejected by the gateway.
9. **Audit everything.** The following are written to an append-only,
   hash-chained log before they take effect, with UTC timestamps:
   - intents
   - risk decisions and their reasons
   - orders sent and broker responses
   - fills
   - limit and config changes
   - arming and disarming
   - kill-switch events

   The log is verifiable and exportable, and never contains secrets.
10. **Secrets stay in the keychain.** The UI never sees tokens or API secrets.
    Logs, fixtures and crash output are scrubbed.
11. **Conservative order types.** Limit orders are the default. Market orders are
    disabled by default and, when enabled, always use market protection. There is
    no short selling or F&O until it is explicitly designed and reviewed.
12. **No real money in development.** CI and tests use mocks and fixtures only.
    Coding agents never place real orders, arm live trading or use real broker
    credentials. The only live test is a supervised, minimum-size check done
    manually by Akshat.
13. **Deterministic and testable.** The risk engine and strategy evaluator are
    pure and have property tests. Paper trading and backtests share the same
    evaluator and fill model, so behaviour matches.
14. **Not advice.** Onboarding and live arming show a clear disclaimer. Nivesh is
    a tool, and the user is responsible for their trades.

---

## 12. Later / out of scope

- Windows build and CI job, using the same codebase (secrets via Windows
  Credential Manager).
- More brokers through the same traits: Upstox, Angel One and Dhan, each passing
  the conformance suite.
- F&O and options chains, alerts and notifications, multi-window or
  multi-monitor layouts, an auto-updater, and signed and notarised releases
  (when a Developer ID is available).
- Optional: licensed real-time data via a broker or vendor feed, and a remappable
  keymap UI.
