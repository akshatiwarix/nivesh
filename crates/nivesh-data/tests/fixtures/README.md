# Provider fixtures

Fixtures pin down parser behaviour: field names, string-vs-number quirks, null
rows and error bodies. CI runs against these files only and never touches the
network.

| Directory | Origin |
|---|---|
| `yahoo/chart_quote_*.json`, `chart_history_*.json`, `chart_not_found.json`, `chart_bad_range.json` | **Recorded** from Yahoo's public chart endpoint (public market data only, no headers or cookies). Re-record with the `yahoo_record_fixtures` ignored test. |
| `yahoo/chart_missing_price.json` | Hand-written malformed response. |
| `nse/ind_nifty50list.csv` | **Recorded** (2026-11-18) from NSE Indices' public constituent file; unmodified. |
| `nse/*` | See the note in that directory. |

Never put tokens, cookies, client ids or real account data in fixtures.

## NSE

NSE fixtures from the development machine's IP were only partly obtainable:

- `nse/all_indices.json` (trimmed to the indices Nivesh uses) and
  `nse/market_status.json` are **recordings**.
- `nse/*.synthetic.json` are **hand-written** to NSE's documented shapes
  (`quote-equity`, `equity-stockIndices`). `quote-equity` returned 403 and
  `equity-stockIndices` returned 404 from that IP. Replace them with
  recordings when you can reach those endpoints; the parsers must pass both.

## BSE

`bse/bhavcopy_sample.csv` is a **recording** of eight rows of BSE's public UDiFF
bhavcopy (`BhavCopy_BSE_CM_0_0_0_20261117_F_0000.CSV`, fetched 2026-11-18),
trimmed to the identifying columns (the parser reads only those). It includes a
fixed-income-group row, a fund row and a repeated ticker to pin the filters.
