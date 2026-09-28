// Keystroke latency probes, driven end to end in Chromium
// (`specs/1-Foundations/ambre-spec-budgets.md` section 1).
//
// Opens a fresh Prose document, fills it to a long document, then plays
// three scenarios: single keys at a human pace, a sustained burst, and a
// 10 KB paste. For each, it reads the `ambre:*` measures the view stamps on
// the page's timeline and prints the median and the 95th percentile.
//
// The Logic thread's `ambre:logic-pass` lives on the worker's timeline, which
// no page script reads: the worker blocks in its wait and never answers one.
// A Chrome trace over the single keys records it instead, beside the page's
// marks and every animation frame, and splits each confirmation on one clock.
//
// Usage (the relay and `trunk serve` running):
//   NODE_PATH=$(npm root -g) node scripts/keystroke_probes.mjs [base-url]
//
// The figures from a fast machine are a reference, not a proof: the budgets
// are set for a slow phone and a slow laptop, and must be measured there.

// `require` honours NODE_PATH, so a global install of Playwright serves.
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)("playwright");
import { randomUUID } from "node:crypto";

const base = process.argv[2] ?? "http://127.0.0.1:8080";
const NAMES = ["ambre:echo", "ambre:confirm", "ambre:caret", "ambre:shape-draft"];
const PARAGRAPH =
  "The quick brown fox jumps over the lazy dog while the text keeps flowing. ".repeat(26);

function percentile(sorted, q) {
  return sorted[Math.round(q * (sorted.length - 1))];
}

async function collect(page) {
  const out = await page.evaluate((names) => {
    const out = {};
    for (const name of names) {
      out[name] = performance.getEntriesByName(name, "measure").map((e) => e.duration);
    }
    out.confirms = performance
      .getEntriesByName("ambre:confirm", "measure")
      .map((e) => [e.startTime, e.startTime + e.duration]);
    out.counts = performance
      .getEntriesByName("ambre:confirm-frames", "mark")
      .map((e) => e.detail);
    out.relays = performance.getEntriesByName("ambre:relay", "mark").map((e) => e.startTime);
    out.origin = performance.timeOrigin;
    performance.clearMeasures();
    performance.clearMarks();
    return out;
  }, NAMES);
  out.split = splitConfirms(out.confirms, out.relays);
  return out;
}

// Split each confirmation at the relay mark: input to the relay taking the
// answer off its channel, then relay to the view. The Logic pass is not read
// here: the worker blocks in a wait and never answers an evaluate.
function splitConfirms(confirms, relays) {
  const split = { "to-relay": [], "to-view": [] };
  for (const [start, end] of confirms) {
    const relay = relays.filter((t) => t >= start && t <= end).pop();
    if (relay === undefined) continue;
    split["to-relay"].push(relay - start);
    split["to-view"].push(end - relay);
  }
  return split;
}

function line(name, samples, unit = "ms") {
  const values = [...(samples ?? [])].sort((a, b) => a - b);
  if (values.length === 0) {
    console.log(`${name.padEnd(18)} no sample`);
    return;
  }
  const digits = unit === "ms" ? 2 : 0;
  const p50 = percentile(values, 0.5).toFixed(digits);
  const p95 = percentile(values, 0.95).toFixed(digits);
  const max = values[values.length - 1].toFixed(digits);
  console.log(`${name.padEnd(18)} n=${String(values.length).padStart(4)} p50=${p50} ${unit} p95=${p95} ${unit} max=${max} ${unit}`);
}

function report(scenario, samples) {
  console.log(`\n== ${scenario}`);
  for (const name of NAMES) line(name, samples[name]);
  line("ambre:confirm-frames", samples.counts, "frames");
  console.log("-- confirmation, split");
  for (const [name, values] of Object.entries(samples.split)) line(`  ${name}`, values);
}

// The trace's own clock, in milliseconds.
const ms = (event) => event.ts / 1000;

// The spans of one measure name, paired begin to end by their async id.
function spans(events, name) {
  const open = new Map();
  const out = [];
  for (const event of events) {
    if (event.name !== name || !String(event.cat).includes("blink.user_timing")) continue;
    const key = `${event.pid}:${event.id ?? event.id2?.local ?? event.id2?.global}`;
    if (event.ph === "b") open.set(key, ms(event));
    if (event.ph === "e" && open.has(key)) {
      out.push([open.get(key), ms(event)]);
      open.delete(key);
    }
  }
  return out.sort((a, b) => a[0] - b[0]);
}

// Split each confirmation of the traced keys on one clock: whether the Logic
// thread was busy when the key arrived, how long it took to start the pass
// that applied it, that pass, when the relay took the answer, and how many
// animation frames went by.
function traceReport(events) {
  events.sort((a, b) => a.ts - b.ts);
  const confirms = spans(events, "ambre:confirm");
  const passes = spans(events, "ambre:logic-pass");
  const relays = events
    .filter((e) => e.name === "ambre:relay" && String(e.cat).includes("blink.user_timing"))
    .map(ms);
  const frames = events.filter((e) => e.name === "FireAnimationFrame").map(ms);
  const gaps = frames.slice(1).map((t, i) => t - frames[i]);
  const split = { busy: [], "to-pass": [], pass: [], "pass-to-relay": [], frames: [] };
  for (const [start, end] of confirms) {
    const running = passes.find(([a, b]) => a < start && b > start);
    split.busy.push(running ? running[1] - start : 0);
    const own = passes.find(([a]) => a >= start);
    const relay = relays.filter((t) => t >= start && t <= end).pop();
    if (own && own[0] <= end) {
      split["to-pass"].push(own[0] - start);
      split.pass.push(own[1] - own[0]);
      if (relay !== undefined) split["pass-to-relay"].push(relay - own[1]);
    }
    split.frames.push(frames.filter((t) => t > start && t <= end).length);
  }
  // A few keys laid out on one line each, in milliseconds from the key:
  // F a frame, R the relay taking answers, P[ ] a Logic pass, C the confirmation.
  const tasks = events.filter((e) => e.name === "RunTask" && e.ph === "X");
  for (const [start, end] of confirms.slice(10, 16)) {
    const at = (t) => (t - start).toFixed(1);
    const marks = [
      ...frames.filter((t) => t > start - 40 && t <= end + 5).map((t) => [t, `F${at(t)}`]),
      ...relays.filter((t) => t > start - 40 && t <= end + 5).map((t) => [t, `R${at(t)}`]),
      ...passes.filter(([a, b]) => b > start - 40 && a <= end + 5).map(([a, b]) => [a, `P[${at(a)},${at(b)}]`]),
      [end, `C${at(end)}`],
    ].sort((x, y) => x[0] - y[0]);
    console.log("  key: " + marks.map(([, label]) => label).join(" "));
  }
  console.log("-- trace: the single keys on one clock");
  console.log(`  ${confirms.length} confirmations, ${passes.length} Logic passes, ${relays.length} relay marks, ${frames.length} frames`);
  line("  frame gap", gaps);
  line("  every pass", passes.map(([a, b]) => b - a));
  for (const [name, values] of Object.entries(split)) line(`  ${name}`, values);
}

async function paste(page, text) {
  await page.evaluate((t) => navigator.clipboard.writeText(t), text);
  await page.keyboard.press("Control+V");
}

async function settle(page) {
  // Let the last answers land and a few frames draw.
  await page.waitForTimeout(1500);
}

// A run that stalls fails instead of hanging.
setTimeout(() => {
  console.error("keystroke_probes: no result after 5 minutes");
  process.exit(2);
}, 300_000).unref();
const health = await fetch(base).catch((e) => ({ ok: false, statusText: e.message }));
if (!health.ok) {
  console.error(`keystroke_probes: ${base} does not answer (${health.statusText})`);
  process.exit(2);
}

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH });
const context = await browser.newContext({ permissions: ["clipboard-read", "clipboard-write"] });
const page = await context.newPage();
page.on("pageerror", (e) => console.error("page error:", e.message));
page.on("console", (m) => { if (m.type() === "error") console.error("console:", m.text().slice(0, 400)); });

await page.goto(`${base}/prose/${randomUUID()}?user=probe&probes`);
const canvas = page.locator("canvas.ambre-canvas");
await canvas.waitFor({ timeout: 120_000 });
await page.waitForTimeout(3000);
const box = await canvas.boundingBox();
await page.mouse.click(box.x + box.width / 2, box.y + 120);
// Text reaches the page through its hidden textarea, which a click on the
// canvas does not focus in a headless browser.
await page.focus("textarea.ambre-input");

// A long document: 40 paragraphs of about 2 000 characters each.
for (let i = 0; i < 40; i++) {
  await paste(page, PARAGRAPH);
  await page.keyboard.press("Enter");
}
await settle(page);
await collect(page);

// Single keys, at a human pace, at the end of a 2 000 character paragraph.
await paste(page, PARAGRAPH);
await settle(page);
await collect(page);
await browser.startTracing(page, { categories: ["blink.user_timing", "devtools.timeline"] });
for (let i = 0; i < 60; i++) {
  await page.keyboard.type("a");
  await page.waitForTimeout(120);
}
await settle(page);
const trace = JSON.parse((await browser.stopTracing()).toString());
report("single keys, 120 ms apart", await collect(page));
traceReport(trace.traceEvents ?? trace);

// A sustained burst: 300 characters with no pause.
await page.keyboard.type("x".repeat(300), { delay: 0 });
await settle(page);
report("burst of 300 keys", await collect(page));

// A 10 KB paste.
await page.keyboard.press("Enter");
await paste(page, "Pasted words run on and on. ".repeat(366));
await settle(page);
report("10 KB paste", await collect(page));

await browser.close();
