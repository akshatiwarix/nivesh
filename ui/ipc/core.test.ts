import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { appInfo } from "./app";
import { call, IpcFailure, toIpcError } from "./core";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: () => false,
}));

const invokeMock = vi.mocked(invoke);

describe("toIpcError", () => {
  it.each([
    [
      "structured command errors pass through",
      { code: "INVALID_ARGUMENT", message: "bad", retryable: false },
      { code: "INVALID_ARGUMENT", message: "bad", retryable: false },
    ],
    [
      "plain strings from tauri become INTERNAL",
      "command app_inf not found",
      { code: "INTERNAL", message: "command app_inf not found", retryable: false },
    ],
    [
      "Error objects keep their message",
      new Error("boom"),
      { code: "INTERNAL", message: "boom", retryable: false },
    ],
    [
      "objects with unknown codes are not trusted",
      { code: "NOPE", message: "x", retryable: true },
      { code: "INTERNAL", message: "[object Object]", retryable: false },
    ],
    ["null", null, { code: "INTERNAL", message: "null", retryable: false }],
  ])("%s", (_name, input, expected) => {
    expect(toIpcError(input)).toEqual(expected);
  });
});

describe("call", () => {
  beforeEach(() => invokeMock.mockReset());

  it("resolves with the command result", async () => {
    invokeMock.mockResolvedValueOnce(42);
    await expect(call<number>("answer", { q: 1 })).resolves.toBe(42);
    expect(invokeMock).toHaveBeenCalledWith("answer", { q: 1 });
  });

  it("rejects with an IpcFailure carrying the error code", async () => {
    invokeMock.mockRejectedValueOnce({ code: "UNAVAILABLE", message: "offline", retryable: true });
    const failure = await call("x").catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(IpcFailure);
    expect(failure).toMatchObject({
      command: "x",
      code: "UNAVAILABLE",
      message: "offline",
      retryable: true,
    });
  });
});

describe("wrappers", () => {
  beforeEach(() => invokeMock.mockReset());

  it("appInfo calls app_info without arguments", async () => {
    const info = {
      name: "Nivesh",
      version: "0.1.0",
      build: "debug",
      os: "linux",
      arch: "x86_64",
      tauriVersion: "2.12.6",
    };
    invokeMock.mockResolvedValueOnce(info);
    await expect(appInfo()).resolves.toEqual(info);
    expect(invokeMock).toHaveBeenCalledWith("app_info", undefined);
  });
});
