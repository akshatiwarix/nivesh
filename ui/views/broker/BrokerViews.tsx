// The broker account views: PORT (holdings + funds), POS (positions), ORD
// (orders), TRD (trades) and FUNDS. One shared file because they are one
// family: the same table styling, the same honesty about which broker
// served the data, and the same empty states.
//
// No view here places, changes or cancels an order — Part 4 is read-only.

import { type Component, createMemo, createResource, createSignal, Show } from "solid-js";
import { Badge } from "../../components/Badge";
import { DataPanel } from "../../components/DataPanel";
import { type Column, DataTable } from "../../components/DataTable";
import { createPoll } from "../../data/poll";
import type { BrokerSessionDto, HoldingDto, OrderDto, PositionsDto, TradeDto } from "../../ipc";
import {
  brokerFunds,
  brokerHoldings,
  brokerOrders,
  brokerPositions,
  brokerStatus,
  brokerTrades,
  inTauri,
  onBrokerSession,
} from "../../ipc";
import { formatPrice } from "../../lib/format";
import "./broker.css";

/** The broker's name badge: DEMO (accent) or KITE (live-ish), or a login hint. */
export const BrokerBadge: Component<{ session: BrokerSessionDto | undefined }> = (props) => {
  const session = () => props.session;
  return (
    <Show when={session()?.broker} fallback={<Badge tone="neutral">NO BROKER</Badge>}>
      <Badge tone={session()?.broker === "demo" ? "paper" : "live"}>
        {session()?.broker === "demo" ? "DEMO" : `${session()?.broker?.toUpperCase()} CONNECTED`}
      </Badge>
    </Show>
  );
};

/** A login banner: what to type, honestly, for each state. */
const LoginHint: Component<{ session: BrokerSessionDto | undefined; error?: string }> = (props) => (
  <p class="broker__hint" role="status">
    <Show when={props.error}>{(text) => text()}</Show>
    {/* "Loading" only until the session is known, not beside every state. */}
    <Show when={!props.error && props.session === undefined}>Loading session…</Show>
    <Show when={!props.error && props.session !== undefined && props.session.broker === undefined}>
      No broker is connected. <b>LOGIN DEMO</b> shows this view with demo data; LOGIN connects Kite.
    </Show>
    <Show when={!props.error && props.session?.broker === "demo"}>
      Showing demo data. LOGIN connects a real Kite session.
    </Show>
    <Show when={!props.error && props.session?.state === "EXPIRED"}>
      The session expired at the daily reset. LOGIN again.
    </Show>
  </p>
);

const HOLDING_COLUMNS: readonly Column<HoldingDto>[] = [
  { id: "symbol", header: "Symbol", width: "14ch", value: (h) => h.symbol, tone: () => "accent" },
  { id: "exchange", header: "Exch", width: "8ch", value: (h) => h.exchange, tone: () => "dim" },
  { id: "qty", header: "Qty", width: "10ch", align: "right", value: (h) => String(h.quantity) },
  {
    id: "avg",
    header: "Avg",
    width: "13ch",
    align: "right",
    value: (h) => formatPaise(h.averagePrice),
  },
  {
    id: "last",
    header: "Last",
    width: "13ch",
    align: "right",
    value: (h) => (h.lastPrice === null ? "—" : formatPaise(h.lastPrice)),
  },
  {
    id: "pnl",
    header: "P&L",
    width: "14ch",
    align: "right",
    value: (h) => (h.pnl === null ? "—" : formatPaise(h.pnl)),
    tone: (h) => pnlTone(h.pnl),
  },
];

/** Paise → rupees, as the views show them. */
function formatPaise(paise: number): string {
  return formatPrice(paise / 100);
}

function pnlTone(paise: number | null | undefined): "up" | "down" | undefined {
  if (paise === null || paise === undefined || paise === 0) return undefined;
  return paise > 0 ? "up" : "down";
}

/** `PORT` / F8: the holdings, with a funds line at the top. */
export const PortfolioView: Component = () => {
  const [session, sessionError] = createSession();
  const holdings = createPoll(
    () => inTauri() && session()?.broker !== undefined,
    () => brokerHoldings(),
    { everyMs: () => 30_000 },
  );
  const funds = createPoll(
    () => inTauri() && session()?.broker !== undefined,
    () => brokerFunds(),
    { everyMs: () => 30_000 },
  );
  const rows = createMemo(() => holdings.data() ?? []);

  const emptyText = () => {
    if (!inTauri()) return "Broker data is available in the Nivesh app.";
    if (holdings.error() && !holdings.data()) return `${holdings.error()}. Retrying.`;
    if (!session()?.broker) return "No broker is connected. LOGIN DEMO shows demo data.";
    if (rows().length === 0) return "No holdings.";
    return undefined;
  };

  return (
    <div class="view port" data-view="port">
      <DataPanel provenance={null} title="PORTFOLIO">
        <BrokerBadge session={session()} />
        <Show when={funds.data()}>
          {(f) => (
            <p class="broker__funds">
              FUNDS {formatPaise(f().available)} AVAILABLE · {formatPaise(f().used)} USED
            </p>
          )}
        </Show>
        <Show when={sessionError()}>
          {(text) => (
            <p class="view__error" role="alert">
              {text()}
            </p>
          )}
        </Show>
        <LoginHint
          session={session()}
          error={
            holdings.error() && !holdings.data() ? `${holdings.error()}. Retrying.` : undefined
          }
        />
        <DataTable
          label="Holdings"
          columns={HOLDING_COLUMNS}
          rows={rows()}
          emptyText={emptyText()}
        />
      </DataPanel>
    </div>
  );
};

const POSITION_COLUMNS: readonly Column<PositionsDto["net"][number]>[] = [
  { id: "symbol", header: "Symbol", width: "14ch", value: (p) => p.symbol, tone: () => "accent" },
  { id: "exchange", header: "Exch", width: "8ch", value: (p) => p.exchange, tone: () => "dim" },
  {
    id: "net",
    header: "Net",
    width: "10ch",
    align: "right",
    value: (p) => String(p.netQuantity),
    tone: (p) => (p.netQuantity < 0 ? "down" : undefined),
  },
  { id: "day", header: "Day", width: "10ch", align: "right", value: (p) => String(p.dayQuantity) },
  {
    id: "avg",
    header: "Avg",
    width: "13ch",
    align: "right",
    value: (p) => formatPaise(p.averagePrice),
  },
  {
    id: "last",
    header: "Last",
    width: "13ch",
    align: "right",
    value: (p) => (p.lastPrice === null ? "—" : formatPaise(p.lastPrice)),
  },
  {
    id: "pnl",
    header: "P&L",
    width: "14ch",
    align: "right",
    value: (p) => (p.pnl === null ? "—" : formatPaise(p.pnl)),
    tone: (p) => pnlTone(p.pnl),
  },
];

/** `POS`: day and net positions, tabbed. */
export const PositionsView: Component = () => {
  const [tab, setTab] = createSignal<"net" | "day">("net");
  const [session] = createSession();
  const poll = createPoll(
    () => inTauri() && session()?.broker !== undefined,
    () => brokerPositions(),
    { everyMs: () => 30_000 },
  );
  const rows = createMemo(() =>
    tab() === "net" ? (poll.data()?.net ?? []) : (poll.data()?.day ?? []),
  );

  const emptyText = () => {
    if (!inTauri()) return "Broker data is available in the Nivesh app.";
    if (poll.error() && !poll.data()) return `${poll.error()}. Retrying.`;
    if (!session()?.broker) return "No broker is connected. LOGIN DEMO shows demo data.";
    if (rows().length === 0) return `No ${tab()} positions.`;
    return undefined;
  };

  return (
    <div class="view pos" data-view="pos">
      <DataPanel provenance={null} title="POSITIONS">
        <BrokerBadge session={session()} />
        <div class="broker__tabs" role="tablist" aria-label="Position lists">
          {(["net", "day"] as const).map((t) => (
            <button
              type="button"
              role="tab"
              class="broker__tab"
              classList={{ "broker__tab--on": t === tab() }}
              aria-selected={t === tab()}
              tabIndex={-1}
              onClick={() => setTab(t)}
            >
              {t.toUpperCase()}
            </button>
          ))}
        </div>
        <LoginHint session={session()} />
        <DataTable
          label={`${tab()} positions`}
          columns={POSITION_COLUMNS}
          rows={rows()}
          emptyText={emptyText()}
        />
      </DataPanel>
    </div>
  );
};

const ORDER_COLUMNS: readonly Column<OrderDto>[] = [
  { id: "id", header: "Order", width: "12ch", value: (o) => o.orderId, tone: () => "dim" },
  { id: "symbol", header: "Symbol", width: "14ch", value: (o) => o.symbol, tone: () => "accent" },
  {
    id: "side",
    header: "Side",
    width: "8ch",
    value: (o) => o.side,
    tone: (o) => (o.side === "BUY" ? "up" : "down"),
  },
  { id: "kind", header: "Type", width: "8ch", value: (o) => o.kind, tone: () => "dim" },
  { id: "product", header: "Prod", width: "8ch", value: (o) => o.product },
  {
    id: "qty",
    header: "Qty",
    width: "8ch",
    align: "right",
    value: (o) => `${o.filledQuantity}/${o.quantity}`,
  },
  {
    id: "price",
    header: "Price",
    width: "12ch",
    align: "right",
    value: (o) => (o.price === null ? "—" : formatPaise(o.price)),
  },
  {
    id: "avg",
    header: "Avg",
    width: "12ch",
    align: "right",
    value: (o) => (o.averagePrice === null ? "—" : formatPaise(o.averagePrice)),
  },
  {
    id: "status",
    header: "Status",
    width: "16ch",
    value: (o) => orderStatusLabel(o.status),
    tone: (o) => orderStatusTone(o.status),
  },
];

function orderStatusLabel(status: OrderDto["status"]): string {
  switch (status) {
    case "PARTIALLY_FILLED":
      return "PART FILLED";
    default:
      return status;
  }
}

function orderStatusTone(status: OrderDto["status"]): "up" | "down" | "dim" | undefined {
  switch (status) {
    case "FILLED":
      return "up";
    case "REJECTED":
      return "down";
    case "CANCELLED":
      return "dim";
    default:
      return undefined;
  }
}

/** `ORD` / F9: the day's orders. Read-only by design. */
export const OrdersView: Component = () => {
  const [session] = createSession();
  const poll = createPoll(
    () => inTauri() && session()?.broker !== undefined,
    () => brokerOrders(),
    { everyMs: () => 30_000 },
  );
  const rows = createMemo(() => poll.data() ?? []);

  const emptyText = () => {
    if (!inTauri()) return "Broker data is available in the Nivesh app.";
    if (poll.error() && !poll.data()) return `${poll.error()}. Retrying.`;
    if (!session()?.broker) return "No broker is connected. LOGIN DEMO shows demo data.";
    if (rows().length === 0) return "No orders today.";
    return undefined;
  };

  return (
    <div class="view ord" data-view="ord">
      <DataPanel provenance={null} title="ORDERS">
        <BrokerBadge session={session()} />
        <LoginHint session={session()} />
        <DataTable label="Orders" columns={ORDER_COLUMNS} rows={rows()} emptyText={emptyText()} />
      </DataPanel>
    </div>
  );
};

const TRADE_COLUMNS: readonly Column<TradeDto>[] = [
  { id: "order", header: "Order", width: "12ch", value: (t) => t.orderId, tone: () => "dim" },
  { id: "symbol", header: "Symbol", width: "14ch", value: (t) => t.symbol, tone: () => "accent" },
  {
    id: "side",
    header: "Side",
    width: "8ch",
    value: (t) => t.side,
    tone: (t) => (t.side === "BUY" ? "up" : "down"),
  },
  { id: "qty", header: "Qty", width: "10ch", align: "right", value: (t) => String(t.quantity) },
  {
    id: "price",
    header: "Price",
    width: "13ch",
    align: "right",
    value: (t) => formatPaise(t.price),
  },
  { id: "time", header: "Time", width: "16ch", value: (t) => timeIst(t.tradedAtMs) },
];

/** UTC epoch ms → HH:MM:SS IST. */
function timeIst(ms: number): string {
  return new Date(ms).toLocaleTimeString("en-IN", {
    timeZone: "Asia/Kolkata",
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** `TRD`: the day's executed fills. */
export const TradesView: Component = () => {
  const [session] = createSession();
  const poll = createPoll(
    () => inTauri() && session()?.broker !== undefined,
    () => brokerTrades(),
    { everyMs: () => 30_000 },
  );
  const rows = createMemo(() => poll.data() ?? []);

  const emptyText = () => {
    if (!inTauri()) return "Broker data is available in the Nivesh app.";
    if (poll.error() && !poll.data()) return `${poll.error()}. Retrying.`;
    if (!session()?.broker) return "No broker is connected. LOGIN DEMO shows demo data.";
    if (rows().length === 0) return "No trades today.";
    return undefined;
  };

  return (
    <div class="view trd" data-view="trd">
      <DataPanel provenance={null} title="TRADES">
        <BrokerBadge session={session()} />
        <LoginHint session={session()} />
        <DataTable label="Trades" columns={TRADE_COLUMNS} rows={rows()} emptyText={emptyText()} />
      </DataPanel>
    </div>
  );
};

/** `FUNDS`: available, used and total. */
export const FundsView: Component = () => {
  const [session] = createSession();
  const poll = createPoll(
    () => inTauri() && session()?.broker !== undefined,
    () => brokerFunds(),
    { everyMs: () => 30_000 },
  );

  const emptyText = () => {
    if (!inTauri()) return "Broker data is available in the Nivesh app.";
    if (poll.error() && !poll.data()) return `${poll.error()}. Retrying.`;
    if (!session()?.broker) return "No broker is connected. LOGIN DEMO shows demo data.";
    return undefined;
  };

  return (
    <div class="view funds" data-view="funds">
      <DataPanel provenance={null} title="FUNDS">
        <BrokerBadge session={session()} />
        <LoginHint session={session()} />
        <Show when={poll.data()} fallback={<p class="broker__hint">{emptyText()}</p>}>
          {(f) => (
            <dl class="broker__funds-grid">
              <div>
                <dt>AVAILABLE</dt>
                <dd>{formatPaise(f().available)}</dd>
              </div>
              <div>
                <dt>USED</dt>
                <dd>{formatPaise(f().used)}</dd>
              </div>
              <div>
                <dt>TOTAL</dt>
                <dd>{formatPaise(f().total)}</dd>
              </div>
            </dl>
          )}
        </Show>
      </DataPanel>
    </div>
  );
};

// ---- shared helpers ---------------------------------------------------------

/** The session, refetched whenever the backend pushes `broker:session`. */
function createSession(): [() => BrokerSessionDto | undefined, () => string | undefined] {
  const [session, { refetch }] = createResource(inTauri, brokerStatus);
  onBrokerSession(() => void refetch());
  return [() => session(), () => (session.error ? String(session.error) : undefined)];
}
