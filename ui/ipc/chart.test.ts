import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BarDto } from "./bindings/BarDto";
import { getHistory, subscribeFormingBar } from "./chart";

const channels: { onmessage?: (m: unknown) => void }[] = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: () => false,
  Channel: class {
    onmessage?: (m: unknown) => void;
    constructor() {
      channels.push(this);
    }
  },
}));

const invokeMock = vi.mocked(invoke);

beforeEach(() => {
  invokeMock.mockReset();
  channels.length = 0;
});

describe("chart wrappers", () => {
  it("getHistory passes the interval and optional range", async () => {
    invokeMock.mockResolvedValueOnce({});
    await getHistory("NSE", "RELIANCE", "5m", "1mo");
    expect(invokeMock).toHaveBeenCalledWith("get_history", {
      exchange: "NSE",
      symbol: "RELIANCE",
      interval: "5m",
      range: "1mo",
    });
  });

  it("subscribeFormingBar forwards bars and unsubscribes once", async () => {
    invokeMock.mockResolvedValueOnce(9); // subscribe -> id
    const seen: BarDto[] = [];
    const stop = await subscribeFormingBar("NSE", "TCS", "1d", undefined, (b) => seen.push(b));
    expect(invokeMock).toHaveBeenCalledWith("subscribe_forming_bar", {
      exchange: "NSE",
      symbol: "TCS",
      interval: "1d",
      range: undefined,
      onBar: channels[0],
    });
    const bar = { t: 1, o: 1, h: 2, l: 0.5, c: 1.5, v: 0 } as BarDto;
    channels[0]?.onmessage?.(bar);
    expect(seen).toEqual([bar]);

    invokeMock.mockResolvedValue(undefined);
    stop();
    stop();
    expect(invokeMock.mock.calls.filter(([c]) => c === "unsubscribe")).toHaveLength(1);
    expect(invokeMock).toHaveBeenCalledWith("unsubscribe", { id: 9 });
  });
});
