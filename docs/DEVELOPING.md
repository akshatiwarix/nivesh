# Developing Nivesh

A keyboard-driven desktop terminal for Indian markets (NSE and BSE): Tauri 2,
a Rust core and a SolidJS UI. The [README](../README.md) is for people using
the app; this page is for building and changing it. See
[`PLAN.md`](../PLAN.md) for the roadmap and [`CLAUDE.md`](../CLAUDE.md) for
contributor conventions.

Status: Parts 1 to 6 are built. Market data is public and DELAYED; paper
trading is the default. Live order placement exists behind the `live-trading`
feature and safety gates, but the app installs no live route yet, so arming a
strategy is always refused. Read [`TRADING_SAFETY.md`](TRADING_SAFETY.md)
before changing anything near orders, and [`RELEASING.md`](RELEASING.md)
before publishing a build.

## Prerequisites

All platforms: Rust (the toolchain is pinned in `rust-toolchain.toml` and
`rustup` installs it on first use), Node.js 24 or newer and
[pnpm](https://pnpm.io) (the version is pinned in `package.json`; with Node's
Corepack, `corepack enable` is enough).

**macOS (12 or newer):** install the Xcode command line tools.

```sh
xcode-select --install
```

**Linux (Ubuntu 22.04 or newer, or the equivalent):**

```sh
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Other distributions: see the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux).

## Develop

```sh
pnpm install
pnpm tauri dev        # app with hot reload
pnpm dev              # UI only, in a browser (no backend, so no persistence)
```

The first `pnpm tauri dev` compiles the Rust workspace and takes a few minutes.

Checks (CI runs exactly these):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace

pnpm lint            # Biome
pnpm typecheck       # tsc
pnpm test            # Vitest
pnpm build           # production UI build into dist/
pnpm gen:bindings    # regenerate ui/ipc/bindings after changing IPC types
```

## Build

```sh
pnpm tauri build                          # bundles for the host OS
pnpm tauri build --bundles dmg            # macOS
pnpm tauri build --bundles deb,appimage   # Linux
```

Bundles land in `target/release/bundle/`. On Linux the AppImage tools need
`file` (in the dependency list above) and either FUSE or
`APPIMAGE_EXTRACT_AND_RUN=1`.

CI builds an Apple Silicon `.dmg` and Linux `.deb` and `.AppImage` on every PR
and push to `main`; download them from the workflow run's artifacts.

## Opening an unsigned `.dmg`

Builds aren't signed or notarised yet (planned for Part 6). The app is ad-hoc
signed, so macOS Gatekeeper will refuse to open it at first:

1. Open the `.dmg` and drag **Nivesh** to **Applications**.
2. In Finder, right-click (or Control-click) **Nivesh** and choose **Open**,
   then **Open** in the dialog.
3. If macOS says the app is damaged or can't be verified (browser downloads are
   quarantined), clear the quarantine flag and open it again:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Nivesh.app
   ```

On macOS 15 and later, if the right-click route is missing, open
**System Settings → Privacy & Security** and choose **Open Anyway** next to the
Nivesh message.

## Using it

Type anywhere: printable keys go to the command bar at the bottom.

| Input | Does |
|---|---|
| `RELIANCE` ⏎ | Quote view for NSE:RELIANCE  |
| `BSE:RELIANCE` ⏎ | Quote view on BSE |
| `NIFTY`, `BANKNIFTY`, `SENSEX` ⏎ | Index view  |
| `HELP`, `?`, `F1` | Every command and key |

Charts and market views (all data is public and labelled DELAYED or STALE):

| Input | Does |
|---|---|
| `RELIANCE CH 5M 1MO` ⏎ | Candlestick chart (interval, then range). `F4`/`CH` charts the focused instrument |
| `ADD SMA 50`, `ADD RSI 14`, `RM RSI` | Indicators on the focused chart (SMA EMA WMA BOLL VWAP, RSI MACD ATR) |
| `MOV [INDEX]` / `F5` | Top gainers, losers, most active |
| `SEC` / `F6` | Sector returns and a constituent heatmap |
| `BRD [INDEX]` | Advances/declines and % above 20/50/200-day averages |
| `NEWS [SYMBOL]` / `F7`, `RELIANCE NEWS` | Headlines from public RSS feeds |

Chart keys (command bar empty): `+`/`-` zoom, `←`/`→` pan, `Home`/`End`, `[`/`]`
interval (or tab in MOV). `Alt+Enter` opens a row in the next panel.

The grammar is `[EXCH:]TARGET [FUNCTION] [ARGS]`, case-insensitive; NSE is the
default exchange and index names win over stock symbols.

Strategies, paper trading and safety (paper is the default; read
[TRADING_SAFETY.md](TRADING_SAFETY.md) before trading live):

| Input | Does |
|---|---|
| `STRAT MA 20 50 1`, `SCRIPT <NAME>` | Build a strategy from a template, or write one as a sandboxed script |
| `BT <NAME>`, `PAPER START <NAME>` | Backtest it; paper-trade it (simulated fills only) |
| `RISK`, `AUDIT`, `SETTINGS` | Limits and usage; the hash-chained audit log; what the app is set to |
| `Ctrl+Shift+K` / `KILL` | Kill switch: blocks orders, cancels open ones, disarms everything (`Ctrl+Alt+Shift+K` works from any app) |
| `LIVE ON`, `ARM <NAME>` | Live trading, each confirmed by a typed phrase |

Releases follow [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).

## Keyboard reference

HELP (`F1`) always shows the current list, generated from the keymap.

| Key | Does |
|---|---|
| any printable key | Focus the command bar and type |
| `Enter` | Run the command (GO) |
| `Esc` | Clear the command bar; when empty, go back in the focused panel |
| `↑` / `↓` | Command history |
| `Tab` | Complete the command (repeat to cycle); with an empty bar, focus next panel |
| `Shift+Tab` | Focus previous panel |
| `Alt+1` / `Alt+2` / `Alt+4` | 1, 2 or 4 panels |
| `F1`–`F10` or `Ctrl+1`…`Ctrl+0` | Function keys (`Ctrl+digit` for Mac keyboards that need Fn). Only F1 (HELP) is assigned so far |
| `?` | HELP, when the command bar isn't focused |

Layout, command history and window position persist across restarts in a local
SQLite database in the app data directory. Times are stored in UTC and shown in
IST.

## Performance budgets

Measured on this repo's Linux dev box (optimised build, `cargo bench`):

| Work (100,000 candles) | Measured | Budget |
|---|---|---|
| SMA / EMA / WMA | 0.2 / 0.2 / 0.3 ms | < 5 ms |
| RSI / MACD / ATR / VWAP | 0.5 / 0.5 / 0.6 / 0.25 ms | < 5 ms |
| Bollinger (20, 2) | 2.0 ms | < 5 ms |
| Resample 1m to 5m / 1h / 1d / 1w / 1mo | 0.5 / 0.6 / 0.35 / 0.65 / 1.9 ms | < 10 ms |

`scripts/perf/chart-perf.html` measures the chart in WebKitGTK (PLAN §2.9: first
render of 5,000 candles < 100 ms, 60 fps pan and zoom). Benchmarks are compiled
in CI but only run by hand: `cargo bench -p nivesh-indicators -p nivesh-core`.

App budgets, on an Apple M5 release build: startup to interactive about
0.6 s (logged as `startup to interactive`), initial JS 52 KB gz (CI fails
over 150 KB: `pnpm check:bundle`), about 160 MB RSS with WebKit.

## Layout

```
crates/nivesh-core    domain types and the IST market calendar (pure)
crates/nivesh-indicators  indicator math (pure)
crates/nivesh-data    providers, cache, news, markets
crates/nivesh-store   SQLite store and migrations
src-tauri/            Tauri app crate: thin commands, window state, logging
ui/                   SolidJS + TypeScript frontend (ui/ipc/bindings is generated)
```

## Third-party

Source Code Pro, bundled in `ui/fonts`, is under the SIL OFL 1.1.
