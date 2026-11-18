import { createRoot } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  inTauri: vi.fn(() => true),
  loadUiState: vi.fn(),
  saveLayout: vi.fn(() => Promise.resolve()),
  saveCommandHistory: vi.fn(() => Promise.resolve()),
}));
vi.mock("../ipc", () => ipc);

import { createPersistence } from "./persistence";
import { createWorkspace } from "./workspace";

const flush = () => new Promise((r) => setTimeout(r, 0));

function setup() {
  const ws = createWorkspace();
  let history!: ReturnType<typeof createPersistence>["history"];
  const dispose = createRoot((d) => {
    history = createPersistence(ws).history;
    return d;
  });
  return { ws, history, dispose };
}

describe("persistence", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    ipc.inTauri.mockReturnValue(true);
    ipc.loadUiState.mockResolvedValue({ layout: 2, history: ["NIFTY", "RELIANCE"] });
  });

  it("restores layout and history without writing them back", async () => {
    const { ws, history, dispose } = setup();
    await flush();
    expect(ws.state.layout).toBe(2);
    expect(history.entries()).toEqual(["NIFTY", "RELIANCE"]);
    expect(ipc.saveCommandHistory).not.toHaveBeenCalled();
    dispose();
  });

  it("saves layout changes and new history entries after loading", async () => {
    const { ws, history, dispose } = setup();
    await flush();
    ipc.saveLayout.mockClear();
    ws.setLayout(4);
    await flush();
    expect(ipc.saveLayout).toHaveBeenCalledWith(4);
    history.push("TCS");
    expect(ipc.saveCommandHistory).toHaveBeenCalledWith(["NIFTY", "RELIANCE", "TCS"]);
    dispose();
  });

  it("does not overwrite saved state before it has loaded", async () => {
    ipc.loadUiState.mockReturnValue(new Promise(() => {})); // never resolves
    const { ws, history, dispose } = setup();
    ws.setLayout(4);
    history.push("X");
    await flush();
    expect(ipc.saveLayout).not.toHaveBeenCalled();
    expect(ipc.saveCommandHistory).not.toHaveBeenCalled();
    dispose();
  });

  it("ignores an invalid stored layout and survives load failures", async () => {
    ipc.loadUiState.mockResolvedValueOnce({ layout: 3, history: [] });
    const a = setup();
    await flush();
    expect(a.ws.state.layout).toBe(1);
    a.dispose();

    ipc.loadUiState.mockRejectedValueOnce(new Error("boom"));
    const b = setup();
    await flush();
    expect(b.ws.state.layout).toBe(1);
    b.dispose();
  });

  it("does nothing outside the app", async () => {
    ipc.inTauri.mockReturnValue(false);
    const { dispose } = setup();
    await flush();
    expect(ipc.loadUiState).not.toHaveBeenCalled();
    dispose();
  });
});
