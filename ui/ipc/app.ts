import type { AppInfo } from "./bindings/AppInfo";
import type { AppPaths } from "./bindings/AppPaths";
import { call } from "./core";

export function appInfo(): Promise<AppInfo> {
  return call<AppInfo>("app_info");
}

/** Tells the backend the shell is interactive; returns ms since process start. */
export function appReady(): Promise<number> {
  return call<number>("app_ready");
}

export function appPaths(): Promise<AppPaths> {
  return call<AppPaths>("app_paths");
}
