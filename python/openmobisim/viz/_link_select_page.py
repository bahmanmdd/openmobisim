# ruff: noqa: E501
"""The page of the select-link map: HTML shell, its own styles and script.

The stylesheet adds to the interactive map's (``PAGE_CSS``), so the two pages look alike. The
script needs no library and no network: it reads the data the builder embeds and draws with a
canvas.
"""

SELECT_HTML = r"""<!doctype html>
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
  <p id="subtitle"></p>
</header>
<div id="side">
<aside id="panel">
  <div class="row"><button id="zout" title="Zoom out">−</button><button id="zin" title="Zoom in">+</button><button id="fit">Fit</button><span class="grow"></span><button id="theme">Night</button></div>
  <div id="layerrow" hidden>
    <h2>Layer</h2>
    <div class="row"><label><input type="radio" name="layer" value="road" checked> Roads (cars, taxis)</label><label><input type="radio" name="layer" value="bike"> Bike network</label></div>
  </div>
  <h2>Departures</h2>
  <div class="row"><input id="bin" type="range" min="0" max="0" value="0" step="1" disabled></div>
  <div class="row"><label><input id="allbins" type="checkbox" checked> All departures</label><span class="grow"></span><span id="binlabel"></span></div>
  <p class="hint">By the time each trip left its origin, not the time it reached the link.</p>
  <h2>Selected link</h2>
  <div id="linkinfo"><p class="hint">Click a link on the map.</p></div>
  <div id="tablebox"></div>
</aside>
<div id="legendgroup" class="card">
  <div id="legendbody"></div>
  <div id="compass"><svg width="18" height="28" viewBox="0 0 22 34"><path d="M11 2 L17 20 L11 16 L5 20 Z" fill="currentColor"/><text x="11" y="32" text-anchor="middle" font-size="10" font-weight="700" fill="currentColor" font-family="sans-serif">N</text></svg><div><div id="scalelabel"></div><div id="scalebar"></div></div></div>
</div>
</div>
<footer><span id="provenance"></span><span id="source"></span><span class="right"><span id="logo"><svg class="mark" viewBox="0 0 64 64" aria-hidden="true"><path d="__LOGO__"/></svg><b>openmobisim</b></span></span></footer>
<script id="meta" type="application/json">__META__</script>
<script id="blob" type="text/plain">__BLOB__</script>
<script>
__JS__
</script>
</body>
</html>
"""

SELECT_CSS = r"""
.hint { margin: 2px 0 4px; font-size: 11px; color: var(--muted); line-height: 1.35; }
#linkinfo .what { font-size: 12.5px; color: var(--ink); line-height: 1.4; }
#linkinfo .what b { font-weight: 650; }
#linkinfo .sub { font-size: 11.5px; color: var(--ink2); margin-top: 2px; }
#binlabel { font: 11px var(--mono); color: var(--ink2); }
table.sl { width: 100%; border-collapse: collapse; margin-top: 8px; font-size: 11.5px; }
table.sl th { text-align: left; font-size: 10px; letter-spacing: .04em; color: var(--muted); font-weight: 600; padding: 2px 3px 4px; border-bottom: 1px solid var(--base); vertical-align: bottom; }
table.sl th.num, table.sl td.num { text-align: right; }
table.sl td { padding: 3px 3px; border-bottom: 1px solid color-mix(in srgb, var(--base) 45%, transparent); color: var(--ink); vertical-align: top; }
table.sl td.num { font-family: var(--mono); font-size: 11px; white-space: nowrap; }
table.sl td .od { display: block; line-height: 1.3; word-break: break-word; }
table.sl td .md { display: block; font-size: 10.5px; color: var(--muted); }
table.sl tr.r { cursor: default; }
table.sl tr.r:hover, table.sl tr.r.hl { background: color-mix(in srgb, var(--base) 40%, transparent); }
table.sl .bar { display: block; height: 2px; margin-top: 2px; background: var(--ink2); border-radius: 1px; }
.more { font-size: 11px; color: var(--muted); margin: 6px 3px 0; }
#legendbody .sw { display: inline-block; vertical-align: middle; margin-right: 6px; }
/* The legend keeps its height; the panel above it scrolls. */
#legendgroup.card { flex: none; max-height: none; }
"""

SELECT_JS = r"""(async function () {
"use strict";
const $ = (id) => document.getElementById(id);
const fail = (e) => { document.body.dataset.state = "error"; const b = $("error"); b.hidden = false; b.textContent = "openmobisim select link: " + (e && e.message ? e.message : e); console.error(e); };
window.addEventListener("error", (ev) => fail(ev.error || ev.message));
window.addEventListener("unhandledrejection", (ev) => fail(ev.reason));

const META = JSON.parse($("meta").textContent), TOK = META.tokens;
async function inflate(b64) {
  const bin = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  const stream = new Blob([bin]).stream().pipeThrough(new DecompressionStream("deflate"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}
const raw = await inflate($("blob").textContent.trim());
const CTOR = { u8: Uint8Array, u32: Uint32Array, i32: Int32Array, f32: Float32Array };
const A = {};
for (const a of META.arrays) A[a.n] = new CTOR[a.t](raw.buffer, a.o, a.c);

// ------------------------------------------------------------- the layers ----
// A layer: its links' geometry (metres around the network's centre), boxes and other directions, its users' paths
// (pl by po), the groups of users of each path (by pg: path, pair, mode, departure bin, trips, people), and for each
// link the paths that cross it (inv by istart), each path once.
function layer(p) {
  const vs = A[p + "vs"]; if (!vs) return null;
  const dx = A[p + "dx"], dy = A[p + "dy"], n = vs.length - 1;
  const xs = new Float64Array(dx.length), ys = new Float64Array(dy.length);
  let cx = 0, cy = 0; for (let i = 0; i < dx.length; i++) { cx += dx[i]; cy += dy[i]; xs[i] = cx * 0.1; ys[i] = cy * 0.1; }
  const x0 = new Float32Array(n), y0 = new Float32Array(n), x1 = new Float32Array(n), y1 = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    let a = Infinity, b = Infinity, c = -Infinity, d = -Infinity;
    for (let k = vs[i]; k < vs[i + 1]; k++) { if (xs[k] < a) a = xs[k]; if (xs[k] > c) c = xs[k]; if (ys[k] < b) b = ys[k]; if (ys[k] > d) d = ys[k]; }
    x0[i] = a; y0[i] = b; x1[i] = c; y1[i] = d;
  }
  const nf = A[p + "nf"], nt = A[p + "nt"], twin = new Int32Array(n).fill(-1), at = new Map();
  for (let i = 0; i < n; i++) at.set(nf[i] + ":" + nt[i], i);
  for (let i = 0; i < n; i++) { const r = at.get(nt[i] + ":" + nf[i]); if (r !== undefined && r !== i) twin[i] = r; }
  const po = A[p + "_po"], pl = A[p + "_pl"], np = po.length - 1;
  const istart = new Uint32Array(n + 1), last = new Int32Array(n).fill(-1);
  for (let q = 0; q < np; q++) for (let i = po[q]; i < po[q + 1]; i++) { const l = pl[i]; if (last[l] !== q) { last[l] = q; istart[l + 1]++; } }
  for (let i = 1; i <= n; i++) istart[i] += istart[i - 1];
  const inv = new Uint32Array(istart[n]), fill = istart.slice(0, n); last.fill(-1);
  for (let q = 0; q < np; q++) for (let i = po[q]; i < po[q + 1]; i++) { const l = pl[i]; if (last[l] !== q) { last[l] = q; inv[fill[l]++] = q; } }
  return { n, vs, xs, ys, x0, y0, x1, y1, nf, nt, twin, len: A[p + "len"], po, pl, pg: A[p + "_pg"], gpair: A[p + "_gpair"], gmode: A[p + "_gmode"], gbin: A[p + "_gbin"], gt: A[p + "_gt"], gp: A[p + "_gp"], istart, inv };
}
const L = { road: layer("r"), bike: META.layers.bike ? layer("b") : null };
const RT = A.rt || null, TIERS = META.road_tiers;
let EX0 = Infinity, EY0 = Infinity, EX1 = -Infinity, EY1 = -Infinity;
{ const g = L.road; for (let i = 0; i < g.n; i++) { EX0 = Math.min(EX0, g.x0[i]); EY0 = Math.min(EY0, g.y0[i]); EX1 = Math.max(EX1, g.x1[i]); EY1 = Math.max(EY1, g.y1[i]); } }
// The places and the pairs; each pair's people by departure bin, every mode (the denominators of the share of a pair).
const NPL = A.px.length, PLX = new Float64Array(NPL), PLY = new Float64Array(NPL);
for (let i = 0; i < NPL; i++) { PLX[i] = A.px[i] * 0.1; PLY[i] = A.py[i] * 0.1; }
const NPAIR = A.q_o.length, NB = META.n_bins, NM = META.modes.length;
const PT = new Float64Array(NPAIR * NB), PTALL = new Float64Array(NPAIR);
for (let i = 0; i < A.t_p.length; i++) { PT[A.t_pair[i] * NB + A.t_bin[i]] += A.t_p[i]; PTALL[A.t_pair[i]] += A.t_p[i]; }

// ---------------------------------------------------------------- state ----
const S = { theme: META.theme, layer: META.start.layer, link: META.start.link, bin: -1, hoverPair: -1, cx: 0, cy: 0, mpp: 1 };
let TH = TOK[S.theme];
function setTheme(name) {
  S.theme = name; TH = TOK[name]; document.documentElement.dataset.theme = name;
  const r = document.documentElement.style;
  for (const k of ["surface", "ink", "ink2", "muted", "base", "mark"]) r.setProperty("--" + k, TH[k]);
  $("theme").textContent = name === "paper" ? "Night" : "Paper";
  legend();
}
const FLOW = () => TH.ramp_volume[1], ORIGIN = () => TH.mark, DEST = () => TH.modes[0];

// ------------------------------------------------------------ the counts ----
// The users of the selected link in the departures shown: by pair and mode (the table), their people on every link
// of the layer (the flow), and by origin and by destination (the discs).
let R = null;
function compute() {
  const g = L[S.layer]; R = { rows: [], vol: new Float32Array(g.n), vmax: 0, orig: new Map(), dest: new Map(), people: 0, trips: 0 };
  if (S.link < 0) return;
  const rows = new Map();
  for (let k = g.istart[S.link]; k < g.istart[S.link + 1]; k++) {
    const q = g.inv[k]; let onPath = 0;
    for (let j = g.pg[q]; j < g.pg[q + 1]; j++) {
      if (S.bin >= 0 && g.gbin[j] !== S.bin) continue;
      const p = g.gp[j], pair = g.gpair[j], key = pair * NM + g.gmode[j];
      let r = rows.get(key); if (!r) { r = { pair, mode: g.gmode[j], trips: 0, people: 0 }; rows.set(key, r); }
      r.trips += g.gt[j]; r.people += p; onPath += p;
      R.trips += g.gt[j]; R.people += p;
      const o = A.q_o[pair], d = A.q_d[pair];
      R.orig.set(o, (R.orig.get(o) || 0) + p); R.dest.set(d, (R.dest.get(d) || 0) + p);
    }
    if (onPath > 0) for (let i = g.po[q]; i < g.po[q + 1]; i++) R.vol[g.pl[i]] += onPath;
  }
  for (let i = 0; i < g.n; i++) if (R.vol[i] > R.vmax) R.vmax = R.vol[i];
  R.rows = [...rows.values()].sort((a, b) => b.people - a.people || a.pair - b.pair || a.mode - b.mode);
}
// One pair's users of the selected link, on every link (the hovered row).
function pairFlow(pair) {
  const g = L[S.layer], vol = new Float32Array(g.n);
  for (let k = g.istart[S.link]; k < g.istart[S.link + 1]; k++) {
    const q = g.inv[k]; let on = 0;
    for (let j = g.pg[q]; j < g.pg[q + 1]; j++) if (g.gpair[j] === pair && (S.bin < 0 || g.gbin[j] === S.bin)) on += g.gp[j];
    if (on > 0) for (let i = g.po[q]; i < g.po[q + 1]; i++) vol[g.pl[i]] += on;
  }
  return vol;
}
const pairPeople = (pair) => (S.bin >= 0 ? PT[pair * NB + S.bin] : PTALL[pair]);

// ------------------------------------------------------------------ view ----
const canvas = $("map"); const ctx = canvas.getContext("2d");
let W = 0, H = 0, DPR = 1;
function resize() { DPR = window.devicePixelRatio || 1; W = canvas.clientWidth; H = canvas.clientHeight; canvas.width = Math.round(W * DPR); canvas.height = Math.round(H * DPR); }
function panelWidth() { return W > 760 ? 378 : 0; }
function fitTo(x0, y0, x1, y1, pad) {
  const top = 78, bottom = 34, w = Math.max(x1 - x0, 60), h = Math.max(y1 - y0, 60), reserve = panelWidth();
  S.mpp = Math.max(w / ((W - reserve) * (1 - pad)), h / ((H - top - bottom) * (1 - pad)));
  S.cx = (x0 + x1) / 2 + reserve / 2 * S.mpp; S.cy = (y0 + y1) / 2 + (top - bottom) / 2 * S.mpp;
}
const sx = (x) => (x - S.cx) / S.mpp + W / 2, sy = (y) => H / 2 - (y - S.cy) / S.mpp;
let PX = new Float32Array(4096), PY = new Float32Array(4096);
function ensure(n) { if (PX.length < n) { PX = new Float32Array(n * 2); PY = new Float32Array(n * 2); } }
// The n points in PX/PY moved `off` px to the right of travel (screen: right of (dx, dy) is (-dy, dx)).
function offsetLine(n, off) {
  if (off === 0 || n < 2) return;
  const nx = new Float32Array(n), ny = new Float32Array(n);
  for (let i = 0; i < n - 1; i++) {
    const dx = PX[i + 1] - PX[i], dy = PY[i + 1] - PY[i], m = Math.hypot(dx, dy);
    if (m < 1e-6) continue;
    nx[i] += -dy / m; ny[i] += dx / m; nx[i + 1] += -dy / m; ny[i + 1] += dx / m;
  }
  for (let i = 0; i < n; i++) { const m = Math.hypot(nx[i], ny[i]) || 1, k = (i > 0 && i < n - 1) ? Math.min(2, 2 / m) : 1 / m; PX[i] += nx[i] * k * off; PY[i] += ny[i] * k * off; }
}
function screenLine(g, l, off, gap) {
  const v0 = g.vs[l], v1 = g.vs[l + 1]; ensure(v1 - v0 + 2);
  let n = 0, lx = 0, ly = 0;
  for (let v = v0; v < v1; v++) { const x = sx(g.xs[v]), y = sy(g.ys[v]); if (n === 0 || v === v1 - 1 || Math.abs(x - lx) + Math.abs(y - ly) >= gap) { PX[n] = x; PY[n] = y; n++; lx = x; ly = y; } }
  offsetLine(n, off);
  return n;
}
function addLine(path, g, l, off, gap) { const n = screenLine(g, l, off, gap); if (n < 2) return; path.moveTo(PX[0], PY[0]); for (let i = 1; i < n; i++) path.lineTo(PX[i], PY[i]); }
// A link's flow width (px) from the people of the selected link's users on it, and its offset to the right of its
// travel when its other direction exists, so the two directions read apart.
const GAP = 2;
const flowWidth = (v, vmax) => 1.4 + 9 * Math.sqrt(v / (vmax || 1));
const sideOffset = (g, l, w) => (g.twin[l] >= 0 ? GAP / 2 + w / 2 : 0);
function inView(g, l, pad) { const hw = W / 2 * S.mpp + pad, hh = H / 2 * S.mpp + pad; return !(g.x1[l] < S.cx - hw || g.x0[l] > S.cx + hw || g.y1[l] < S.cy - hh || g.y0[l] > S.cy + hh); }

let MOVING = false;
function draw() {
  ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
  ctx.fillStyle = TH.surface; ctx.fillRect(0, 0, W, H);
  ctx.lineCap = "round"; ctx.lineJoin = "round";
  const gap = MOVING ? 3 : 1.2, pad = 20 * S.mpp;
  // The road network, by tier; quieter when the bike layer is the one shown.
  const road = L.road, base = TIERS.map(() => new Path2D()), far = S.mpp > 6;
  for (let l = 0; l < road.n; l++) if (inView(road, l, pad)) addLine(base[RT ? RT[l] : 1], road, l, 0, gap);
  ctx.strokeStyle = TH.base; ctx.globalAlpha = S.layer === "road" ? 1 : 0.55;
  for (let t = 0; t < TIERS.length; t++) { ctx.lineWidth = Math.max(far ? 0.5 : 0.8, (far ? 0.5 : 0.9) * TIERS[t]); ctx.stroke(base[t]); }
  ctx.globalAlpha = 1;
  if (S.layer === "bike" && L.bike) {
    const b = L.bike, p = new Path2D(); for (let l = 0; l < b.n; l++) if (inView(b, l, pad)) addLine(p, b, l, 0, gap);
    ctx.strokeStyle = TH.modes[1]; ctx.globalAlpha = 0.35; ctx.lineWidth = 0.8; ctx.stroke(p); ctx.globalAlpha = 1;
  }
  const g = L[S.layer];
  if (R && R.vmax > 0) {
    // The selected link's users on every link: one width per bucket, the widest last.
    const vol = S.hoverPair >= 0 ? pairFlow(S.hoverPair) : R.vol, buckets = new Map();
    for (let l = 0; l < g.n; l++) {
      if (!(vol[l] > 0) || !inView(g, l, pad)) continue;
      const w = Math.round(flowWidth(vol[l], R.vmax) * 2) / 2;
      let p = buckets.get(w); if (!p) { p = new Path2D(); buckets.set(w, p); }
      addLine(p, g, l, sideOffset(g, l, w), gap);
    }
    const keys = [...buckets.keys()].sort((a, b) => a - b);
    if (!MOVING) for (const w of keys) { ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 1.4; ctx.stroke(buckets.get(w)); }
    for (const w of keys) { ctx.strokeStyle = S.hoverPair >= 0 ? TH.ink2 : FLOW(); ctx.lineWidth = w; ctx.stroke(buckets.get(w)); }
  }
  // The selected link itself.
  if (S.link >= 0) {
    const w = R && R.vol[S.link] > 0 ? flowWidth(R.vol[S.link], R.vmax) : 3, p = new Path2D();
    addLine(p, g, S.link, sideOffset(g, S.link, w), 1);
    ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 4; ctx.stroke(p);
    ctx.strokeStyle = TH.ink; ctx.lineWidth = w + 1.5; ctx.stroke(p);
  }
  if (R) drawPlaces();
  drawScale();
}
// Where the users came from (filled discs) and went (rings), each sized by people; the hovered row's pair outlined.
function drawPlaces() {
  let pmax = 0; for (const v of R.orig.values()) pmax = Math.max(pmax, v); for (const v of R.dest.values()) pmax = Math.max(pmax, v);
  if (!pmax) return;
  const rad = (p) => 3 + 11 * Math.sqrt(p / pmax);
  const hp = S.hoverPair, ho = hp >= 0 ? A.q_o[hp] : -1, hd = hp >= 0 ? A.q_d[hp] : -1;
  const sorted = (m) => [...m.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0]);
  for (const [k, p] of sorted(R.dest)) { const x = sx(PLX[k]), y = sy(PLY[k]), r = rad(p); ctx.beginPath(); ctx.arc(x, y, r, 0, 6.2832); ctx.globalAlpha = hp >= 0 && k !== hd ? 0.25 : 1; ctx.fillStyle = TH.surface; ctx.fill(); ctx.strokeStyle = DEST(); ctx.lineWidth = 2.2; ctx.stroke(); }
  for (const [k, p] of sorted(R.orig)) { const x = sx(PLX[k]), y = sy(PLY[k]), r = rad(p); ctx.beginPath(); ctx.arc(x, y, r, 0, 6.2832); ctx.globalAlpha = hp >= 0 && k !== ho ? 0.25 : 0.9; ctx.fillStyle = ORIGIN(); ctx.fill(); ctx.strokeStyle = TH.surface; ctx.lineWidth = 1; ctx.stroke(); }
  ctx.globalAlpha = 1;
  if (hp >= 0) for (const [k, ring] of [[ho, true], [hd, false]]) { ctx.beginPath(); ctx.arc(sx(PLX[k]), sy(PLY[k]), rad((ring ? R.orig : R.dest).get(k) || 0) + 3, 0, 6.2832); ctx.strokeStyle = TH.ink; ctx.lineWidth = 1.6; ctx.stroke(); }
}
function niceNumber(x) { const p = Math.pow(10, Math.floor(Math.log10(x))); for (const m of [5, 2, 1]) if (m * p <= x) return m * p; return p; }
function drawScale() { const m = niceNumber(S.mpp * 90); $("scalebar").style.width = (m / S.mpp) + "px"; $("scalelabel").textContent = m >= 1000 ? (m / 1000) + " km" : m + " m"; }

// ------------------------------------------------------------------ pick ----
function distToPoly(g, l, mx, my, off) {
  let best = Infinity; const n = screenLine(g, l, off, 0);
  for (let i = 0; i < n - 1; i++) {
    const ax = PX[i], ay = PY[i], dx = PX[i + 1] - ax, dy = PY[i + 1] - ay, t = Math.max(0, Math.min(1, ((mx - ax) * dx + (my - ay) * dy) / (dx * dx + dy * dy || 1)));
    best = Math.min(best, Math.hypot(mx - ax - t * dx, my - ay - t * dy));
  }
  return best;
}
// The link of the shown layer under the pointer (within 7 px), each direction on its own side.
function pick(px, py) {
  const g = L[S.layer], mx = S.cx + (px - W / 2) * S.mpp, my = S.cy - (py - H / 2) * S.mpp, reach = 20 * S.mpp;
  let best = -1, bd = 7;
  for (let l = 0; l < g.n; l++) {
    if (g.x1[l] < mx - reach || g.x0[l] > mx + reach || g.y1[l] < my - reach || g.y0[l] > my + reach) continue;
    const used = R && R.vol[l] > 0, w = used ? flowWidth(R.vol[l], R.vmax) : 2, d = distToPoly(g, l, px, py, sideOffset(g, l, w)) - (used ? w / 2 : 0);
    if (d < bd) { bd = d; best = l; }
  }
  return best;
}

// ----------------------------------------------------------------- panel ----
const esc = (x) => String(x).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const clock = (s) => { const m = Math.round(s / 60); return String(Math.floor(m / 60) % 24).padStart(2, "0") + ":" + String(m % 60).padStart(2, "0"); };
const whole = (x) => Math.abs(x - Math.round(x)) < 1e-6;
const fmt = (x) => (whole(x) ? Math.round(x).toLocaleString("en") : x.toFixed(1));
const pct = (x) => (100 * x).toFixed(x >= 0.995 || x < 0.0995 ? 0 : 1) + " %";
function binText() {
  const b0 = META.first_bin, s = META.bin_s;
  return S.bin < 0 ? clock(b0 * s) + "–" + clock((b0 + NB) * s) : clock((b0 + S.bin) * s) + "–" + clock((b0 + S.bin + 1) * s);
}
const MAX_ROWS = 40;
function panel() {
  $("binlabel").textContent = binText();
  const g = L[S.layer], info = $("linkinfo"), box = $("tablebox");
  if (S.link < 0) { info.innerHTML = '<p class="hint">Click a link on the map.</p>'; box.innerHTML = ""; return; }
  const name = S.layer === "road" ? "Road link" : "Bike link";
  info.innerHTML = '<div class="what"><b>' + name + " " + S.link + "</b> · " + Math.round(g.len[S.link]) + " m, node " + g.nf[S.link] + " → " + g.nt[S.link] + "</div>"
    + '<div class="sub">' + (R.trips ? fmt(R.trips) + " trips (" + fmt(R.people) + " people) cross it, departing " + binText() + ", from " + R.orig.size + " origins to " + R.dest.size + " destinations." : "No trip crosses it among the departures shown.") + "</div>";
  if (!R.rows.length) { box.innerHTML = ""; return; }
  let h = '<table class="sl"><thead><tr><th>Origin → destination<br>mode</th><th class="num">Trips</th><th class="num" title="The pair’s people over all the people crossing the link">Share of link</th><th class="num" title="The pair’s people crossing the link over all its people, every mode, departing in the bins shown">Share of pair</th></tr></thead><tbody>';
  const shown = R.rows.slice(0, MAX_ROWS);
  shown.forEach((r, i) => {
    const sl = r.people / R.people, sp = r.people / (pairPeople(r.pair) || 1);
    h += '<tr class="r" data-i="' + i + '"><td><span class="od">' + esc(META.places[A.q_o[r.pair]]) + " → " + esc(META.places[A.q_d[r.pair]]) + '</span><span class="md">' + esc(META.mode_names[r.mode]) + "</span></td>"
      + '<td class="num">' + fmt(r.trips) + (whole(r.people) && r.people === r.trips ? "" : "<br><span class=md>" + fmt(r.people) + " people</span>") + "</td>"
      + '<td class="num">' + pct(sl) + '<span class="bar" style="width:' + (100 * sl).toFixed(1) + '%"></span></td>'
      + '<td class="num">' + pct(sp) + '<span class="bar" style="width:' + (100 * sp).toFixed(1) + '%"></span></td></tr>';
  });
  h += "</tbody></table>";
  if (R.rows.length > MAX_ROWS) { let p = 0; for (const r of R.rows.slice(MAX_ROWS)) p += r.people; h += '<p class="more">and ' + (R.rows.length - MAX_ROWS) + " more pair and mode rows (" + fmt(p) + " people).</p>"; }
  box.innerHTML = h;
  box.querySelectorAll("tr.r").forEach((tr) => {
    tr.addEventListener("mouseenter", () => { S.hoverPair = shown[+tr.dataset.i].pair; redraw(); });
    tr.addEventListener("mouseleave", () => { S.hoverPair = -1; redraw(); });
  });
}
function legend() {
  const sw = (svg) => '<svg class="sw" width="22" height="14" viewBox="0 0 22 14">' + svg + "</svg>";
  $("legendbody").innerHTML = '<div class="lg"><h3>Users of the selected link</h3>'
    + '<div class="kinds" style="flex-direction:column;gap:4px">'
    + "<span>" + sw('<line x1="1" y1="7" x2="21" y2="7" stroke="' + FLOW() + '" stroke-width="5" stroke-linecap="round"/>') + "their flow on every link: width = people</span>"
    + "<span>" + sw('<circle cx="11" cy="7" r="5.5" fill="' + ORIGIN() + '"/>') + "where they set out: size = people</span>"
    + "<span>" + sw('<circle cx="11" cy="7" r="5" fill="none" stroke="' + DEST() + '" stroke-width="2.2"/>') + "where they went: size = people</span>"
    + "<span>" + sw('<line x1="1" y1="7" x2="21" y2="7" stroke="' + TH.ink + '" stroke-width="4" stroke-linecap="round"/>') + "the selected link</span>"
    + "</div><div class=\"note\">" + (META.zones ? "Places are zones: each trip end at the nearest zone point." : "Places are road nodes: each trip end at the node its trip is routed from or to.") + "</div></div>";
}
function redraw() { draw(); }
function update() { compute(); panel(); draw(); }

// ---------------------------------------------------------------- events ----
let drag = null;
canvas.addEventListener("pointerdown", (e) => { drag = { x: e.clientX, y: e.clientY, cx: S.cx, cy: S.cy, moved: false }; canvas.setPointerCapture(e.pointerId); });
canvas.addEventListener("pointermove", (e) => {
  if (drag) {
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) { drag.moved = true; MOVING = true; S.cx = drag.cx - dx * S.mpp; S.cy = drag.cy + dy * S.mpp; draw(); }
    return;
  }
  canvas.style.cursor = pick(e.offsetX, e.offsetY) >= 0 ? "pointer" : "default";
});
canvas.addEventListener("pointerup", (e) => {
  const was = drag; drag = null; MOVING = false;
  if (was && !was.moved) { const l = pick(e.offsetX, e.offsetY); if (l >= 0) { S.link = l; S.hoverPair = -1; update(); } } else draw();
});
function zoomAt(px, py, f) {
  const mx = S.cx + (px - W / 2) * S.mpp, my = S.cy - (py - H / 2) * S.mpp;
  S.mpp = Math.min(Math.max(S.mpp * f, 0.05), 5000); S.cx = mx - (px - W / 2) * S.mpp; S.cy = my + (py - H / 2) * S.mpp; draw();
}
canvas.addEventListener("wheel", (e) => { e.preventDefault(); zoomAt(e.offsetX, e.offsetY, Math.exp(e.deltaY * 0.0015)); }, { passive: false });
$("zin").addEventListener("click", () => zoomAt(W / 2, H / 2, 0.5));
$("zout").addEventListener("click", () => zoomAt(W / 2, H / 2, 2));
$("fit").addEventListener("click", () => { fitTo(EX0, EY0, EX1, EY1, 0.06); draw(); });
$("theme").addEventListener("click", () => { setTheme(S.theme === "paper" ? "night" : "paper"); draw(); });
const slider = $("bin"), all = $("allbins");
slider.max = String(Math.max(0, NB - 1));
all.addEventListener("change", () => { slider.disabled = all.checked; S.bin = all.checked ? -1 : +slider.value; update(); });
slider.addEventListener("input", () => { if (!all.checked) { S.bin = +slider.value; update(); } });
if (L.bike) {
  $("layerrow").hidden = false;
  document.querySelectorAll('input[name="layer"]').forEach((r) => r.addEventListener("change", () => { if (r.checked) { S.layer = r.value; S.link = -1; S.hoverPair = -1; update(); } }));
}
window.addEventListener("resize", () => { resize(); draw(); });

// ----------------------------------------------------------------- start ----
$("title").textContent = META.title;
$("subtitle").textContent = "Click a link: the origin-destination pairs whose trips cross it, where they come from and go.";
$("provenance").textContent = META.provenance; $("source").textContent = META.source;
resize(); setTheme(S.theme); fitTo(EX0, EY0, EX1, EY1, 0.06); update();
document.body.dataset.state = "ready";
})();
"""
