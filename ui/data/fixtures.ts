// Builders for market-data DTOs, shared by view and feed tests.
import type {
  ConstituentsInfoDto,
  HubUpdateDto,
  IndexQuoteDto,
  ProvenanceDto,
  QuoteDto,
  SourcedIndexQuoteDto,
} from "../ipc";

export const AS_OF = Date.UTC(2026, 10, 18, 5, 44, 0); // 11:14:00 IST

export function prov(provider = "nse", freshness: ProvenanceDto["freshness"] = "DELAYED") {
  return { provider, freshness, asOfMs: AS_OF, fetchedAtMs: AS_OF + 1000 } satisfies ProvenanceDto;
}

export function quote(symbol: string, ltp: number, prevClose = 100): QuoteDto {
  return {
    exchange: "NSE",
    symbol,
    name: `${symbol} Limited`,
    ltp,
    prevClose,
    change: ltp - prevClose,
    changePct: ((ltp - prevClose) / prevClose) * 100,
    open: null,
    high: ltp + 1,
    low: ltp - 1,
    volume: 1_234_567,
    week52High: null,
    week52Low: null,
    upperBand: null,
    lowerBand: null,
  };
}

export function indexQuote(code: string, title: string, last: number): IndexQuoteDto {
  return {
    index: code,
    title,
    last,
    prevClose: last - 10,
    change: 10,
    changePct: 0.04,
    open: null,
    high: last + 5,
    low: last - 5,
    week52High: null,
    week52Low: null,
    advances: 11,
    declines: 39,
  };
}

export function indexRow(code: string, title: string, last: number, provider = "nse") {
  return {
    quote: indexQuote(code, title, last),
    provenance: prov(provider),
  } satisfies SourcedIndexQuoteDto;
}

export function allIndicesUpdate(...rows: SourcedIndexQuoteDto[]): HubUpdateDto {
  return { kind: "allIndices", rows };
}

export function indexUpdate(
  code: string,
  title: string,
  last: number,
  symbols: string[],
  provider = "nse",
  constituentsInfo: ConstituentsInfoDto | null = null,
): HubUpdateDto {
  return {
    kind: "index",
    snapshot: {
      quote: indexQuote(code, title, last),
      constituents: symbols.map((s, i) => quote(s, 1000 + i)),
      constituentsInfo,
      provenance: prov(provider),
    },
  };
}
