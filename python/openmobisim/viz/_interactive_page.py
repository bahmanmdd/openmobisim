# ruff: noqa: E501
"""The page of the interactive map: HTML shell, stylesheet and script.

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
<div id="side">
<aside id="panel">
  <h2>Zoom</h2>
  <div id="levels"><button>Overview</button><button>Region</button><button>District</button><button>Street</button><button>Detail</button></div>
  <div class="row"><button id="zout" title="Zoom out">−</button><button id="zin" title="Zoom in">+</button><button id="fit">Fit</button><span id="levelname"></span></div>
  <div class="row"><span>Export</span><button id="exportpng" title="An image at twice the screen's resolution">PNG</button><button id="exportsvg" title="Vector graphics, for print and editing">SVG</button><span class="grow"></span><button id="theme">Night</button></div>
  <div id="layergroup" hidden>
    <h2>Layers</h2>
    <div class="layers" id="layerrows"></div>
  </div>
  <div id="routegroup">
    <h2>Trips between two places</h2>
    <div class="row"><select id="pairpick" title="The pairs with the most travellers first"></select></div>
    <div class="row"><button id="prev" title="The pair before">‹</button><button id="next" title="The next pair">›</button><button id="pickmap" title="Click an origin on the map, then a destination">Pick on map</button><button id="clear">Clear</button></div>
    <p id="routehint">Choose a pair above or pick one on the map: every way its travellers went, by every mode. Or click a road to see the pairs whose trips use it.</p>
    <div id="tripcard"></div>
  </div>
  <div id="trafficgroup">
    <h2>Traffic</h2>
    <div class="row"><label>Colour by <select id="colour"><option value="delay">Delay</option><option value="volume">Volume</option><option value="vc">Volume ÷ capacity</option></select></label></div>
    <div class="row"><input id="bin" type="range" min="0" max="0" value="0" step="1"></div>
    <div class="row"><label><input id="allbins" type="checkbox" checked> All bins</label><button id="play">Play</button></div>
    <div class="row" id="spillrow" hidden><label title="Links whose queue would not have fitted on the road: where it would spill back (point queue)"><input id="spill" type="checkbox"> Spillback links <span id="spillcount"></span></label></div>
  </div>
  <div id="linkinfo" hidden></div>
</aside>
<div id="legendgroup" class="card">
  <div id="legendbody"></div>
  <div id="compass"><svg width="18" height="28" viewBox="0 0 22 34"><path d="M11 2 L17 20 L11 16 L5 20 Z" fill="currentColor"/><text x="11" y="32" text-anchor="middle" font-size="10" font-weight="700" fill="currentColor" font-family="sans-serif">N</text></svg><div><div id="scalelabel"></div><div id="scalebar"></div></div></div>
</div>
</div>
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
/* Everything the reader uses in one column on the right (S306): the controls, then the legend with the compass and
   the scale, kept in view at the column's foot while the controls scroll. */
#side { position: fixed; top: 14px; right: 14px; bottom: 40px; width: 306px; display: flex; flex-direction: column; gap: 10px; pointer-events: none; }
#side > * { pointer-events: auto; }
aside#panel { flex: 0 1 auto; min-height: 0; overflow: auto; background: color-mix(in srgb, var(--surface) 94%, transparent); border: 1px solid var(--base); border-radius: 12px; padding: 12px 14px 14px; box-shadow: 0 6px 26px rgba(0,0,0,.18); font-size: 12.5px; backdrop-filter: blur(6px); }
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
#legendgroup.card { flex: 0 1 auto; margin-top: auto; max-height: 48%; display: flex; flex-direction: column; overflow: hidden; padding: 10px 14px 8px; border-radius: 12px; background: color-mix(in srgb, var(--surface) 90%, transparent); border: 1px solid color-mix(in srgb, var(--base) 70%, transparent); backdrop-filter: blur(4px); font-size: 12px; box-shadow: 0 3px 14px rgba(0,0,0,.10); }
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
.layers { display: grid; grid-template-columns: 1fr 1fr; gap: 2px 10px; margin-bottom: 4px; } .layers label { margin: 2px 0; }
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
#legendbody { overflow: auto; min-height: 0; }
#compass { flex: none; display: flex; align-items: flex-end; gap: 12px; color: var(--ink); font: 11px var(--mono); }
#legendbody:not([hidden]) + #compass { border-top: 1px solid color-mix(in srgb, var(--base) 70%, transparent); padding-top: 8px; }
#compass svg { display: block; flex: none; }
#scalebar { height: 6px; border: 2px solid var(--ink); border-top: 0; margin-top: 3px; }
footer { position: fixed; left: 0; right: 0; bottom: 0; height: 30px; display: flex; align-items: center; justify-content: space-between; padding: 0 20px; border-top: 1px solid var(--base); background: color-mix(in srgb, var(--surface) 92%, transparent); font: 11px var(--mono); color: var(--muted); }
footer .right { display: flex; align-items: center; gap: 10px; font-family: var(--sans); flex: none; white-space: nowrap; }
footer #provenance { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-right: 12px; }
footer #source { flex: none; white-space: nowrap; margin-right: 16px; }
#credit { color: var(--ink2); font-size: 12px; padding-right: 10px; border-right: 1px solid var(--base); }
#logo { display: inline-flex; align-items: center; gap: 4px; color: var(--ink); font-weight: 700; font-size: 13px; }
#logo .mark { width: 16px; height: 16px; fill: var(--mark); } #logo b { margin-left: 3px; position: relative; top: -0.1em; }  /* the lowercase body centred on the mark, not the line box */
.grow { flex: 1; }
/* The layers, each with its mark (S305) */
.layers label .sw { width: 18px; height: 0; display: inline-block; vertical-align: middle; }
.layers label .sw.box { width: 9px; height: 9px; border: 1.5px solid var(--ink2); border-radius: 2px; }
.layers label .sw.ramp { height: 5px; border-radius: 2px; }
/* Trips between two places (S305) */
#pairpick { width: 100%; font-size: 11.5px; padding: 4px 6px; }
#tripcard .od { font-size: 13px; color: var(--ink); line-height: 1.45; margin-top: 4px; }
#tripcard .od b { font-weight: 650; }
#tripcard .mk { display: inline-flex; align-items: center; justify-content: center; width: 16px; height: 16px; border-radius: 50%; background: var(--ink); color: var(--surface); font: 700 9.5px var(--sans); margin-right: 4px; vertical-align: 1px; }
#tripcard .odsub { font-size: 11.5px; color: var(--muted); margin: 1px 0 8px; }
#tripcard .split { display: flex; height: 10px; border-radius: 5px; overflow: hidden; margin: 2px 0 5px; background: var(--base); }
#tripcard .split span { display: block; height: 100%; }
#tripcard .chips { display: flex; flex-wrap: wrap; gap: 4px 5px; margin-bottom: 8px; }
#tripcard .chip { font-size: 11px; padding: 2px 7px 2px 5px; border-radius: 10px; border: 1px solid var(--base); display: inline-flex; align-items: center; gap: 5px; cursor: pointer; color: var(--ink2); user-select: none; }
#tripcard .chip i { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
#tripcard .chip.off { opacity: .4; text-decoration: line-through; }
#tripcard .opt { padding: 6px 6px 7px; border-radius: 8px; cursor: pointer; border: 1px solid transparent; }
#tripcard .opt:hover, #tripcard .opt.hl { background: color-mix(in srgb, var(--base) 38%, transparent); border-color: color-mix(in srgb, var(--base) 80%, transparent); }
#tripcard .opt .top { display: flex; align-items: baseline; gap: 6px; font-size: 12px; }
#tripcard .opt .top b { font-weight: 600; color: var(--ink); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
#tripcard .opt .top .t { font: 600 12px var(--mono); color: var(--ink); white-space: nowrap; }
#tripcard .opt .top .dot { width: 9px; height: 9px; border-radius: 50%; flex: none; align-self: center; }
#tripcard .opt .sub { font-size: 10.5px; color: var(--muted); margin-top: 3px; display: flex; justify-content: space-between; gap: 8px; }
#tripcard .opt .share { height: 3px; border-radius: 2px; margin-top: 3px; background: var(--base); overflow: hidden; }
#tripcard .opt .share span { display: block; height: 100%; }
#tripcard .opt.unused .top b, #tripcard .opt.unused .top .t { color: var(--muted); font-weight: 500; }
#tripcard .strip { display: flex; gap: 2px; height: 14px; margin-top: 5px; }
#tripcard .strip span { display: block; height: 100%; min-width: 3px; border-radius: 3px; font: 600 9px/14px var(--sans); color: #fff; text-align: center; overflow: hidden; white-space: nowrap; }
#tripcard .strip .walk { background-image: radial-gradient(circle, var(--c) 1.6px, transparent 1.8px); background-size: 6px 14px; background-position: center; }
#tripcard .strip .wait { background: repeating-linear-gradient(135deg, var(--base) 0 3px, transparent 3px 6px); }
#tripcard .strip .park { background: var(--base); }
#tripcard .group { font-size: 10px; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); margin: 8px 0 2px 6px; font-weight: 600; }
#tripcard .more { font-size: 11px; color: var(--ink2); margin: 6px 6px 0; }
#tripcard .more a { color: var(--ink); cursor: pointer; text-decoration: underline; text-underline-offset: 2px; }
#tripcard .foot { font-size: 10.5px; color: var(--muted); margin: 8px 6px 0; line-height: 1.4; }
#legendbody .kinds em { font-style: normal; }
#legendbody .kinds .mkr { display: inline-flex; align-items: center; justify-content: center; width: 13px; height: 13px; border-radius: 50%; background: var(--ink); color: var(--surface); font: 700 8px var(--sans); margin-right: 4px; }
#legendbody .kinds .stop { display: inline-block; width: 8px; height: 8px; border-radius: 50%; border: 1.5px solid var(--ink2); background: var(--surface); margin-right: 4px; vertical-align: -1px; }
#legendbody .kinds .badge { display: inline-block; padding: 0 4px; border-radius: 4px; font: 700 9px/13px var(--sans); color: #fff; margin-right: 4px; }
body[data-picking] canvas#map { cursor: crosshair; }
@media (max-width: 760px) { .head { max-width: calc(100vw - 20px); } #side { top: auto; max-height: 42vh; width: calc(100vw - 28px); } #legendgroup.card { display: none; } }
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
// Each link's road tier (S302): the base network's width by tier, as multiples of its base width, and the
// grey margin around a ribbon (px at a zoom scale of 1). Without roads, one width and no margin.
const RT = A.rt || null, TIERS = RT ? META.road_tiers : [1], CASING_PX = RT ? META.road_casings : [0];
const tierOf = (l) => (RT ? RT[l] : 0);
// Each link's upstream and downstream node (S306), and its other direction: the link between the same two nodes
// the other way (-1 for a one-way link). A two-way road is two links on one geometry, each drawn on its own side.
const NF = A.nf || null, NT = A.nt || null, TWIN = new Int32Array(NL).fill(-1);
if (NF && NT) {
  const at = new Map(); for (let l = 0; l < NL; l++) at.set(NF[l] + ":" + NT[l], l);
  for (let l = 0; l < NL; l++) { const r = at.get(NT[l] + ":" + NF[l]); if (r !== undefined && r !== l) TWIN[l] = r; }
}
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

// trips between two places (S305): each origin-destination pair's journeys, by every mode
const TR = META.trips || null, HAS_ROUTES = !!TR;
const LEG = { drive: 0, bike: 1, walk: 2, ride: 3, connect: 4, wait: 5, park: 6 }, NONE = 4294967295;
const KIND_NAMES = ["rail", "metro", "tram", "bus", "ferry", "other"];
let NP = 0, OPT_PAIR = null, LOPT_START = null, LOPTS = null;
// The bike and walk layers' links, for the legs ridden or walked on them (the flow layers' own when drawn).
function layerXY(prefix) {
  const vs = A[prefix + "vs"], dx = A[prefix + "dx"], dy = A[prefix + "dy"];
  if (!vs) return null;
  const xs = new Float64Array(dx.length), ys = new Float64Array(dy.length);
  let cx = 0, cy = 0; for (let i = 0; i < dx.length; i++) { cx += dx[i]; cy += dy[i]; xs[i] = cx * 0.1; ys[i] = cy * 0.1; }
  return { vs, xs, ys };
}
let GEO = {};
if (HAS_ROUTES) {
  NP = A.qs.length - 1;
  OPT_PAIR = new Uint32Array(A.om.length);
  for (let p = 0; p < NP; p++) for (let o = A.qs[p]; o < A.qs[p + 1]; o++) OPT_PAIR[o] = p;
  // Which options drive on each road link: a clicked road lists the pairs whose trips use it.
  const cnt = new Uint32Array(NL + 1);
  const each = (f) => { for (let o = 0; o < A.om.length; o++) for (let g = A.ol[o]; g < A.ol[o + 1]; g++) if (A.lk[g] === LEG.drive) for (let i = A.la[g]; i < A.lb[g]; i++) f(A.pl[i], o); };
  each((l) => cnt[l + 1]++);
  for (let i = 1; i <= NL; i++) cnt[i] += cnt[i - 1];
  LOPT_START = cnt; const fill = cnt.slice(); LOPTS = new Uint32Array(cnt[NL]);
  each((l, o) => { LOPTS[fill[l]++] = o; });
  GEO = { bike: FLOWS.bike || layerXY("b"), walk: FLOWS.walk || layerXY("w") };
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
const S = { theme: META.theme, colour: "delay", bin: -1, bins: [0, NB], playing: false, pair: -1, link: -1, hover: -1, hoverItem: null, lyOrder: [], hoverOpt: -1, routes: true, allCar: false, allOpts: false, hidden: new Set(), picking: 0, pickFrom: null, dim: true, spill: false, cx: 0, cy: 0, mpp: 1, ly: { road: true, transit: false, parking: false, bike: false, walk: false } };
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
// The right column's width over the map (the controls and the legend, S306): on the right of a wide window.
function panelWidth() { return W > 760 ? 340 : 0; }
function fitTo(x0, y0, x1, y1, pad) {
  // Fit into the part of the window the right column, the title above and the footer below leave free (S269: a
  // tall network ran under both); nothing is reserved on the left (S306).
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
// ---- the widths, from the room the roads leave on screen (S306) ----
// ROOM: the screen space (px) each drawn road direction has around it where the roads are dense: the screen is cut
// into cells of ROOM_CELL px, each cell's length of drawn road directions summed, and the room is 2 x cell area
// / that length at the cells' 75th percentile (a grid of streets s px apart, two directions each, gives s / 2).
// Kept in metres from the last still frame, so it scales with the zoom while moving.
const ROOM_CELL = 128, ROOM_SHARE = 0.25, WFLOOR = 0.9, WCAP = 30;
let ROOM_M = 0, EXPORTING = false;
function measureRoom(cand) {
  const C = ROOM_CELL, nx = Math.ceil(W / C), ny = Math.ceil(H / C), cells = new Float64Array(nx * ny);
  const lim = Math.min(classLimit(), 13);  // the drivable roads drawn at this zoom: those that carry ribbons
  for (const l of cand) {
    if (CLS[l] > lim) continue;
    for (let v = VS[l]; v < VS[l + 1] - 1; v++) {
      const ax = sx(X[v]), ay = sy(Y[v]), bx = sx(X[v + 1]), by = sy(Y[v + 1]), d = Math.hypot(bx - ax, by - ay);
      const n = Math.max(1, Math.ceil(d / (C / 2)));
      for (let i = 0; i < n; i++) {
        const t = (i + 0.5) / n, x = ax + (bx - ax) * t, y = ay + (by - ay) * t;
        if (x >= 0 && y >= 0 && x < W && y < H) cells[Math.floor(y / C) * nx + Math.floor(x / C)] += d / n;
      }
    }
  }
  const used = []; for (const c of cells) if (c > 0) used.push(c);
  if (!used.length) return;
  used.sort((a, b) => a - b);
  ROOM_M = 2 * C * C / used[Math.floor(0.75 * (used.length - 1))] * S.mpp;
}
function roomPx() { return ROOM_M > 0 ? ROOM_M / S.mpp : 40; }
// The widest ribbon (the 99th percentile's volume): a quarter of the room, so a two-way road at its busiest takes
// half the space between it and the next road; between 2.5 and WCAP px.
function ribbonMax() { return Math.max(2.5, Math.min(WCAP, ROOM_SHARE * roomPx())); }
// The space between a road's two directions (px): each ribbon sits on its own side of the centre line, to the
// right of its travel, so the two read apart at every zoom (S306: 0.5 px between them read as one ribbon).
let GAP = 2;  // set by each frame: a fifth of the widest ribbon, 1.5 to 6 px
function ribbonOffset(l, w) { return GAP / 2 + w / 2; }
// Whether link l is drawn as a ribbon of road traffic now.
function hasRibbon(l) { return S.ly.road && HAS_TRAFFIC && vol[l] > 0; }
// The grey margin (capacity) on a ribbon's outer side only, so it never fills the space between the directions;
// the major roads' margin is 40 % of the widest ribbon, the other tiers' in proportion.
const CASING_TOP = Math.max(...CASING_PX) || 1;
function casingSide(l) { return casingOf(tierOf(l)); }
function casingOf(tier) { return CASING_PX[tier] / CASING_TOP * 0.4 * ribbonMax(); }
// A ribbon's width grows linearly with its volume (S306: the square root's range was too small to read): from
// WFLOOR at no traffic to ribbonMax() at the 99th percentile's volume and above.
function ribbonWidthOf(f) { const top = ribbonMax(); return WFLOOR + (top - WFLOOR) * f; }
function ribbonWidth(l) { return ribbonWidthOf(Math.min(vol[l], VMAX) / VMAX); }
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
    let m = Math.hypot(nx[i], ny[i]), ax = nx[i], ay = ny[i], k = 1;
    if (m < 0.35) { ax = fx[i]; ay = fy[i]; m = Math.hypot(ax, ay) || 1; }   // a U-turn cancels the normals: keep the last segment's
    // At a bend between two segments, the mitre: as far out as keeps both offset segments `off` from their line
    // (at most twice `off`), so the two directions of a road stay apart around a corner too (S306).
    else if (i > 0 && i < n - 1) k = Math.min(2, 2 / m);
    PX[i] += ax / m * off * k; PY[i] += ay / m * off * k;
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
function addLine(path, l, off, gap) {
  const n = screenLine(l, off, gap);
  if (n < 2) return 0;
  path.moveTo(PX[0], PY[0]); for (let i = 1; i < n; i++) path.lineTo(PX[i], PY[i]);
  return n;
}
let DRAWN = 0, MOVING = false, LAYER_ALPHA = 1;
function draw() {
  DRAWN = 0; const t0 = performance.now();
  ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
  ctx.fillStyle = TH.surface; ctx.fillRect(0, 0, W, H);
  const hw = W / 2 * S.mpp, hh = H / 2 * S.mpp, pad = 8 * S.mpp;
  const cand = candidates(S.cx - hw - pad, S.cy - hh - pad, S.cx + hw + pad, S.cy + hh + pad);
  const lim = classLimit() - (MOVING ? 3 : 0), gap = MOVING ? 3 : 1.2;
  const showAll = classLimit() >= 16;
  if (!MOVING && !EXPORTING) measureRoom(cand);  // an export keeps the screen's widths
  GAP = Math.max(1.5, Math.min(6, ribbonMax() / 5));
  const base = TIERS.map(() => new Path2D()), casings = new Map(), faint = new Path2D(), buckets = new Map();
  const dimming = S.pair >= 0 && S.routes && S.dim;
  for (const l of cand) {
    const busy = HAS_TRAFFIC && vol[l] > 0 && (vol[l] >= VHIGH * (S.mpp > 8 ? 1 : 0.4) || CLS[l] <= lim);
    const drivable = CLS[l] < 14;
    // A link this zoom leaves out is drawn faint, not left out (S269), but while moving.
    if ((CLS[l] > lim && !busy && !(showAll && S.mpp < 2.5)) || (!drivable && S.mpp > 2.5)) {
      if (!MOVING) addLine(faint, l, 0, gap);
      continue;
    }
    addLine(base[tierOf(l)], l, 0, gap); DRAWN++;
    if (S.ly.road && HAS_TRAFFIC && vol[l] > 0 && CLS[l] <= Math.max(lim, 16) && (CLS[l] <= lim || busy)) {
      const w = ribbonWidth(l), wi = Math.min(127, Math.round(w * 2)), key = colourIndex(l) * 128 + wi;
      let b = buckets.get(key); if (!b) { b = new Path2D(); buckets.set(key, b); }
      addLine(b, l, ribbonOffset(l, w), gap);
      if (CASING_PX[tierOf(l)] > 0) { const ck = tierOf(l) * 128 + wi; let c = casings.get(ck); if (!c) { c = new Path2D(); casings.set(ck, c); } addLine(c, l, ribbonOffset(l, w) + casingSide(l) / 2, gap); }
    }
  }
  ctx.lineCap = "round"; ctx.lineJoin = "round";
  ctx.globalAlpha = 0.55; ctx.strokeStyle = TH.base; ctx.lineWidth = 0.4; ctx.stroke(faint); ctx.globalAlpha = 1;
  // The road hierarchy (S302): the minor tiers first, so the major roads sit on top.
  ctx.strokeStyle = TH.base; for (let t = 0; t < TIERS.length; t++) { ctx.lineWidth = (S.mpp > 6 ? 0.5 : 0.9) * TIERS[t]; ctx.stroke(base[t]); }
  const keys = [...buckets.keys()].sort((a, b) => a - b), tbl = TABLES[S.colour];
  ctx.globalAlpha = dimming ? 0.35 : 1;
  if (TH.glow) { for (const k of keys) { const w = (k & 127) / 2; ctx.strokeStyle = tbl[k >> 7]; ctx.globalAlpha = (dimming ? 0.35 : 1) * 0.14; ctx.lineWidth = w + Math.min(w * 1.8, GAP * 0.8); ctx.stroke(buckets.get(k)); } ctx.globalAlpha = dimming ? 0.35 : 1; }
  else if (!MOVING) { for (const k of keys) { ctx.strokeStyle = TH.surface; ctx.lineWidth = (k & 127) / 2 + 1.4; ctx.stroke(buckets.get(k)); } }
  // The casing (S302): each ribbon widened on its outer side by its road's tier, a grey margin = capacity; major
  // roads on top.
  { const a = ctx.globalAlpha; ctx.globalAlpha = 1; ctx.strokeStyle = TH.base; ctx.lineCap = "butt";
    for (const k of [...casings.keys()].sort((x, y) => x - y)) { ctx.lineWidth = (k & 127) / 2 + casingOf(k >> 7); ctx.stroke(casings.get(k)); }
    ctx.lineCap = "round"; ctx.globalAlpha = a; }
  for (const k of keys) { ctx.strokeStyle = tbl[k >> 7]; ctx.lineWidth = (k & 127) / 2; ctx.stroke(buckets.get(k)); }
  ctx.globalAlpha = 1;
  if (!MOVING && S.mpp < 3 && S.ly.road) drawChevrons(cand, lim);
  // The other layers recede while a pair's trips are shown (S305), as the road traffic does.
  LAYER_ALPHA = dimming ? 0.3 : 1;
  if (S.ly.bike && FLOWS.bike) drawFlow(FLOWS.bike, TH.modes[1], [9, 5]);
  if (S.ly.walk && FLOWS.walk) drawFlow(FLOWS.walk, TH.modes[2], [2, 4]);
  ctx.globalAlpha = LAYER_ALPHA;
  if (S.ly.transit && LY.transit) drawTransit();
  if (S.ly.parking && LY.parking) drawParking();
  ctx.globalAlpha = 1; LAYER_ALPHA = 1;
  if (S.pair >= 0 && S.routes && HAS_ROUTES) drawTrips();
  if (S.picking) drawPick();
  if (S.spill && SB) drawSpill(cand);
  highlight(S.link, TH.ink, 3); if (S.hover !== S.link) highlight(S.hover, TH.ink2, 2);
  highlightItem(S.hoverItem);
  drawScale();
  document.body.dataset.room = roomPx().toFixed(1); document.body.dataset.wmax = ribbonMax().toFixed(1);
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
  ctx.globalAlpha = 0.45 * LAYER_ALPHA; ctx.strokeStyle = colour; ctx.lineWidth = 0.6; ctx.stroke(faint); ctx.globalAlpha = LAYER_ALPHA;
  const keys = [...buckets.keys()].sort((a, b) => a - b);
  for (const k of keys) { ctx.strokeStyle = TH.surface; ctx.lineWidth = k / 2 + 1.4; ctx.stroke(buckets.get(k)); }
  // Over road traffic, dashed (cyclists) or dotted (walkers): another mode, not another road colour.
  ctx.setLineDash(S.ly.road && HAS_TRAFFIC ? dash : []);
  for (const k of keys) { ctx.strokeStyle = colour; ctx.lineWidth = k / 2; ctx.stroke(buckets.get(k)); }
  ctx.setLineDash([]); ctx.globalAlpha = 1;
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
  const mark = { road: '<i class="sw ramp" style="background:linear-gradient(90deg,' + TH.ramp_delay.join(",") + ')"></i>', transit: '<i class="sw" style="border-top:3px solid ' + TH.modes[3] + '"></i>', parking: '<i class="sw box"></i>', bike: '<i class="sw" style="border-top:3px dashed ' + TH.modes[1] + '"></i>', walk: '<i class="sw" style="border-top:3px dotted ' + TH.modes[2] + '"></i>' };
  $("layerrows").innerHTML = rows.map(([k, name, hint]) => `<label title="${hint}"><input type="checkbox" data-layer="${k}"${S.ly[k] ? " checked" : ""}> ${mark[k]} ${name}</label>`).join("");
  document.querySelectorAll("#layerrows input").forEach((box) => box.addEventListener("change", (e) => {
    const k = e.target.dataset.layer; S.ly[k] = e.target.checked;
    $("trafficgroup").hidden = !HAS_TRAFFIC || !S.ly.road; renderLegend();
    // The pointer picks in the layer ticked last that is still on (S269).
    S.lyOrder = S.lyOrder.filter((x) => x !== k); if (e.target.checked) S.lyOrder.push(k);
    S.hover = -1; S.hoverItem = null; redraw(false); writeHash();
  }));
}
function drawChevrons(cand, lim) {
  ctx.fillStyle = TH.surface; ctx.globalAlpha = 0.9;
  for (const l of cand) {
    if (!(vol[l] > 0) || CLS[l] > lim) continue;
    const n = screenLine(l, ribbonOffset(l, ribbonWidth(l)), 1.2); if (n < 2) continue;
    let len = 0; for (let i = 0; i < n - 1; i++) len += Math.hypot(PX[i + 1] - PX[i], PY[i + 1] - PY[i]); if (len < 26) continue;
    const m = (n - 1) >> 1, dx = PX[m + 1] - PX[m], dy = PY[m + 1] - PY[m], d = Math.hypot(dx, dy) || 1, tx = dx / d, ty = dy / d;
    const cx = (PX[m] + PX[m + 1]) / 2, cy = (PY[m] + PY[m + 1]) / 2, s = ribbonWidth(l) * 0.42 + 0.9;
    ctx.beginPath(); ctx.moveTo(cx + tx * s, cy + ty * s); ctx.lineTo(cx - tx * s * 0.6 - ty * s * 0.75, cy - ty * s * 0.6 + tx * s * 0.75); ctx.lineTo(cx - tx * s * 0.6 + ty * s * 0.75, cy - ty * s * 0.6 - tx * s * 0.75); ctx.closePath(); ctx.fill();
  }
  ctx.globalAlpha = 1;
}
function drawSpill(cand) {
  const p = new Path2D(); let any = false;
  for (const l of cand) if (SB[l] && addLine(p, l, hasRibbon(l) ? ribbonOffset(l, ribbonWidth(l)) : 0, 0.5)) any = true;
  if (!any) return;
  ctx.setLineDash([5, 4]); ctx.strokeStyle = TH.surface; ctx.lineWidth = 5; ctx.stroke(p);
  ctx.strokeStyle = TH.ink; ctx.lineWidth = 2.5; ctx.stroke(p); ctx.setLineDash([]);
}
function highlight(l, colour, w) {
  if (l < 0) return;
  // A ribbon is outlined on its own side, so the direction picked shows (S306); a link without one, on its line.
  if (hasRibbon(l)) {
    const rw = ribbonWidth(l), p = new Path2D(); if (!addLine(p, l, ribbonOffset(l, rw), 0.5)) return;
    ctx.lineCap = "butt"; ctx.strokeStyle = colour; ctx.lineWidth = rw + 2 * w; ctx.stroke(p);
    ctx.strokeStyle = TABLES[S.colour][colourIndex(l)]; ctx.lineWidth = rw; ctx.stroke(p); ctx.lineCap = "round";
    return;
  }
  const p = new Path2D(); if (!addLine(p, l, 0, 0.5)) return;
  ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 3; ctx.stroke(p); ctx.strokeStyle = colour; ctx.lineWidth = w; ctx.stroke(p);
}
// ---- trips between two places (S305) ----
// A mode's colour, as the mode-share chart draws it; a leg's colour, by what it is.
const MODE_INDEX = { car: 0, bike: 1, walk: 2, transit: 3, car_transit: 4, bike_transit: 5 };
function modeColour(i) { const m = TR.modes[i], sv = TOK.services[S.theme] || {}; return sv[m] || TH.modes[MODE_INDEX[m] !== undefined ? MODE_INDEX[m] : 3]; }
function legColour(g, o) {
  const k = A.lk[g], m = TR.modes[A.om[o]], sv = TOK.services[S.theme] || {};
  if (k === LEG.drive) return m === "taxi" ? sv.taxi : m === "car" ? carShade(o) : TH.modes[0];
  if (k === LEG.bike) return m.startsWith("bike_share") ? sv.bike_share : TH.modes[1];
  if (k === LEG.ride) return TH.modes[3];
  return TH.modes[2];
}
function kindColour(line) { const k = KIND_NAMES.indexOf(TR.line_kinds[line]); return TH.kinds[k >= 0 ? k : 5]; }
// A leg's points in metres: along its links, its line's stops, or straight between two places.
function legPoints(g) {
  const k = A.lk[g], out = [];
  if (k === LEG.drive || k === LEG.bike || k === LEG.walk) {
    const G = k === LEG.drive ? { vs: VS, xs: X, ys: Y } : GEO[k === LEG.bike ? "bike" : "walk"];
    if (!G) return out;
    let wx = NaN, wy = NaN;
    for (let i = A.la[g]; i < A.lb[g]; i++) {
      const l = A.pl[i];
      for (let v = G.vs[l]; v < G.vs[l + 1]; v++) {
        if (v === G.vs[l] && G.xs[v] === wx && G.ys[v] === wy) continue;
        out.push(G.xs[v], G.ys[v]); wx = G.xs[v]; wy = G.ys[v];
      }
    }
  } else if (k === LEG.ride) { for (let i = A.la[g]; i < A.lb[g]; i++) { const st = A.pst[i]; out.push(A.tsx[st] * 0.1, A.tsy[st] * 0.1); } }
  else if (k === LEG.connect && A.la[g] !== NONE) out.push(A.ptx[A.la[g]] * 0.1, A.pty[A.la[g]] * 0.1, A.ptx[A.lb[g]] * 0.1, A.pty[A.lb[g]] * 0.1);
  return out;
}
let LEGPTS = new Map();   // the selected pair's legs' points, made once per pair
function pairOptions(p) { const out = []; for (let o = A.qs[p]; o < A.qs[p + 1]; o++) out.push(o); return out; }
// A pair shows its most travelled journeys, MAX_SHOWN of them until asked for all (S305: fourteen car routes side
// by side were a tangle); the car routes nobody took only when asked for.
const MAX_SHOWN = 6;
function shownOption(o) {
  if (S.hidden.has(A.om[o])) return false;
  if (!(A.on[o] > 0)) return S.allCar;
  return S.allOpts || o - A.qs[OPT_PAIR[o]] < MAX_SHOWN;
}
// A car route's colour: the car's, paler the less it is travelled (by its rank among the pair's car routes).
function carShade(o) { const r = carRank(o) - 1; return r === 0 ? TH.modes[0] : mix(TH.modes[0], TH.surface, Math.min(0.55, 0.12 * r)); }
function optColour(o) { return TR.modes[A.om[o]] === "car" ? carShade(o) : modeColour(A.om[o]); }
function mix(a, b, f) { const x = hex(a), y = hex(b); return "rgb(" + [0, 1, 2].map((i) => Math.round(x[i] + (y[i] - x[i]) * f)).join(",") + ")"; }
// The screen path of points `pts` (metres), moved `off` px to the right of travel; its middle.
function screenPath(pts, off) {
  const n0 = pts.length / 2; ensure(n0 + 2);
  let n = 0, lx = 0, ly = 0;
  for (let i = 0; i < n0; i++) {
    const x = sx(pts[2 * i]), y = sy(pts[2 * i + 1]);
    if (n === 0 || i === n0 - 1 || Math.abs(x - lx) + Math.abs(y - ly) >= 1.2) { PX[n] = x; PY[n] = y; n++; lx = x; ly = y; }
  }
  offsetLine(n, off);
  const path = new Path2D(); if (n < 2) return { path, n, mid: null };
  path.moveTo(PX[0], PY[0]); for (let i = 1; i < n; i++) path.lineTo(PX[i], PY[i]);
  // The point halfway along, for a badge.
  let len = 0; for (let i = 0; i < n - 1; i++) len += Math.hypot(PX[i + 1] - PX[i], PY[i + 1] - PY[i]);
  let half = len / 2, mid = [PX[0], PY[0]];
  for (let i = 0; i < n - 1; i++) { const d = Math.hypot(PX[i + 1] - PX[i], PY[i + 1] - PY[i]); if (half <= d) { const f = d ? half / d : 0; mid = [PX[i] + (PX[i + 1] - PX[i]) * f, PY[i] + (PY[i + 1] - PY[i]) * f]; break; } half -= d; }
  return { path, n, mid, first: [PX[0], PY[0]], last: [PX[n - 1], PY[n - 1]], len };
}
function badge(x, y, text, fill) {
  ctx.font = "700 10px " + FONT; const w = Math.max(16, ctx.measureText(text).width + 8), h = 15;
  // Corners cut, not rounded: straight segments draw alike on the screen and in a vector export.
  ctx.beginPath(); const r = 3, x0 = x - w / 2, y0 = y - h / 2;
  ctx.moveTo(x0 + r, y0); ctx.lineTo(x0 + w - r, y0); ctx.lineTo(x0 + w, y0 + r); ctx.lineTo(x0 + w, y0 + h - r); ctx.lineTo(x0 + w - r, y0 + h); ctx.lineTo(x0 + r, y0 + h); ctx.lineTo(x0, y0 + h - r); ctx.lineTo(x0, y0 + r); ctx.closePath();
  ctx.fillStyle = fill; ctx.fill(); ctx.strokeStyle = TH.surface; ctx.lineWidth = 1.5; ctx.stroke();
  ctx.fillStyle = "#fff"; ctx.textAlign = "center"; ctx.textBaseline = "middle"; ctx.fillText(text, x, y + 0.5);
}
function ring(x, y, r, fill, line) { ctx.beginPath(); ctx.arc(x, y, r, 0, 6.2832); ctx.fillStyle = fill; ctx.fill(); ctx.strokeStyle = line; ctx.lineWidth = 1.6; ctx.stroke(); }
function carRank(o) { let r = 0; for (let q = A.qs[OPT_PAIR[o]]; q < o; q++) if (TR.modes[A.om[q]] === "car") r++; return r + 1; }
function drawTrips() {
  const p = S.pair, opts = pairOptions(p).filter(shownOption);
  let most = 1; for (const o of opts) most = Math.max(most, A.on[o]);
  // The order drawn: nobody's routes first, the most travelled last, the hovered on top.
  opts.sort((a, b) => (a === S.hoverOpt) - (b === S.hoverOpt) || A.on[a] - A.on[b]);
  // Each option's width grows linearly with its travellers (S306), the widest a quarter of the room the roads leave
  // on screen (as the ribbons'), 4 to 16 px; a route nobody took 1.6 px.
  const room = roomPx(), top = Math.max(4, Math.min(16, 0.25 * room));
  const width = (o) => (A.on[o] > 0 ? 1.5 + (top - 1.5) * A.on[o] / most : 1.6);
  // Options on the same network side by side, the most travelled nearest the line, each beside the one before by
  // its own width and 1.5 px (S306: fixed steps let a wide option cover its neighbours); all of them within a third
  // of the room (16 px at least), narrower in proportion if not.
  const fam = new Map(), acc = { r: 0, b: 0, w: 0, t: 0 };
  for (const o of [...opts].sort((a, b) => A.on[b] - A.on[a] || a - b)) {
    const uses = { r: 0, b: 0, w: 0, t: 0 }, f = { r: 0, b: 0, w: 0, t: 0 };
    for (let g = A.ol[o]; g < A.ol[o + 1]; g++) { const k = A.lk[g]; if (k === LEG.drive) uses.r = 1; else if (k === LEG.bike) uses.b = 1; else if (k === LEG.walk) uses.w = 1; else if (k === LEG.ride) uses.t = 1; }
    for (const x in uses) if (uses[x]) { f[x] = acc[x] + width(o) / 2 + 1; acc[x] += width(o) + 1.5; }
    fam.set(o, f);
  }
  const fit = Math.min(1, Math.max(16, 0.35 * room) / Math.max(acc.r, acc.b, acc.w, acc.t, 1));
  const badges = [], stops = [], parks = [];
  for (const o of opts) {
    const hl = S.hoverOpt === o, dim = S.hoverOpt >= 0 && !hl, unused = !(A.on[o] > 0);
    const w = Math.max(1.2, width(o) * fit) + (hl ? 1.2 : 0), f = fam.get(o);
    ctx.globalAlpha = dim ? 0.16 : 1;
    // From the door to where the journey meets its network, and from there to the door, when they are apart:
    // a thin dotted straight line (S305).
    let firstPt = null, lastPt = null;
    for (let g = A.ol[o]; g < A.ol[o + 1]; g++) { const pts = LEGPTS.get(g); if (pts && pts.length >= 2) { if (!firstPt) firstPt = [pts[0], pts[1]]; lastPt = [pts[pts.length - 2], pts[pts.length - 1]]; } }
    for (const [door, at] of [[[A.qox[p] * 0.1, A.qoy[p] * 0.1], firstPt], [[A.qdx[p] * 0.1, A.qdy[p] * 0.1], lastPt]]) {
      if (!at || Math.hypot(door[0] - at[0], door[1] - at[1]) < 25) continue;
      const path = new Path2D(); path.moveTo(sx(door[0]), sy(door[1])); path.lineTo(sx(at[0]), sy(at[1]));
      ctx.lineCap = "round"; ctx.setLineDash([0.1, 4.6]); ctx.strokeStyle = TH.modes[2]; ctx.lineWidth = 2; ctx.stroke(path); ctx.setLineDash([]);
    }
    let prevEnd = null;
    for (let g = A.ol[o]; g < A.ol[o + 1]; g++) {
      const k = A.lk[g];
      if (k === LEG.park) { if (prevEnd) parks.push({ at: prevEnd, dim }); continue; }
      const pts = LEGPTS.get(g); if (!pts || pts.length < 4) continue;
      const fi = k === LEG.drive ? f.r : k === LEG.bike ? f.b : k === LEG.walk ? f.w : k === LEG.ride ? f.t : 0;
      const sp = screenPath(pts, k === LEG.connect ? 0 : fi * fit); if (sp.n < 2) continue;
      const c = legColour(g, o), dotted = k === LEG.walk || k === LEG.connect;
      if (dotted) {
        // A walk along its streets as wide as the option; a walk to or from a stop (straight: its streets
        // are not stored) thinner.
        const ww = k === LEG.connect ? 2.6 : Math.max(w, 2.6);
        ctx.lineCap = "round"; ctx.setLineDash([0.1, ww + 2.6]);
        ctx.strokeStyle = TH.surface; ctx.lineWidth = ww + 2.2; ctx.stroke(sp.path);
        ctx.strokeStyle = c; ctx.lineWidth = ww; ctx.stroke(sp.path); ctx.setLineDash([]);
      } else {
        if (unused) ctx.setLineDash([6, 4]);
        ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 2.6; ctx.stroke(sp.path);
        ctx.strokeStyle = c; ctx.lineWidth = w; ctx.stroke(sp.path); ctx.setLineDash([]);
      }
      if (k === LEG.ride) {
        stops.push({ at: sp.first, dim }, { at: sp.last, dim });
        if (!dim && sp.len > 34) badges.push([sp.mid, TR.lines[A.lline[g]] || "", kindColour(A.lline[g])]);
      }
      if (k === LEG.drive && TR.modes[A.om[o]] === "car" && !dim && sp.len > 40 && (hl || opts.length <= 8)) badges.push([sp.mid, String(carRank(o)), TH.modes[0]]);
      prevEnd = sp.last;
    }
  }
  ctx.globalAlpha = 1;
  for (const s0 of stops) { ctx.globalAlpha = s0.dim ? 0.25 : 1; ring(s0.at[0], s0.at[1], 3.6, TH.surface, TH.ink2); }
  for (const pk of parks) {
    ctx.globalAlpha = pk.dim ? 0.25 : 1; const [x, y] = pk.at, r = 7;
    ctx.fillStyle = TH.ink; ctx.fillRect(x - r, y - r, 2 * r, 2 * r); ctx.strokeStyle = TH.surface; ctx.lineWidth = 1.5; ctx.strokeRect(x - r, y - r, 2 * r, 2 * r);
    ctx.fillStyle = TH.surface; ctx.font = "700 10px " + FONT; ctx.textAlign = "center"; ctx.textBaseline = "middle"; ctx.fillText("P", x, y + 0.5);
  }
  ctx.globalAlpha = 1;
  for (const [at, text, fill] of badges) badge(at[0], at[1], text, fill);
  const ends = [[A.qox[p], A.qoy[p], "A"], [A.qdx[p], A.qdy[p], "B"]];
  for (const [qx, qy, lab] of ends) {
    const x = sx(qx * 0.1), y = sy(qy * 0.1);
    ring(x, y, 9, TH.ink, TH.surface);
    ctx.fillStyle = TH.surface; ctx.font = "bold 10px " + FONT; ctx.textAlign = "center"; ctx.textBaseline = "middle"; ctx.fillText(lab, x, y + 0.5);
  }
}
// Picking a pair on the map: the places pairs start from, then the destinations from the one picked.
function pickPlaces() {
  const out = new Map();
  for (let p = 0; p < NP; p++) {
    if (S.picking === 2 && (A.qox[p] !== S.pickFrom[0] || A.qoy[p] !== S.pickFrom[1])) continue;
    const x = S.picking === 1 ? A.qox[p] : A.qdx[p], y = S.picking === 1 ? A.qoy[p] : A.qdy[p], key = x + "," + y;
    const e = out.get(key); if (e) e.n += A.qn[p]; else out.set(key, { x, y, n: A.qn[p], p });
  }
  return [...out.values()];
}
function drawPick() {
  const places = pickPlaces(); let most = 1; for (const e of places) most = Math.max(most, e.n);
  for (const e of places) { const r = 3 + 5 * Math.sqrt(e.n / most); ring(sx(e.x * 0.1), sy(e.y * 0.1), r, S.picking === 1 ? TH.ink : TH.mark, TH.surface); }
  if (S.picking === 2) { const x = sx(S.pickFrom[0] * 0.1), y = sy(S.pickFrom[1] * 0.1); ring(x, y, 9, TH.ink, TH.surface); ctx.fillStyle = TH.surface; ctx.font = "bold 10px " + FONT; ctx.textAlign = "center"; ctx.textBaseline = "middle"; ctx.fillText("A", x, y + 0.5); }
}
function pickAt(px, py) {
  const places = pickPlaces(); let best = null, bd = 28;
  for (const e of places) { const d = Math.hypot(sx(e.x * 0.1) - px, sy(e.y * 0.1) - py); if (d < bd) { bd = d; best = e; } }
  if (!best) return;
  if (S.picking === 1) { S.picking = 2; S.pickFrom = [best.x, best.y]; pickHint(); redraw(false); return; }
  S.picking = 0; S.pickFrom = null; delete document.body.dataset.picking; selectPair(best.p, true);
}
function pickHint() {
  $("routehint").hidden = false;
  $("routehint").textContent = S.picking === 1 ? "Click where the trips start (dots: the places, sized by travellers; Esc to stop)." : S.picking === 2 ? "Now click where they end." : $("routehint").dataset.text;
}
const FONT = getComputedStyle(document.body).fontFamily;
function niceNumber(x) { const p = Math.pow(10, Math.floor(Math.log10(x))); for (const m of [5, 2, 1]) if (m * p <= x) return m * p; return p; }
function drawScale() {
  const m = niceNumber(W * S.mpp * 0.11), px = m / S.mpp;
  $("scalebar").style.width = px + "px"; $("scalelabel").textContent = m >= 1000 ? (m / 1000) + " km" : m + " m";
}

// ---------------------------------------------------------- interaction ----
// The distance (m) from (mx, my) to link l's line moved `off` metres to the right of its travel (S306: each
// direction of a two-way road is picked on its own side).
function distToLink(l, mx, my, off) {
  let best = Infinity;
  for (let v = VS[l]; v < VS[l + 1] - 1; v++) {
    let ax = X[v], ay = Y[v], bx = X[v + 1], by = Y[v + 1]; const dx = bx - ax, dy = by - ay, L = dx * dx + dy * dy;
    if (off && L > 0) { const m = Math.sqrt(L), nx = dy / m * off, ny = -dx / m * off; ax += nx; ay += ny; bx += nx; by += ny; }
    let t = L > 0 ? ((mx - ax) * dx + (my - ay) * dy) / L : 0; t = Math.max(0, Math.min(1, t));
    const d = Math.hypot(mx - (ax + t * dx), my - (ay + t * dy)); if (d < best) best = d;
  }
  return best;
}
function pick(px, py) {
  const mx = S.cx + (px - W / 2) * S.mpp, my = S.cy - (py - H / 2) * S.mpp;
  // A ribbon is picked anywhere on it or within a few px of its edge; a bare line within 7 px.
  let best = -1, bd = Infinity; const lim = classLimit(), reach = (7 + (HAS_TRAFFIC ? GAP / 2 + ribbonWidthOf(1) : 0)) * S.mpp;
  for (const l of candidates(mx - reach, my - reach, mx + reach, my + reach)) {
    if (CLS[l] > lim && !(vol[l] > 0)) continue; if (CLS[l] >= 14 && S.mpp > 2.5) continue;
    const ribbon = hasRibbon(l), w = ribbon ? ribbonWidth(l) : 0;
    const d = distToLink(l, mx, my, ribbon ? ribbonOffset(l, w) * S.mpp : 0) - (ribbon ? w / 2 * S.mpp : 0);
    if (d < bd && d < (ribbon ? 4 : 7) * S.mpp) { bd = d; best = l; }
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
// A hover card as HTML, every string from the run's data (stop names, parking ids) escaped (P-44).
function escHtml(x) { return String(x).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"); }
function tipHtml(head, rows) { return "<b>" + escHtml(head) + "</b>" + rows.map((r) => "<div><span>" + escHtml(r[0]) + "</span>" + escHtml(r[1]) + "</div>").join(""); }
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
  t.innerHTML = tipHtml(head, rows);
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
  if (NF) rows.push(["Direction", "from node " + NF[l] + " to node " + NT[l]]);
  if (TWIN[l] >= 0) rows.push(["Other direction", "link " + TWIN[l] + (HAS_TRAFFIC ? ", " + fmt(vol[TWIN[l]]) + " PCU/h" : "")]);
  if (HAS_TRAFFIC) { rows.push(["Volume", fmt(vol[l]) + " PCU/h"]); if (vol[l] > 0) { rows.push(["Delay", fmt(delay[l] * 100) + "% over free flow"]); rows.push(["Mean time", meanT[l].toFixed(1) + " s (free flow " + (FF[l] + SW[l]).toFixed(1) + " s" + (SW[l] > 0 ? ", the signal's mean wait of " + SW[l].toFixed(1) + " s included" : "") + ")"]); } }
  if (CAP[l] > 0) rows.push(["Capacity", fmt(CAP[l]) + " PCU/h" + (SG[l] ? " at its signal" : "") + (vol[l] > 0 ? " (" + fmt(vol[l] / CAP[l] * 100) + "% used)" : "")]);
  if (HAS_ROUTES) { let n = 0; for (let i = LOPT_START[l]; i < LOPT_START[l + 1]; i++) n += A.on[LOPTS[i]]; if (n) rows.push(["Trips shown", fmt(n) + " travellers drive on it (click for their pairs)"]); }
  t.innerHTML = tipHtml("link " + l, rows);
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
canvas.addEventListener("pointerup", (e) => {
  if (drag && !drag.moved && S.picking) pickAt(e.offsetX, e.offsetY);
  else if (drag && !drag.moved && hoverLayer() === "road") selectLink(pick(e.offsetX, e.offsetY));
  drag = null; writeHash();
});
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
function minutes(sec) { const m = sec / 60; return m < 10 ? m.toFixed(1) : String(Math.round(m)); }
function placeName(p, end) { const n = TR.places[p][end]; return n || (end ? "destination" : "origin"); }
function pairText(p) { return "#" + (p + 1) + " · " + placeName(p, 0) + " → " + placeName(p, 1) + " · " + fmt(A.qn[p]) + (A.qn[p] === 1 ? " traveller" : " travellers"); }
function selectPair(k, fit) {
  if (!HAS_ROUTES) return;
  S.pair = k; S.hoverOpt = -1; LEGPTS = new Map(); renderLegend();
  if (k >= 0) {
    let x0 = Math.min(A.qox[k], A.qdx[k]) * 0.1, x1 = Math.max(A.qox[k], A.qdx[k]) * 0.1, y0 = Math.min(A.qoy[k], A.qdy[k]) * 0.1, y1 = Math.max(A.qoy[k], A.qdy[k]) * 0.1;
    for (const o of pairOptions(k)) for (let g = A.ol[o]; g < A.ol[o + 1]; g++) {
      const pts = legPoints(g); LEGPTS.set(g, pts);
      if (A.on[o] > 0) for (let i = 0; i < pts.length; i += 2) { x0 = Math.min(x0, pts[i]); x1 = Math.max(x1, pts[i]); y0 = Math.min(y0, pts[i + 1]); y1 = Math.max(y1, pts[i + 1]); }
    }
    if (fit !== false) { fitTo(x0, y0, x1, y1, 0.22); S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, S.mpp)); }
  }
  $("pairpick").value = String(k); tripPanel(); renderLegend(); redraw(false); levelButtons(); writeHash();
}
// A journey's legs as a strip, each as wide as its share of the time (S305: as a journey planner shows them).
function strip(o) {
  const legs = []; let total = 0;
  for (let g = A.ol[o]; g < A.ol[o + 1]; g++) { const sec = A.lsec[g]; if (sec > 0) { legs.push(g); total += sec; } }
  if (!total) return "";
  const word = { 0: "drive", 1: "ride a bike", 2: "walk", 3: "ride", 4: "walk", 5: "wait", 6: "park, or reach the parked vehicle" };
  return '<div class="strip">' + legs.map((g) => {
    const k = A.lk[g], c = legColour(g, o), sec = A.lsec[g], share = sec / total;
    let cls = "", style = "flex:" + share.toFixed(4) + " 1 0;", text = "";
    if (k === LEG.walk || k === LEG.connect) { cls = "walk"; style += "--c:" + c; }
    else if (k === LEG.wait) cls = "wait";
    else if (k === LEG.park) cls = "park";
    else { style += "background:" + c; if (k === LEG.ride && share > 0.12) text = escHtml(TR.lines[A.lline[g]] || ""); }
    const what = k === LEG.ride ? "ride " + (TR.lines[A.lline[g]] || "") : word[k];
    return '<span class="' + cls + '" style="' + style + '" title="' + escHtml(what) + " · " + minutes(sec) + ' min">' + text + "</span>";
  }).join("") + "</div>";
}
function optionTitle(o) {
  const m = TR.modes[A.om[o]];
  if (m === "car") return "Car · route " + carRank(o);
  return TR.labels[o];
}
function tripPanel() {
  const box = $("tripcard");
  if (S.pair < 0) { box.innerHTML = ""; $("routehint").hidden = false; pickHint(); return; }
  $("routehint").hidden = true;
  const p = S.pair, total = A.qn[p], opts = pairOptions(p), byMode = new Map();
  for (const o of opts) byMode.set(A.om[o], (byMode.get(A.om[o]) || 0) + A.on[o]);
  const km = Math.hypot(A.qdx[p] - A.qox[p], A.qdy[p] - A.qoy[p]) * 0.1 / 1000;
  let h = '<div class="od"><span class="mk">A</span><b>' + escHtml(placeName(p, 0)) + '</b> → <span class="mk">B</span><b>' + escHtml(placeName(p, 1)) + "</b></div>";
  h += '<div class="odsub">' + fmt(total) + (total === 1 ? " traveller" : " travellers") + " · " + km.toFixed(1) + " km apart as the crow flies</div>";
  const modes = [...byMode.entries()].filter(([, n]) => n > 0).sort((a, b) => b[1] - a[1]);
  const all = total || 1;
  h += '<div class="split" title="How the pair\'s travellers went, by mode">' + modes.map(([m, n]) => '<span style="width:' + (100 * n / all).toFixed(2) + "%;background:" + modeColour(m) + '" title="' + escHtml(TR.mode_names[m]) + " " + Math.round(100 * n / all) + '%"></span>').join("") + "</div>";
  h += '<div class="chips">' + modes.map(([m, n]) => '<span class="chip' + (S.hidden.has(m) ? " off" : "") + '" data-m="' + m + '" title="Show or hide on the map"><i style="background:' + modeColour(m) + '"></i>' + escHtml(TR.mode_names[m]) + " " + Math.round(100 * n / all) + "%</span>").join("") + "</div>";
  const unused = opts.filter((o) => !(A.on[o] > 0)).length;
  let minCost = Infinity; for (const o of opts) if (isFinite(A.oc[o])) minCost = Math.min(minCost, A.oc[o]);
  for (const o of opts) {
    if (!shownOption(o)) continue;
    const n = A.on[o], share = n / all, used = n > 0;
    let sub = used ? fmt(n) + (n === 1 ? " traveller" : " travellers") + " · " + Math.round(share * 100) + "%" : "in the route set, not taken";
    let right = "";
    if (isFinite(A.oc[o]) && A.oc[o] > 0 && TR.modes[A.om[o]] === "car") right = '<span title="The route\'s cost over the pair\'s cheapest car route; its overlap with the other routes, by length">+' + Math.round((A.oc[o] / minCost - 1) * 100) + "% · overlap " + Math.round(A.oo[o] * 100) + "%</span>";
    h += '<div class="opt' + (used ? "" : " unused") + (S.hoverOpt === o ? " hl" : "") + '" data-o="' + o + '"><div class="top"><i class="dot" style="background:' + optColour(o) + '"></i><b>' + escHtml(optionTitle(o)) + '</b><span class="t">' + (used && isFinite(A.ot[o]) ? minutes(A.ot[o]) + " min" : "") + "</span></div>" +
      (used ? strip(o) : "") + '<div class="sub"><span>' + sub + "</span>" + right + "</div>" + (used ? '<div class="share"><span style="width:' + (100 * share).toFixed(1) + "%;background:" + optColour(o) + '"></span></div>' : "") + "</div>";
  }
  const usedCount = opts.filter((o) => A.on[o] > 0 && !S.hidden.has(A.om[o])).length;
  if (usedCount > MAX_SHOWN) h += '<div class="more"><a id="toggleall">' + (S.allOpts ? "Show the " + MAX_SHOWN + " most travelled only" : "Show all " + usedCount + " journeys") + "</a></div>";
  if (A.qr[p] > 0) h += '<div class="more">' + fmt(A.qr[p]) + (A.qr[p] === 1 ? " traveller" : " travellers") + " took other ways, each under 1 % of the pair.</div>";
  if (unused) h += '<div class="more"><a id="togglecar">' + (S.allCar ? "Hide" : "Show") + " the " + unused + " car route" + (unused === 1 ? "" : "s") + " nobody took</a></div>";
  h += '<div class="foot">Times are what the travellers expected door to door when they chose (for a trip given the car, the drive time of its route), averaged over them; the strip splits it into walking, waiting, riding, driving and parking. Hover a journey to see it alone; click it to zoom to it.</div>';
  box.innerHTML = h;
  box.querySelectorAll(".chip").forEach((c) => c.addEventListener("click", () => { const m = +c.dataset.m; if (S.hidden.has(m)) S.hidden.delete(m); else S.hidden.add(m); S.hoverOpt = -1; tripPanel(); redraw(false); writeHash(); }));
  box.querySelectorAll(".opt").forEach((row) => {
    const o = +row.dataset.o;
    row.addEventListener("pointerenter", () => { S.hoverOpt = o; redraw(false); });
    row.addEventListener("pointerleave", () => { S.hoverOpt = -1; redraw(false); });
    row.addEventListener("click", () => {
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      for (let g = A.ol[o]; g < A.ol[o + 1]; g++) { const pts = LEGPTS.get(g) || []; for (let i = 0; i < pts.length; i += 2) { x0 = Math.min(x0, pts[i]); x1 = Math.max(x1, pts[i]); y0 = Math.min(y0, pts[i + 1]); y1 = Math.max(y1, pts[i + 1]); } }
      if (isFinite(x0)) { fitTo(x0, y0, x1, y1, 0.25); S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, S.mpp)); redraw(false); levelButtons(); writeHash(); }
    });
  });
  const ta = $("toggleall"); if (ta) ta.addEventListener("click", () => { S.allOpts = !S.allOpts; S.hoverOpt = -1; tripPanel(); redraw(false); writeHash(); });
  const t = $("togglecar"); if (t) t.addEventListener("click", () => { S.allCar = !S.allCar; tripPanel(); redraw(false); writeHash(); });
}
function selectLink(l) {
  S.link = l; const box = $("linkinfo");
  if (l < 0) { box.hidden = true; redraw(false); writeHash(); return; }
  box.hidden = false;
  let h = "<b>link " + l + "</b> · " + CLASSES[CLS[l]] + " · " + LEN[l].toFixed(0) + " m" + (NF ? "<div>from node " + NF[l] + " to node " + NT[l] + "</div>" : "");
  if (HAS_ROUTES) {
    const os = LOPTS.subarray(LOPT_START[l], LOPT_START[l + 1]), byPair = new Map();
    for (const o of os) if (A.on[o] > 0) byPair.set(OPT_PAIR[o], (byPair.get(OPT_PAIR[o]) || 0) + A.on[o]);
    let n = 0; for (const v of byPair.values()) n += v;
    h += "<div>" + fmt(n) + " travellers of " + byPair.size + " pair" + (byPair.size === 1 ? "" : "s") + " shown drive on it</div>";
    box.innerHTML = h;
    const list = document.createElement("div"); list.className = "pairs";
    [...byPair.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0]).slice(0, 12).forEach(([k, c]) => {
      const b = document.createElement("button"); b.textContent = placeName(k, 0) + " → " + placeName(k, 1) + " · " + fmt(c); b.title = pairText(k); b.addEventListener("click", () => selectPair(k, true)); list.appendChild(b);
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
// The legend (S305): a section for each layer that is on, and the trips' marks while a pair is shown.
function tripLegendItems() {
  const items = [["car", TH.modes[0], ""], ["transit ride", TH.modes[3], ""], ["bike", TH.modes[1], ""], ["walk", TH.modes[2], "dot"]];
  const sv = TOK.services[S.theme] || {}, present = new Set(pairOptions(S.pair).filter((o) => A.on[o] > 0).map((o) => TR.modes[A.om[o]]));
  if (present.has("taxi")) items.splice(1, 0, ["taxi", sv.taxi, ""]);
  if (present.has("bike_share") || present.has("bike_share_transit")) items.splice(items.length - 1, 0, ["shared bike", sv.bike_share, ""]);
  return items;
}
function renderLegend() {
  const line = (c, style) => '<i style="border-top:' + (style === "dash" ? "3px dashed " : style === "dot" ? "3px dotted " : "4px solid ") + c + '"></i>';
  const parts = [];
  if (HAS_ROUTES && S.pair >= 0 && S.routes) {
    parts.push('<div class="lg"><h3>Trips between two places</h3><div class="kinds">' + tripLegendItems().map(([name, c, st]) => "<span>" + line(c, st) + name + "</span>").join("") +
      '</div><div class="kinds"><span><em class="mkr">A</em>origin</span><span><em class="mkr">B</em>destination</span><span><em class="stop"></em>stop</span><span><em class="badge" style="background:' + (TR.lines.length ? kindColour(0) : TH.kinds[1]) + '">' + escHtml(TR.lines[0] || "M1") + '</em>line</span><span><em class="mkr" style="border-radius:2px">P</em>parking</span></div><div class="note">width = travellers; a walk to or from a stop is drawn straight; dashed: a car route nobody took</div></div>');
  }
  if (HAS_TRAFFIC && S.ly.road) {
    const r = roadLegend();
    const ramp = r.bands
      ? '<div class="bar" style="background:' + bandGradient(r.stops) + '"></div><div class="bands">' + r.bands.map((t) => "<span>" + t + "</span>").join("") + "</div>"
      : '<div class="bar" style="background:linear-gradient(90deg,' + r.stops.join(",") + ')"></div><div class="ends"><span>' + r.low + "</span><span>" + r.high + "</span></div>";
    parts.push('<div class="lg"><h3>Road traffic</h3><div class="sub">' + r.label + "</div>" + ramp + '<div class="note">width = volume per direction' + (RT ? "; grey margin: road capacity" : "") + "</div></div>");
  }
  if (S.ly.transit && LY.transit) parts.push('<div class="lg"><h3>Transit lines</h3><div class="kinds">' + transitKinds().map(([name, k]) => "<span>" + line(TH.kinds[k]) + name + "</span>").join("") + '</div><div class="note">width = passengers on board; ○ stops, size = boardings</div></div>');
  if (S.ly.bike && FLOWS.bike) parts.push('<div class="lg"><h3>Cyclists</h3><div class="kinds"><span>' + line(TH.modes[1], "dash") + 'cyclists</span></div><div class="note">width = cyclists; faint = the bike network</div></div>');
  if (S.ly.walk && FLOWS.walk) parts.push('<div class="lg"><h3>Walkers</h3><div class="kinds"><span>' + line(TH.modes[2], "dot") + 'walkers</span></div><div class="note">width = walkers; faint = the walk network</div></div>');
  if (S.ly.parking && LY.parking) parts.push('<div class="lg"><h3>Parkings</h3><div class="kinds"><span><b></b>car</span><span><b class="ci"></b>bike</span></div><div class="bar" style="background:linear-gradient(90deg,' + TABLES.delay.join(",") + ')"></div><div class="ends"><span>empty</span><span>full</span></div><div class="note">size = capacity; colour = busiest share in use</div></div>');
  $("legendbody").innerHTML = parts.join("");
  // The card always holds the compass and the scale (S306), the legend above them when a layer is on.
  $("legendbody").hidden = parts.length === 0;
}
function writeHash() {
  const p = new URLSearchParams(); p.set("t", S.theme); p.set("c", S.colour);
  p.set("b", S.bins[1] - S.bins[0] === NB ? "all" : S.bins[0] + (S.bins[1] - S.bins[0] > 1 ? "-" + S.bins[1] : ""));
  if (S.pair >= 0) p.set("p", S.pair); if (S.link >= 0) p.set("l", S.link);
  if (S.hidden.size) p.set("hm", [...S.hidden].join(".")); if (S.allCar) p.set("ac", "1"); if (S.allOpts) p.set("ao", "1");
  const on = Object.keys(LAYER_KEY).filter((k) => S.ly[k]).map((k) => LAYER_KEY[k]).join(""); if (on !== "r") p.set("ly", on || "-");
  p.set("z", [S.cx.toFixed(1), S.cy.toFixed(1), S.mpp.toFixed(3)].join(","));
  history.replaceState(null, "", "#" + p.toString()); document.body.dataset.view = p.toString();
}
function readHash() {
  const p = new URLSearchParams(location.hash.slice(1));
  if (p.get("t") && TOK[p.get("t")]) S.theme = p.get("t");
  if (["delay", "volume", "vc"].includes(p.get("c"))) S.colour = p.get("c");
  if (p.get("ly") !== null) for (const k of Object.keys(LAYER_KEY)) S.ly[k] = p.get("ly").includes(LAYER_KEY[k]);
  if (p.get("hm")) for (const m of p.get("hm").split(".")) if (m !== "") S.hidden.add(+m);
  if (p.get("ac") === "1") S.allCar = true;
  if (p.get("ao") === "1") S.allOpts = true;
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
    // A halo behind a label (the transit stops' names, S305: missing, an SVG export with transit on failed).
    strokeText(t, x, y) {
      const anchor = { center: "middle", right: "end", end: "end" }[r.textAlign] || "start";
      out.push(`<text x="${x}" y="${y}" fill="none" stroke="${r.strokeStyle}" stroke-width="${r.lineWidth}" stroke-linejoin="round" text-anchor="${anchor}" dominant-baseline="central" style="font:${esc(r.font)}"${style(r)}>${esc(t)}</text>`);
    },
    strokeRect(x, y, w, h) { out.push(`<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="none" stroke="${r.strokeStyle}" stroke-width="${r.lineWidth}"${style(r)}/>`); },
    measureText(t) { const c = document.createElement("canvas").getContext("2d"); c.font = r.font; return c.measureText(t); },
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
  if (HAS_ROUTES && S.pair >= 0 && S.routes) others.unshift({ title: "Trips " + placeName(S.pair, 0) + " → " + placeName(S.pair, 1) + " · width = travellers", items: tripLegendItems() });
  return { title: $("title").textContent, subtitle: $("subtitle").textContent, legend, others, provenance: $("provenance").textContent, source: $("source").textContent, credit: $("credit").hidden ? "" : $("credit").textContent };
}
const HEAD = 64, FOOT = 46;
// An export has no panel: what the screen shows beside it is centred in the export's whole width, at the same
// scale, instead of sitting left of centre beside an empty band where the panel was (R-10).
function viewWithMoving(drawInto) {
  const moving = MOVING, hover = S.hover, item = S.hoverItem, cx = S.cx; MOVING = false; S.hover = -1; S.hoverItem = null;
  S.cx -= panelWidth() / 2 * S.mpp; EXPORTING = true;
  try { drawInto(); } finally { MOVING = moving; S.hover = hover; S.hoverItem = item; S.cx = cx; EXPORTING = false; }
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
    const pick = $("pairpick"), shown = Math.min(NP, 500);
    pick.innerHTML = '<option value="-1">' + fmt(NP) + " pairs, the busiest first…</option>" + Array.from({ length: shown }, (_, k) => '<option value="' + k + '">' + escHtml(pairText(k)) + "</option>").join("");
    pick.addEventListener("change", (e) => selectPair(+e.target.value, true));
    $("prev").addEventListener("click", () => selectPair(S.pair <= 0 ? NP - 1 : S.pair - 1, true)); $("next").addEventListener("click", () => selectPair((S.pair + 1) % NP, true));
    $("pickmap").addEventListener("click", () => { S.picking = S.picking ? 0 : 1; S.pickFrom = null; if (S.picking) document.body.dataset.picking = "1"; else delete document.body.dataset.picking; pickHint(); redraw(false); });
    $("clear").addEventListener("click", () => { S.picking = 0; delete document.body.dataset.picking; selectPair(-1, false); selectLink(-1); });
  }
  if (SB) $("spill").addEventListener("change", (e) => { S.spill = e.target.checked; redraw(false); });
  window.addEventListener("resize", () => { resize(); redraw(false); });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape") { if (S.picking) { S.picking = 0; S.pickFrom = null; delete document.body.dataset.picking; pickHint(); redraw(false); return; } selectLink(-1); if (S.pair >= 0) selectPair(-1, false); } });
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
  $("routegroup").hidden = !HAS_ROUTES; $("routehint").dataset.text = $("routehint").textContent;
  $("spillrow").hidden = !SB; if (SB) $("spillcount").textContent = "(" + fmt(META.n_spillback) + ")";
  if (HAS_TRAFFIC) { $("bin").min = FIRST; $("bin").max = NB - 1; $("bin").value = FIRST; $("bin").disabled = true; }
  const p = readHash(); setTheme(S.theme); $("colour").value = S.colour; resize();
  if (HAS_TRAFFIC) { const all = S.bins[1] - S.bins[0] === NB; $("allbins").checked = all; $("bin").disabled = all; $("bin").value = all ? FIRST : S.bins[0]; }
  fitTo(EX0, EY0, EX1, EY1, 0.06); FIT.mpp = S.mpp; MAX_MPP = FIT.mpp * 1.6; MIN_MPP = Math.max(0.25, Math.min(0.5, FIT.mpp / 200));
  if (HAS_TRAFFIC) aggregate(S.bins[0], S.bins[1]); subtitle();
  layerRows(); wire(); $("trafficgroup").hidden = !HAS_TRAFFIC || !S.ly.road;
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
