import type { Exchange, Parsed } from "../commands";
import { describeError } from "../data/errors";
import { askChallenge, confirmable, engageKill, setKill } from "../data/risk";
import {
  type ItemRef,
  parseItemRef,
  type WatchlistStore,
  watchlistStore,
} from "../data/watchlists";
import {
  auditExport,
  auditVerify,
  backtestStrategy,
  brokerDemo,
  brokerLogin,
  brokerLogout,
  inTauri,
  killReset,
  killStatus,
  liveSwitch,
  paperStart,
  paperStop,
  riskClearLimits,
  riskSetLimit,
  strategyArm,
  strategyDisarm,
  strategySave,
} from "../ipc";
import { formatIstHm } from "../lib/time";
import { DEFAULT_UNIVERSE, findUniverse } from "../lib/universe";
import { parseChartArgs, parseInterval } from "../views/chart/model";
import { sendViewCommand } from "../views/context";
import { showOnboarding } from "./onboardingState";
import { parseStratArgs } from "./strat";
import type { Workspace } from "./workspace";

/** What a command returns: an error message for the command bar, or nothing. */
export type CommandResult = string | undefined | Promise<string | undefined>;

/**
 * Runs a parsed command against the workspace. Returns an error message
 * for the command bar (possibly later, for commands that call the backend),
 * or undefined on success.
 */
export function dispatch(parsed: Parsed, ws: Workspace): CommandResult {
  switch (parsed.kind) {
    case "empty":
      return undefined;
    case "error":
      return parsed.message;
    case "instrument":
      return openInstrument(parsed.exchange, parsed.symbol, parsed.fn, parsed.args, ws);
    case "index":
      ws.open({ kind: "index", id: parsed.id, title: parsed.title });
      return undefined;
    case "command":
      return runCommand(parsed.id, ws, parsed.args);
  }
}

function openInstrument(
  exchange: Exchange,
  symbol: string,
  fn: string | undefined,
  args: readonly string[],
  ws: Workspace,
): CommandResult {
  switch (fn) {
    case undefined:
      ws.open({ kind: "quote", exchange, symbol });
      return undefined;
    case "ch": {
      const chart = parseChartArgs(args);
      if (typeof chart === "string") return chart;
      ws.open({ kind: "chart", exchange, symbol, ...chart });
      return undefined;
    }
    case "news":
      ws.open({ kind: "news", symbol });
      return undefined;
    default:
      return `${fn.toUpperCase()} is not available yet.`;
  }
}

/** The instrument the focused panel is showing, if it is showing one. */
export function focusedInstrument(
  ws: Workspace,
): { exchange: Exchange; symbol: string } | undefined {
  const view = ws.currentView(ws.state.focused);
  return view.kind === "quote" || view.kind === "chart"
    ? { exchange: view.exchange, symbol: view.symbol }
    : undefined;
}

/** Runs a registered command by id (also used by function keys). */
export function runCommand(
  id: string,
  ws: Workspace,
  args: readonly string[] = [],
  watchlists: WatchlistStore = watchlistStore,
): CommandResult {
  switch (id) {
    case "help":
      // Opening HELP twice shouldn't stack two help views.
      if (ws.currentView(ws.state.focused).kind !== "help") ws.open({ kind: "help" });
      return undefined;
    case "mkt":
      if (ws.currentView(ws.state.focused).kind !== "mkt") ws.open({ kind: "mkt" });
      return undefined;
    case "wl":
      return runWatchlist(ws, args, watchlists);
    case "news": {
      const symbol = args[0]?.toUpperCase().replace(/^(NSE|BSE):/, "");
      if (symbol !== undefined && !/^[A-Z0-9][A-Z0-9&\-_.]*$/.test(symbol)) {
        return `Invalid symbol "${args[0]}".`;
      }
      ws.open(symbol === undefined ? { kind: "news" } : { kind: "news", symbol });
      return undefined;
    }
    case "sec":
      if (ws.currentView(ws.state.focused).kind !== "sec") ws.open({ kind: "sec" });
      return undefined;
    case "brd": {
      const universe = args[0] === undefined ? DEFAULT_UNIVERSE : findUniverse(args[0]);
      if (!universe) return `${args[0]} is not an index with constituents. Try NIFTY or BANKNIFTY.`;
      ws.open({ kind: "brd", index: universe.id });
      return undefined;
    }
    case "mov": {
      const universe = args[0] === undefined ? DEFAULT_UNIVERSE : findUniverse(args[0]);
      if (!universe) return `${args[0]} is not an index with constituents. Try NIFTY or BANKNIFTY.`;
      ws.open({ kind: "mov", index: universe.id });
      return undefined;
    }
    case "port":
      if (ws.currentView(ws.state.focused).kind !== "port") ws.open({ kind: "port" });
      return undefined;
    case "pos":
      if (ws.currentView(ws.state.focused).kind !== "pos") ws.open({ kind: "pos" });
      return undefined;
    case "ord":
      if (ws.currentView(ws.state.focused).kind !== "ord") ws.open({ kind: "ord" });
      return undefined;
    case "trd":
      if (ws.currentView(ws.state.focused).kind !== "trd") ws.open({ kind: "trd" });
      return undefined;
    case "funds":
      if (ws.currentView(ws.state.focused).kind !== "funds") ws.open({ kind: "funds" });
      return undefined;
    case "strat": {
      // STRAT [TEMPLATE fast slow qty] builds from a template and saves it;
      // bare STRAT lists what is saved.
      if (args.length === 0) {
        if (ws.currentView(ws.state.focused).kind !== "strat") ws.open({ kind: "strat" });
        return undefined;
      }
      const strat = parseStratArgs(args);
      if (typeof strat === "string") return strat;
      if (!inTauri()) return "Strategies run in the Nivesh app.";
      return (async () => {
        try {
          await strategySave(strat);
          if (ws.currentView(ws.state.focused).kind !== "strat") ws.open({ kind: "strat" });
          return undefined;
        } catch (err) {
          return describeError(err);
        }
      })();
    }
    case "bt": {
      const [name, ...rest] = args;
      if (name === undefined) return "Which strategy? BT <NAME> [SYMBOL] [INTERVAL] [RANGE].";
      // An optional symbol comes first; anything that reads as an interval
      // is the interval (the older BT <NAME> [INTERVAL] [RANGE] form).
      const symbol =
        rest[0] !== undefined && parseInterval(rest[0]) === undefined ? rest.shift() : undefined;
      const interval = rest[0]?.toUpperCase() ?? "1D";
      const range = rest[1]?.toUpperCase() ?? "1Y";
      ws.open({ kind: "bt", name });
      if (!inTauri()) return undefined;
      return (async () => {
        try {
          await backtestStrategy(name, interval, range, symbol);
          return undefined;
        } catch (err) {
          return describeError(err);
        }
      })();
    }
    case "paper": {
      const sub = args[0]?.toUpperCase();
      if (sub === undefined) {
        if (ws.currentView(ws.state.focused).kind !== "paper") ws.open({ kind: "paper" });
        return undefined;
      }
      if (sub !== "START" && sub !== "STOP") {
        return "PAPER START <NAME> [SYMBOL] [INTERVAL] or PAPER STOP <NAME>, or bare PAPER for the dashboard.";
      }
      const name = args[1];
      if (name === undefined) {
        return `PAPER ${sub} needs the strategy's name.`;
      }
      if (!inTauri()) return "Paper trading runs in the Nivesh app.";
      return (async () => {
        try {
          if (sub === "START") {
            const symbol = args[2]?.replace(/^(NSE|BSE):/, "");
            await paperStart(name, symbol, args[3]);
          } else await paperStop(name);
          if (ws.currentView(ws.state.focused).kind !== "paper") ws.open({ kind: "paper" });
          return undefined;
        } catch (err) {
          return describeError(err);
        }
      })();
    }
    case "risk":
      return runRisk(args, ws);
    case "audit":
      return runAudit(args, ws);
    case "settings":
      ws.open({ kind: "settings" });
      return undefined;
    case "welcome":
      showOnboarding(false);
      return undefined;
    case "script": {
      const name = args[0];
      if (name === undefined || args.length > 1)
        return "SCRIPT <NAME>: one word, e.g. SCRIPT TREND.";
      ws.open({ kind: "script", name });
      return undefined;
    }
    case "kill": {
      const sub = args[0]?.toUpperCase();
      if (sub !== undefined && sub !== "RESET") return "KILL, or KILL RESET.";
      if (!inTauri()) return "The kill switch is in the Nivesh app.";
      if (sub === undefined) return engageKill();
      openRisk(ws);
      return startKillReset();
    }
    case "live": {
      const sub = args[0]?.toUpperCase();
      if (sub !== "ON" && sub !== "OFF") return "LIVE ON or LIVE OFF.";
      if (!inTauri()) return "Live trading is controlled in the Nivesh app.";
      openRisk(ws);
      return confirmable("switch live trading on", (typed) => liveSwitch(sub === "ON", typed));
    }
    case "arm": {
      const name = args[0];
      if (name === undefined) return "ARM <NAME>: which strategy?";
      if (!inTauri()) return "Arming runs in the Nivesh app.";
      openRisk(ws);
      return confirmable(`arm ${name} for live trading`, (typed) => strategyArm(name, typed));
    }
    case "disarm": {
      const name = args[0];
      if (name === undefined) return "DISARM <NAME>: which strategy?";
      if (!inTauri()) return "Arming runs in the Nivesh app.";
      openRisk(ws);
      return confirmable(`disarm ${name}`, async () => {
        await strategyDisarm(name);
        return { done: true, confirm: null };
      });
    }
    case "login": {
      // LOGIN DEMO starts the demo broker; LOGIN starts the Kite login whose
      // URL the backend already opened in the system browser.
      const demo = args[0]?.toUpperCase() === "DEMO";
      if (!inTauri()) {
        return demo
          ? "The demo broker runs in the Nivesh app."
          : "Broker login runs in the Nivesh app.";
      }
      return runBrokerLogin(demo);
    }
    case "logout":
      return runBrokerLogout();
    case "add":
    case "rm": {
      if (ws.currentView(ws.state.focused).kind !== "chart") {
        return `${id.toUpperCase()} works on a chart. Open one first, e.g. RELIANCE CH`;
      }
      const result = sendViewCommand(ws.state.focused, id, args);
      return result === "unhandled" ? "The chart is still loading. Try again." : result;
    }
    case "ch": {
      const target = focusedInstrument(ws);
      if (!target) return "Open an instrument first, e.g. RELIANCE CH 5M";
      const chart = parseChartArgs(args);
      if (typeof chart === "string") return chart;
      ws.open({ kind: "chart", ...target, ...chart });
      return undefined;
    }
    default:
      return `${id.toUpperCase()} is not available yet.`;
  }
}

export const WL_USAGE = "Usage: WL [ADD|RM SYMBOL… | NEW|USE|DEL NAME]";

/** LOGIN / LOGIN DEMO: report the outcome in the command bar. */
export function runBrokerLogin(demo: boolean): Promise<string | undefined> {
  return (async () => {
    try {
      if (demo) {
        await brokerDemo();
        return undefined;
      }
      const outcome = await brokerLogin();
      return outcome.url
        ? "The Kite login page opened in your browser. LOGOUT cancels."
        : undefined;
    } catch (err) {
      return describeError(err);
    }
  })();
}

/** LOGOUT: end whichever broker session is active. */
export function runBrokerLogout(): Promise<string | undefined> {
  return (async () => {
    try {
      await brokerLogout();
      return "Logged out.";
    } catch (err) {
      return describeError(err);
    }
  })();
}

function showWatchlist(ws: Workspace) {
  if (ws.currentView(ws.state.focused).kind !== "wl") ws.open({ kind: "wl" });
}

/** `WL`, `WL ADD TCS INFY`, `WL RM TCS`, `WL NEW TECH`, `WL USE TECH`, `WL DEL TECH`. */
function runWatchlist(
  ws: Workspace,
  args: readonly string[],
  store: WatchlistStore,
): CommandResult {
  const [sub, ...rest] = args;
  if (sub === undefined) {
    showWatchlist(ws);
    return undefined;
  }
  const refs: ItemRef[] = rest.map(parseItemRef);
  const name = rest.join(" ");
  const act = async (work: () => Promise<void>): Promise<string | undefined> => {
    try {
      await store.load();
      await work();
      showWatchlist(ws);
      return undefined;
    } catch (err) {
      return describeError(err);
    }
  };

  switch (sub) {
    case "ADD":
      return rest.length === 0
        ? "WL ADD needs a symbol, e.g. WL ADD TCS"
        : act(() => store.add(refs));
    case "RM":
    case "REMOVE":
      return rest.length === 0
        ? "WL RM needs a symbol, e.g. WL RM TCS"
        : act(() => store.remove(refs));
    case "NEW":
      return name === "" ? "WL NEW needs a name, e.g. WL NEW TECH" : act(() => store.create(name));
    case "USE":
      return name === ""
        ? "WL USE needs a list name or number"
        : act(async () => {
            const list = store.findList(name);
            if (!list) throw new Error(`No watchlist named ${name}`);
            await store.activate(list.id);
          });
    case "DEL":
      return name === ""
        ? "WL DEL needs the list's name, e.g. WL DEL TECH"
        : act(async () => {
            // Deleting is destructive: the exact name must be typed.
            const list = store.state.lists.find((l) => l.name === name.toUpperCase());
            if (!list) throw new Error(`No watchlist named ${name}`);
            await store.delete(list.id);
          });
    default:
      return WL_USAGE;
  }
}

/** Opens RISK in the focused panel unless it is already there. */
function openRisk(ws: Workspace): void {
  if (ws.currentView(ws.state.focused).kind !== "risk") ws.open({ kind: "risk" });
}

const RISK_USAGE =
  "RISK, RISK SET <QTY|VALUE|EXPOSURE|POSITIONS|OPM|OPD|LOSS|BAND> <N> [FOR <NAME>], or RISK CLEAR <NAME>.";

/** RISK [SET <FIELD> <N> [FOR <NAME>] | CLEAR <NAME>]. */
function runRisk(args: readonly string[], ws: Workspace): CommandResult {
  const sub = args[0]?.toUpperCase();
  if (sub === undefined) {
    openRisk(ws);
    return undefined;
  }
  if (sub === "SET") {
    const [, field, raw, forWord, name, extra] = args;
    const value = Number(raw?.replace(/,/g, ""));
    if (field === undefined || raw === undefined || !Number.isFinite(value)) return RISK_USAGE;
    if (forWord !== undefined && (forWord.toUpperCase() !== "FOR" || name === undefined)) {
      return RISK_USAGE;
    }
    if (extra !== undefined) return RISK_USAGE;
    if (!inTauri()) return "The risk controls are in the Nivesh app.";
    openRisk(ws);
    const target = name === undefined ? "the global" : `${name}'s`;
    return confirmable(`loosen ${target} ${field.toUpperCase()} limit`, (typed) =>
      riskSetLimit(field, value, name, typed),
    );
  }
  if (sub === "CLEAR") {
    const name = args[1];
    if (name === undefined || args.length > 2) return RISK_USAGE;
    if (!inTauri()) return "The risk controls are in the Nivesh app.";
    openRisk(ws);
    return confirmable(`remove ${name}'s own limits`, (typed) => riskClearLimits(name, typed));
  }
  return RISK_USAGE;
}

/** KILL RESET: only when engaged and past the cool-down, with the phrase. */
async function startKillReset(): Promise<string | undefined> {
  try {
    const status = await killStatus();
    setKill(status);
    if (!status.engaged) return "The kill switch is not engaged.";
    if (status.resettableAt !== null && status.resettableAt > Date.now()) {
      return `The kill switch can be reset from ${formatIstHm(status.resettableAt)} IST (5-minute cool-down).`;
    }
    const phrase = status.resetPhrase;
    askChallenge({
      action: "reset the kill switch",
      phrase,
      run: async (typed) => {
        // Rust checks the phrase too; this only keeps the box open on a typo.
        if (typed !== phrase) return { done: false, confirm: phrase };
        setKill(await killReset(typed));
        return { done: true, confirm: null };
      },
    });
    return `To reset the kill switch, type ${phrase} in the RISK panel (Esc cancels).`;
  } catch (err) {
    return describeError(err);
  }
}

const AUDIT_USAGE = "AUDIT [KIND <PREFIX>] [FOR <NAME>], AUDIT VERIFY, or AUDIT EXPORT CSV|JSONL.";

/** AUDIT [KIND <PREFIX>] [FOR <NAME>] | VERIFY | EXPORT CSV|JSONL. */
function runAudit(args: readonly string[], ws: Workspace): CommandResult {
  const sub = args[0]?.toUpperCase();
  if (sub === "VERIFY") {
    if (!inTauri()) return "The audit log is in the Nivesh app.";
    return (async () => {
      try {
        const v = await auditVerify();
        return v.intact
          ? `Audit chain intact: ${v.rows} records verified.`
          : `Audit chain BROKEN at record #${v.brokenAt}: ${v.reason}.`;
      } catch (err) {
        return describeError(err);
      }
    })();
  }
  if (sub === "EXPORT") {
    const format = args[1]?.toLowerCase();
    if ((format !== "csv" && format !== "jsonl") || args.length > 2) return AUDIT_USAGE;
    if (!inTauri()) return "The audit log is in the Nivesh app.";
    return (async () => {
      try {
        return `Audit log saved to ${await auditExport(format)}.`;
      } catch (err) {
        return describeError(err);
      }
    })();
  }
  let filter: string | undefined;
  let strategy: string | undefined;
  for (let i = 0; i < args.length; i += 2) {
    const word = args[i]?.toUpperCase();
    const value = args[i + 1];
    if (value === undefined) return AUDIT_USAGE;
    if (word === "KIND") filter = value.toLowerCase();
    else if (word === "FOR") strategy = value;
    else return AUDIT_USAGE;
  }
  const view = {
    kind: "audit" as const,
    ...(filter ? { filter } : {}),
    ...(strategy ? { strategy } : {}),
  };
  ws.open(view);
  return undefined;
}
