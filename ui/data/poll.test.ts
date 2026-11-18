import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPoll } from "./poll";

const flush = async () => {
  await vi.advanceTimersByTimeAsync(0);
};

describe("createPoll", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    Object.defineProperty(document, "hidden", { value: false, configurable: true });
  });

  it("fetches at once, then on the schedule, and stops when disposed", async () => {
    const fetcher = vi.fn().mockResolvedValue(1);
    const dispose = createRoot((d) => {
      createPoll(() => "a", fetcher, { everyMs: () => 1000 });
      return d;
    });
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(2000);
    expect(fetcher).toHaveBeenCalledTimes(3);
    dispose();
    await vi.advanceTimersByTimeAsync(5000);
    expect(fetcher).toHaveBeenCalledTimes(3);
  });

  it("keeps the last data when a refresh fails and clears the error on the next success", async () => {
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce("first")
      .mockRejectedValueOnce(new Error("blocked"))
      .mockResolvedValue("third");
    const poll = createRoot(() => createPoll(() => 1, fetcher, { everyMs: () => 1000 }));
    await flush();
    expect(poll.data()).toBe("first");
    await vi.advanceTimersByTimeAsync(1000);
    expect(poll.data()).toBe("first");
    expect(poll.error()).toBe("blocked");
    await vi.advanceTimersByTimeAsync(1000);
    expect(poll.data()).toBe("third");
    expect(poll.error()).toBeUndefined();
  });

  it("drops the old data when the source changes and ignores a late answer to the old one", async () => {
    let resolveOld: (v: string) => void = () => {};
    const fetcher = vi.fn((s: string) =>
      s === "old" ? new Promise<string>((r) => (resolveOld = r)) : Promise.resolve(`data:${s}`),
    );
    const [source, setSource] = createSignal("old");
    const poll = createRoot(() => createPoll(source, fetcher, { everyMs: () => 1000 }));
    await flush();
    setSource("new");
    await flush();
    expect(poll.data()).toBe("data:new");
    resolveOld("data:old");
    await flush();
    expect(poll.data()).toBe("data:new");
  });

  it("does nothing for a false source and pauses while the window is hidden", async () => {
    const fetcher = vi.fn().mockResolvedValue(1);
    const [source, setSource] = createSignal<string | false>(false);
    createRoot(() => createPoll(source, fetcher, { everyMs: () => 1000 }));
    await flush();
    expect(fetcher).not.toHaveBeenCalled();
    Object.defineProperty(document, "hidden", { value: true, configurable: true });
    setSource("a");
    await vi.advanceTimersByTimeAsync(3500);
    expect(fetcher).not.toHaveBeenCalled();
    Object.defineProperty(document, "hidden", { value: false, configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
});
