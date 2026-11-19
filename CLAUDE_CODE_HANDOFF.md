# Nivesh: handoff for Claude Code

**Date:** Nov 19, 2026. **Prepared by:** the Conductor agent session that
built Parts 3c, 4 and most of 5.
**Takeover scope:** the remainder of Part 5 (strategies) and all of
Part 6 (live trading & safety), per `PLAN.md`.

This document is the takeover brief. The long-form history lives in
`CONDUCTOR_HANDOFF.md` (what each part did, with deviations) and
`PLAN.md` (the source of truth for architecture, commits and acceptance
criteria). **Read `PLAN.md` first** — especially §2 (architecture),
§2.7 (IPC), §3 (delivery workflow), Part 5/6 sections, §10 (data-source
limits) and §11 (trading safety — non-negotiable).

## Repo

- GitHub: `akshatiwarix/nivesh` (**public** — free Actions minutes, so CI
  is never a budget problem). All work goes through PRs; **never commit
  or force-push to `main`**.
- Local: `~/nivesh` (Akshat's checkout) plus Conductor workspaces under
  `~/conductor/workspaces/nivesh/`. Run `git pull` in your checkout
  before starting.

## Where the build stands

Merged on `main` (every PR merged only after all CI checks passed):

- **Parts 1–4, complete.** Foundation, browse-mode data (NSE/Yahoo with
  fallback + circuit breakers + caching), charts & market views
  (candlestick + indicators in Rust, movers/sectors/breadth/news), the
  3c gap fixes, and **all of Part 4** (Zerodha Kite: secrets in the
  Keychain, login with a one-shot loopback listener, account endpoints
  through a conformance suite, the daily instrument dump, the WebSocket
  binary ticker, a live-quote provider that joins the chain at login,
  `LOGIN`/`LOGIN DEMO`/`LOGOUT`, the read-only broker views
  `PORT`/`POS`/`ORD`/`TRD`/`FUNDS`, a labelled **DEMO broker** for the
  no-spend demo, and a secrets-leak test).
- **Part 5, through the paper runner** (PRs #23–#31): the versioned rule
  model + validator + bar-by-bar evaluator (closed bars only), the three
  built-in templates (MA crossover, RSI reversion, N-bar breakout), the
  pure risk engine with the **`ApprovedOrder` token** (fields private,
  `RiskEngine::check` the only minter — the gate is non-optional by
  construction), the order gateway (stale-intent refusal, then the risk
  check, then paper routing), the deterministic backtester
  (next-bar-open fills, no-lookahead property test, golden snapshots),
  the paper runner, store persistence (migration v6), and the
  `STRAT`/`BT`/`PAPER` views.

Test counts at the last push: **471 Rust + 341 UI**, clippy
`-D warnings`, Biome + tsc clean.

## What is LEFT

### Part 5 remainder (small, do first)

1. **Wire the paper runner to live bars through the gateway.**
   `paper_start` currently sets the resumable running flag; the loop
   that feeds closed bars (from the quote hub / chart's forming-bar
   stream) through `nivesh-exec::OrderGateway::submit` into
   `PaperRunner` does not exist yet. Design: one task per running
   strategy; the gateway holds the per-symbol last-closed-bar map;
   state persists via `store.save_paper_state` after each bar. Resume on
   restart reads the same state.
2. **Fees/cost model (STT + slippage)** in the backtester and paper
   fills — data-driven, defaults approximating Zerodha equity charges.
   **Akshat's playbook rule: no strategy is finished without a cost
   model.**
3. **Backtest chart markers + equity/drawdown curves** on the
   candlestick chart (the trade table and metrics exist).
4. **Walk-forward / out-of-sample** checks and **volatility-based
   position sizing** (fixed-% sizing exists; ATR-based sizing is the
   playbook's ask).
5. Backtests run on one liquid symbol (RELIANCE) in v1 — make the
   symbol selectable.

### Part 6 (all of it — `PLAN.md` "Part 6" has the full commit list)

In order, per the plan:

1. **Full risk-limit model** in `nivesh-risk`: global + per-strategy
   limits (which can only be *tighter*), price bands, products/segments
   (cash only), trading window. Pure, property-tested.
2. **Hash-chained append-only audit log** in `nivesh-store`: SHA-256
   chain, verification routine, CSV/JSONL export. The audit write must
   complete **before** an order is sent.
3. **Kill switch** in `nivesh-exec`: blocks new orders, cancels
   Nivesh-tagged open orders, disarms live strategies, persists across
   restart, auto-triggers (daily-loss breach, rejection storm, broker
   disconnect while live, stale data, clock drift). Reset needs typed
   confirmation + cooldown.
4. **Kite order placement** behind the `live-trading` Cargo feature:
   `BrokerExecution` on the Kite adapter (implementing the trait that
   already exists in `nivesh-broker`), tag-based idempotency, market
   protection, no blind retries. **Before enabling live orders, verify
   Zerodha's current SEBI-2025 algo requirements** (static IP
   registration, order-per-second thresholds, tagging).
5. **Reconciliation** at startup/reconnect/periodically; mismatch →
   pause strategy + `risk:alert`.
6. **Per-strategy live arming**: global master switch (off by default),
   typed confirmation Rust validates, connected broker streaming LIVE
   data, configured limits, expires at session end. DELAYED-derived
   intents are rejected by the gateway.
7. **IPC + UI**: `RISK` (usage vs limits), kill-switch control
   (status bar + Ctrl+Shift+K + `KILL`), `AUDIT` viewer with chain
   verification + export, red LIVE banner on armed strategies.
8. **Sandboxed Rhai scripting** (`nivesh-script`): limited API, no
   filesystem/network, operation/time/memory limits; scripts paper-only
   until armed like any strategy.
9. **E2E safety scenarios** against the mock Kite server (limit breach →
   rejected+audited; daily-loss breach → auto-kill; kill switch cancels
   open orders and survives restart; DELAYED-derived intents rejected;
   ambiguous placement reconciled not retried; scripts hit limits).
10. **Onboarding, settings, startup/bundle polish**; trading-safety
    guide + release checklist.

## How to work here (rules that are already the house style)

- **Workflow:** small stacked PRs, Conventional Commits
  (`type(scope): summary`, lowercase, ≤72 chars), each PR merged with a
  merge commit **only after all CI checks pass** (ubuntu checks,
  macos-15-intel checks, macOS dmg bundle, Linux bundles). Never commit
  directly to `main`. Branch names `part-<n>-<slug>`.
- **Before every push:** `cargo fmt --all`, `cargo clippy --workspace
  --all-targets --all-features -- -D warnings`, `cargo test
  --workspace`, `pnpm lint && pnpm typecheck && pnpm test && pnpm
  build`, and `pnpm gen:bindings` after touching IPC types (commit the
  bindings; CI fails if stale). The Rust toolchain is pinned via
  `rust-toolchain.toml` (1.100.0, installed on this Mac).
- **Architecture rules** (from `CLAUDE.md`, all enforced by review):
  heavy logic lives in Rust crates; Tauri commands are thin; the UI
  never calls `invoke` directly (only `ui/ipc/` wrappers) and never
  touches the network; money is integer paise, display data f64; store
  UTC, show IST; every new dependency needs justification.
- **Trading safety is non-negotiable** (`PLAN.md` §11 + `CLAUDE.md`):
  paper by default; only `nivesh-exec::OrderGateway` may reach a broker
  and only with an `ApprovedOrder`; the audit write precedes the order;
  fail closed; agents **never** place real orders, arm live trading or
  use real credentials — tests and CI use mocks and fixtures only.
- **Honest data:** provenance on everything (LIVE only from the broker
  stream; public data is DELAYED with source + as-of; STALE from cache;
  PAPER for all simulated state). Never splice two providers into one
  series.
- **Data-source limits** (`PLAN.md` §10): NSE needs browser-like
  headers + cookie warm-up and blocks aggressively — never circumvent
  (no proxies, no fingerprint spoofing); fall back to Yahoo instead.
  Kite rate limits: quote 1/s, historical 3/s, rest 10/s (enforced in
  `nivesh-broker-kite::rest`).
- **CI flake:** the dmg bundle job can fail with `hdiutil: couldn't
  eject - Resource busy`; the workflow already turns Spotlight off and
  retries 3×. If it still fails, rerun the failed job.

## Gotchas learned the hard way

- `pnpm gen:bindings` runs a Rust example — run it **after** the Rust
  changes compile, then commit `ui/ipc/bindings/`.
- Integration-test files (in `tests/`) are not covered by clippy's
  `allow-unwrap-in-tests` unless the code sits inside a `#[cfg(test)]`
  module — wrap test helpers that way.
- The workspace lints deny `unwrap`/`expect` outside tests: no panic
  paths in library code, comments do not satisfy clippy.
- ts-rs types must be listed in `src-tauri/src/ipc/bindings.rs`'s
  `export()` or the bindings are silently not regenerated; every new
  command needs entries in `lib.rs` `generate_handler!`, `build.rs`
  `COMMANDS`, and `capabilities/main.json`.
- `Store::connection()` is the narrow escape hatch for one-off queries;
  prefer adding a method in `nivesh-store`.
- The broker API root is injectable (`NIVESH_KITE_URL` debug hook):
  **tests must never reach the real broker.** Same for
  `NIVESH_NSE_URL`/`NIVESH_YAHOO_URL`/`NIVESH_FORCE_FAIL` (data side).
- Demo broker (`LOGIN DEMO`) is the no-spend demo; keep it labelled
  PAPER/DEMO everywhere and never a default.

## Akshat-specific notes

- No Kite Connect developer account yet (`docs/KITE_CONNECT.md` has the
  ₹500/month signup walkthrough). The demo covers demonstrations until
  he subscribes; his manual holdings-vs-Kite-web check happens at his
  first real login.
- He wants: a working demo he can show, no spend right now, small
  chunks pushed to GitHub, no check-in pauses (build straight through),
  ~10–15 PRs per part merged as each goes green.
- The macOS `.dmg` from CI is ad-hoc signed, not notarised — first
  launch needs Open Anyway or `xattr -dr com.apple.quarantine`.
