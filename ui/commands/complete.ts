// Tab completion for the first word of the command line. Pure.

export interface Completion {
  /** Words that start with the typed prefix, in order. */
  candidates: string[];
}

/**
 * Candidates for the word being typed. Only the first token is completed:
 * once the user has typed a space they are in function/argument position.
 * A leading `EXCH:` is kept and not completed.
 */
export function completions(input: string, words: readonly string[]): Completion {
  if (/\s/.test(input.trimStart())) return { candidates: [] };
  const typed = input.trimStart().toUpperCase();
  if (typed === "" || typed.includes(":")) return { candidates: [] };
  return { candidates: words.filter((w) => w.startsWith(typed) && w !== typed) };
}

/**
 * The text Tab should produce. Repeated presses cycle through candidates:
 * pass the previous result in `cycle` to continue.
 */
export function applyTab(
  input: string,
  words: readonly string[],
  cycle?: { base: string; index: number },
): { text: string; cycle?: { base: string; index: number } } {
  const base = cycle?.base ?? input;
  const { candidates } = completions(base, words);
  const [first] = candidates;
  if (first === undefined) return { text: input };
  if (candidates.length === 1) return { text: `${first} ` };
  const index = cycle ? (cycle.index + 1) % candidates.length : 0;
  return { text: candidates[index] ?? first, cycle: { base, index } };
}
