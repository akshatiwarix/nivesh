# Nivesh: Conductor handoff

Last updated: Nov 19, 2026 (IST). This doc is the extra layer on top of the repo: what's done, what's left, and the decisions already made. `PLAN.md` is still the source of truth for the full spec.

## What Nivesh is

A Bloomberg Terminal style desktop app for Indian markets (NSE and BSE). It has a no-login browse mode on free delayed data, an optional broker connection, and rule-based algo trading (paper trading by default).

## Decisions made

- Tauri 2 desktop app with a Rust workspace core and a SolidJS + TypeScript + Vite frontend. macOS first.
- Zerodha Kite Connect is the first broker.
- Browse mode uses free delayed data through pluggable providers (NSE public endpoints, falling back to Yahoo).
- Strategies are rule based. Paper trading is the default; live trading is opt-in per strategy.
- Six parts, each one PR of about 10 to 15 commits, merged into `main` with a merge commit once all CI checks pass.
- Secrets are never committed. Broker keys come from environment variables or the macOS Keychain.
- Parts 5 and 6 follow the trading policy in `PLAN.md` and the user's trading playbook: no strategy is finished without a cost model (STT and slippage), out-of-sample or walk-forward testing, volatility-based position sizing, and drawdown kill switches.

## Done

- **Part 1:** app foundation (PR #1, merged).
- **Part 2:** browse-mode data (PR #2, merged). **Part 2b:** gap fixes (PR #3, merged), covering bundled NIFTY constituents (`pnpm refresh:data`), search ranking, 4,370 BSE stocks, and NSE retry backoff.
- **Part 3:** charts and market views (PR #4, 16 commits, merge commit `6d03041`). All CI checks passed:
  - `Checks (ubuntu-22.04)`
  - `Checks (macos-15-intel)`
  - `Bundle (macOS aarch64 .dmg)`
  - `Bundle (Linux .deb and .AppImage)`
  - Tests: 356 Rust and 334 UI.
- **Part 3 commands:**
  - `RELIANCE CH 1D 1Y` opens a chart.
  - `ADD SMA 50`, `ADD BOLL`, `ADD RSI 14` and `ADD MACD` add indicators, and `RM RSI` removes one.
  - `MOV` or F5 opens movers, `SEC` or F6 the sector heatmap, `BRD` breadth, and `NEWS` or F7 news (`NEWS RELIANCE` filters it).
- The macOS `.dmg` was built in CI and sent to the user.
- **Gatekeeper:** the `.dmg` is ad-hoc signed, not notarized, so macOS blocks the first launch. Click Done (not Move to Bin), then open System Settings, go to Privacy & Security and click Open Anyway, or run `xattr -dr com.apple.quarantine /Applications/Nivesh.app`.
- Work was paused after part 3 merged, at the user's request. An 11 am IST reminder on Nov 19 said the build is paused until the user says go, and asked about starting part 4. Part 4 has not been started.

## Not done

- **Part 4:** Zerodha Kite broker connection (see `PLAN.md`, Part 4). Not started.
- **Part 5:** strategies.
- **Part 6:** live trading and safety.
- The app is not Apple signed or notarized. Fixing that needs a paid Apple Developer account plus signing in CI.

## Known part 3 gaps

- "Most active" movers are usually empty because NSE blocks that endpoint.
- Heatmap tiles are all the same size because there's no market-cap data.
- NIFTY MEDIA shows no returns.
- SENSEX has no constituents.
- Indicators skip the forming bar until the chart reloads.
- The first chart drew in about 121 ms in software rendering, against a 100 ms target.
- The charting library is 62.6 KB against a planned 45 KB.
- The `.dmg` hasn't been tested on an Apple Silicon Mac.
- CI moved the Mac jobs to `macos-15-intel` because of runner timeouts. They could move back to `macos-latest` later.
