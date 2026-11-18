import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HubUpdateDto } from "./bindings/HubUpdateDto";
import { searchInstruments, setWindowVisible, subscribeQuotes } from "./data";

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

describe("data wrappers", () => {
  it("searchInstruments passes query and limit", async () => {
    invokeMock.mockResolvedValueOnce([]);
    await searchInstruments("relia", 5);
    expect(invokeMock).toHaveBeenCalledWith("search_instruments", { query: "relia", limit: 5 });
  });

  it("setWindowVisible calls the command", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await setWindowVisible(false);
    expect(invokeMock).toHaveBeenCalledWith("set_window_visible", { visible: false });
  });
});

describe("subscribeQuotes", () => {
  it("subscribes with a channel, forwards updates, and unsubscribes once", async () => {
    invokeMock.mockResolvedValueOnce(7); // subscribe_quotes -> id
    const seen: HubUpdateDto[] = [];
    const stop = await subscribeQuotes([{ kind: "allIndices" }], (u) => seen.push(u));

    expect(invokeMock).toHaveBeenCalledWith("subscribe_quotes", {
      topics: [{ kind: "allIndices" }],
      onUpdate: channels[0],
    });
    const update: HubUpdateDto = { kind: "allIndices", rows: [] };
    channels[0]?.onmessage?.(update);
    expect(seen).toEqual([update]);

    invokeMock.mockResolvedValue(undefined);
    stop();
    stop(); // idempotent
    expect(invokeMock).toHaveBeenCalledWith("unsubscribe", { id: 7 });
    expect(invokeMock.mock.calls.filter(([c]) => c === "unsubscribe")).toHaveLength(1);
  });

  it("an unsubscribe failure is swallowed", async () => {
    invokeMock.mockResolvedValueOnce(1);
    const stop = await subscribeQuotes([], () => undefined);
    invokeMock.mockRejectedValueOnce("gone");
    expect(() => stop()).not.toThrow();
  });

  it("rejects with the structured error when the backend refuses", async () => {
    invokeMock.mockRejectedValueOnce({
      code: "INVALID_ARGUMENT",
      message: "bad topic",
      retryable: false,
    });
    await expect(subscribeQuotes([], () => undefined)).rejects.toMatchObject({
      code: "INVALID_ARGUMENT",
    });
  });
});
