# Kite Connect setup

How to connect Nivesh to your Zerodha account through Kite Connect. This
costs **₹500/month** (per the official FAQ, updated July 2025 — the
historical-candle add-on that used to cost ₹2,000 is now included free).
You only need this when you want real, live broker data: everything else
in Nivesh works without it, and `LOGIN DEMO` shows the broker views with
clearly-labelled demo data instead.

## One-time setup

1. **A Zerodha trading account** with **TOTP 2FA enabled**
   ([how](https://support.zerodha.com/category/trading-and-markets/general-kite/login-credentials-of-trading-platforms/articles/time-based-otp-setup)).
   Login will not work without it.
2. **A Kite Connect developer account**: sign up at
   [developers.kite.trade](https://developers.kite.trade/login).
3. **Create an app** in the developer console. You get two strings:
   - **API key** (not secret — Nivesh shows it on the status bar)
   - **API secret** (secret — Nivesh stores it in the macOS Keychain and
     never shows it again)
4. **Set the app's redirect URL** to exactly:
   ```
   http://127.0.0.1:7801/nivesh/login
   ```
   Nivesh listens on that address (one-shot, only while a login is
   running) to catch the redirect. Kite does not echo a `state` nonce, so
   the registered loopback redirect plus the single-use listener is the
   protection.
5. **Subscribe the app** (₹500/month; the historical-candle API is
   included).

## In Nivesh

- `LOGIN` opens the Kite login page in your system browser (the URL is
  built by the app for its own API key, so the browser is opened by the
  app itself). Log in; Kite redirects back to the loopback listener; the
  day's access token is exchanged — signed with the documented SHA-256
  checksum — and stored in the keychain. The status bar shows
  `KITE CONNECTED`.
- `LOGOUT` invalidates the token server-side and clears the local copy.

## Daily use

- **The access token dies at the daily reset (~06:00 IST).** Every
  morning, `LOGIN` again. Nivesh shows `KITE EXPIRED` and prompts for a
  re-login rather than erroring.
- Live quotes flow through the app's own provider, labelled **LIVE** in
  the status badge and every view. When the session drops or the token
  expires, quotes fall back to the public sources and their DELAYED
  badges, and the status bar says so.
- Quote polling stays within Kite's documented limits (the quote
  endpoint is capped at 1 request/second; the app's own limiter enforces
  it). Historical candles through Kite are **3 req/s**.

## Entitlements and limits (from the docs, current as of writing)

- Quotes: **1 req/s**; historical candles: **3 req/s**; everything else
  10 req/s.
- WebSocket: up to **3000 instruments** per connection, 3 connections
  per API key.
- Order placement: **10 req/s**, 400 orders/min, 5000/day per API key,
  25 modifications per order. Nivesh's default risk limits (10 orders a
  minute, 200 a day) sit far below these.

## Before any live order: register a static IP

SEBI's retail algo framework (in force for Kite Connect since 1 April
2025) requires API orders to come from a **static IP registered with
Zerodha**. Register one primary (and optionally one secondary) IP in the
Kite Connect developer console before arming anything live. Without it,
Zerodha rejects the orders. Reading data (quotes, holdings) is not
affected. Staying at or under 10 orders per second keeps a self-built algo
outside exchange registration; Nivesh's limits are far below that.

## Security

- The API key and secret, and the day's access token, live only in the
  **macOS Keychain** (service `nivesh`). They never reach SQLite, log
  files or the webview; an automated test in CI proves the token never
  appears in any log line.
- Order placement is compiled only with the `live-trading` Cargo
  feature, which release builds leave off. Even with it, only the order
  gateway can send an order, and only one the risk engine approved, the
  audit log recorded, and the kill switch allows. Going live needs your
  explicit, typed confirmation per strategy each session.
