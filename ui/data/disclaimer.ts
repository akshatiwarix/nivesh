// The "not advice" disclaimer (PLAN.md §11 item 14), shown at onboarding
// and on every live-trading confirmation.

export const DISCLAIMER =
  "Nivesh is a tool, not investment advice. You are responsible for every trade it places for you, and trading can lose money.";

/** Whether a confirmation is about live trading (and so shows the disclaimer). */
export function isLiveAction(action: string): boolean {
  return /\blive\b/i.test(action);
}
