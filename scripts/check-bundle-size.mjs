#!/usr/bin/env node
// Initial JS bundle budget (PLAN.md §2.9): the gzipped JS that index.html
// loads up front must stay under 150 KB. Lazily imported chunks (the chart
// library) are not counted. Run after `pnpm build`.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

const BUDGET_KB = 150;
const dist = "dist";
const html = readFileSync(join(dist, "index.html"), "utf8");
const initial = [
  ...html.matchAll(/<script[^>]+src="\/?([^"]+\.js)"/g),
  ...html.matchAll(/<link[^>]+rel="modulepreload"[^>]+href="\/?([^"]+\.js)"/g),
].map((m) => m[1]);

if (initial.length === 0) {
  console.error("no initial scripts found in dist/index.html; run pnpm build first");
  process.exit(1);
}

let total = 0;
for (const file of new Set(initial)) {
  const kb = gzipSync(readFileSync(join(dist, file))).length / 1024;
  total += kb;
  console.log(`${kb.toFixed(1).padStart(7)} KB gz  ${file}`);
}
console.log(`${total.toFixed(1).padStart(7)} KB gz  initial JS (budget ${BUDGET_KB} KB)`);
if (total > BUDGET_KB) {
  console.error(`over budget by ${(total - BUDGET_KB).toFixed(1)} KB`);
  process.exit(1);
}
