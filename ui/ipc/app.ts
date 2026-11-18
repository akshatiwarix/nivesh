import type { AppInfo } from "./bindings/AppInfo";
import { call } from "./core";

export function appInfo(): Promise<AppInfo> {
  return call<AppInfo>("app_info");
}
