// The map page's delay band against the hand values, in Node, no browser and no dependency: the page's own `band`,
// `delayBand` and `aggregate`, cut from `page/data.js`, run on one bin of hand-made traffic. The cases are those of
// `rules.rs` (`a_link_whose_cars_lose_under_2_s_each_is_in_the_first_band`), which holds the encoder's
// `rules::delay_band` (the one the summary reads) to the same values, so the page and the summary colour every car
// link alike. The bands' edges (15 %, 50 %, 100 % over free flow) and the floor (2 s lost per car) are given here by
// hand, as the page's META carries them (`hand_run.rs` checks the page carries 2 s). CI runs it as a step of its own;
// by hand: `node crates/map/tests/page_delay_band.mjs`.
// For bikes: bikes and walkers are drawn by their counts, never by delay; nothing here concerns them.
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

const js = fs.readFileSync(new URL("../page/data.js", import.meta.url), "utf8");
// A function's whole text, from its name to its closing brace.
function cut(start) {
  const i = js.indexOf(start);
  assert.ok(i >= 0, "data.js has no " + start);
  let depth = 0, j = js.indexOf("{", i);
  for (; j < js.length; j++) { if (js[j] === "{") depth++; if (js[j] === "}" && --depth === 0) break; }
  return js.slice(i, j + 1);
}
const floorLine = js.split("\n").find((line) => line.startsWith("const FLOOR ="));
assert.ok(floorLine, "data.js has no const FLOOR");

// Each car link: its cars' mean crossing time and its free-flow time, seconds, and its band by hand (0: 0–15 %,
// 1: 15–50 %, 2: 50–100 %, 3: 100 % and over).
const CASES = [
  [0.433, 0.045, 0], // 862 % over free flow, 0.39 s lost per car: under the floor
  [11.9, 10, 0], // 19 %, 1.9 s per car
  [12, 10, 1], // 20 %, exactly 2 s per car: coloured
  [16, 10, 2], // 60 %
  [25, 10, 3], // 150 %
  [15, 10, 2], // exactly 50 %: an edge belongs to the band above it
  [20, 10, 3], // exactly 100 %
  [145, 144, 0], // 0.7 %, 1 s per car
  [150, 144, 0], // 4 %, 6 s per car: past the floor, under 15 %
  [180, 144, 1], // 25 %, 36 s per car
  [100, 144, 0], // faster than free flow
  [5, 0, 0], // no free-flow time
];
const CARS = 3; // each link entered by 3 cars in the one bin, their crossing times summed
const NL = CASES.length;
const META = { bin_seconds: 300, peak_seconds: 900, delay_bands: [0.15, 0.5, 1], delay_floor_s: 2 };
// The page's arrays, in the encoder's types: free flow (f32), the bin's first row (u32), each row's link and cars
// (u32) and the sum of its cars' crossing times (f32).
const page = {
  META, NL, Math, Float32Array, BANDS: META.delay_bands,
  FF: Float32Array.from(CASES, (c) => c[1]),
  BS: Uint32Array.from([0, NL]), RL: Uint32Array.from(CASES, (_, l) => l), RP: Uint32Array.from(CASES, () => CARS),
  RS: Float32Array.from(CASES, (c) => CARS * c[0]),
  vol: new Float32Array(NL), peak: new Float32Array(NL), delay: new Float32Array(NL), meanT: new Float32Array(NL),
  percentile: () => 1, PEAK: 0, VMAX: 1, VHIGH: 0, GLOBAL_VMAX: 1,
};
vm.createContext(page);
const code = [cut("function band("), floorLine, cut("function delayBand("), cut("function aggregate(")].join("\n");
const got = vm.runInContext(code + "\naggregate(0, 1);\nArray.from({ length: NL }, (_, l) => delayBand(l));", page);
assert.deepEqual([...got], CASES.map((c) => c[2]));
console.log("the page's delay band: " + NL + " car links, each in its band by hand");
