# Nivesh

A keyboard-driven, Bloomberg-style desktop terminal for Indian markets (NSE and
BSE). Tauri 2, a Rust core and a SolidJS UI.

Status: Part 1 (foundation). The shell, command bar, keymap, help, market
calendar and local store work. There is **no market data yet and the app makes
no network requests**. Real data arrives in Part 2. Paper trading is the default
and live trading doesn't exist yet. See [`PLAN.md`](PLAN.md) for the roadmap and
[`CLAUDE.md`](CLAUDE.md) for contributor conventions.

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
| `RELIANCE` ⏎ | Quote view for NSE:RELIANCE (placeholder until Part 2) |
| `BSE:RELIANCE` ⏎ | Quote view on BSE |
| `NIFTY`, `BANKNIFTY`, `SENSEX` ⏎ | Index view (placeholder until Part 2) |
| `HELP`, `?`, `F1` | Every command and key |

The grammar is `[EXCH:]TARGET [FUNCTION] [ARGS]`, case-insensitive; NSE is the
default exchange and index names win over stock symbols.

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

## Layout

```
crates/nivesh-core    domain types and the IST market calendar (pure)
crates/nivesh-store   SQLite store and migrations
src-tauri/            Tauri app crate: thin commands, window state, logging
ui/                   SolidJS + TypeScript frontend (ui/ipc/bindings is generated)
```

## Third-party

Source Code Pro, bundled in `ui/fonts`, is under the SIL OFL 1.1.
