# Provider fixtures

Fixtures pin down parser behaviour: field names, string-vs-number quirks, null
rows and error bodies. CI runs against these files only and never touches the
network.

| Directory | Origin |
|---|---|
| `yahoo/chart_quote_*.json`, `chart_history_*.json`, `chart_not_found.json`, `chart_bad_range.json` | **Recorded** from Yahoo's public chart endpoint (public market data only, no headers or cookies). Re-record with the `yahoo_record_fixtures` ignored test. |
| `yahoo/chart_missing_price.json` | Hand-written malformed response. |
| `nse/*` | See the note in that directory. |

Never put tokens, cookies, client ids or real account data in fixtures.
