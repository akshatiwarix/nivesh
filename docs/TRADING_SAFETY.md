# Trading safety guide

How Nivesh keeps automated trading under your control: risk limits,
arming, the kill switch and the audit log. Read this before you turn live
trading on.

> Nivesh is a tool, not investment advice. You are responsible for every
> trade it places for you, and trading can lose money.

## The short version

- **Paper is the default.** Backtests (`BT`) and paper runs (`PAPER START`)
  never send an order to a broker.
- **Live needs three things, each confirmed by typing a phrase:**
  1. Live trading switched on (`LIVE ON`).
  2. The strategy armed (`ARM <NAME>`). Arming lasts until the end of the
     day's session at most.
  3. LIVE data. An intent made from DELAYED or STALE data is refused and is
     never sent anywhere, not even to paper.
- **Every order passes the risk engine first.** Nothing reaches the broker
  without an approval from it.
- **The kill switch stops everything at once:** `Ctrl+Shift+K` (it works
  even while you type), `KILL`, or the status-bar button.
- **Everything is written to a hash-chained audit log** before it happens.
  `AUDIT VERIFY` checks the chain hasn't been altered.

## Risk limits

Every order is checked against the global limits. A strategy can also have
its own limits; these only ever tighten the global ones, never loosen them.

| Limit | `RISK SET` field | Default |
|---|---|---|
| Shares per order | `QTY` | 1,000 |
| Value per order | `VALUE` | ₹5,00,000 |
| Exposure per symbol | `EXPOSURE` | ₹10,00,000 |
| Open positions | `POSITIONS` | 20 |
| Orders per minute | `OPM` | 10 |
| Orders per day | `OPD` | 200 |
| Daily loss (engages the kill switch) | `LOSS` | ₹50,000 |
| Price band around the last price | `BAND` | 10 % |

There are two more limits you can't change from the command bar:

- **Products:** delivery only.
- **Trading window:** the NSE session, 09:15–15:30 IST.

How to change limits:

- **Change a global limit:** `RISK SET LOSS 2000`.
- **Change one strategy's limit:** `RISK SET QTY 10 FOR TREND`.
- **Drop a strategy's own limits:** `RISK CLEAR TREND`.

**Tightening** takes effect at once. **Loosening** asks you to type
`LOOSEN LIMITS`. It is recorded in the audit log before it takes effect,
and it disarms the affected strategies. Removing a strategy's own limits
also counts as loosening.

The `RISK` view shows the limits, today's usage against each one, and
recent alerts.

## Arming a strategy

1. **Backtest it** (`BT <NAME>`) and **paper-trade it** (`PAPER START <NAME>`)
   until you trust it.
2. **Switch live trading on:** `LIVE ON`, then type `ENABLE LIVE TRADING`.
   On its own this places nothing.
3. **Arm the strategy:** `ARM <NAME>`, then type `ARM <NAME> LIVE`.

   Arming is refused unless all of these hold:
   - live trading is on
   - the kill switch is not engaged
   - the strategy is running and not paused
   - a broker that can place orders is connected
   - its data is LIVE
   - the strategy has its own limits
   - it holds no paper position
   - the session has not ended
4. **Watch it.** While anything is armed, a red **LIVE** banner shows the
   armed strategies and when each arming expires.

These events disarm a strategy:

- `DISARM <NAME>`
- the session ending
- the kill switch engaging
- the strategy being paused, stopped or restarted
- live trading switched off
- its limits being loosened

**Scripts follow exactly the same path.** They are sandboxed: no files,
network or clock, and they are limited to 200,000 operations and 50 ms per
bar. A script that errors decides nothing.

### When an order's fate is unknown

Sometimes the broker can't confirm a placement, for example after a
timeout or a 5xx error. When that happens, Nivesh **does not resend the
order**. Instead it:

1. pauses the strategy
2. raises an alert
3. records the order with its tag, so reconciliation can find it at the
   broker later

When you have checked the broker's order book, `RESUME <NAME>` resumes
the strategy. It is refused while one of the strategy's live orders is
still unresolved, and it never re-arms: run `ARM <NAME>` again to trade
live.

## The kill switch

**Engaging it:**

- By hand: press `Ctrl+Shift+K` in Nivesh, press `Ctrl+Alt+Shift+K` from
  any app (it works with Nivesh hidden), run `KILL`, or click the
  status-bar button. None of them asks for confirmation.
- Automatically, when any of these happens:
  - today's loss reaches the `LOSS` limit
  - 5 rejections within a minute
  - a live strategy's data is more than 30 s old
  - the broker disconnects
  - the clock drifts more than 2 s from the exchange

**What it does:**

- blocks every new order
- disarms every strategy
- cancels every open Nivesh-tagged order at the broker. It never touches
  orders you placed yourself.
- records the trip and each cancellation in the audit log

**It survives a restart.** An engaged switch is saved and comes back
engaged.

**Resetting it:** run `KILL RESET` after a 5-minute cool-down, then type
`RESET KILL SWITCH`. Resetting does not re-arm anything.

## The audit log

`AUDIT` lists every event, newest first:

- intents
- risk approvals and rejections
- routing, acks and failures
- limit and live-switch changes
- arming
- kill-switch trips and cancellations

You can narrow the list or work with the chain:

- **Filter:** `AUDIT KIND order` (by event kind) or `AUDIT FOR TREND` (by
  strategy).
- **Verify:** `AUDIT VERIFY` re-checks the hash chain.
- **Export:** `AUDIT EXPORT CSV` or `AUDIT EXPORT JSONL` saves a copy to
  Downloads.

If an event can't be recorded, the action it belongs to doesn't happen.
