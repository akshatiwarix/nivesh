# Releasing Nivesh to the public

This guide covers putting a downloadable `.dmg` in people's hands, what each
step costs, and how live data and trading fit in later. Work through
[RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) for every release.

## 1. Publish a download (free, works today)

The pieces are already in the repo:

- **`.github/workflows/release.yml`:** pushing a tag such as `v0.1.0` builds
  the Apple silicon `.dmg` and publishes it as a GitHub Release named
  `Nivesh_aarch64.dmg`.
- **`site/`:** the landing page, deployed by Vercel. Its download button
  points at `releases/latest/download/Nivesh_aarch64.dmg`, so it always
  serves the newest release.
- **README:** its download link points at the same file.

To publish a release:

1. Make sure `main` is green.
2. Make sure the version is the same in all three places:
   - `package.json`
   - `Cargo.toml` (`[workspace.package]`)
   - `src-tauri/tauri.conf.json`
3. Tag and push. The workflow refuses a tag that doesn't match `package.json`.

   ```sh
   git checkout main && git pull
   git tag v0.1.0
   git push origin v0.1.0
   ```
4. Wait for the **Release** workflow (about 20 minutes). Then check that the
   release appears under **Releases** and is **not** marked pre-release.
   `releases/latest` skips pre-releases.
5. Download the `.dmg` from the README link on a different Mac and install it
   the way the README describes.

Optional: give the Vercel project a custom domain in Vercel → Project →
Settings → Domains.

## 2. Remove the "unidentified developer" warning (USD 99 a year)

Unsigned apps make people dig through System Settings before macOS will open
them. Many people give up at that point. Signing and notarising removes the
warning.

1. **Join the Apple Developer Program.** Use an individual or company account.
   A company account shows the company's name.
2. **Create a Developer ID Application certificate** in Xcode or on the
   developer site. Export it as a `.p12` file with a password.
3. **Create an app-specific password** for notarisation at
   appleid.apple.com.
4. **Add these GitHub Actions secrets** (repo → Settings → Secrets → Actions).
   Never commit them.

   | Secret | Value |
   |---|---|
   | `APPLE_CERTIFICATE` | `base64 -i cert.p12` output |
   | `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
   | `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
   | `APPLE_ID` | your Apple ID email |
   | `APPLE_PASSWORD` | the app-specific password |
   | `APPLE_TEAM_ID` | your 10-character team ID |

5. **Pass the secrets to the "Build .dmg" step** in `release.yml` as `env:`,
   and remove `"signingIdentity": "-"` from `src-tauri/tauri.conf.json`.
   Tauri signs and notarises automatically when these variables are set.
6. **Verify the result** with the `codesign`, `spctl` and `stapler` commands
   in [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).
7. **Update the docs.** In the README, replace the "can't be opened" section
   with a plain "drag to Applications".

## 3. Things worth adding before a wide launch

- **Automatic updates.** Without them, people stay on whatever version they
  first downloaded. Tauri's updater plugin can check GitHub Releases and
  install new versions. It needs a signing key of its own, kept as a secret.
- **Intel Macs.** The release is Apple silicon only. A universal build
  (`--target universal-apple-darwin`) covers Intel Macs too, at about twice
  the download size.
- **Windows and Linux.** CI already builds Linux `.deb` and `.AppImage` files.
  Attaching them to the release is a small change to `release.yml`. Windows
  needs a Windows CI job and, ideally, code signing.
- **Crash reports.** Today, crashes are only visible in each user's local logs
  (`~/Library/Logs/app.nivesh.terminal`). An opt-in crash reporter would show
  you problems you'd otherwise never hear about.
- **A license.** The repository has none, which legally means "all rights
  reserved". Pick one, such as MIT or Apache-2.0 if you want others to reuse
  the code, or keep it closed, and add a `LICENSE` file.

## 4. Live data and live trading

### Today

- **Market data.** Prices come from free public sources (the NSE website and
  Yahoo Finance) and news comes from RSS. All of it is labelled DELAYED.
- **Trading.** Nothing can place a real order. Order code exists behind the
  `live-trading` feature, but the app installs no live route, so arming a
  strategy is always refused.

### Live prices (each user brings their own Zerodha access)

Kite Connect is licensed per developer account, so every user needs their
own. That means you don't pay for your users, and you don't redistribute
exchange data, which needs an exchange licence.

Each user:

1. Has a Zerodha account with TOTP two-factor login turned on.
2. Creates a Kite Connect app at developers.kite.trade (₹500 a month), with
   the redirect URL `http://127.0.0.1:7801/nivesh/login`.
3. Types `LOGIN` in Nivesh, enters their API key and secret once, and logs in
   through Zerodha in the browser.

From then on, prices stream live through Kite's ticker and panels show
**LIVE** instead of DELAYED. The user's holdings, positions, orders and funds
appear under `PORT`, `POS`, `ORD` and `FUNDS`.

[KITE_CONNECT.md](KITE_CONNECT.md) is the full walkthrough.

### Real orders

Turning on real orders is deliberately a separate, later step.

1. **Build:** wire a live route in the app (`install_live_route`) and ingest
   fills from Kite order updates. Neither is built yet.
2. **Verify:** do the supervised, minimum-size live test in
   [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) yourself.
3. **Get legal advice first if other people's money is involved.** SEBI's
   rules for retail algorithmic trading (in force since 2025) route algos
   through brokers. Automated strategies above the order-rate threshold must
   be registered with the exchange through the broker. Offering automated
   trading to the public also raises questions about investment advice and
   research-analyst rules. A personal tool trading your own account is a
   much simpler position than a public product that trades for others.

## 5. Other things to know

- **Public data sources.** NSE's website and Yahoo's endpoints are not
  official APIs. They can block or change without notice, which you can
  already see as "NSE BLOCKED → YAHOO" in the status bar. Their terms may not
  allow automated use at scale.
  - The app spreads its requests out, caches results, and falls back between
    sources. The more users you have, the more likely the sources are to
    block them.
  - For a large audience, a licensed data feed (or each user's own Kite
    access) is the sturdier path.
- **The disclaimer is shown in three places:**
  - the onboarding screen
  - every live-trading confirmation
  - the README

  Keep it in all three.
- **Privacy.** Nivesh has no accounts, analytics or telemetry. Everything
  stays in the user's local database. If you add analytics or crash reports,
  make them opt-in and say so in the README.
