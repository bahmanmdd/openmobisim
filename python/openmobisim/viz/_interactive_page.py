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
<title>__TITLE__</title>
<style>
__CSS__
</style>
</head>
<body data-state="loading">
<canvas id="map"></canvas>
<div id="error" hidden></div>
<header class="head">
  <span class="lamps"><i style="background:__EMBER__"></i><i style="background:__AMBER__"></i><i style="background:__ION__"></i></span><h1 id="title"></h1>
  <p id="subtitle"></p><p id="note" hidden></p>
</header>
<aside id="panel">
  <h2>Zoom</h2>
  <div id="levels"><button>Overview</button><button>Region</button><button>District</button><button>Street</button><button>Detail</button></div>
  <div class="row"><button id="zout" title="Zoom out">−</button><button id="zin" title="Zoom in">+</button><button id="fit">Fit</button><span id="levelname"></span></div>
  <div id="trafficgroup">
    <h2>Traffic</h2>
    <div class="row"><label>Colour by <select id="colour"><option value="delay">Delay</option><option value="volume">Volume</option><option value="vc">Volume ÷ capacity</option></select></label><button id="theme">Night</button></div>
    <div class="row"><input id="bin" type="range" min="0" max="0" value="0" step="1"></div>
    <div class="row"><label><input id="allbins" type="checkbox" checked> All bins</label><button id="play">Play</button></div>
    <div class="legend"><h2 id="legendlabel">Delay over free flow</h2><div class="bar" id="colourbar"></div><div class="ends"><span id="legendlow">0%</span><span id="legendhigh">70%+</span></div></div>
  </div>
  <div id="routegroup">
    <h2>Route alternatives</h2>
    <div class="row"><label>Pair <input id="pairno" type="number" min="1" value=""></label><span id="pairs"></span></div>
    <div class="row"><button id="prev">‹</button><button id="next">›</button><button id="richest">Most alternatives</button><button id="clear">Clear</button></div>
    <div class="row"><label><input id="showroutes" type="checkbox" checked> Show routes</label><label><input id="dim" type="checkbox" checked> Dim traffic</label></div>
    <p id="routehint">Pick a pair above, or click a link and choose one of the pairs that use it.</p>
    <div id="routelist"></div>
  </div>
  <div id="linkinfo" hidden></div>
</aside>
<div id="compass"><svg width="22" height="34" viewBox="0 0 22 34"><path d="M11 2 L17 20 L11 16 L5 20 Z" fill="currentColor"/><text x="11" y="32" text-anchor="middle" font-size="10" font-weight="700" fill="currentColor" font-family="sans-serif">N</text></svg><div id="scalelabel"></div><div id="scalebar"></div></div>
<footer><span id="provenance"></span><span id="source"></span><span class="right"><span id="budget"></span><span id="credit" hidden></span><span id="logo"><i style="background:__EMBER__"></i><i style="background:__AMBER__"></i><i style="background:__ION__"></i><b>openmobisim</b></span></span></footer>
<div id="tip" hidden></div>
<script id="meta" type="application/json">__META__</script>
<script id="blob" type="text/plain">__BLOB__</script>
<script>
__JS__
</script>
</body>
</html>
"""

PAGE_CSS = r""":root { --surface:#fbfcfd; --ink:#0a1626; --ink2:#3d4c5f; --muted:#55657a; --base:#c5cfda; --sans: Inter, "Helvetica Neue", "Segoe UI", system-ui, sans-serif; --mono: "JetBrains Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace; }
* { box-sizing: border-box; }
html, body { margin: 0; height: 100%; background: var(--surface); color: var(--ink); font-family: var(--sans); overflow: hidden; }
canvas#map { position: fixed; inset: 0; width: 100vw; height: 100vh; display: block; cursor: grab; touch-action: none; }
canvas#map:active { cursor: grabbing; }
#error { position: fixed; top: 12px; left: 50%; transform: translateX(-50%); background: #9d0221; color: #fff; padding: 10px 16px; border-radius: 8px; z-index: 20; font: 13px var(--mono); }
.head { position: fixed; top: 8px; left: 10px; max-width: min(760px, calc(100vw - 350px)); padding: 8px 14px 9px; border-radius: 12px; background: color-mix(in srgb, var(--surface) 84%, transparent); backdrop-filter: blur(3px); pointer-events: none; }
.head .lamps { display: inline-flex; gap: 5px; vertical-align: middle; margin-right: 14px; }
.head .lamps i, #logo i { width: 12px; height: 12px; border-radius: 50%; display: inline-block; }
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
.route { display: flex; align-items: center; gap: 7px; padding: 3px 4px; border-radius: 6px; font-family: var(--mono); font-size: 11.5px; cursor: default; }
.route:hover { background: color-mix(in srgb, var(--base) 45%, transparent); }
.route i { width: 22px; height: 4px; border-radius: 2px; display: inline-block; }
.route span { color: var(--muted); }
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
#logo i { width: 8px; height: 8px; } #logo b { margin-left: 5px; }
@media (max-width: 760px) { .head { max-width: calc(100vw - 20px); } aside#panel { top: auto; bottom: 40px; max-height: 42vh; width: calc(100vw - 28px); } #compass { display: none; } }
"""

PAGE_JS = r"""(async function () {
"use strict";
const $ = (id) => document.getElementById(id);
const fail = (e) => { document.body.dataset.state = "error"; const b = $("error"); b.hidden = false; b.textContent = "openmobisim map: " + (e && e.message ? e.message : e); console.error(e); };
window.addEventListener("error", (ev) => fail(ev.error || ev.message));
window.addEventListener("unhandledrejection", (ev) => fail(ev.reason));

const META = JSON.parse($("meta").textContent);
const TOK = META.tokens;

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
const VS = A.vs, CLS = A.cls, LEN = A.len, FF = A.ff, CAP = A.cap;
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
    if (tot[l] > 0) { meanT[l] = sec[l] / tot[l]; delay[l] = 1 - Math.min(FF[l] / Math.max(meanT[l], 1e-6), 1); pos.push(vol[l]); } else { meanT[l] = 0; delay[l] = 0; }
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
const S = { theme: META.theme, colour: "delay", bin: -1, bins: [0, NB], playing: false, pair: -1, link: -1, hover: -1, hoverRoute: -1, routes: true, dim: true, cx: 0, cy: 0, mpp: 1 };
let TH = TOK[S.theme], TABLES = {};
function setTheme(name) {
  S.theme = name; TH = TOK[name]; document.documentElement.dataset.theme = name;
  const r = document.documentElement.style;
  r.setProperty("--surface", TH.surface); r.setProperty("--ink", TH.ink); r.setProperty("--ink2", TH.ink2); r.setProperty("--muted", TH.muted); r.setProperty("--base", TH.base);
  TABLES = { delay: rampTable(TH.ramp_delay, NCOL), volume: rampTable(TH.ramp_volume, NCOL), vc: rampTable(TH.ramp_delay, NCOL) };
  $("colourbar").style.background = "linear-gradient(90deg," + TH.ramp_delay.join(",") + ")";
  if (S.colour === "volume") $("colourbar").style.background = "linear-gradient(90deg," + TH.ramp_volume.join(",") + ")";
  $("theme").textContent = name === "paper" ? "Night" : "Paper";
}

// ---------------------------------------------------------------- view ----
const canvas = $("map"), ctx = canvas.getContext("2d");
let W = 0, H = 0, DPR = 1;
const FIT = { mpp: 1 };
const LEVELS = ["Overview", "Region", "District", "Street", "Detail"];
let MIN_MPP = 0.25, MAX_MPP = 1;
function resize() {
  DPR = window.devicePixelRatio || 1; W = canvas.clientWidth; H = canvas.clientHeight;
  canvas.width = Math.round(W * DPR); canvas.height = Math.round(H * DPR);
}
function fitTo(x0, y0, x1, y1, pad) {
  // Fit into the part of the window the panel leaves free (it sits on the right of a wide window).
  const w = Math.max(x1 - x0, 60), h = Math.max(y1 - y0, 60), reserve = W > 760 ? 340 : 0;
  S.mpp = Math.max(w / ((W - reserve) * (1 - pad)), h / (H * (1 - pad)));
  S.cx = (x0 + x1) / 2 + reserve / 2 * S.mpp; S.cy = (y0 + y1) / 2;
}
function levelMpp(i) { return Math.max(MIN_MPP, FIT.mpp / Math.pow(3.2, i)); }
function currentLevel() { let best = 0, bd = 1e9; for (let i = 0; i < LEVELS.length; i++) { const d = Math.abs(Math.log(S.mpp / levelMpp(i))); if (d < bd) { bd = d; best = i; } } return best; }
function classLimit() { return S.mpp > 25 ? 5 : S.mpp > 8 ? 9 : S.mpp > 2.5 ? 13 : 16; }
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
  if (S.colour === "delay") t = (delay[l] - 0.04) / 0.70;
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
  const base = new Path2D(), buckets = new Map();
  const dimming = S.pair >= 0 && S.routes && S.dim;
  for (const l of cand) {
    const busy = HAS_TRAFFIC && vol[l] > 0 && (vol[l] >= VHIGH * (S.mpp > 8 ? 1 : 0.4) || CLS[l] <= lim);
    const drivable = CLS[l] < 14;
    if (CLS[l] > lim && !busy && !(showAll && S.mpp < 2.5)) continue;
    if (!drivable && S.mpp > 2.5) continue;
    addLine(base, l, 0, gap); DRAWN++;
    if (HAS_TRAFFIC && vol[l] > 0 && CLS[l] <= Math.max(lim, 16) && (CLS[l] <= lim || busy)) {
      const w = ribbonWidth(l), wi = Math.min(31, Math.round(w * 2)), key = colourIndex(l) * 32 + wi;
      let b = buckets.get(key); if (!b) { b = new Path2D(); buckets.set(key, b); }
      addLine(b, l, w / 2 + 0.25, gap);
    }
  }
  ctx.lineCap = "round"; ctx.lineJoin = "round";
  ctx.strokeStyle = TH.base; ctx.lineWidth = S.mpp > 6 ? 0.5 : 0.9; ctx.stroke(base);
  const keys = [...buckets.keys()].sort((a, b) => a - b), tbl = TABLES[S.colour];
  ctx.globalAlpha = dimming ? 0.35 : 1;
  if (TH.glow) { for (const k of keys) { ctx.strokeStyle = tbl[k >> 5]; ctx.globalAlpha = (dimming ? 0.35 : 1) * 0.14; ctx.lineWidth = (k & 31) / 2 * 2.8 + 1.2; ctx.stroke(buckets.get(k)); } ctx.globalAlpha = dimming ? 0.35 : 1; }
  else if (!MOVING) { for (const k of keys) { ctx.strokeStyle = TH.surface; ctx.lineWidth = (k & 31) / 2 + 1.4; ctx.stroke(buckets.get(k)); } }
  for (const k of keys) { ctx.strokeStyle = tbl[k >> 5]; ctx.lineWidth = (k & 31) / 2; ctx.stroke(buckets.get(k)); }
  ctx.globalAlpha = 1;
  if (!MOVING && S.mpp < 3) drawChevrons(cand, lim);
  if (S.pair >= 0 && S.routes && HAS_ROUTES) drawRoutes();
  highlight(S.link, TH.ink, 3); if (S.hover !== S.link) highlight(S.hover, TH.ink2, 2);
  drawScale();
  document.body.dataset.drawms = (performance.now() - t0).toFixed(1); document.body.dataset.drawn = DRAWN; document.body.dataset.level = LEVELS[currentLevel()];
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
function highlight(l, colour, w) {
  if (l < 0) return;
  const p = new Path2D(); if (!addLine(p, l, 0, 0.5)) return;
  ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 3; ctx.stroke(p); ctx.strokeStyle = colour; ctx.lineWidth = w; ctx.stroke(p);
}
function routeColour(r) { const p = TOK.route[S.theme]; return p[r % p.length]; }
function drawRoutes() {
  const k = S.pair, r0 = SS[k], r1 = SS[k + 1], step = 3.8;
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
const CLASSES = META.class_names;
function fmt(x, d) { return Number(x).toLocaleString(undefined, { maximumFractionDigits: d === undefined ? 0 : d }); }
function tip(l, ev) {
  const t = $("tip");
  if (l < 0) { t.hidden = true; return; }
  const rows = [["Class", CLASSES[CLS[l]] + " · " + LEN[l].toFixed(0) + " m"]];
  if (HAS_TRAFFIC) { rows.push(["Volume", fmt(vol[l]) + " PCU/h"]); if (vol[l] > 0) { rows.push(["Delay", fmt(delay[l] * 100) + "% over free flow"]); rows.push(["Mean time", meanT[l].toFixed(1) + " s (free " + FF[l].toFixed(1) + ")"]); } }
  if (CAP[l] > 0) rows.push(["Capacity", fmt(CAP[l]) + " PCU/h" + (vol[l] > 0 ? " (" + fmt(vol[l] / CAP[l] * 100) + "% used)" : "")]);
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
  const l = pick(e.offsetX, e.offsetY); if (l !== S.hover) { S.hover = l; redraw(false); } tip(l, e);
});
canvas.addEventListener("pointerup", (e) => { if (drag && !drag.moved) selectLink(pick(e.offsetX, e.offsetY)); drag = null; writeHash(); });
canvas.addEventListener("pointerleave", () => { S.hover = -1; $("tip").hidden = true; });
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
function selectPair(k, fit) {
  if (!HAS_ROUTES) return;
  S.pair = k; S.hoverRoute = -1;
  if (k >= 0 && fit !== false) {
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (let r = SS[k]; r < SS[k + 1]; r++) for (let i = RST[r]; i < RST[r + 1]; i++) { const l = RLK[i]; if (BX0[l] < x0) x0 = BX0[l]; if (BY0[l] < y0) y0 = BY0[l]; if (BX1[l] > x1) x1 = BX1[l]; if (BY1[l] > y1) y1 = BY1[l]; }
    fitTo(x0, y0, x1, y1, 0.3); S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, S.mpp));
  }
  $("pairno").value = k >= 0 ? k + 1 : ""; routePanel(); redraw(false); levelButtons(); writeHash();
}
function routePanel() {
  const box = $("routelist"); box.innerHTML = "";
  if (S.pair < 0) { $("routehint").hidden = false; return; }
  $("routehint").hidden = true;
  const k = S.pair, r0 = SS[k], n = pairRoutes(k), best = RC[r0];
  if (n === 0) { box.textContent = "No route between this pair."; return; }
  let used = 0; if (RU) for (let i = 0; i < n; i++) used += RU[r0 + i];
  for (let i = 0; i < n; i++) {
    const r = r0 + i, row = document.createElement("div"); row.className = "route";
    const u = RU ? RU[r] : 0;
    row.innerHTML = '<i style="background:' + routeColour(i) + '"></i><div><b>' + (i + 1) + "</b> " + (RC[r] / 60).toFixed(1) + " min <span>+" + Math.round((RC[r] / best - 1) * 100) + "% · overlap " + Math.round(RO[r] * 100) + "%</span>" +
      (RU ? "<small>" + fmt(u) + (u === 1 ? " traveller" : " travellers") + (used > 0 ? " · " + Math.round(u / used * 100) + "%" : "") + "</small>" : "") + "</div>";
    row.addEventListener("pointerenter", () => { S.hoverRoute = i; redraw(false); }); row.addEventListener("pointerleave", () => { S.hoverRoute = -1; redraw(false); });
    box.appendChild(row);
  }
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
function binLabel() {
  if (!HAS_TRAFFIC) return "no traffic recorded";
  const [a, b] = S.bins, s = META.bin_seconds / 60;
  return b - a === NB && NB > 1 ? "all bins, " + fmt(a * s, 1) + "–" + fmt(b * s, 1) + " min" : fmt(a * s, 1) + "–" + fmt(b * s, 1) + " min";
}
function setBins(a, b) { S.bins = [a, b]; aggregate(a, b); subtitle(); redraw(false); writeHash(); }
const COLOUR_WORDS = { delay: "delay over free flow", volume: "volume", vc: "volume over capacity" };
function subtitle() {
  $("subtitle").textContent = binLabel() + " · one ribbon per direction · width = volume, colour = " + COLOUR_WORDS[S.colour];
  $("legendlabel").textContent = { delay: "Delay over free flow", volume: "Volume per direction", vc: "Volume over capacity" }[S.colour];
  $("legendlow").textContent = S.colour === "volume" ? "0" : S.colour === "vc" ? "0" : "0%"; $("legendhigh").textContent = S.colour === "volume" ? fmt(VMAX) + "+" : S.colour === "vc" ? "1.0+" : "70%+";
  $("colourbar").style.background = "linear-gradient(90deg," + (S.colour === "volume" ? TH.ramp_volume : TH.ramp_delay).join(",") + ")";
}
function writeHash() {
  const p = new URLSearchParams(); p.set("t", S.theme); p.set("c", S.colour);
  p.set("b", S.bins[1] - S.bins[0] === NB ? "all" : S.bins[0] + (S.bins[1] - S.bins[0] > 1 ? "-" + S.bins[1] : ""));
  if (S.pair >= 0) p.set("p", S.pair); if (S.link >= 0) p.set("l", S.link);
  p.set("z", [S.cx.toFixed(1), S.cy.toFixed(1), S.mpp.toFixed(3)].join(","));
  history.replaceState(null, "", "#" + p.toString()); document.body.dataset.view = p.toString();
}
function readHash() {
  const p = new URLSearchParams(location.hash.slice(1));
  if (p.get("t") && TOK[p.get("t")]) S.theme = p.get("t");
  if (["delay", "volume", "vc"].includes(p.get("c"))) S.colour = p.get("c");
  const b = p.get("b"); if (b && b !== "all" && HAS_TRAFFIC) { const [x, y] = b.split("-").map(Number); S.bins = [Math.max(0, Math.min(NB - 1, x)), Math.min(NB, y || x + 1)]; }
  return p;
}
function wire() {
  $("theme").addEventListener("click", () => { setTheme(S.theme === "paper" ? "night" : "paper"); subtitle(); redraw(false); writeHash(); });
  $("colour").addEventListener("change", (e) => { S.colour = e.target.value; subtitle(); redraw(false); writeHash(); });
  $("allbins").addEventListener("change", (e) => { if (e.target.checked) setBins(0, NB); else setBins(+$("bin").value, +$("bin").value + 1); $("bin").disabled = e.target.checked; });
  $("bin").addEventListener("input", (e) => { $("allbins").checked = false; $("bin").disabled = false; setBins(+e.target.value, +e.target.value + 1); });
  $("play").addEventListener("click", () => {
    S.playing = !S.playing; $("play").textContent = S.playing ? "Pause" : "Play";
    if (S.playing) { $("allbins").checked = false; $("bin").disabled = false; const tick = () => { if (!S.playing) return; const nb = (S.bins[1] - S.bins[0] === NB ? 0 : (S.bins[0] + 1) % NB); $("bin").value = nb; setBins(nb, nb + 1); setTimeout(tick, 450); }; tick(); }
  });
  document.querySelectorAll("#levels button").forEach((b, i) => b.addEventListener("click", () => goLevel(i)));
  $("zin").addEventListener("click", () => zoomAt(W / 2, H / 2, 0.5)); $("zout").addEventListener("click", () => zoomAt(W / 2, H / 2, 2));
  $("fit").addEventListener("click", () => { fitTo(EX0, EY0, EX1, EY1, 0.06); redraw(false); levelButtons(); writeHash(); });
  if (HAS_ROUTES) {
    const nk = SS.length - 1;
    $("pairno").max = nk; $("pairs").textContent = "of " + fmt(nk);
    $("pairno").addEventListener("change", (e) => { const k = +e.target.value - 1; selectPair(k >= 0 && k < nk ? k : -1, true); });
    $("prev").addEventListener("click", () => selectPair((S.pair - 1 + nk) % nk, true)); $("next").addEventListener("click", () => selectPair((S.pair + 1) % nk, true));
    $("richest").addEventListener("click", () => { let b = 0, bn = -1; for (let k = 0; k < nk; k++) if (pairRoutes(k) > bn) { bn = pairRoutes(k); b = k; } selectPair(b, true); });
    $("clear").addEventListener("click", () => { selectPair(-1, false); selectLink(-1); });
    $("showroutes").addEventListener("change", (e) => { S.routes = e.target.checked; redraw(false); });
    $("dim").addEventListener("change", (e) => { S.dim = e.target.checked; redraw(false); });
  }
  window.addEventListener("resize", () => { resize(); redraw(false); });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape") { selectLink(-1); if (S.pair >= 0) selectPair(-1, false); } });
}

// ---------------------------------------------------------------- start ----
function start() {
  $("title").textContent = META.title; $("note").textContent = META.note || ""; $("note").hidden = !META.note;
  $("provenance").textContent = META.provenance; $("provenance").title = META.provenance; $("source").textContent = META.source; if (META.credit) { $("credit").textContent = META.credit; $("credit").hidden = false; }
  $("logo").hidden = !META.logo; $("budget").textContent = META.size_note;
  document.title = META.title + " · openmobisim";
  $("trafficgroup").hidden = !HAS_TRAFFIC; $("routegroup").hidden = !HAS_ROUTES;
  if (HAS_TRAFFIC) { $("bin").max = NB - 1; $("bin").disabled = true; }
  const p = readHash(); setTheme(S.theme); $("colour").value = S.colour; resize();
  if (HAS_TRAFFIC) { const all = S.bins[1] - S.bins[0] === NB; $("allbins").checked = all; $("bin").disabled = all; $("bin").value = all ? 0 : S.bins[0]; }
  fitTo(EX0, EY0, EX1, EY1, 0.06); FIT.mpp = S.mpp; MAX_MPP = FIT.mpp * 1.6; MIN_MPP = Math.max(0.25, Math.min(0.5, FIT.mpp / 200));
  if (HAS_TRAFFIC) aggregate(S.bins[0], S.bins[1]); subtitle();
  wire();
  if (p.get("z")) { const [x, y, m] = p.get("z").split(",").map(Number); if (isFinite(m)) { S.cx = x; S.cy = y; S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, m)); } }
  if (p.get("p") !== null && HAS_ROUTES) selectPair(+p.get("p"), !p.get("z"));
  if (p.get("l") !== null) selectLink(+p.get("l"));
  levelButtons(); draw(); writeHash();
  document.body.dataset.state = "ready"; document.body.dataset.links = NL; document.body.dataset.rows = RL.length;
}
start();
})();
"""
