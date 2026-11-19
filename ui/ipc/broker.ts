// Typed wrappers for the broker commands. The UI never sees a secret:
// setup takes credentials, login returns a URL the backend already opened,
// and the account data is display data.

import { listen } from "@tauri-apps/api/event";
import type { BrokerLoginDto } from "./bindings/BrokerLoginDto";
import type { BrokerSessionDto } from "./bindings/BrokerSessionDto";
import type { FundsDto } from "./bindings/FundsDto";
import type { HoldingDto } from "./bindings/HoldingDto";
import type { OrderDto } from "./bindings/OrderDto";
import type { PositionsDto } from "./bindings/PositionsDto";
import type { TradeDto } from "./bindings/TradeDto";
import { call, inTauri } from "./core";

export function brokerSetup(apiKey: string, apiSecret: string): Promise<void> {
  return call<void>("broker_setup", { apiKey, apiSecret });
}

export function brokerLogin(): Promise<BrokerLoginDto> {
  return call<BrokerLoginDto>("broker_login", {});
}

export function brokerDemo(): Promise<BrokerSessionDto> {
  return call<BrokerSessionDto>("broker_demo", {});
}

export function brokerLogout(): Promise<BrokerSessionDto> {
  return call<BrokerSessionDto>("broker_logout", {});
}

export function brokerStatus(): Promise<BrokerSessionDto> {
  return call<BrokerSessionDto>("broker_status", {});
}

export function brokerHoldings(): Promise<HoldingDto[]> {
  return call<HoldingDto[]>("broker_holdings", {});
}

export function brokerPositions(): Promise<PositionsDto> {
  return call<PositionsDto>("broker_positions", {});
}

export function brokerOrders(): Promise<OrderDto[]> {
  return call<OrderDto[]>("broker_orders", {});
}

export function brokerTrades(): Promise<TradeDto[]> {
  return call<TradeDto[]>("broker_trades", {});
}

export function brokerFunds(): Promise<FundsDto> {
  return call<FundsDto>("broker_funds", {});
}

/** The backend pushes `broker:session` on every login/logout outcome. */
export function onBrokerSession(handler: (session: BrokerSessionDto) => void): () => void {
  if (!inTauri()) return () => {};
  let unlisten: (() => void) | undefined;
  let closed = false;
  void listen<BrokerSessionDto>("broker:session", (event) => handler(event.payload))
    .then((stop) => {
      if (closed) stop();
      else unlisten = stop;
    })
    .catch(() => undefined);
  return () => {
    closed = true;
    unlisten?.();
  };
}
