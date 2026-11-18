# CLAUDE.md: conventions for coding agents

Nivesh is a Bloomberg-style desktop terminal for Indian markets, built with Tauri 2,
a Rust core and a SolidJS UI. **Read `PLAN.md` first.** It defines the architecture,
the six parts and the intended commits for each part. If your work has to diverge
from it, update `PLAN.md` in the same PR and say so in the PR description.

## Repo layout

```
Cargo.toml            workspace (members: crates/*, src-tauri)
crates/nivesh-*       Rust libraries: core, store, data, indicators, secrets, broker,
                      broker-kite, strategy, risk, exec, script
src-tauri/            Tauri 2 app crate (thin command handlers, no business logic)
ui/                   SolidJS + TypeScript frontend (ui/ipc/bindings is generated)
package.json          frontend scripts + Tauri CLI (pnpm)
.github/workflows/    CI
```

Test fixtures live in `crates/<crate>/tests/fixtures/`.

## Build, test, lint

These scripts are created in Part 1. CI runs exactly these commands, so run the
relevant ones before every commit.

```sh
# Rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace

# Frontend
pnpm install --frozen-lockfile
pnpm lint            # biome check
pnpm typecheck       # tsc --noEmit
pnpm test            # vitest run
pnpm build           # vite build
pnpm gen:bindings    # regenerate ts-rs bindings after changing IPC types; commit them

# App
pnpm tauri dev
pnpm tauri build                              # bundles for the host OS
pnpm tauri build --bundles dmg                # macOS
pnpm tauri build --bundles deb,appimage       # Linux
```

- **Linux system dependencies:** `libwebkit2gtk-4.1-dev build-essential curl wget
  file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`.
- **Network tests:** tests that hit real networks are `#[ignore]`. Run them only
  by hand (`cargo test -p nivesh-data -- --ignored`), never in CI.

## Commits

- **Format.** Use Conventional Commits: `type(scope): imperative summary`,
  lowercase, at most 72 characters.
  - Types: `feat fix refactor perf test docs chore ci build`.
  - Scopes: crate short names (`core`, `data`, `store`, `indicators`, `secrets`,
    `broker`, `broker-kite`, `strategy`, `risk`, `exec`, `script`), or `app`,
    `ui`, `ci`, `deps`.
- **Size.** Keep commits small and meaningful, and make sure each one builds and
  passes tests. Don't pad, and don't leave WIP or "fix typo" commits on a PR
  branch: fold them in before pushing. Use the body to explain *why* when it
  isn't obvious.
- **Author.** The author is already configured. Don't change the git config or
  pass `--author`.
- **Lockfiles.** Commit `Cargo.lock` and `pnpm-lock.yaml`.
- **Never commit secrets.** That means `.env*`, API keys or secrets, access or
  request tokens, keychain exports and signing certificates. Scrub fixtures of
  tokens, client IDs, account numbers and real holdings. Check
  `git diff --cached` before every commit.

## Branches and PRs

- **One part, one branch.** Name it `part-<n>-<slug>` (e.g. `part-1-foundation`)
  and cut it from the latest `main`.
- **One part, one PR** into `main`, titled `Part <n>: <name>`. The PR body holds:
  - a summary
  - the commit list
  - the part's acceptance criteria from `PLAN.md`, ticked with evidence
  - how to test
  - screenshots for UI changes
- **Merging.** Merge with a merge commit (no squash or rebase), and only once CI
  is green. Never commit or force-push directly to `main`. Push and open PRs only
  when the task asks for it.
- **Check-ins.** After Part 2 is merged and after Part 4 is merged, stop and
  check in with Akshat before starting the next part.

## Architecture rules

- **Heavy logic lives in Rust.** That covers data fetching, caching, indicators,
  strategies, backtests, risk and broker I/O. The UI renders and dispatches. It
  has no business logic, no indicator math and **no network access**: all HTTP
  and WebSocket traffic goes through Rust.
- **Tauri commands are thin:** parse arguments, call a crate service, map the
  error. The frontend calls IPC only through `ui/ipc/` wrappers, never through
  `invoke` in components.
- **Rust style:**
  - Libraries use `thiserror`. `anyhow` is allowed only in `src-tauri`.
  - No `unsafe`.
  - No `unwrap`/`expect` outside tests, unless you comment why the invariant
    holds.
  - Money uses integer paise. Indicators and display data use `f64`.
  - Store time in UTC and display it in IST.
- **Dependencies.** Every new one must be justified in the PR: compile cost for
  Rust, gzipped size for the frontend.

## Keep the UI snappy

- **No heavy dependencies.** That means no UI kits, CSS frameworks, CSS-in-JS,
  Monaco, moment or lodash. Lazy-load the chart library.
- **Rendering.** Batch streaming updates per animation frame, virtualise long
  lists and use fixed row heights. No blur, shadows or animations (WebKitGTK is
  slow).
- **Keyboard first.** Every feature must work from the keyboard. Register
  commands and keys in the registry or keymap so HELP stays accurate.
- **Budgets.** Stay within the performance budgets in `PLAN.md` §2.9, and call
  out any regression in the PR.

## Data labelling

- Market data crosses IPC as `Sourced<T>` and carries its provenance. Every view
  renders it through the shared provenance badge, using these labels:

  | Label | Use for |
  |---|---|
  | `LIVE` | Broker real-time data only |
  | `DELAYED` | Public sources (NSE, Yahoo, RSS), shown with source and as-of time |
  | `STALE` | Old or offline-cached data |
  | `PAPER` | Simulated trading state |

- Never label public data LIVE, and never splice providers within one series.
- Respect provider rate limits. Never add proxy rotation, fingerprint spoofing or
  other ways to evade blocking; fall back to another provider instead.

## Trading safety (non-negotiable)

- **Paper trading is the default** for new installs, new strategies, scripts and
  every restart. Never change that default.
- **Live trading is opt-in per strategy** and needs typed confirmation. It
  expires at session end and must never use DELAYED, STALE or EOD data.
- **Orders go through one path.** Only `nivesh-exec::OrderGateway` may call
  `BrokerExecution`, and only with an `ApprovedOrder` from the risk engine. Never
  add flags, env vars, debug shortcuts or IPC commands that bypass risk checks,
  the kill switch or the audit log.
- **Fail closed** on stale data, disconnects or unknown order state. Never
  blindly retry order placement.
- **Agents never place real orders,** never arm live trading and never use real
  broker credentials. Tests and CI use mocks and fixtures only.
- **Test the failure paths.** Changes to `nivesh-risk`, `nivesh-exec` or any
  `BrokerExecution` implementation need tests for the failure paths and a safety
  note in the PR.
