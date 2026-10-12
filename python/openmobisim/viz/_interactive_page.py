# ruff: noqa: E501
"""The page of the interactive map (S167): HTML shell, stylesheet and script.

Kept as text in Python so the map is one self-contained file and nothing has to
be found on disk at run time. The script needs no library and no network: it
reads the data the builder embeds, draws with a canvas, and keeps its whole
state in the URL hash so a view can be shared and reproduced.
"""

PAGE_HTML = r"""<!doctype html>
<html lang="en" data-theme="paper">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="generator" content="__GENERATOR__">
<title>__TITLE__</title>
<style>
__CSS__
</style>
</head>
<body data-state="loading">
<canvas id="map"></canvas>
<div id="error" hidden></div>
<header class="head">
  <svg class="mark" viewBox="0 0 64 64" aria-hidden="true"><path d="__LOGO__"/></svg><h1 id="title"></h1>
  <p id="subtitle"></p><p id="note" hidden></p>
</header>
<aside id="panel">
  <h2>Zoom</h2>
  <div id="levels"><button>Overview</button><button>Region</button><button>District</button><button>Street</button><button>Detail</button></div>
  <div class="row"><button id="zout" title="Zoom out">−</button><button id="zin" title="Zoom in">+</button><button id="fit">Fit</button><span id="levelname"></span></div>
  <div class="row"><span>Export the view</span><button id="exportpng" title="An image at twice the screen's resolution">PNG</button><button id="exportsvg" title="Vector graphics, for print and editing">SVG</button></div>
  <div id="layergroup" hidden>
    <h2>Layers</h2>
    <div class="layers" id="layerrows"></div>
  </div>
  <div id="trafficgroup">
    <h2>Traffic</h2>
    <div class="row"><label>Colour by <select id="colour"><option value="delay">Delay</option><option value="volume">Volume</option><option value="vc">Volume ÷ capacity</option></select></label><button id="theme">Night</button></div>
    <div class="row"><input id="bin" type="range" min="0" max="0" value="0" step="1"></div>
    <div class="row"><label><input id="allbins" type="checkbox" checked> All bins</label><button id="play">Play</button></div>
    <div class="row" id="spillrow" hidden><label title="Links whose queue would not have fitted on the road: where it would spill back (point queue)"><input id="spill" type="checkbox"> Spillback links <span id="spillcount"></span></label></div>
  </div>
  <div id="routegroup">
    <h2>Route alternatives</h2>
    <div class="row"><label>Pair <input id="pairno" type="number" min="1" value=""></label><span id="pairs"></span></div>
    <div class="row"><button id="prev">‹</button><button id="next">›</button><button id="richest">Most alternatives</button><button id="clear">Clear</button></div>
    <div class="row"><label><input id="showroutes" type="checkbox" checked> Show routes</label><label><input id="dim" type="checkbox" checked> Dim traffic</label></div>
    <div class="row"><label title="A pair's routes, the most chosen first">Routes <select id="nroutes"><option value="5">5 most chosen</option><option value="0">All</option></select></label></div>
    <p id="routehint">Pick a pair above, or click a link and choose one of the pairs that use it.</p>
    <div id="routelist"></div>
  </div>
  <div id="linkinfo" hidden></div>
  <div id="legendgroup">
    <h2>Legend</h2>
    <div id="legendbody"></div>
  </div>
</aside>
<div id="compass"><svg width="22" height="34" viewBox="0 0 22 34"><path d="M11 2 L17 20 L11 16 L5 20 Z" fill="currentColor"/><text x="11" y="32" text-anchor="middle" font-size="10" font-weight="700" fill="currentColor" font-family="sans-serif">N</text></svg><div id="scalelabel"></div><div id="scalebar"></div></div>
<footer><span id="provenance"></span><span id="source"></span><span class="right"><span id="budget"></span><span id="credit" hidden></span><span id="logo"><svg class="mark" viewBox="0 0 64 64" aria-hidden="true"><path d="__LOGO__"/></svg><b>openmobisim</b></span></span></footer>
<div id="tip" hidden></div>
<script id="meta" type="application/json">__META__</script>
<script id="blob" type="text/plain">__BLOB__</script>
<script>
__JS__
</script>
</body>
</html>
"""

PAGE_CSS = r""":root { --surface:#fbfcfd; --ink:#0a1626; --ink2:#3d4c5f; --muted:#55657a; --base:#c5cfda; --mark:#04938e; --sans: Inter, "Helvetica Neue", "Segoe UI", system-ui, sans-serif; --mono: "JetBrains Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace; }
* { box-sizing: border-box; }
html, body { margin: 0; height: 100%; background: var(--surface); color: var(--ink); font-family: var(--sans); overflow: hidden; }
canvas#map { position: fixed; inset: 0; width: 100vw; height: 100vh; display: block; cursor: grab; touch-action: none; }
canvas#map:active { cursor: grabbing; }
#error { position: fixed; top: 12px; left: 50%; transform: translateX(-50%); background: #9d0221; color: #fff; padding: 10px 16px; border-radius: 8px; z-index: 20; font: 13px var(--mono); }
.head { position: fixed; top: 8px; left: 10px; max-width: min(760px, calc(100vw - 350px)); padding: 8px 14px 9px; border-radius: 12px; background: color-mix(in srgb, var(--surface) 84%, transparent); backdrop-filter: blur(3px); pointer-events: none; }
.head .mark { width: 26px; height: 26px; vertical-align: middle; margin-right: 10px; fill: var(--mark); }
.head h1 { display: inline; margin: 0; font-size: 24px; font-weight: 700; letter-spacing: -0.01em; vertical-align: middle; }
.head p { margin: 5px 0 0; font-size: 13px; color: var(--ink2); }
.head p#note { font-style: italic; color: var(--muted); font-size: 12px; }
aside#panel { position: fixed; top: 14px; right: 14px; width: 306px; max-height: calc(100vh - 60px); overflow: auto; background: color-mix(in srgb, var(--surface) 94%, transparent); border: 1px solid var(--base); border-radius: 12px; padding: 12px 14px 14px; box-shadow: 0 6px 26px rgba(0,0,0,.18); font-size: 12.5px; backdrop-filter: blur(6px); }
aside h2 { margin: 12px 0 7px; font-size: 10.5px; letter-spacing: .09em; text-transform: uppercase; color: var(--muted); font-weight: 600; }
aside h2:first-child { margin-top: 0; }
button, select, input[type=number] { font: inherit; color: var(--ink); background: transparent; border: 1px solid var(--base); border-radius: 7px; padding: 4px 9px; cursor: pointer; }
button:hover:not(:disabled) { border-color: var(--ink2); }
button:disabled { opacity: .35; cursor: default; }
button.on { background: var(--ink); color: var(--surface); border-color: var(--ink); }
.row { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; margin: 5px 0; }
#levels { display: grid; grid-template-columns: repeat(5, 1fr); gap: 4px; margin-bottom: 6px; }
#levels button { padding: 4px 0; font-size: 10.5px; }
#levelname { font-family: var(--mono); color: var(--ink2); font-size: 11px; }
input[type=range] { width: 100%; accent-color: var(--ink2); }
input[type=number] { width: 62px; }
label { display: inline-flex; align-items: center; gap: 5px; color: var(--ink2); }
.legend .bar { height: 10px; border-radius: 3px; margin: 4px 0 2px; }
.legend .ends { display: flex; justify-content: space-between; font: 11px var(--mono); color: var(--ink2); }
.legend .bands { display: flex; font: 11px var(--mono); color: var(--ink2); }
.legend .bands span { flex: 1; text-align: center; }
.legend .kinds { display: flex; flex-wrap: wrap; gap: 4px 12px; font-size: 11.5px; color: var(--ink2); margin-top: 4px; }
.legend .kinds i { width: 18px; height: 4px; border-radius: 2px; display: inline-block; margin-right: 5px; vertical-align: middle; }
.legend .note { font: 11px var(--mono); color: var(--muted); margin-top: 3px; }
/* The legend, a section per mode (S269) */
#legendbody .lg { margin: 0 0 10px; }
#legendbody h3 { margin: 0 0 3px; font-size: 11.5px; font-weight: 600; color: var(--ink); }
#legendbody .sub { font-size: 11px; color: var(--ink2); }
#legendbody .bar { height: 9px; border-radius: 3px; margin: 4px 0 2px; }
#legendbody .ends { display: flex; justify-content: space-between; font: 10.5px var(--mono); color: var(--ink2); }
#legendbody .bands { display: flex; font: 10.5px var(--mono); color: var(--ink2); }
#legendbody .bands span { flex: 1; text-align: center; }
#legendbody .kinds { display: flex; flex-wrap: wrap; gap: 3px 12px; font-size: 11px; color: var(--ink2); margin-top: 3px; }
#legendbody .kinds i { width: 20px; height: 0; display: inline-block; margin-right: 5px; vertical-align: middle; }
#legendbody .kinds b { display: inline-block; width: 9px; height: 9px; margin-right: 5px; vertical-align: -1px; border: 1px solid var(--ink); }
#legendbody .kinds b.ci { border-radius: 50%; }
#legendbody .note { font-size: 10.5px; color: var(--muted); margin-top: 3px; }
.route { display: flex; align-items: center; gap: 7px; padding: 3px 4px; border-radius: 6px; font-family: var(--mono); font-size: 11.5px; cursor: default; }
.route:hover { background: color-mix(in srgb, var(--base) 45%, transparent); }
.route i { width: 22px; height: 4px; border-radius: 2px; display: inline-block; }
.route span { color: var(--muted); }
.layers { display: grid; grid-template-columns: 1fr 1fr; gap: 2px 10px; } .layers label { margin: 2px 0; }
.route small { display: block; margin-top: 1px; font-size: 10.5px; color: var(--ink); font-weight: 600; }
.route { align-items: flex-start; }
.route i { margin-top: 6px; flex: none; }
#routehint, #linkinfo div { color: var(--ink2); font-size: 12px; }
#linkinfo { border-top: 1px solid var(--base); margin-top: 10px; padding-top: 8px; }
.pairs { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
.pairs button { font-size: 11px; padding: 2px 7px; }
#tip { position: fixed; pointer-events: none; z-index: 10; background: var(--ink); color: var(--surface); padding: 8px 10px; border-radius: 8px; font-size: 12px; min-width: 190px; box-shadow: 0 4px 16px rgba(0,0,0,.3); }
#tip b { display: block; margin-bottom: 3px; font-family: var(--mono); }
#tip div { display: flex; justify-content: space-between; gap: 14px; } #tip span { opacity: .65; }
#compass { position: fixed; left: 12px; bottom: 40px; padding: 8px 10px 7px; border-radius: 10px; background: color-mix(in srgb, var(--surface) 84%, transparent); color: var(--ink); font: 11.5px var(--mono); pointer-events: none; }
#compass svg { display: block; margin: 0 0 6px 2px; }
#scalebar { height: 6px; border: 2px solid var(--ink); border-top: 0; margin-top: 3px; }
footer { position: fixed; left: 0; right: 0; bottom: 0; height: 30px; display: flex; align-items: center; justify-content: space-between; padding: 0 20px; border-top: 1px solid var(--base); background: color-mix(in srgb, var(--surface) 92%, transparent); font: 11px var(--mono); color: var(--muted); }
footer .right { display: flex; align-items: center; gap: 10px; font-family: var(--sans); flex: none; white-space: nowrap; }
footer #provenance { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-right: 12px; }
footer #source { flex: none; white-space: nowrap; margin-right: 16px; }
#credit { color: var(--ink2); font-size: 12px; padding-right: 10px; border-right: 1px solid var(--base); }
#logo { display: inline-flex; align-items: center; gap: 4px; color: var(--ink); font-weight: 700; font-size: 13px; }
#logo .mark { width: 16px; height: 16px; fill: var(--mark); } #logo b { margin-left: 3px; position: relative; top: -0.1em; }  /* the lowercase body centred on the mark, not the line box */
@media (max-width: 760px) { .head { max-width: calc(100vw - 20px); } aside#panel { top: auto; bottom: 40px; max-height: 42vh; width: calc(100vw - 28px); } #compass { display: none; } }
"""

PAGE_JS = r"""(async function () {
"use strict";
const $ = (id) => document.getElementById(id);
const fail = (e) => { document.body.dataset.state = "error"; const b = $("error"); b.hidden = false; b.textContent = "openmobisim map: " + (e && e.message ? e.message : e); console.error(e); };
window.addEventListener("error", (ev) => fail(ev.error || ev.message));
window.addEventListener("unhandledrejection", (ev) => fail(ev.reason));

const META = JSON.parse($("meta").textContent);
const TOK = META.tokens, LOGO = "__LOGO__";

// ---------------------------------------------------------------- data ----
async function inflate(b64) {
  const bin = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  const stream = new Blob([bin]).stream().pipeThrough(new DecompressionStream("deflate"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}
const raw = await inflate($("blob").textContent.trim());
const CTOR = { u8: Uint8Array, u32: Uint32Array, i32: Int32Array, f32: Float32Array };
const A = {};
for (const a of META.arrays) A[a.n] = new CTOR[a.t](raw.buffer, a.o, a.c);

const NL = META.n_links, NV = META.n_verts, NB = META.n_bins;
const VS = A.vs, CLS = A.cls, LEN = A.len, FF = A.ff, CAP = A.cap, SG = A.sg || new Uint8Array(CAP.length), SW = A.sw || new Float32Array(CAP.length);
const SB = A.sb || null;
// The other layers (T-4), each drawn only when its box is ticked.
const LY = META.layers || {};
const LAYER_ROWS = [
  ["road", "Road traffic", "ribbons: width = volume, colour as below"],
  ["transit", "Transit lines", "width = passengers on board, colour = kind of service"],
  ["parking", "Parkings", "■ car, ● bike; size = capacity, colour = busiest share in use"],
  ["bike", "Cyclists", "on the bike network, the whole run"],
  ["walk", "Walkers", "on the walk network, the whole run"],
];
const LAYER_KEY = { road: "r", transit: "t", parking: "p", bike: "b", walk: "w" };
function flowLayer(prefix) {
  const vs = A[prefix + "vs"], dx = A[prefix + "dx"], dy = A[prefix + "dy"], v = A[prefix + "v"];
  const n = vs.length - 1, xs = new Float64Array(dx.length), ys = new Float64Array(dy.length);
  let cx = 0, cy = 0; for (let i = 0; i < dx.length; i++) { cx += dx[i]; cy += dy[i]; xs[i] = cx * 0.1; ys[i] = cy * 0.1; }
  let vmax = 0; for (let i = 0; i < n; i++) vmax = Math.max(vmax, v[i]);
  // Each link's box, to leave out what is off screen and to pick under the pointer (S269).
  const x0 = new Float32Array(n), y0 = new Float32Array(n), x1 = new Float32Array(n), y1 = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    let a = Infinity, b = Infinity, c = -Infinity, d = -Infinity;
    for (let k = vs[i]; k < vs[i + 1]; k++) { if (xs[k] < a) a = xs[k]; if (xs[k] > c) c = xs[k]; if (ys[k] < b) b = ys[k]; if (ys[k] > d) d = ys[k]; }
    x0[i] = a; y0[i] = b; x1[i] = c; y1[i] = d;
  }
  return { n, vs, xs, ys, v, vmax: vmax || 1, x0, y0, x1, y1, len: A[prefix + "l"] || null, inf: A[prefix + "i"] || null };
}
const FLOWS = { bike: LY.bike ? flowLayer("b") : null, walk: LY.walk ? flowLayer("w") : null };
const X = new Float64Array(NV), Y = new Float64Array(NV);
{ let cx = 0, cy = 0; for (let i = 0; i < NV; i++) { cx += A.dx[i]; cy += A.dy[i]; X[i] = cx * 0.1; Y[i] = cy * 0.1; } }

// link bounding boxes and a uniform grid over them
const BX0 = new Float32Array(NL), BY0 = new Float32Array(NL), BX1 = new Float32Array(NL), BY1 = new Float32Array(NL);
let EX0 = Infinity, EY0 = Infinity, EX1 = -Infinity, EY1 = -Infinity;
for (let l = 0; l < NL; l++) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (let v = VS[l]; v < VS[l + 1]; v++) { if (X[v] < x0) x0 = X[v]; if (X[v] > x1) x1 = X[v]; if (Y[v] < y0) y0 = Y[v]; if (Y[v] > y1) y1 = Y[v]; }
  BX0[l] = x0; BY0[l] = y0; BX1[l] = x1; BY1[l] = y1;
  if (x0 < EX0) EX0 = x0; if (y0 < EY0) EY0 = y0; if (x1 > EX1) EX1 = x1; if (y1 > EY1) EY1 = y1;
}
const GN = 256, GCX = Math.max((EX1 - EX0) / GN, 5), GCY = Math.max((EY1 - EY0) / GN, 5);
const GW = Math.min(GN, Math.ceil((EX1 - EX0) / GCX) + 1), GH = Math.min(GN, Math.ceil((EY1 - EY0) / GCY) + 1);
const GSTART = new Uint32Array(GW * GH + 1), GLINK = (() => {
  const cnt = new Uint32Array(GW * GH + 1);
  const cell = (v, lo, c, n) => Math.min(n - 1, Math.max(0, Math.floor((v - lo) / c)));
  for (let l = 0; l < NL; l++) for (let gy = cell(BY0[l], EY0, GCY, GH); gy <= cell(BY1[l], EY0, GCY, GH); gy++) for (let gx = cell(BX0[l], EX0, GCX, GW); gx <= cell(BX1[l], EX0, GCX, GW); gx++) cnt[gy * GW + gx + 1]++;
  for (let i = 1; i < cnt.length; i++) cnt[i] += cnt[i - 1];
  GSTART.set(cnt);
  const fill = cnt.slice(), out = new Uint32Array(cnt[cnt.length - 1]);
  for (let l = 0; l < NL; l++) for (let gy = cell(BY0[l], EY0, GCY, GH); gy <= cell(BY1[l], EY0, GCY, GH); gy++) for (let gx = cell(BX0[l], EX0, GCX, GW); gx <= cell(BX1[l], EX0, GCX, GW); gx++) out[fill[gy * GW + gx]++] = l;
  return out;
})();
const SEEN = new Uint32Array(NL); let FRAME = 0;
function candidates(x0, y0, x1, y1) {
  FRAME++;
  const out = [];
  const cell = (v, lo, c, n) => Math.min(n - 1, Math.max(0, Math.floor((v - lo) / c)));
  for (let gy = cell(y0, EY0, GCY, GH); gy <= cell(y1, EY0, GCY, GH); gy++) for (let gx = cell(x0, EX0, GCX, GW); gx <= cell(x1, EX0, GCX, GW); gx++) {
    const c = gy * GW + gx;
    for (let i = GSTART[c]; i < GSTART[c + 1]; i++) { const l = GLINK[i]; if (SEEN[l] !== FRAME) { SEEN[l] = FRAME; if (BX1[l] >= x0 && BX0[l] <= x1 && BY1[l] >= y0 && BY0[l] <= y1) out.push(l); } }
  }
  return out;
}

// per-bin traffic
const BS = A.bs, RL = A.rl, RP = A.rp, RS = A.rs;
const HAS_TRAFFIC = NB > 0 && RL.length > 0;
const vol = new Float32Array(NL), delay = new Float32Array(NL), meanT = new Float32Array(NL);
let VMAX = 1;
function percentile(values, p) { if (!values.length) return 1; const s = Float32Array.from(values).sort(); return s[Math.min(s.length - 1, Math.floor(p * s.length))]; }
let GLOBAL_VMAX = 1;
if (HAS_TRAFFIC) { const v = []; for (let r = 0; r < RL.length; r++) v.push(RP[r] / META.bin_seconds * 3600); GLOBAL_VMAX = Math.max(1, percentile(v, 0.99)); }
function aggregate(b0, b1) {
  const tot = new Float32Array(NL), sec = new Float32Array(NL);
  for (let b = b0; b < b1; b++) for (let r = BS[b]; r < BS[b + 1]; r++) { tot[RL[r]] += RP[r]; sec[RL[r]] += RS[r]; }
  const span = Math.max(1, b1 - b0) * META.bin_seconds, pos = [];
  for (let l = 0; l < NL; l++) {
    vol[l] = tot[l] / span * 3600;
    if (tot[l] > 0) { meanT[l] = sec[l] / tot[l]; delay[l] = 1 - Math.min((FF[l] + SW[l]) / Math.max(meanT[l], 1e-6), 1); pos.push(vol[l]); } else { meanT[l] = 0; delay[l] = 0; }
  }
  VMAX = (b1 - b0 === 1) ? GLOBAL_VMAX : Math.max(1, percentile(pos, 0.99));
  let hi = pos.length ? percentile(pos, 0.95) : 0; VHIGH = hi;
}
let VHIGH = 0;

// routes
const HAS_ROUTES = !!META.has_routes;
let SS, RST, RLK, RC, RO, RU, ROUTE_KEY, LROUTE_START, LROUTES;
if (HAS_ROUTES) {
  SS = A.ss; RST = A.rst; RLK = A.rlk; RC = A.rc; RO = A.ro; RU = A.ru || null;
  const nk = SS.length - 1, nr = RC.length;
  ROUTE_KEY = new Uint32Array(nr);
  for (let k = 0; k < nk; k++) for (let r = SS[k]; r < SS[k + 1]; r++) ROUTE_KEY[r] = k;
  const cnt = new Uint32Array(NL + 1);
  for (let i = 0; i < RLK.length; i++) cnt[RLK[i] + 1]++;
  for (let i = 1; i <= NL; i++) cnt[i] += cnt[i - 1];
  LROUTE_START = cnt; const fill = cnt.slice(); LROUTES = new Uint32Array(RLK.length);
  for (let r = 0; r < nr; r++) for (let i = RST[r]; i < RST[r + 1]; i++) LROUTES[fill[RLK[i]]++] = r;
}

// ------------------------------------------------------------- colours ----
function hex(h) { return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)]; }
function rampTable(stops, n) {
  const c = stops.map(hex), out = [];
  for (let i = 0; i < n; i++) {
    const t = i / (n - 1) * (c.length - 1), k = Math.min(c.length - 2, Math.floor(t)), f = t - k;
    out.push("rgb(" + [0, 1, 2].map((j) => Math.round(c[k][j] + (c[k + 1][j] - c[k][j]) * f)).join(",") + ")");
  }
  return out;
}
const NCOL = 24;
// The first bin with traffic (S269): the slider and Play start there, not at midnight.
const FIRST = META.first_bin || 0;
// The delay's bands (S268): the upper edges of each, as META gives them, and their labels.
const BANDS = META.delay_bands, BAND_LABELS = META.delay_band_labels;
function band(d) { let b = 0; while (b < BANDS.length && d >= BANDS[b]) b++; return b; }
function bandColours() { const n = BAND_LABELS.length; return BAND_LABELS.map((_, i) => TABLES.delay[Math.round(i / (n - 1) * (NCOL - 1))]); }
const S = { theme: META.theme, colour: "delay", bin: -1, bins: [0, NB], playing: false, pair: -1, link: -1, hover: -1, hoverItem: null, lyOrder: [], hoverRoute: -1, routes: true, nroutes: META.routes_shown || 0, dim: true, spill: false, cx: 0, cy: 0, mpp: 1, ly: { road: true, transit: false, parking: false, bike: false, walk: false } };
let TH = TOK[S.theme], TABLES = {};
function setTheme(name) {
  S.theme = name; TH = TOK[name]; document.documentElement.dataset.theme = name;
  const r = document.documentElement.style;
  r.setProperty("--surface", TH.surface); r.setProperty("--ink", TH.ink); r.setProperty("--ink2", TH.ink2); r.setProperty("--muted", TH.muted); r.setProperty("--base", TH.base); r.setProperty("--mark", TH.mark);
  TABLES = { delay: rampTable(TH.ramp_delay, NCOL), volume: rampTable(TH.ramp_volume, NCOL), vc: rampTable(TH.ramp_delay, NCOL) };
  if (typeof FLOWS !== "undefined") renderLegend();
  $("theme").textContent = name === "paper" ? "Night" : "Paper";
}

// ---------------------------------------------------------------- view ----
const canvas = $("map");
let ctx = canvas.getContext("2d");  // `let`: an export draws the same view into another context
let W = 0, H = 0, DPR = 1;
const FIT = { mpp: 1 };
const LEVELS = ["Overview", "Region", "District", "Street", "Detail"];
let MIN_MPP = 0.25, MAX_MPP = 1;
function resize() {
  DPR = window.devicePixelRatio || 1; W = canvas.clientWidth; H = canvas.clientHeight;
  canvas.width = Math.round(W * DPR); canvas.height = Math.round(H * DPR);
}
// The panel's width over the map: it sits on the right of a wide window.
function panelWidth() { return W > 760 ? 340 : 0; }
function fitTo(x0, y0, x1, y1, pad) {
  // Fit into the part of the window the panel, the title above and the footer below leave free (S269: a tall
  // network ran under both).
  const top = 78, bottom = 34;
  const w = Math.max(x1 - x0, 60), h = Math.max(y1 - y0, 60), reserve = panelWidth();
  S.mpp = Math.max(w / ((W - reserve) * (1 - pad)), h / ((H - top - bottom) * (1 - pad)));
  S.cx = (x0 + x1) / 2 + reserve / 2 * S.mpp; S.cy = (y0 + y1) / 2 + (top - bottom) / 2 * S.mpp;
}
function levelMpp(i) { return Math.max(MIN_MPP, FIT.mpp / Math.pow(3.2, i)); }
function currentLevel() { let best = 0, bd = 1e9; for (let i = 0; i < LEVELS.length; i++) { const d = Math.abs(Math.log(S.mpp / levelMpp(i))); if (d < bd) { bd = d; best = i; } } return best; }
// Which road classes show at this zoom; a small network (a benchmark) shows all of them at every zoom (S269: Sioux
// Falls's 76 links, all unclassified, showed nothing at an overview).
function classLimit() { return NL < 5000 ? 16 : S.mpp > 25 ? 5 : S.mpp > 8 ? 9 : S.mpp > 2.5 ? 13 : 16; }
const sx = (x) => (x - S.cx) / S.mpp + W / 2, sy = (y) => H / 2 - (y - S.cy) / S.mpp;

// ---------------------------------------------------------------- draw ----
const WFLOOR = 0.9;
function ribbonWidth(l) {
  const zs = Math.min(2.2, Math.max(0.2, 1.15 * Math.sqrt(2600 / (W * S.mpp))));
  const f = Math.sqrt(Math.min(vol[l], VMAX) / VMAX);
  return Math.max((0.5 + 5.2 * f) * zs, WFLOOR);
}
function colourIndex(l) {
  let t;
  if (S.colour === "delay") t = band(delay[l]) / (BANDS.length);
  else if (S.colour === "vc") t = CAP[l] > 0 ? vol[l] / CAP[l] : 0;
  else t = Math.sqrt(Math.min(vol[l], VMAX) / VMAX);
  return Math.max(0, Math.min(NCOL - 1, Math.round(t * (NCOL - 1))));
}
// Screen-space polyline of link l, offset to the driver's right by `off` pixels, thinned to ~`gap` px.
let PX = new Float32Array(4096), PY = new Float32Array(4096);
function ensure(n) { if (PX.length < n) { PX = new Float32Array(n * 2); PY = new Float32Array(n * 2); } }
// Move the n points in PX/PY to the driver's right by `off` pixels, by vertex normals (screen: right of (dx,dy) is (-dy,dx)), averaged.
function offsetLine(n, off) {
  if (off === 0 || n < 2) return;
  const nx = new Float32Array(n), ny = new Float32Array(n), fx = new Float32Array(n), fy = new Float32Array(n);
  for (let i = 0; i < n - 1; i++) {
    const dx = PX[i + 1] - PX[i], dy = PY[i + 1] - PY[i], m = Math.hypot(dx, dy);
    if (m < 1e-6) continue;
    const ux = -dy / m, uy = dx / m;
    nx[i] += ux; ny[i] += uy; nx[i + 1] += ux; ny[i + 1] += uy;
    if (fx[i] === 0 && fy[i] === 0) { fx[i] = ux; fy[i] = uy; }
    fx[i + 1] = ux; fy[i + 1] = uy;
  }
  for (let i = 0; i < n; i++) {
    let m = Math.hypot(nx[i], ny[i]), ax = nx[i], ay = ny[i];
    if (m < 0.35) { ax = fx[i]; ay = fy[i]; m = Math.hypot(ax, ay) || 1; }   // a U-turn cancels the normals: keep the last segment's
    PX[i] += ax / m * off; PY[i] += ay / m * off;
  }
}
// Screen-space polyline of link l, offset to the driver's right by `off` pixels, thinned to ~`gap` px.
function screenLine(l, off, gap) {
  const v0 = VS[l], v1 = VS[l + 1]; ensure(v1 - v0 + 2);
  let n = 0, lx = 0, ly = 0;
  for (let v = v0; v < v1; v++) {
    const x = sx(X[v]), y = sy(Y[v]);
    if (n === 0 || v === v1 - 1 || Math.abs(x - lx) + Math.abs(y - ly) >= gap) { PX[n] = x; PY[n] = y; n++; lx = x; ly = y; }
  }
  offsetLine(n, off);
  return n;
}
// One route as a single polyline, so a turn is offset like a bend and never leaves a gap or a spike.
function routeLine(r, off, gap) {
  let total = 0; for (let i = RST[r]; i < RST[r + 1]; i++) total += VS[RLK[i] + 1] - VS[RLK[i]];
  ensure(total + 2);
  let n = 0, lx = 0, ly = 0, wx = NaN, wy = NaN;
  for (let i = RST[r]; i < RST[r + 1]; i++) {
    const l = RLK[i], v0 = VS[l], v1 = VS[l + 1];
    for (let v = v0; v < v1; v++) {
      if (v === v0 && X[v] === wx && Y[v] === wy) continue;   // the node this link shares with the last one
      const x = sx(X[v]), y = sy(Y[v]);
      if (n === 0 || v === v0 || v === v1 - 1 || Math.abs(x - lx) + Math.abs(y - ly) >= gap) { PX[n] = x; PY[n] = y; n++; lx = x; ly = y; }
      wx = X[v]; wy = Y[v];
    }
  }
  offsetLine(n, off);
  return n;
}
function addLine(path, l, off, gap) {
  const n = screenLine(l, off, gap);
  if (n < 2) return 0;
  path.moveTo(PX[0], PY[0]); for (let i = 1; i < n; i++) path.lineTo(PX[i], PY[i]);
  return n;
}
let DRAWN = 0, MOVING = false;
function draw() {
  DRAWN = 0; const t0 = performance.now();
  ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
  ctx.fillStyle = TH.surface; ctx.fillRect(0, 0, W, H);
  const hw = W / 2 * S.mpp, hh = H / 2 * S.mpp, pad = 8 * S.mpp;
  const cand = candidates(S.cx - hw - pad, S.cy - hh - pad, S.cx + hw + pad, S.cy + hh + pad);
  const lim = classLimit() - (MOVING ? 3 : 0), gap = MOVING ? 3 : 1.2;
  const showAll = classLimit() >= 16;
  const base = new Path2D(), faint = new Path2D(), buckets = new Map();
  const dimming = S.pair >= 0 && S.routes && S.dim;
  for (const l of cand) {
    const busy = HAS_TRAFFIC && vol[l] > 0 && (vol[l] >= VHIGH * (S.mpp > 8 ? 1 : 0.4) || CLS[l] <= lim);
    const drivable = CLS[l] < 14;
    // A link this zoom leaves out is drawn faint, not left out (S269), but while moving.
    if ((CLS[l] > lim && !busy && !(showAll && S.mpp < 2.5)) || (!drivable && S.mpp > 2.5)) {
      if (!MOVING) addLine(faint, l, 0, gap);
      continue;
    }
    addLine(base, l, 0, gap); DRAWN++;
    if (S.ly.road && HAS_TRAFFIC && vol[l] > 0 && CLS[l] <= Math.max(lim, 16) && (CLS[l] <= lim || busy)) {
      const w = ribbonWidth(l), wi = Math.min(31, Math.round(w * 2)), key = colourIndex(l) * 32 + wi;
      let b = buckets.get(key); if (!b) { b = new Path2D(); buckets.set(key, b); }
      addLine(b, l, w / 2 + 0.25, gap);
    }
  }
  ctx.lineCap = "round"; ctx.lineJoin = "round";
  ctx.globalAlpha = 0.55; ctx.strokeStyle = TH.base; ctx.lineWidth = 0.4; ctx.stroke(faint); ctx.globalAlpha = 1;
  ctx.strokeStyle = TH.base; ctx.lineWidth = S.mpp > 6 ? 0.5 : 0.9; ctx.stroke(base);
  const keys = [...buckets.keys()].sort((a, b) => a - b), tbl = TABLES[S.colour];
  ctx.globalAlpha = dimming ? 0.35 : 1;
  if (TH.glow) { for (const k of keys) { ctx.strokeStyle = tbl[k >> 5]; ctx.globalAlpha = (dimming ? 0.35 : 1) * 0.14; ctx.lineWidth = (k & 31) / 2 * 2.8 + 1.2; ctx.stroke(buckets.get(k)); } ctx.globalAlpha = dimming ? 0.35 : 1; }
  else if (!MOVING) { for (const k of keys) { ctx.strokeStyle = TH.surface; ctx.lineWidth = (k & 31) / 2 + 1.4; ctx.stroke(buckets.get(k)); } }
  for (const k of keys) { ctx.strokeStyle = tbl[k >> 5]; ctx.lineWidth = (k & 31) / 2; ctx.stroke(buckets.get(k)); }
  ctx.globalAlpha = 1;
  if (!MOVING && S.mpp < 3 && S.ly.road) drawChevrons(cand, lim);
  if (S.ly.bike && FLOWS.bike) drawFlow(FLOWS.bike, TH.modes[1], [9, 5]);
  if (S.ly.walk && FLOWS.walk) drawFlow(FLOWS.walk, TH.modes[2], [2, 4]);
  if (S.ly.transit && LY.transit) drawTransit();
  if (S.ly.parking && LY.parking) drawParking();
  if (S.pair >= 0 && S.routes && HAS_ROUTES) drawRoutes();
  if (S.spill && SB) drawSpill(cand);
  highlight(S.link, TH.ink, 3); if (S.hover !== S.link) highlight(S.hover, TH.ink2, 2);
  highlightItem(S.hoverItem);
  drawScale();
  document.body.dataset.drawms = (performance.now() - t0).toFixed(1); document.body.dataset.drawn = DRAWN; document.body.dataset.level = LEVELS[currentLevel()];
}
function flowLine(p, f, i) {
  p.moveTo(sx(f.xs[f.vs[i]]), sy(f.ys[f.vs[i]]));
  for (let v = f.vs[i] + 1; v < f.vs[i + 1]; v++) p.lineTo(sx(f.xs[v]), sy(f.ys[v]));
}
function drawFlow(f, colour, dash) {
  const hw = W / 2 * S.mpp, hh = H / 2 * S.mpp, vx0 = S.cx - hw, vx1 = S.cx + hw, vy0 = S.cy - hh, vy1 = S.cy + hh;
  const faint = new Path2D(), buckets = new Map();
  for (let i = 0; i < f.n; i++) {
    if (f.x1[i] < vx0 || f.x0[i] > vx1 || f.y1[i] < vy0 || f.y0[i] > vy1) continue;
    if (!(f.v[i] > 0)) { if (!MOVING) flowLine(faint, f, i); continue; }  // the layer's network, faint (S269)
    const w = Math.min(15.5, 1 + 7 * Math.sqrt(f.v[i] / f.vmax)), key = Math.round(w * 2);
    let p = buckets.get(key); if (!p) { p = new Path2D(); buckets.set(key, p); }
    flowLine(p, f, i);
  }
  ctx.globalAlpha = 0.45; ctx.strokeStyle = colour; ctx.lineWidth = 0.6; ctx.stroke(faint); ctx.globalAlpha = 1;
  const keys = [...buckets.keys()].sort((a, b) => a - b);
  for (const k of keys) { ctx.strokeStyle = TH.surface; ctx.lineWidth = k / 2 + 1.4; ctx.stroke(buckets.get(k)); }
  // Over road traffic, dashed (cyclists) or dotted (walkers): another mode, not another road colour.
  ctx.setLineDash(S.ly.road && HAS_TRAFFIC ? dash : []);
  for (const k of keys) { ctx.strokeStyle = colour; ctx.lineWidth = k / 2; ctx.stroke(buckets.get(k)); }
  ctx.setLineDash([]);
}
function drawTransit() {
  const sxs = A.tsx, sys = A.tsy, n = A.tl.length, show = LY.transit.shown;
  let top = 0; for (let i = 0; i < n; i++) top = Math.max(top, A.tl[i]); top = top || 1;
  for (let i = 0; i < n; i++) {  // busiest last, on top
    const a = A.ta[i], b = A.tb[i], w = 1.2 + 6 * Math.sqrt(A.tl[i] / top);
    const x0 = sx(sxs[a] * 0.1), y0 = sy(sys[a] * 0.1), x1 = sx(sxs[b] * 0.1), y1 = sy(sys[b] * 0.1);
    const d = Math.hypot(x1 - x0, y1 - y0) || 1, ox = -(y1 - y0) / d * (w / 2 + 0.5), oy = (x1 - x0) / d * (w / 2 + 0.5);
    const p = new Path2D(); p.moveTo(x0 + ox, y0 + oy); p.lineTo(x1 + ox, y1 + oy);
    ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 1.4; ctx.stroke(p);
    ctx.strokeStyle = TH.kinds[A.tk[i]]; ctx.lineWidth = w; ctx.stroke(p);
  }
  let most = 0; for (let i = 0; i < A.tsb.length; i++) most = Math.max(most, A.tsb[i]); most = most || 1;
  const names = LY.transit.stops, label = names.length <= 40 || S.mpp < 3;
  ctx.font = "10.5px " + FONT; ctx.textAlign = "left"; ctx.textBaseline = "alphabetic";
  for (let i = 0; i < A.tsx.length; i++) {
    if (show && !show[i]) continue;
    const x = sx(sxs[i] * 0.1), y = sy(sys[i] * 0.1);
    if (x < -20 || y < -20 || x > W + 20 || y > H + 20) continue;
    const r = 2 + 4 * Math.sqrt(A.tsb[i] / most);
    ctx.beginPath(); ctx.arc(x, y, r, 0, 6.2832); ctx.fillStyle = TH.surface; ctx.fill(); ctx.strokeStyle = TH.ink2; ctx.lineWidth = 1; ctx.stroke();
    if (label) { ctx.lineWidth = 3; ctx.strokeStyle = TH.surface; ctx.strokeText(names[i], x + r + 3, y - r - 1); ctx.fillStyle = TH.ink2; ctx.fillText(names[i], x + r + 3, y - r - 1); }
  }
}
function drawParking() {
  let most = 0; for (let i = 0; i < A.pc.length; i++) most = Math.max(most, A.pc[i]); most = most || 1;
  const tbl = TABLES.delay;
  for (let i = 0; i < A.pc.length; i++) {
    const x = sx(A.px[i] * 0.1), y = sy(A.py[i] * 0.1), r = 3 + 6 * Math.sqrt(A.pc[i] / most);
    const fill = tbl[Math.min(tbl.length - 1, Math.floor(Math.min(1, A.po[i]) * tbl.length))];
    ctx.beginPath(); if (A.pv[i]) ctx.arc(x + r * 0.9, y, r * 0.8, 0, 6.2832); else { ctx.moveTo(x - r, y - r); ctx.lineTo(x + r, y - r); ctx.lineTo(x + r, y + r); ctx.lineTo(x - r, y + r); ctx.closePath(); }
    ctx.fillStyle = fill; ctx.fill(); ctx.strokeStyle = TH.ink; ctx.lineWidth = 1; ctx.stroke();
  }
}
function layerRows() {
  const present = { road: HAS_TRAFFIC, transit: !!LY.transit, parking: !!LY.parking, bike: !!FLOWS.bike, walk: !!FLOWS.walk };
  const rows = LAYER_ROWS.filter(([k]) => present[k]);
  $("layergroup").hidden = rows.length < 2;  // one layer alone needs no switch
  $("layerrows").innerHTML = rows.map(([k, name, hint]) => `<label title="${hint}"><input type="checkbox" data-layer="${k}"${S.ly[k] ? " checked" : ""}> ${name}</label>`).join("");
  document.querySelectorAll("#layerrows input").forEach((box) => box.addEventListener("change", (e) => {
    const k = e.target.dataset.layer; S.ly[k] = e.target.checked;
    // The pointer picks in the layer ticked last that is still on (S269).
    S.lyOrder = S.lyOrder.filter((x) => x !== k); if (e.target.checked) S.lyOrder.push(k);
    S.hover = -1; S.hoverItem = null; redraw(false); writeHash();
  }));
}
function drawChevrons(cand, lim) {
  ctx.fillStyle = TH.surface; ctx.globalAlpha = 0.9;
  for (const l of cand) {
    if (!(vol[l] > 0) || CLS[l] > lim) continue;
    const n = screenLine(l, ribbonWidth(l) / 2 + 0.25, 1.2); if (n < 2) continue;
    let len = 0; for (let i = 0; i < n - 1; i++) len += Math.hypot(PX[i + 1] - PX[i], PY[i + 1] - PY[i]); if (len < 26) continue;
    const m = (n - 1) >> 1, dx = PX[m + 1] - PX[m], dy = PY[m + 1] - PY[m], d = Math.hypot(dx, dy) || 1, tx = dx / d, ty = dy / d;
    const cx = (PX[m] + PX[m + 1]) / 2, cy = (PY[m] + PY[m + 1]) / 2, s = ribbonWidth(l) * 0.42 + 0.9;
    ctx.beginPath(); ctx.moveTo(cx + tx * s, cy + ty * s); ctx.lineTo(cx - tx * s * 0.6 - ty * s * 0.75, cy - ty * s * 0.6 + tx * s * 0.75); ctx.lineTo(cx - tx * s * 0.6 + ty * s * 0.75, cy - ty * s * 0.6 - tx * s * 0.75); ctx.closePath(); ctx.fill();
  }
  ctx.globalAlpha = 1;
}
function drawSpill(cand) {
  const p = new Path2D(); let any = false;
  for (const l of cand) if (SB[l] && addLine(p, l, 0, 0.5)) any = true;
  if (!any) return;
  ctx.setLineDash([5, 4]); ctx.strokeStyle = TH.surface; ctx.lineWidth = 5; ctx.stroke(p);
  ctx.strokeStyle = TH.ink; ctx.lineWidth = 2.5; ctx.stroke(p); ctx.setLineDash([]);
}
function highlight(l, colour, w) {
  if (l < 0) return;
  const p = new Path2D(); if (!addLine(p, l, 0, 0.5)) return;
  ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 3; ctx.stroke(p); ctx.strokeStyle = colour; ctx.lineWidth = w; ctx.stroke(p);
}
function routeColour(r) { const p = TOK.route[S.theme]; return p[r % p.length]; }
function drawRoutes() {
  const k = S.pair, r0 = SS[k], r1 = r0 + shownRoutes(k), step = 3.8;
  for (let r = r0; r < r1; r++) {
    const own = r - r0, path = new Path2D(), off = (own + 0.5) * step;
    const n = routeLine(r, off, 1.2);
    if (n >= 2) { path.moveTo(PX[0], PY[0]); for (let j = 1; j < n; j++) path.lineTo(PX[j], PY[j]); }
    const hl = S.hoverRoute === own, dim = S.hoverRoute >= 0 && !hl;
    ctx.globalAlpha = dim ? 0.3 : 1;
    ctx.strokeStyle = TH.surface; ctx.lineWidth = (hl ? 6 : 4.6); ctx.stroke(path);
    ctx.strokeStyle = routeColour(own); ctx.lineWidth = hl ? 4.4 : 3; ctx.stroke(path);
  }
  ctx.globalAlpha = 1;
  const first = RLK[RST[r0]], last = RLK[RST[r0 + 1] - 1];
  for (const [l, end, lab] of [[first, 0, "A"], [last, 1, "B"]]) {
    const v = end ? VS[l + 1] - 1 : VS[l], x = sx(X[v]), y = sy(Y[v]);
    ctx.beginPath(); ctx.arc(x, y, 8, 0, 6.2832); ctx.fillStyle = TH.ink; ctx.fill(); ctx.strokeStyle = TH.surface; ctx.lineWidth = 1.5; ctx.stroke();
    ctx.fillStyle = TH.surface; ctx.font = "bold 10px " + FONT; ctx.textAlign = "center"; ctx.textBaseline = "middle"; ctx.fillText(lab, x, y + 0.5);
  }
}
const FONT = getComputedStyle(document.body).fontFamily;
function niceNumber(x) { const p = Math.pow(10, Math.floor(Math.log10(x))); for (const m of [5, 2, 1]) if (m * p <= x) return m * p; return p; }
function drawScale() {
  const m = niceNumber(W * S.mpp * 0.11), px = m / S.mpp;
  $("scalebar").style.width = px + "px"; $("scalelabel").textContent = m >= 1000 ? (m / 1000) + " km" : m + " m";
}

// ---------------------------------------------------------- interaction ----
function distToLink(l, mx, my) {
  let best = Infinity;
  for (let v = VS[l]; v < VS[l + 1] - 1; v++) {
    const ax = X[v], ay = Y[v], bx = X[v + 1], by = Y[v + 1], dx = bx - ax, dy = by - ay, L = dx * dx + dy * dy;
    let t = L > 0 ? ((mx - ax) * dx + (my - ay) * dy) / L : 0; t = Math.max(0, Math.min(1, t));
    const d = Math.hypot(mx - (ax + t * dx), my - (ay + t * dy)); if (d < best) best = d;
  }
  return best;
}
function pick(px, py) {
  const mx = S.cx + (px - W / 2) * S.mpp, my = S.cy - (py - H / 2) * S.mpp, tol = 7 * S.mpp;
  let best = -1, bd = tol; const lim = classLimit();
  for (const l of candidates(mx - tol, my - tol, mx + tol, my + tol)) {
    if (CLS[l] > lim && !(vol[l] > 0)) continue; if (CLS[l] >= 14 && S.mpp > 2.5) continue;
    const d = distToLink(l, mx, my); if (d < bd) { bd = d; best = l; }
  }
  return best;
}
// The layer the pointer picks in: the one ticked last that is still on and has something to pick (S269).
function hoverLayer() {
  const has = { road: true, bike: !!FLOWS.bike, walk: !!FLOWS.walk, transit: !!LY.transit, parking: !!LY.parking };
  for (let i = S.lyOrder.length - 1; i >= 0; i--) { const k = S.lyOrder[i]; if (S.ly[k] && has[k]) return k; }
  return "road";
}
function segDist(px, py, ax, ay, bx, by) {
  const dx = bx - ax, dy = by - ay, t = Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy || 1)));
  return Math.hypot(px - ax - t * dx, py - ay - t * dy);
}
function pickItem(px, py) {
  const kind = hoverLayer(), mx = S.cx + (px - W / 2) * S.mpp, my = S.cy - (py - H / 2) * S.mpp, tol = 7 * S.mpp;
  if (kind === "road") { const l = pick(px, py); return l >= 0 ? { kind, id: l } : null; }
  let best = -1, bd = tol;
  if (kind === "bike" || kind === "walk") {
    const f = FLOWS[kind];
    for (let i = 0; i < f.n; i++) {
      if (f.x1[i] < mx - tol || f.x0[i] > mx + tol || f.y1[i] < my - tol || f.y0[i] > my + tol) continue;
      for (let v = f.vs[i]; v + 1 < f.vs[i + 1]; v++) { const d = segDist(mx, my, f.xs[v], f.ys[v], f.xs[v + 1], f.ys[v + 1]); if (d < bd) { bd = d; best = i; } }
    }
    return best >= 0 ? { kind, id: best } : null;
  }
  if (kind === "transit") {
    const show = LY.transit.shown;
    for (let i = 0; i < A.tsx.length; i++) {  // a stop under the pointer first
      if (show && !show[i]) continue;
      const d = Math.hypot(A.tsx[i] * 0.1 - mx, A.tsy[i] * 0.1 - my); if (d < bd) { bd = d; best = i; }
    }
    if (best >= 0) return { kind, stop: true, id: best };
    for (let i = 0; i < A.ta.length; i++) {
      const a = A.ta[i], b = A.tb[i], d = segDist(mx, my, A.tsx[a] * 0.1, A.tsy[a] * 0.1, A.tsx[b] * 0.1, A.tsy[b] * 0.1);
      if (d < bd) { bd = d; best = i; }
    }
    return best >= 0 ? { kind, stop: false, id: best } : null;
  }
  for (let i = 0; i < A.px.length; i++) { const d = Math.hypot(A.px[i] * 0.1 - mx, A.py[i] * 0.1 - my); if (d < bd + 4 * S.mpp) { bd = d; best = i; } }
  return best >= 0 ? { kind: "parking", id: best } : null;
}
function sameItem(a, b) { return a === b || (!!a && !!b && a.kind === b.kind && a.id === b.id && !!a.stop === !!b.stop); }
const INFRA = ["mixed traffic", "painted lane or cycle street", "separated track"];
function tipItem(it, ev) {
  if (!it || it.kind === "road") { tip(it ? it.id : -1, ev); return; }
  const t = $("tip"), i = it.id; let head, rows = [];
  if (it.kind === "bike" || it.kind === "walk") {
    const f = FLOWS[it.kind];
    head = (it.kind === "bike" ? "bike" : "walk") + " link";
    if (f.inf) rows.push(["Infrastructure", INFRA[f.inf[i]] || "—"]);
    if (f.len) rows.push(["Length", fmt(f.len[i]) + " m"]);
    rows.push([it.kind === "bike" ? "Cyclists" : "Walkers", fmt(f.v[i]) + " over the run"]);
  } else if (it.kind === "transit" && it.stop) {
    head = "stop " + LY.transit.stops[i];
    rows.push(["Boardings", fmt(A.tsb[i]) + " over the run"]);
  } else if (it.kind === "transit") {
    const a = A.ta[i], b = A.tb[i];
    head = LY.transit.kinds[A.tk[i]] + " between two stops";
    rows.push(["From", LY.transit.stops[a]]); rows.push(["To", LY.transit.stops[b]]);
    rows.push(["Passengers", fmt(A.tl[i]) + " on board, over the day's runs"]);
  } else {
    head = (A.pv[i] ? "bike" : "car") + " parking " + LY.parking.ids[i];
    rows.push(["Capacity", fmt(A.pc[i]) + (A.pv[i] ? " bikes" : " cars")]);
    rows.push(["Busiest", fmt(A.po[i] * 100) + "% in use"]);
  }
  t.innerHTML = "<b>" + head + "</b>" + rows.map((r) => "<div><span>" + r[0] + "</span>" + r[1] + "</div>").join("");
  t.hidden = false; t.style.left = Math.min(W - 250, ev.clientX + 14) + "px"; t.style.top = Math.min(H - 150, ev.clientY + 14) + "px";
}
// The hovered element of a layer other than the road, outlined (S269).
function highlightItem(it) {
  if (!it || it.kind === "road") return;
  const p = new Path2D(); let ring = null;
  if (it.kind === "bike" || it.kind === "walk") flowLine(p, FLOWS[it.kind], it.id);
  else if (it.kind === "transit" && !it.stop) { const a = A.ta[it.id], b = A.tb[it.id]; p.moveTo(sx(A.tsx[a] * 0.1), sy(A.tsy[a] * 0.1)); p.lineTo(sx(A.tsx[b] * 0.1), sy(A.tsy[b] * 0.1)); }
  else if (it.kind === "transit") ring = [sx(A.tsx[it.id] * 0.1), sy(A.tsy[it.id] * 0.1)];
  else ring = [sx(A.px[it.id] * 0.1), sy(A.py[it.id] * 0.1)];
  if (ring) { ctx.beginPath(); ctx.arc(ring[0], ring[1], 9, 0, 6.2832); ctx.strokeStyle = TH.ink; ctx.lineWidth = 2; ctx.stroke(); return; }
  ctx.strokeStyle = TH.surface; ctx.lineWidth = 5; ctx.stroke(p); ctx.strokeStyle = TH.ink; ctx.lineWidth = 2.2; ctx.stroke(p);
}
const CLASSES = META.class_names;
function fmt(x, d) { return Number(x).toLocaleString(undefined, { maximumFractionDigits: d === undefined ? 0 : d }); }
function tip(l, ev) {
  const t = $("tip");
  if (l < 0) { t.hidden = true; return; }
  const rows = [["Class", CLASSES[CLS[l]] + " · " + LEN[l].toFixed(0) + " m"]];
  if (HAS_TRAFFIC) { rows.push(["Volume", fmt(vol[l]) + " PCU/h"]); if (vol[l] > 0) { rows.push(["Delay", fmt(delay[l] * 100) + "% over free flow"]); rows.push(["Mean time", meanT[l].toFixed(1) + " s (free flow " + (FF[l] + SW[l]).toFixed(1) + " s" + (SW[l] > 0 ? ", the signal's mean wait of " + SW[l].toFixed(1) + " s included" : "") + ")"]); } }
  if (CAP[l] > 0) rows.push(["Capacity", fmt(CAP[l]) + " PCU/h" + (SG[l] ? " at its signal" : "") + (vol[l] > 0 ? " (" + fmt(vol[l] / CAP[l] * 100) + "% used)" : "")]);
  if (HAS_ROUTES) { const n = LROUTE_START[l + 1] - LROUTE_START[l]; if (n) rows.push(["Routes", n + " use it"]); }
  t.innerHTML = "<b>link " + l + "</b>" + rows.map((r) => "<div><span>" + r[0] + "</span>" + r[1] + "</div>").join("");
  t.hidden = false; t.style.left = Math.min(W - 250, ev.clientX + 14) + "px"; t.style.top = Math.min(H - 150, ev.clientY + 14) + "px";
}
let raf = 0, idle = 0;
function redraw(moving) {
  MOVING = !!moving; if (raf) return;
  raf = requestAnimationFrame(() => { raf = 0; draw(); });
  clearTimeout(idle); if (moving) idle = setTimeout(() => { MOVING = false; draw(); writeHash(); }, 160);
}
let drag = null;
canvas.addEventListener("pointerdown", (e) => { drag = { x: e.clientX, y: e.clientY, cx: S.cx, cy: S.cy, moved: false }; canvas.setPointerCapture(e.pointerId); });
canvas.addEventListener("pointermove", (e) => {
  if (drag) { const dx = e.clientX - drag.x, dy = e.clientY - drag.y; if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true; S.cx = drag.cx - dx * S.mpp; S.cy = drag.cy + dy * S.mpp; redraw(true); return; }
  const it = pickItem(e.offsetX, e.offsetY), l = it && it.kind === "road" ? it.id : -1;
  if (l !== S.hover || !sameItem(it, S.hoverItem)) { S.hover = l; S.hoverItem = it; redraw(false); }
  tipItem(it, e);
});
canvas.addEventListener("pointerup", (e) => { if (drag && !drag.moved && hoverLayer() === "road") selectLink(pick(e.offsetX, e.offsetY)); drag = null; writeHash(); });
canvas.addEventListener("pointerleave", () => { S.hover = -1; S.hoverItem = null; $("tip").hidden = true; redraw(false); });
canvas.addEventListener("wheel", (e) => { e.preventDefault(); zoomAt(e.offsetX, e.offsetY, Math.exp(e.deltaY * 0.0015)); }, { passive: false });
canvas.addEventListener("dblclick", (e) => zoomAt(e.offsetX, e.offsetY, 0.4));
function zoomAt(px, py, factor) {
  const mx = S.cx + (px - W / 2) * S.mpp, my = S.cy - (py - H / 2) * S.mpp;
  S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, S.mpp * factor));
  S.cx = mx - (px - W / 2) * S.mpp; S.cy = my + (py - H / 2) * S.mpp; redraw(true); levelButtons();
}
function goLevel(i) { S.mpp = levelMpp(i); redraw(false); levelButtons(); writeHash(); }
function levelButtons() { const cur = currentLevel(); document.querySelectorAll("#levels button").forEach((b, i) => { b.classList.toggle("on", i === cur); b.disabled = i > 0 && levelMpp(i) <= MIN_MPP && levelMpp(i - 1) <= MIN_MPP; }); $("levelname").textContent = LEVELS[cur]; }

// ---------------------------------------------------------- selection ----
function pairRoutes(k) { return SS[k + 1] - SS[k]; }
// The routes shown of pair k: its first S.nroutes (the most chosen; the page holds them in that order, S269), or all.
function shownRoutes(k) { const n = pairRoutes(k); return S.nroutes > 0 ? Math.min(n, S.nroutes) : n; }
function selectPair(k, fit) {
  if (!HAS_ROUTES) return;
  S.pair = k; S.hoverRoute = -1;
  if (k >= 0 && fit !== false) {
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (let r = SS[k]; r < SS[k] + shownRoutes(k); r++) for (let i = RST[r]; i < RST[r + 1]; i++) { const l = RLK[i]; if (BX0[l] < x0) x0 = BX0[l]; if (BY0[l] < y0) y0 = BY0[l]; if (BX1[l] > x1) x1 = BX1[l]; if (BY1[l] > y1) y1 = BY1[l]; }
    fitTo(x0, y0, x1, y1, 0.3); S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, S.mpp));
  }
  $("pairno").value = k >= 0 ? k + 1 : ""; routePanel(); redraw(false); levelButtons(); writeHash();
}
function routePanel() {
  const box = $("routelist"); box.innerHTML = "";
  if (S.pair < 0) { $("routehint").hidden = false; return; }
  $("routehint").hidden = true;
  const k = S.pair, r0 = SS[k], all = pairRoutes(k), n = shownRoutes(k);
  if (all === 0) { box.textContent = "No route between this pair."; return; }
  let best = Infinity, used = 0;
  for (let i = 0; i < all; i++) { best = Math.min(best, RC[r0 + i]); if (RU) used += RU[r0 + i]; }
  for (let i = 0; i < n; i++) {
    const r = r0 + i, row = document.createElement("div"); row.className = "route";
    const u = RU ? RU[r] : 0;
    row.innerHTML = '<i style="background:' + routeColour(i) + '"></i><div><b>' + (i + 1) + "</b> " + (RC[r] / 60).toFixed(1) + " min <span>+" + Math.round((RC[r] / best - 1) * 100) + "% · overlap " + Math.round(RO[r] * 100) + "%</span>" +
      (RU ? "<small>" + fmt(u) + (u === 1 ? " traveller" : " travellers") + (used > 0 ? " · " + Math.round(u / used * 100) + "%" : "") + "</small>" : "") + "</div>";
    row.addEventListener("pointerenter", () => { S.hoverRoute = i; redraw(false); }); row.addEventListener("pointerleave", () => { S.hoverRoute = -1; redraw(false); });
    box.appendChild(row);
  }
  if (n < all) { const more = document.createElement("small"); more.className = "note"; more.textContent = (all - n) + " more, less chosen: Routes → All"; box.appendChild(more); }
}
function selectLink(l) {
  S.link = l; const box = $("linkinfo");
  if (l < 0) { box.hidden = true; redraw(false); writeHash(); return; }
  box.hidden = false;
  let h = "<b>link " + l + "</b> · " + CLASSES[CLS[l]] + " · " + LEN[l].toFixed(0) + " m";
  if (HAS_ROUTES) {
    const rs = LROUTES.subarray(LROUTE_START[l], LROUTE_START[l + 1]), byPair = new Map();
    for (const r of rs) byPair.set(ROUTE_KEY[r], (byPair.get(ROUTE_KEY[r]) || 0) + 1);
    h += "<div>" + rs.length + " route" + (rs.length === 1 ? "" : "s") + " of " + byPair.size + " pair" + (byPair.size === 1 ? "" : "s") + " use it</div>";
    box.innerHTML = h;
    const list = document.createElement("div"); list.className = "pairs";
    [...byPair.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0]).slice(0, 12).forEach(([k, c]) => {
      const b = document.createElement("button"); b.textContent = "pair " + (k + 1) + " · " + c + " of " + pairRoutes(k); b.addEventListener("click", () => selectPair(k, true)); list.appendChild(b);
    });
    box.appendChild(list);
  } else box.innerHTML = h;
  redraw(false); writeHash();
}

// ------------------------------------------------------------ controls ----
// A bin's time of day, "08:15" (S269: clock times, not minutes since midnight).
function clock(seconds) { const m = Math.round(seconds / 60), h = Math.floor(m / 60); return String(h).padStart(2, "0") + ":" + String(m % 60).padStart(2, "0"); }
function binLabel() {
  if (!HAS_TRAFFIC) return "no traffic recorded";
  const [a, b] = S.bins, s = META.bin_seconds;
  return b - a === NB && NB > 1 ? "all bins, " + clock((META.first_bin || 0) * s) + "–" + clock(b * s) : clock(a * s) + "–" + clock(b * s);
}
function setBins(a, b) { S.bins = [a, b]; aggregate(a, b); subtitle(); redraw(false); writeHash(); }
const COLOUR_WORDS = { delay: "delay over free flow", volume: "volume", vc: "volume over capacity" };
function subtitle() {
  $("subtitle").textContent = binLabel() + " · one ribbon per direction · width = volume, colour = " + COLOUR_WORDS[S.colour];
  renderLegend();
}
// The kinds of transit service the run has, by name and index.
function transitKinds() { if (!LY.transit) return []; const seen = new Set(A.tk); return LY.transit.kinds.map((name, k) => [name, k]).filter(([, k]) => seen.has(k)); }
// What the road's colour shows now (S268, S269): its title, its bands or ramp.
function roadLegend() {
  if (S.colour === "delay") return { label: "Delay over free flow", bands: BAND_LABELS, stops: bandColours() };
  if (S.colour === "vc") return { label: "Volume over capacity", low: "0", high: "1.0+", stops: TABLES.vc };
  return { label: "Volume per direction (PCU/h)", low: "0", high: fmt(VMAX) + "+", stops: TABLES.volume };
}
function bandGradient(c) { const n = c.length; return "linear-gradient(90deg," + c.map((x, i) => x + " " + (100 * i / n).toFixed(2) + "% " + (100 * (i + 1) / n).toFixed(2) + "%").join(",") + ")"; }
// The legend: a section per mode the page has, always all of them, in the colours drawn (S269).
function renderLegend() {
  const line = (c, style) => '<i style="border-top:' + (style === "dash" ? "3px dashed " : style === "dot" ? "3px dotted " : "4px solid ") + c + '"></i>';
  const parts = [];
  if (HAS_TRAFFIC) {
    const r = roadLegend();
    const ramp = r.bands
      ? '<div class="bar" style="background:' + bandGradient(r.stops) + '"></div><div class="bands">' + r.bands.map((t) => "<span>" + t + "</span>").join("") + "</div>"
      : '<div class="bar" style="background:linear-gradient(90deg,' + r.stops.join(",") + ')"></div><div class="ends"><span>' + r.low + "</span><span>" + r.high + "</span></div>";
    parts.push('<div class="lg"><h3>Road traffic</h3><div class="sub">' + r.label + "</div>" + ramp + '<div class="note">width = volume per direction</div></div>');
  }
  if (LY.transit) parts.push('<div class="lg"><h3>Transit lines</h3><div class="kinds">' + transitKinds().map(([name, k]) => "<span>" + line(TH.kinds[k]) + name + "</span>").join("") + '</div><div class="note">width = passengers on board; ○ stops, size = boardings</div></div>');
  if (FLOWS.bike) parts.push('<div class="lg"><h3>Cyclists</h3><div class="kinds"><span>' + line(TH.modes[1], "dash") + 'cyclists</span></div><div class="note">width = cyclists; faint = the bike network</div></div>');
  if (FLOWS.walk) parts.push('<div class="lg"><h3>Walkers</h3><div class="kinds"><span>' + line(TH.modes[2], "dot") + 'walkers</span></div><div class="note">width = walkers; faint = the walk network</div></div>');
  if (LY.parking) parts.push('<div class="lg"><h3>Parkings</h3><div class="kinds"><span><b></b>car</span><span><b class="ci"></b>bike</span></div><div class="bar" style="background:linear-gradient(90deg,' + TABLES.delay.join(",") + ')"></div><div class="ends"><span>empty</span><span>full</span></div><div class="note">size = capacity; colour = busiest share in use</div></div>');
  if (HAS_ROUTES) parts.push('<div class="lg"><h3>Route alternatives</h3><div class="kinds">' + [0, 1, 2, 3, 4].map((i) => "<span>" + line(routeColour(i)) + (i + 1) + "</span>").join("") + '</div><div class="note">most chosen first; A origin, B destination</div></div>');
  $("legendbody").innerHTML = parts.join("");
  $("legendgroup").hidden = parts.length === 0;
}
function writeHash() {
  const p = new URLSearchParams(); p.set("t", S.theme); p.set("c", S.colour);
  p.set("b", S.bins[1] - S.bins[0] === NB ? "all" : S.bins[0] + (S.bins[1] - S.bins[0] > 1 ? "-" + S.bins[1] : ""));
  if (S.pair >= 0) p.set("p", S.pair); if (S.link >= 0) p.set("l", S.link);
  const on = Object.keys(LAYER_KEY).filter((k) => S.ly[k]).map((k) => LAYER_KEY[k]).join(""); if (on !== "r") p.set("ly", on || "-");
  p.set("z", [S.cx.toFixed(1), S.cy.toFixed(1), S.mpp.toFixed(3)].join(","));
  history.replaceState(null, "", "#" + p.toString()); document.body.dataset.view = p.toString();
}
function readHash() {
  const p = new URLSearchParams(location.hash.slice(1));
  if (p.get("t") && TOK[p.get("t")]) S.theme = p.get("t");
  if (["delay", "volume", "vc"].includes(p.get("c"))) S.colour = p.get("c");
  if (p.get("ly") !== null) for (const k of Object.keys(LAYER_KEY)) S.ly[k] = p.get("ly").includes(LAYER_KEY[k]);
  const b = p.get("b"); if (b && b !== "all" && HAS_TRAFFIC) { const [x, y] = b.split("-").map(Number); S.bins = [Math.max(0, Math.min(NB - 1, x)), Math.min(NB, y || x + 1)]; }
  return p;
}
// ---------------------------------------------------------------- export ----
// The view as it is, with its title, legend and credit: a PNG at twice the screen's resolution, or an SVG drawn
// by the same `draw()` into a recorder that writes vector paths instead of pixels.
class RecPath {
  constructor() { this.d = []; }
  moveTo(x, y) { this.d.push("M" + x.toFixed(2) + " " + y.toFixed(2)); }
  lineTo(x, y) { this.d.push("L" + x.toFixed(2) + " " + y.toFixed(2)); }
  closePath() { this.d.push("Z"); }
  arc(x, y, r) { this.d.push("M" + (x - r).toFixed(2) + " " + y.toFixed(2) + "a" + r + " " + r + " 0 1 0 " + (2 * r) + " 0a" + r + " " + r + " 0 1 0 " + (-2 * r) + " 0"); }
}
function svgRecorder() {
  const out = [], esc = (t) => String(t).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");
  let current = new RecPath(), dash = null;
  const style = (r) => ` opacity="${r.globalAlpha}"`;
  const r = {
    out, strokeStyle: "#000", fillStyle: "#000", lineWidth: 1, globalAlpha: 1, lineCap: "round", lineJoin: "round",
    font: "10px sans-serif", textAlign: "left", textBaseline: "alphabetic",
    setTransform() {}, setLineDash(a) { dash = a.length ? a.join(" ") : null; },
    beginPath() { current = new RecPath(); }, moveTo(x, y) { current.moveTo(x, y); }, lineTo(x, y) { current.lineTo(x, y); },
    closePath() { current.closePath(); }, arc(x, y, rad) { current.arc(x, y, rad); },
    fillRect(x, y, w, h) { out.push(`<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="${r.fillStyle}"/>`); },
    stroke(path) {
      const d = (path || current).d.join(""); if (!d) return;
      out.push(`<path d="${d}" fill="none" stroke="${r.strokeStyle}" stroke-width="${r.lineWidth}" stroke-linecap="${r.lineCap}" stroke-linejoin="${r.lineJoin}"${dash ? ` stroke-dasharray="${dash}"` : ""}${style(r)}/>`);
    },
    fill(path) { const d = (path || current).d.join(""); if (d) out.push(`<path d="${d}" fill="${r.fillStyle}"${style(r)}/>`); },
    fillText(t, x, y) {
      const anchor = { center: "middle", right: "end", end: "end" }[r.textAlign] || "start";
      out.push(`<text x="${x}" y="${y}" fill="${r.fillStyle}" text-anchor="${anchor}" dominant-baseline="central" style="font:${esc(r.font)}">${esc(t)}</text>`);
    },
  };
  return r;
}
// The title above the map and the legend, provenance and credit below it: the words and colours the page shows.
function exportFrame() {
  const legend = HAS_TRAFFIC && S.ly.road ? roadLegend() : null;
  // The other layers shown, each a title and its swatches [name, colour, style] (S269).
  const others = [];
  if (S.ly.transit && LY.transit) others.push({ title: "Transit lines · width = passengers on board", items: transitKinds().map(([name, k]) => [name, TH.kinds[k], ""]) });
  if (S.ly.bike && FLOWS.bike) others.push({ title: "Cyclists · width = cyclists", items: [["cyclists", TH.modes[1], "dash"]] });
  if (S.ly.walk && FLOWS.walk) others.push({ title: "Walkers · width = walkers", items: [["walkers", TH.modes[2], "dot"]] });
  if (S.ly.parking && LY.parking) others.push({ title: "Parkings · size = capacity", items: [["car (■)", TH.ink2, ""], ["bike (●)", TH.ink2, ""]] });
  return { title: $("title").textContent, subtitle: $("subtitle").textContent, legend, others, provenance: $("provenance").textContent, source: $("source").textContent, credit: $("credit").hidden ? "" : $("credit").textContent };
}
const HEAD = 64, FOOT = 46;
// An export has no panel: what the screen shows beside it is centred in the export's whole width, at the same
// scale, instead of sitting left of centre beside an empty band where the panel was (R-10).
function viewWithMoving(drawInto) {
  const moving = MOVING, hover = S.hover, item = S.hoverItem, cx = S.cx; MOVING = false; S.hover = -1; S.hoverItem = null;
  S.cx -= panelWidth() / 2 * S.mpp;
  try { drawInto(); } finally { MOVING = moving; S.hover = hover; S.hoverItem = item; S.cx = cx; }
}
// Where an export's mark goes: before the wordmark (right-aligned at W - 16, baseline y + 20, bold
// 11 px), as measured in the viewer's font, and centred on its lowercase body (an "x"'s height).
function markPlace(y) {
  const c = document.createElement("canvas").getContext("2d"), s = 0.22;
  c.font = "700 11px " + FONT;
  const word = c.measureText("openmobisim").width, x = c.measureText("x").actualBoundingBoxAscent || 6;
  return { x: W - 16 - word - 4 - 51 * s, y: y + 20 - x / 2 - 32 * s, s };
}
function exportSVG() {
  const f = exportFrame(), rec = svgRecorder(), keep = ctx, keepPath = window.Path2D;
  ctx = rec; window.Path2D = RecPath;
  try { viewWithMoving(draw); } finally { ctx = keep; window.Path2D = keepPath; draw(); }
  const esc = (t) => String(t).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");
  const total = H + HEAD + FOOT, parts = [];
  // The SVG namespace from the page's own compass (the page names no address anywhere).
  const ns = $("compass").querySelector("svg").namespaceURI;
  parts.push(`<svg xmlns="${ns}" width="${W}" height="${total}" viewBox="0 0 ${W} ${total}" font-family="${esc(FONT)}">`);
  parts.push(`<rect width="${W}" height="${total}" fill="${TH.surface}"/>`);
  parts.push(`<g transform="translate(0 ${HEAD})">${rec.out.join("")}</g>`);
  // The bands above and below the map, painted over whatever of it runs past the view.
  parts.push(`<rect width="${W}" height="${HEAD}" fill="${TH.surface}"/><rect y="${HEAD + H}" width="${W}" height="${FOOT}" fill="${TH.surface}"/>`);
  parts.push(`<text x="16" y="26" font-size="17" font-weight="700" fill="${TH.ink}">${esc(f.title)}</text>`);
  parts.push(`<text x="16" y="48" font-size="11.5" fill="${TH.ink2}">${esc(f.subtitle)}</text>`);
  const y = HEAD + H + 18;
  if (f.legend) {  // at the top right, as in the package's figures (S269)
    const n = f.legend.stops.length, w = 200, step = w / n, x0 = W - 16 - w, ly = 30;
    parts.push(`<text x="${x0}" y="${ly - 8}" font-size="10.5" fill="${TH.ink2}">${esc(f.legend.label)}</text>`);
    parts.push(f.legend.stops.map((c, i) => `<rect x="${(x0 + i * step).toFixed(2)}" y="${ly}" width="${(f.legend.bands ? step - 1.5 : step + 0.3).toFixed(2)}" height="7" fill="${c}"/>`).join(""));
    if (f.legend.bands) parts.push(f.legend.bands.map((t, i) => `<text x="${(x0 + (i + 0.5) * step).toFixed(2)}" y="${ly + 19}" font-size="9.5" text-anchor="middle" fill="${TH.ink2}">${esc(t)}</text>`).join(""));
    else parts.push(`<text x="${x0}" y="${ly + 19}" font-size="9.5" fill="${TH.ink2}">${esc(f.legend.low)}</text><text x="${x0 + w}" y="${ly + 19}" font-size="9.5" text-anchor="end" fill="${TH.ink2}">${esc(f.legend.high)}</text>`);
  }
  let ox = 16;  // the other layers' legends at the bottom left, side by side
  for (const g of f.others) {
    parts.push(`<text x="${ox}" y="${y - 2}" font-size="10" fill="${TH.ink2}">${esc(g.title)}</text>`);
    let x = ox;
    for (const [name, c, style] of g.items) {
      const dash = style === "dash" ? ' stroke-dasharray="5 3"' : style === "dot" ? ' stroke-dasharray="1.5 2.5"' : "";
      parts.push(`<line x1="${x}" y1="${y + 11}" x2="${x + 16}" y2="${y + 11}" stroke="${c}" stroke-width="3"${dash}/><text x="${x + 21}" y="${y + 14}" font-size="9.5" fill="${TH.ink2}">${esc(name)}</text>`);
      x += 32 + name.length * 6;
    }
    ox = Math.max(x, ox + g.title.length * 5.6) + 24;
  }
  parts.push(`<text x="${W - 16}" y="${y + 4}" font-size="9.5" text-anchor="end" fill="${TH.ink2}">${esc([f.provenance, f.source, f.credit].filter(Boolean).join(" · "))}</text>`);
  parts.push(`<text x="${W - 16}" y="${y + 20}" font-size="11" font-weight="700" text-anchor="end" fill="${TH.ink}">openmobisim</text>`);
  const m = markPlace(y);
  parts.push(`<path transform="translate(${m.x.toFixed(2)} ${m.y.toFixed(2)}) scale(${m.s})" d="${LOGO}" fill="${TH.mark}"/>`);
  parts.push("</svg>");
  return parts.join("");
}
function exportPNG() {
  const f = exportFrame(), scale = 2, off = document.createElement("canvas");
  off.width = Math.round(W * scale); off.height = Math.round((H + HEAD + FOOT) * scale);
  const o = off.getContext("2d"), map = document.createElement("canvas");
  map.width = Math.round(W * scale); map.height = Math.round(H * scale);
  const keep = ctx, keepDpr = DPR; ctx = map.getContext("2d"); DPR = scale;
  try { viewWithMoving(draw); } finally { ctx = keep; DPR = keepDpr; draw(); }
  o.setTransform(scale, 0, 0, scale, 0, 0);
  o.fillStyle = TH.surface; o.fillRect(0, 0, W, H + HEAD + FOOT);
  o.drawImage(map, 0, HEAD, W, H);
  o.fillStyle = TH.ink; o.font = "700 17px " + FONT; o.fillText(f.title, 16, 26);
  o.fillStyle = TH.ink2; o.font = "11.5px " + FONT; o.fillText(f.subtitle, 16, 48);
  const y = HEAD + H + 18;
  if (f.legend) {  // at the top right, as in the package's figures (S269)
    const n = f.legend.stops.length, w = 200, step = w / n, x0 = W - 16 - w, ly = 30;
    o.textAlign = "left"; o.fillStyle = TH.ink2; o.font = "10.5px " + FONT; o.fillText(f.legend.label, x0, ly - 8);
    if (f.legend.bands) {
      f.legend.stops.forEach((c, i) => { o.fillStyle = c; o.fillRect(x0 + i * step, ly, step - 1.5, 7); });
      o.fillStyle = TH.ink2; o.font = "9.5px " + FONT; o.textAlign = "center";
      f.legend.bands.forEach((t, i) => o.fillText(t, x0 + (i + 0.5) * step, ly + 19));
    } else {
      const g = o.createLinearGradient(x0, 0, x0 + w, 0);
      f.legend.stops.forEach((c, i) => g.addColorStop(i / Math.max(n - 1, 1), c));
      o.fillStyle = g; o.fillRect(x0, ly, w, 7);
      o.fillStyle = TH.ink2; o.font = "9.5px " + FONT; o.fillText(f.legend.low, x0, ly + 19); o.textAlign = "right"; o.fillText(f.legend.high, x0 + w, ly + 19);
    }
  }
  let ox = 16;  // the other layers' legends at the bottom left, side by side
  for (const g of f.others) {
    o.textAlign = "left"; o.fillStyle = TH.ink2; o.font = "10px " + FONT; o.fillText(g.title, ox, y - 2);
    const tw = o.measureText(g.title).width; o.font = "9.5px " + FONT;
    let x = ox;
    for (const [name, c, style] of g.items) {
      o.strokeStyle = c; o.lineWidth = 3; o.setLineDash(style === "dash" ? [5, 3] : style === "dot" ? [1.5, 2.5] : []);
      o.beginPath(); o.moveTo(x, y + 11); o.lineTo(x + 16, y + 11); o.stroke(); o.setLineDash([]);
      o.fillStyle = TH.ink2; o.fillText(name, x + 21, y + 14); x += 32 + o.measureText(name).width;
    }
    ox = Math.max(x, ox + tw) + 24;
  }
  o.textAlign = "right"; o.fillStyle = TH.ink2; o.font = "9.5px " + FONT;
  o.fillText([f.provenance, f.source, f.credit].filter(Boolean).join(" · "), W - 16, y + 4);
  o.fillStyle = TH.ink; o.font = "700 11px " + FONT; o.fillText("openmobisim", W - 16, y + 20);
  const m = markPlace(y);
  o.save(); o.translate(m.x, m.y); o.scale(m.s, m.s); o.fillStyle = TH.mark; o.fill(new Path2D(LOGO)); o.restore();
  return off.toDataURL("image/png");
}
function download(name, url) { const a = document.createElement("a"); a.href = url; a.download = name; document.body.appendChild(a); a.click(); a.remove(); }
function exportName(ext) { return (META.title || "openmobisim map").replace(/[^\w\- ]+/g, "").trim().replace(/\s+/g, "_") + "." + ext; }
function exportAs(kind, save = true) {
  try {
    const data = kind === "svg" ? exportSVG() : exportPNG();
    document.body.dataset.exported = kind + ":" + data.length;
    if (save) download(exportName(kind), kind === "svg" ? URL.createObjectURL(new Blob([data], { type: "image/svg+xml" })) : data);
    else { const t = document.createElement("textarea"); t.id = "exported"; t.hidden = true; t.textContent = data; document.body.appendChild(t); }
  } catch (e) { document.body.dataset.exported = kind + ":error " + e; throw e; }
}
function wire() {
  $("theme").addEventListener("click", () => { setTheme(S.theme === "paper" ? "night" : "paper"); subtitle(); redraw(false); writeHash(); });
  $("colour").addEventListener("change", (e) => { S.colour = e.target.value; subtitle(); redraw(false); writeHash(); });
  $("allbins").addEventListener("change", (e) => { if (e.target.checked) setBins(0, NB); else setBins(+$("bin").value, +$("bin").value + 1); $("bin").disabled = e.target.checked; });
  $("bin").addEventListener("input", (e) => { $("allbins").checked = false; $("bin").disabled = false; setBins(+e.target.value, +e.target.value + 1); });
  $("play").addEventListener("click", () => {
    S.playing = !S.playing; $("play").textContent = S.playing ? "Pause" : "Play";
    if (S.playing) { $("allbins").checked = false; $("bin").disabled = false; const tick = () => { if (!S.playing) return; const nb = (S.bins[1] - S.bins[0] === NB || S.bins[0] + 1 >= NB ? FIRST : S.bins[0] + 1); $("bin").value = nb; setBins(nb, nb + 1); setTimeout(tick, 450); }; tick(); }
  });
  document.querySelectorAll("#levels button").forEach((b, i) => b.addEventListener("click", () => goLevel(i)));
  $("zin").addEventListener("click", () => zoomAt(W / 2, H / 2, 0.5)); $("zout").addEventListener("click", () => zoomAt(W / 2, H / 2, 2));
  $("fit").addEventListener("click", () => { fitTo(EX0, EY0, EX1, EY1, 0.06); redraw(false); levelButtons(); writeHash(); });
  $("exportpng").addEventListener("click", () => exportAs("png")); $("exportsvg").addEventListener("click", () => exportAs("svg"));
  if (HAS_ROUTES) {
    const nk = SS.length - 1;
    $("pairno").max = nk; $("pairs").textContent = "of " + fmt(nk);
    $("pairno").addEventListener("change", (e) => { const k = +e.target.value - 1; selectPair(k >= 0 && k < nk ? k : -1, true); });
    $("prev").addEventListener("click", () => selectPair((S.pair - 1 + nk) % nk, true)); $("next").addEventListener("click", () => selectPair((S.pair + 1) % nk, true));
    // The pair with the most routes; among equals, the most travellers (S269: a pair everyone left shows nothing).
    $("richest").addEventListener("click", () => {
      let b = 0, bn = -1, bu = -1;
      for (let k = 0; k < nk; k++) {
        const n = pairRoutes(k); let u = 0; if (RU) for (let r = SS[k]; r < SS[k + 1]; r++) u += RU[r];
        if (n > bn || (n === bn && u > bu)) { bn = n; bu = u; b = k; }
      }
      selectPair(b, true);
    });
    $("clear").addEventListener("click", () => { selectPair(-1, false); selectLink(-1); });
    $("showroutes").addEventListener("change", (e) => { S.routes = e.target.checked; redraw(false); });
    $("nroutes").value = String(S.nroutes); if ($("nroutes").value !== String(S.nroutes)) $("nroutes").value = "0";
    $("nroutes").addEventListener("change", (e) => { S.nroutes = +e.target.value; routePanel(); redraw(false); });
    $("dim").addEventListener("change", (e) => { S.dim = e.target.checked; redraw(false); });
  }
  if (SB) $("spill").addEventListener("change", (e) => { S.spill = e.target.checked; redraw(false); });
  window.addEventListener("resize", () => { resize(); redraw(false); });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape") { selectLink(-1); if (S.pair >= 0) selectPair(-1, false); } });
}

// ---------------------------------------------------------------- start ----
function start() {
  $("title").textContent = META.title; $("note").textContent = META.note || ""; $("note").hidden = !META.note;
  $("provenance").textContent = META.provenance; $("provenance").title = META.provenance; $("source").textContent = META.source; if (META.credit) { $("credit").textContent = META.credit; $("credit").hidden = false; }
  $("logo").hidden = !META.logo; $("budget").textContent = META.size_note;
  document.title = META.title + " · openmobisim";
  // The tab icon: the mark, as a data URL made here (the page names no address).
  const icon = document.createElement("link"); icon.rel = "icon";
  icon.href = "data:image/svg+xml," + encodeURIComponent(`<svg xmlns="${$("compass").querySelector("svg").namespaceURI}" viewBox="0 0 64 64"><path d="${LOGO}" fill="${TOK.paper.mark}"/></svg>`);
  document.head.appendChild(icon);
  $("trafficgroup").hidden = !HAS_TRAFFIC; $("routegroup").hidden = !HAS_ROUTES;
  $("spillrow").hidden = !SB; if (SB) $("spillcount").textContent = "(" + fmt(META.n_spillback) + ")";
  if (HAS_TRAFFIC) { $("bin").min = FIRST; $("bin").max = NB - 1; $("bin").value = FIRST; $("bin").disabled = true; }
  const p = readHash(); setTheme(S.theme); $("colour").value = S.colour; resize();
  if (HAS_TRAFFIC) { const all = S.bins[1] - S.bins[0] === NB; $("allbins").checked = all; $("bin").disabled = all; $("bin").value = all ? FIRST : S.bins[0]; }
  fitTo(EX0, EY0, EX1, EY1, 0.06); FIT.mpp = S.mpp; MAX_MPP = FIT.mpp * 1.6; MIN_MPP = Math.max(0.25, Math.min(0.5, FIT.mpp / 200));
  if (HAS_TRAFFIC) aggregate(S.bins[0], S.bins[1]); subtitle();
  layerRows(); wire();
  S.lyOrder = Object.keys(LAYER_KEY).filter((k) => S.ly[k]);
  if (p.get("z")) { const [x, y, m] = p.get("z").split(",").map(Number); if (isFinite(m)) { S.cx = x; S.cy = y; S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, m)); } }
  if (p.get("p") !== null && HAS_ROUTES) selectPair(+p.get("p"), !p.get("z"));
  if (p.get("l") !== null) selectLink(+p.get("l"));
  levelButtons(); draw(); writeHash();
  document.body.dataset.state = "ready"; document.body.dataset.links = NL; document.body.dataset.rows = RL ? RL.length : 0;
  if (["png", "svg"].includes(p.get("export"))) exportAs(p.get("export"), false);  // `#export=svg`: made, not saved (a check)
}
start();
})();
"""
