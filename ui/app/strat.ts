// Parses STRAT's arguments: a template, its positional numbers, then
// optional keyword settings.
//
//   STRAT MA 20 50 1            fast, slow, quantity
//   STRAT RSI 14 SL 2 T 5       2% stop-loss, 5% target
//   STRAT BREAKOUT 20 10 RISK 1 20-bar high in, 10-bar low out, ATR sizing
//                               risking 1% of equity per trade
//
// Percentages may be written as percent (2) or as a fraction (0.02).

export type StratTemplate = "MA" | "RSI" | "BREAKOUT";

export interface StratArgs {
  template: StratTemplate;
  name: string;
  fast: number;
  slow: number;
  quantity: number;
  stopLoss?: number;
  target?: number;
  risk?: number;
}

const TEMPLATES: readonly StratTemplate[] = ["MA", "RSI", "BREAKOUT"];
const KEYWORDS = { SL: "stopLoss", T: "target", RISK: "risk" } as const;
type Keyword = keyof typeof KEYWORDS;

export const STRAT_USAGE =
  "STRAT MA 20 50 1 · STRAT RSI 14 · STRAT BREAKOUT 20 10, then optionally SL 2 · T 5 · RISK 1 (percent)";

/** `2` and `0.02` both mean 2%. */
function fraction(value: number): number {
  return value >= 1 ? value / 100 : value;
}

export function parseStratArgs(args: readonly string[]): StratArgs | string {
  const template = args[0]?.toUpperCase() as StratTemplate | undefined;
  if (template === undefined || !TEMPLATES.includes(template)) {
    return `Which template? ${STRAT_USAGE}.`;
  }
  const numbers: number[] = [];
  const settings: Partial<Record<(typeof KEYWORDS)[Keyword], number>> = {};
  let i = 1;
  for (; i < args.length; i++) {
    const word = args[i]?.toUpperCase() ?? "";
    if (word in KEYWORDS) break;
    const n = Number(word);
    if (!Number.isFinite(n) || n <= 0)
      return `"${args[i]}" is not a positive number. ${STRAT_USAGE}.`;
    numbers.push(n);
  }
  for (; i < args.length; i += 2) {
    const word = args[i]?.toUpperCase() ?? "";
    if (!(word in KEYWORDS)) return `Unknown setting "${args[i]}". ${STRAT_USAGE}.`;
    const value = Number(args[i + 1]);
    if (args[i + 1] === undefined || !Number.isFinite(value) || value <= 0) {
      return `${word} needs a positive percentage, e.g. ${word} 2.`;
    }
    settings[KEYWORDS[word as Keyword]] = fraction(value);
  }
  const fast = numbers[0] ?? (template === "RSI" ? 14 : 20);
  // The breakout's second number is its exit channel, which is shorter.
  const slow = numbers[1] ?? (template === "BREAKOUT" ? 10 : 50);
  const quantity = numbers[2] ?? 1;
  const base =
    template === "MA"
      ? `MA ${fast}/${slow}`
      : template === "RSI"
        ? `RSI ${fast}`
        : `Breakout ${fast}/${slow}`;
  // The name says what differs, so variants don't collide.
  const suffix = [
    settings.stopLoss !== undefined ? `SL${+(settings.stopLoss * 100).toFixed(2)}` : undefined,
    settings.target !== undefined ? `T${+(settings.target * 100).toFixed(2)}` : undefined,
    settings.risk !== undefined ? `R${+(settings.risk * 100).toFixed(2)}` : undefined,
  ].filter((x) => x !== undefined);
  return {
    template,
    name: suffix.length > 0 ? `${base} ${suffix.join(" ")}` : base,
    fast,
    slow,
    quantity,
    ...settings,
  };
}
