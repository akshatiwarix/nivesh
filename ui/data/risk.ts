// The safety controls' UI state, shared by the command bar and the RISK
// view: the pending typed confirmation, the recent alerts, and a counter
// that tells the view to refetch after an action.

import { createSignal } from "solid-js";
import type { RiskActionDto, RiskAlertDto } from "../ipc";
import { describeError } from "./errors";

/** A riskier action waiting for its phrase. */
export interface Challenge {
  /** What it does, in words: "arm TREND for live trading". */
  action: string;
  /** What to type, exactly. */
  phrase: string;
  /** Sends the action again with what was typed. */
  run: (typed: string) => Promise<RiskActionDto>;
}

const [challenge, setChallenge] = createSignal<Challenge>();
const [changed, setChanged] = createSignal(0);
const [alerts, setAlerts] = createSignal<readonly RiskAlertDto[]>([]);

export { alerts, challenge, changed };

/** Most recent alerts kept. */
const MAX_ALERTS = 20;

export function pushAlert(alert: RiskAlertDto): void {
  setAlerts((list) => [alert, ...list].slice(0, MAX_ALERTS));
}

/** Tells the RISK view something changed. */
export function riskChanged(): void {
  setChanged((n) => n + 1);
}

export function cancelChallenge(): void {
  setChallenge(undefined);
}

/**
 * Runs an action that may need a typed phrase. Done: the view refreshes.
 * Needs the phrase: it becomes the pending challenge, and the message
 * says where to type it. Returns a message for the command bar.
 */
export async function confirmable(
  action: string,
  send: (typed?: string) => Promise<RiskActionDto>,
): Promise<string | undefined> {
  try {
    const first = await send();
    if (first.done) {
      setChallenge(undefined);
      riskChanged();
      return undefined;
    }
    const phrase = first.confirm ?? "";
    setChallenge({ action, phrase, run: (typed) => send(typed) });
    return `To ${action}, type ${phrase} in the RISK panel (Esc cancels).`;
  } catch (err) {
    return describeError(err);
  }
}

/**
 * Answers the pending challenge. Resolves to an error message, or
 * undefined when it went through.
 */
export async function answerChallenge(typed: string): Promise<string | undefined> {
  const pending = challenge();
  if (pending === undefined) return "Nothing is waiting for confirmation.";
  try {
    const answer = await pending.run(typed.trim());
    if (!answer.done) return `That did not match. Type ${answer.confirm ?? pending.phrase}.`;
    setChallenge(undefined);
    riskChanged();
    return undefined;
  } catch (err) {
    setChallenge(undefined);
    return describeError(err);
  }
}
