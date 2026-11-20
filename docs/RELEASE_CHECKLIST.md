# Release checklist

Work through this list for every release. Steps marked **Akshat only**
are manual and are never done by an agent or by CI.

## 1. Before tagging

- [ ] `main` is green. Every job in `ci.yml` passes, including:
  - `cargo test -p nivesh-broker-kite --features live-trading`, which runs
    against a mock broker only
  - the initial bundle budget
- [ ] Check the PLAN.md §2.9 budgets on a release build (`pnpm tauri build --no-bundle`):
  - [ ] Cold start under 1.0 s on Apple Silicon. Read
        `startup to interactive ms=…` in the log
        (`~/Library/Logs/app.nivesh.terminal/`), or look at SETTINGS.
  - [ ] Initial JS under 150 KB gz (`pnpm build && pnpm check:bundle`).
  - [ ] RSS under 300 MB, counting the app and its WebKit processes.
  - [ ] Idle CPU at about 0–1 % with the window hidden.
- [ ] Bump the version in `package.json`, `Cargo.toml` (workspace) and
      `src-tauri/tauri.conf.json`.
- [ ] Update the changelog.
- [ ] `git grep` finds no secrets.
- [ ] `.env` files are untracked.
- [ ] `secrets_leak` tests pass.

## 2. Build and sign (macOS)

Releases are unsigned (`signingIdentity: "-"`) until a Developer ID is
available. When one is:

- [ ] **Store the signing material as GitHub Actions secrets only. Never
      commit it.** The secrets are:
  - `APPLE_CERTIFICATE`: the Developer ID Application `.p12`, base64
  - `APPLE_CERTIFICATE_PASSWORD`
  - `APPLE_SIGNING_IDENTITY`, for example
    `Developer ID Application: Name (TEAMID)`
  - `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password) and
    `APPLE_TEAM_ID`, for notarisation
- [ ] Set `bundle.macOS.signingIdentity` from `APPLE_SIGNING_IDENTITY` in
      the release job. Keep `"-"` for PR builds.
- [ ] Build: `pnpm tauri build --target aarch64-apple-darwin --bundles dmg`.
      With the secrets above set, Tauri signs and notarises the bundle.
- [ ] Verify the signature, notarisation and stapled ticket:
  ```sh
  codesign --verify --deep --strict --verbose=2 Nivesh.app
  spctl --assess --type execute --verbose Nivesh.app
  xcrun stapler validate Nivesh.dmg
  ```
- [ ] Linux: CI builds the `.deb` and `.AppImage`. Install the `.deb` on a
      clean Ubuntu 22.04 and launch it.

## 3. Smoke test on paper (anyone)

On a clean install of the release build:

- [ ] Onboarding appears on first run. It explains DELAYED data, that
      trading is paper by default, and the disclaimer. "I understand"
      closes it, and it doesn't come back.
- [ ] `RELIANCE` shows a quote labelled DELAYED.
- [ ] The chart, `WL`, `MKT` and `NEWS` all load.
- [ ] Turning live trading on:
  - [ ] `STRAT MA 20 50 1` then `BT` produces a result.
  - [ ] `PAPER START` produces paper fills only.
  - [ ] `LIVE ON` asks for `ENABLE LIVE TRADING`.
  - [ ] Esc cancels the confirmation.
- [ ] `Ctrl+Shift+K` engages the kill switch:
  - [ ] the status bar shows it engaged
  - [ ] it is still engaged after a restart
  - [ ] `KILL RESET` waits out its cool-down
- [ ] `AUDIT VERIFY` reports the chain is intact.
- [ ] `SETTINGS` shows the version, startup time and folders.

## 4. Supervised live smoke test (Akshat only)

**Agents and CI never do this step.** It uses real money and real broker
credentials, and Akshat does it by hand, watching throughout.

- [ ] Do it during market hours, from a funded Zerodha account that
      Akshat controls.
- [ ] Set the strategy's own limits to the smallest possible size: `QTY 1`,
      one cheap liquid stock, and a `LOSS` limit of a few hundred rupees.
- [ ] `LOGIN` and confirm the data shows LIVE.
- [ ] `LIVE ON` and `ARM <NAME>`, then watch for one order.
- [ ] Confirm all of these:
  - [ ] the order appears at Zerodha with an `NV…` tag
  - [ ] `AUDIT` shows `order.routed` followed by `order.acked`
  - [ ] the LIVE banner is showing
- [ ] Press `Ctrl+Shift+K`. Confirm the open order is cancelled at Zerodha
      and the strategy is disarmed.
- [ ] Run `AUDIT VERIFY`, then `AUDIT EXPORT JSONL` and keep the export
      with the release notes.
- [ ] Before publishing, switch live trading off (`LIVE OFF`) and log out.

## 5. Publish

- [ ] Tag `vX.Y.Z` on `main` and push the tag.
- [ ] Attach the `.dmg`, `.deb` and `.AppImage` to the GitHub release,
      along with their SHA-256 checksums.
- [ ] Note any budget regressions or known gaps in the release notes.
