# Nivesh: Conductor handoff

Last updated: Nov 19, 2026 (IST). This doc is the extra layer on top of the repo: what's done, what's left, and the decisions already made. `PLAN.md` is still the source of truth for the full spec.

## What Nivesh is

A Bloomberg Terminal style desktop app for Indian markets (NSE and BSE). It has a no-login browse mode on free delayed data, an optional broker connection, and rule-based algo trading (paper trading by default).

## Decisions made

- Tauri 2 desktop app with a Rust workspace core and a SolidJS + TypeScript + Vite frontend. macOS first.
- Zerodha Kite Connect is the first broker.
- Browse mode uses free delayed data through pluggable providers (NSE public endpoints, falling back to Yahoo).
- Strategies are rule based. Paper trading is the default; live trading is opt-in per strategy.
- **Delivery workflow (updated Nov 19, 2026, by Akshat):** each part is built as ~10-15 small stacked PRs instead of one branch per part. Every PR is pushed as soon as its chunk is done, merged into `main` with a merge commit as soon as its own CI is green (no waiting for the part to finish), and no check-in pauses between parts. `CLAUDE.md`'s "one part, one branch" section is superseded on this point; commit conventions, merge-commit rule and never-commit-to-main still hold.
- Secrets are never committed. Broker keys come from environment variables or the macOS Keychain.
- Parts 5 and 6 follow the trading policy in `PLAN.md` and the user's trading playbook: no strategy is finished without a cost model (STT and slippage), out-of-sample or walk-forward testing, volatility-based position sizing, and drawdown kill switches.

## Done

- **Part 1:** app foundation (PR #1, merged).
- **Part 2:** browse-mode data (PR #2, merged). **Part 2b:** gap fixes (PR #3, merged), covering bundled NIFTY constituents (`pnpm refresh:data`), search ranking, 4,370 BSE stocks, and NSE retry backoff.
- **Part 3:** charts and market views (PR #4, 16 commits, merge commit `6d03041`). All CI checks passed:
  - `Checks (ubuntu-22.04)`, `Checks (macos-15-intel)`
  - `Bundle (macOS aarch64 .dmg)`, `Bundle (Linux .deb and .AppImage)`
  - Tests: 356 Rust and 334 UI.
- **Part 3 commands:** `RELIANCE CH 1D 1Y` opens a chart; `ADD SMA 50`, `ADD BOLL`, `ADD RSI 14` and `ADD MACD` add indicators and `RM RSI` removes one; `MOV`/F5 movers, `SEC`/F6 sector heatmap, `BRD` breadth, `NEWS`/F7 news (`NEWS RELIANCE` filters it).
- **Part 3c gap fixes (Nov 19, 2026):** five PRs, built as small stacked PRs and merged as each went green (see `PLAN.md` "Part 3c gap fixes" for the details):
  - Most active is ranked from the daily bhavcopy's traded value (EOD, labelled), so it is no longer usually empty (PR #5).
  - Heatmap tiles are sized by the same turnover data instead of all-equal (PR #6).
  - SENSEX has a curated, dated constituent list, so its index view, movers and heatmap work (PR #7).
  - Chart indicators follow the forming bar (at most twice a second), so they no longer lag one bar (PR #8).
  - NIFTY MEDIA gets a 1D return derived from its constituents; Yahoo's daily series for it is genuinely empty (verified live) (PR #9).
  - CI: the flaky `hdiutil` eject failure in the dmg job is mitigated (Spotlight off + bundle retry). The Rust toolchain (pinned 1.100.0) was installed on the dev Mac.
  - The `.dmg` was verified on Apple Silicon locally: mounts, arm64 binary, ad-hoc signature valid, app launches (PR #10 wraps up the docs).
  - Remaining known part 3 gaps after 3c: NSE movers rows carry no company name; the tag index is built at startup; benches compile in CI but never run there.
- The macOS `.dmg` was built in CI and sent to the user.
- **Gatekeeper:** the `.dmg` is ad-hoc signed, not notarized, so macOS blocks the first launch. Click Done (not Move to Bin), then open System Settings, go to Privacy & Security and click Open Anyway, or run `xattr -dr com.apple.quarantine /Applications/Nivesh.app`.

## Done (Part 4, Nov 19-20, 2026)

- **Part 4: Zerodha Kite broker connection** — 12 small stacked PRs (#11-#22), each merged as its CI went green:
  - `nivesh-secrets` (keychain + in-memory stores, secrecy redaction), `nivesh-broker` (traits split by capability so read-only cannot grow order placement, account types in paise, the shared order state machine, a conformance suite every adapter must pass).
  - `nivesh-broker-kite`: REST client (per-endpoint-class limits from the docs: quote 1/s, historical 3/s, rest 10/s), login with a one-shot `127.0.0.1:7801/nivesh/login` listener and the SHA-256 checksum, account mapping through the conformance suite, the daily instrument dump into `broker_tokens` (store migration v5), the WebSocket binary ticker (packet-length-keyed parser + the never-panics proptest), and a live-quote provider registered into the chain at login (provenance LIVE) and removed at logout.
  - App: `BrokerManager` (keychain setup, single-use login guard, `broker:session` events, restore-at-startup), `LOGIN`/`LOGIN DEMO`/`LOGOUT`, and the read-only views `PORT`/F8, `POS`, `ORD`/F9, `TRD`, `FUNDS` — every view labels its source (DEMO badge or KITE CONNECTED).
  - The **DEMO broker** (`DemoBroker`/`DemoStream` in `nivesh-broker`): the whole account experience on scrubbed fixtures + a synthetic walk, no credentials and no network, labelled demo everywhere. `LOGIN DEMO` starts it; `LOGOUT` ends it without touching a Kite setup.
  - The secrets-leak test: a tracing capture proves the access token and API secret never appear in any log line across a full login + quote flow.
  - Tests and CI never touch the real broker: the API root is injected (`NIVESH_KITE_URL` debug hook), and everything runs against wiremock fixtures.
  - **Signup for the real thing** (`docs/KITE_CONNECT.md`): ₹500/month (official FAQ, July 2025; historical data included), TOTP 2FA required, redirect URL `http://127.0.0.1:7801/nivesh/login`, daily re-login at the 06:00 IST token flush. Akshat has no developer account yet — the demo covers the showable version until he subscribes.
- **Check-in note (PLAN said after Part 4):** superseded by Akshat's Nov 19 instruction — no pauses; the manual holdings-vs-Kite-web check happens whenever he first connects with a real account.

## Not done

- **Part 5:** strategies (rules, backtests, paper trading) — next.
- **Part 6:** live trading and safety.
- The app is not Apple signed or notarized. Fixing that needs a paid Apple Developer account plus signing in CI.
