
// ---------------------------------------------------------------- draw ----
// ---- the widths, from the room the roads leave on screen ----
// ROOM: the screen space (px) each drawn road direction has around it where the roads are dense: the screen is cut
// into cells of ROOM_CELL px, each cell's length of drawn road directions summed, and the room is 2 x cell area
// / that length at the cells' 75th percentile (a grid of streets s px apart, two directions each, gives s / 2).
// Kept in metres from the last still frame, so it scales with the zoom while moving.
const ROOM_CELL = 128, ROOM_SHARE = 0.15, WFLOOR = 1.0, WCAP = 18;
let ROOM_M = 0, EXPORTING = false;
// The thinnest base line (px), at an overview and zoomed in: the minor roads read on both themes.
const BASE_FLOOR = [0.6, 0.85];
function measureRoom(cand) {
  const C = ROOM_CELL, nx = Math.ceil(W / C), ny = Math.ceil(H / C), cells = new Float64Array(nx * ny);
  const lim = Math.min(classLimit(), ZOOM.roads);  // the roads drawn at this zoom: those that carry ribbons
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
// The widest ribbon (a road of the network's largest capacity): a quarter of the room up to ROOM_KNEE px of room,
// and beyond it growing with the room's square root only, between 2.5 and WCAP px.
const ROOM_KNEE = 32;
function ribbonMaxDrawn() { const r = roomPx(); return Math.max(2.5, Math.min(WCAP, ROOM_SHARE * Math.min(r, ROOM_KNEE) * Math.sqrt(Math.max(r, ROOM_KNEE) / ROOM_KNEE))); }
function ribbonMax() { return Math.max(WFLOOR, ribbonMaxDrawn()); }
// The space between a road's two directions (px): each ribbon sits on its own side of the centre line, to the
// right of its travel, so the two read apart at every zoom.
let GAP = 2;  // set by each frame: a fifth of the widest ribbon, 1 to 4 px
function ribbonOffset(l, w) { return GAP / 2 + w / 2; }
// Whether link l is drawn as a ribbon of road traffic now.
function hasRibbon(l) { return S.ly.road && HAS_TRAFFIC && vol[l] > 0; }
// A ribbon's width is its road's capacity over the network's large roads' (the 99th percentile of the car links'
// capacities), from WFLOOR to ribbonMax(): every road of one capacity is one width, whatever its traffic; the traffic
// is the colour and the hover. A car link without a limit is as wide as the widest.
const RR = (() => {
  const caps = []; for (let l = 0; l < NL; l++) if ((LM[l] & CAR) && CAP[l] > 0) caps.push(CAP[l]);
  if (!caps.length) return null;
  const ref = percentile(caps, 0.99), out = new Float32Array(NL);
  for (let l = 0; l < NL; l++) out[l] = !(LM[l] & CAR) ? 0 : CAP[l] > 0 ? CAP[l] / ref : 1;
  return out;
})();
function ribbonWidthOf(f) { const top = ribbonMax(); return WFLOOR + (top - WFLOOR) * Math.sqrt(f); }
function ribbonWidth(l) { return ribbonWidthOf(RR ? Math.min(1, RR[l]) : 1); }
// The base line's width, as multiples of the thinnest, and which of BASE_WIDTHS a link takes. With road classes, by
// its road's tier (version 1's): 0 a link without cars, 1 to 4 a car link by its capacity over the large roads' (RR),
// from 0, then from each of META's bounds on. Without them, by its drawing class (0, the largest roads, the widest).
const BASE_WIDTHS = CLASSED ? META.road_tiers : [0.5, 0.8, 1.2, 2.0, 3.0];
function baseOf(l) {
  if (!CLASSED) return BASE_WIDTHS.length - 1 - CLS[l];
  if (!(LM[l] & CAR) || !RR) return 0;
  let t = 1; for (const b of META.road_tier_bounds) if (RR[l] >= b) t++;
  return t;
}
// Link l's colour, as its place in palette(): its band, or its step of the volume's ramp.
function colourIndex(l) {
  if (S.colour === "delay") return band(delay[l]);
  if (S.colour === "vc") return band(CAP[l] > 0 ? peak[l] / CAP[l] : 0, VC_BANDS);
  return Math.max(0, Math.min(NCOL - 1, Math.round(Math.sqrt(Math.min(vol[l], VMAX) / VMAX) * (NCOL - 1))));
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
    // (at most twice `off`), so the two directions of a road stay apart around a corner too.
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
let MOVING = false, LAYER_ALPHA = 1;
// How strongly the road traffic and the other layers recede while a pair's trips are shown.
const DIM_ALPHA = 0.16;
// Whether link l carries enough traffic to be drawn beyond this zoom's classes; whether it is drawn only faint (or not
// at all while moving); whether its ribbon is drawn. The drawing and the pointer read the same rules.
function busyAt(l, lim) { return HAS_TRAFFIC && vol[l] > 0 && (vol[l] >= VHIGH * (S.mpp > 8 ? 1 : 0.4) || CLS[l] <= lim); }
function faintAt(l, lim) { return (CLS[l] > lim && !busyAt(l, lim) && !(classLimit() >= ALL_CLASSES && S.mpp < 2.5)) || (!isRoad(l) && S.mpp > 2.5); }
function ribbonAt(l, lim) { return !faintAt(l, lim) && S.ly.road && HAS_TRAFFIC && vol[l] > 0 && (CLS[l] <= lim || busyAt(l, lim)); }
function draw() {
  ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
  ctx.fillStyle = TH.surface; ctx.fillRect(0, 0, W, H);
  const hw = W / 2 * S.mpp, hh = H / 2 * S.mpp, pad = 8 * S.mpp;
  const cand = candidates(S.cx - hw - pad, S.cy - hh - pad, S.cx + hw + pad, S.cy + hh + pad);
  const lim = classLimit() - (MOVING ? ZOOM.moving : 0), gap = MOVING ? 3 : 1.2;
  if (!MOVING && !EXPORTING) measureRoom(cand);  // an export keeps the screen's widths
  GAP = Math.max(1, Math.min(4, ribbonMaxDrawn() / 5));
  const base = BASE_WIDTHS.map(() => new Path2D()), faint = new Path2D(), buckets = new Map();
  const dimming = S.pair >= 0;
  for (const l of cand) {
    // A link this zoom leaves out is drawn faint, not left out, but while moving.
    if (faintAt(l, lim)) {
      if (!MOVING) addLine(faint, l, 0, gap);
      continue;
    }
    addLine(base[baseOf(l)], l, 0, gap);
    if (ribbonAt(l, lim)) {
      const w = ribbonWidth(l), wi = Math.min(127, Math.round(w * 2)), key = colourIndex(l) * 128 + wi;
      let b = buckets.get(key); if (!b) { b = new Path2D(); buckets.set(key, b); }
      addLine(b, l, ribbonOffset(l, w), gap);
    }
  }
  ctx.lineCap = "round"; ctx.lineJoin = "round";
  // The links this zoom leaves out, a little stronger than a hairline so they read on both themes; no base line is
  // thinner than BASE_FLOOR.
  ctx.globalAlpha = 0.8; ctx.strokeStyle = TH.base; ctx.lineWidth = 0.55; ctx.stroke(faint); ctx.globalAlpha = 1;
  // The road hierarchy: the minor roads first, so the major roads sit on top.
  ctx.strokeStyle = TH.base; for (let t = 0; t < BASE_WIDTHS.length; t++) { const far = S.mpp > 6; ctx.lineWidth = Math.max(BASE_FLOOR[far ? 0 : 1], (far ? 0.5 : 0.9) * BASE_WIDTHS[t]); ctx.stroke(base[t]); }
  const keys = [...buckets.keys()].sort((a, b) => a - b), tbl = palette();
  // While a pair's trips are shown, the road traffic is a faint background.
  const back = dimming ? DIM_ALPHA : 1;
  ctx.globalAlpha = back;
  if (TH.glow && !dimming) { for (const k of keys) { const w = (k & 127) / 2; ctx.strokeStyle = tbl[k >> 7]; ctx.globalAlpha = 0.14; ctx.lineWidth = w + Math.min(w * 1.8, GAP * 0.8); ctx.stroke(buckets.get(k)); } ctx.globalAlpha = back; }
  else if (!MOVING) { for (const k of keys) { ctx.strokeStyle = TH.surface; ctx.lineWidth = (k & 127) / 2 + 1; ctx.stroke(buckets.get(k)); } }
  for (const k of keys) { ctx.strokeStyle = tbl[k >> 7]; ctx.lineWidth = (k & 127) / 2; ctx.stroke(buckets.get(k)); }
  ctx.globalAlpha = 1;
  if (!MOVING && S.mpp < 3 && S.ly.road && !dimming) drawChevrons(cand, lim);
  // The other layers recede while a pair's trips are shown, as the road traffic does.
  LAYER_ALPHA = dimming ? DIM_ALPHA : 1;
  if (S.ly.bike && FLOWS.bike) drawFlow(cand, FLOWS.bike, TH.modes[1], [9, 5]);
  if (S.ly.walk && FLOWS.walk) drawFlow(cand, FLOWS.walk, TH.modes[2], [2, 4]);
  ctx.globalAlpha = LAYER_ALPHA;
  if (S.ly.transit && HAS_TRANSIT) drawTransit();
  ctx.globalAlpha = 1; LAYER_ALPHA = 1;
  if (S.pair >= 0) drawTrips();
  if (S.picking) drawPick();
  highlight(S.link, TH.ink, 3); if (S.hover !== S.link) highlight(S.hover, TH.ink2, 2);
  if (S.sel) highlightItem(S.sel);
  highlightItem(S.hoverItem);
  drawScale();
}
// A bike or walk link's width (px) and its offset to the right of its travel: a link whose other direction is on
// the layer too sits on its own side, FLOW_GAP px from the centre line, as the road's ribbons do; a one-way link on
// the line. The width is the encoder's rule (rules::flow_share), one for both layers: from 1 px, the full 8 px at
// META.flow_full travellers over the run and above.
const FLOW_GAP = 1.5, FLOW_FULL = META.flow_full;
function flowWidth(f, l) { return 1 + 7 * Math.sqrt(Math.min(f.v[l], FLOW_FULL) / FLOW_FULL); }
function flowOffset(f, l, w) { return f.twin[l] >= 0 ? FLOW_GAP / 2 + w / 2 : 0; }
function drawFlow(cand, f, colour, dash) {
  const faint = new Path2D(), buckets = new Map();
  for (const l of cand) {
    if (!(LM[l] & f.bit)) continue;
    if (!(f.v[l] > 0)) { if (!MOVING) addLine(faint, l, 0, 0); continue; }  // the layer's links, faint
    const w = flowWidth(f, l), key = Math.round(w * 2);
    let p = buckets.get(key); if (!p) { p = new Path2D(); buckets.set(key, p); }
    addLine(p, l, flowOffset(f, l, key / 2), 0);
  }
  ctx.globalAlpha = 0.45 * LAYER_ALPHA; ctx.strokeStyle = colour; ctx.lineWidth = 0.6; ctx.stroke(faint); ctx.globalAlpha = LAYER_ALPHA;
  const keys = [...buckets.keys()].sort((a, b) => a - b);
  for (const k of keys) { ctx.strokeStyle = TH.surface; ctx.lineWidth = k / 2 + 1.4; ctx.stroke(buckets.get(k)); }
  // Over road traffic, dashed (cyclists) or dotted (walkers): another mode, not another road colour.
  ctx.setLineDash(S.ly.road && HAS_TRAFFIC ? dash : []);
  for (const k of keys) { ctx.strokeStyle = colour; ctx.lineWidth = k / 2; ctx.stroke(buckets.get(k)); }
  ctx.setLineDash([]); ctx.globalAlpha = 1;
}
// A transit hop between two stops: its width (px), and its ends on screen moved to the right of its travel, so a line
// run both ways shows, and is picked, as two hops side by side.
let HOP_TOP = 0;
function hopTop() { if (!HOP_TOP) { for (let i = 0; i < A.tl.length; i++) HOP_TOP = Math.max(HOP_TOP, A.tl[i]); HOP_TOP = HOP_TOP || 1; } return HOP_TOP; }
function hopWidth(i, top) { return 1.2 + 6 * Math.sqrt(A.tl[i] / (top || hopTop())); }
function hopScreen(i, w) {
  const a = A.ta[i], b = A.tb[i], x0 = sx(A.tsx[a] * 0.1), y0 = sy(A.tsy[a] * 0.1), x1 = sx(A.tsx[b] * 0.1), y1 = sy(A.tsy[b] * 0.1);
  const d = Math.hypot(x1 - x0, y1 - y0) || 1, ox = -(y1 - y0) / d * (w / 2 + 0.5), oy = (x1 - x0) / d * (w / 2 + 0.5);
  return [x0 + ox, y0 + oy, x1 + ox, y1 + oy];
}
function drawTransit() {
  const top = hopTop();
  for (let i = 0; i < A.tl.length; i++) {  // busiest last, on top
    const w = hopWidth(i, top), [x0, y0, x1, y1] = hopScreen(i, w);
    const p = new Path2D(); p.moveTo(x0, y0); p.lineTo(x1, y1);
    ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 1.4; ctx.stroke(p);
    ctx.strokeStyle = TH.kinds[A.tk[i]]; ctx.lineWidth = w; ctx.stroke(p);
  }
  let most = 0; for (let i = 0; i < A.tsb.length; i++) most = Math.max(most, A.tsb[i]); most = most || 1;
  // Stop ids only when few or zoomed in, and never under a pair's trips.
  const label = (STOPS.length <= 40 || S.mpp < 3) && LAYER_ALPHA === 1;
  ctx.font = "10.5px " + FONT; ctx.textAlign = "left"; ctx.textBaseline = "alphabetic";
  for (let i = 0; i < A.tsx.length; i++) {
    const x = sx(A.tsx[i] * 0.1), y = sy(A.tsy[i] * 0.1);
    if (x < -20 || y < -20 || x > W + 20 || y > H + 20) continue;
    const r = 2 + 4 * Math.sqrt(A.tsb[i] / most);
    ctx.beginPath(); ctx.arc(x, y, r, 0, 6.2832); ctx.fillStyle = TH.surface; ctx.fill(); ctx.strokeStyle = TH.ink2; ctx.lineWidth = 1; ctx.stroke();
    if (label) { ctx.lineWidth = 3; ctx.strokeStyle = TH.surface; ctx.strokeText(STOPS[i], x + r + 3, y - r - 1); ctx.fillStyle = TH.ink2; ctx.fillText(STOPS[i], x + r + 3, y - r - 1); }
  }
}
function layerRows() {
  const present = { road: HAS_TRAFFIC, transit: HAS_TRANSIT, bike: !!FLOWS.bike, walk: !!FLOWS.walk };
  const rows = LAYER_ROWS.filter(([k]) => present[k]);
  $("layergroup").hidden = rows.length < 2;  // one layer alone needs no switch
  const mark = { road: '<i class="sw ramp" style="background:linear-gradient(90deg,' + TH.ramp_delay.join(",") + ')"></i>', transit: '<i class="sw" style="border-top:3px solid ' + TH.modes[3] + '"></i>', bike: '<i class="sw" style="border-top:3px dashed ' + TH.modes[1] + '"></i>', walk: '<i class="sw" style="border-top:3px dotted ' + TH.modes[2] + '"></i>' };
  $("layerrows").innerHTML = rows.map(([k, name, hint]) => `<label title="${hint}"><input type="checkbox" data-layer="${k}"${S.ly[k] ? " checked" : ""}> ${mark[k]} ${name}</label>`).join("");
  document.querySelectorAll("#layerrows input").forEach((box) => box.addEventListener("change", (e) => {
    const k = e.target.dataset.layer; S.ly[k] = e.target.checked;
    if (!S.ly.road) stopPlay();  // road traffic hidden, its Pause button with it: the play stops
    $("trafficgroup").hidden = !HAS_TRAFFIC || !S.ly.road; subtitle();
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
function highlight(l, colour, w) {
  if (l < 0) return;
  // A ribbon is outlined on its own side, so the direction picked shows; a link without one, on its line.
  if (hasRibbon(l)) {
    const rw = ribbonWidth(l), p = new Path2D(); if (!addLine(p, l, ribbonOffset(l, rw), 0.5)) return;
    ctx.lineCap = "butt"; ctx.strokeStyle = colour; ctx.lineWidth = rw + 2 * w; ctx.stroke(p);
    ctx.strokeStyle = palette()[colourIndex(l)]; ctx.lineWidth = rw; ctx.stroke(p); ctx.lineCap = "round";
    return;
  }
  const p = new Path2D(); if (!addLine(p, l, 0, 0.5)) return;
  ctx.strokeStyle = TH.surface; ctx.lineWidth = w + 3; ctx.stroke(p); ctx.strokeStyle = colour; ctx.lineWidth = w; ctx.stroke(p);
}
// ---- trips between two places ----
// One colour per journey, and one source of it: the list's dot, share bar and strip, the map's legs, badges and door
// links, and an export's legend all call journeyColour(o). The colour is the journey's rank in its pair, the most
// travelled first (the order the page holds them in), the sixth and every one after it in the muted ink.
function journeyColour(o) { const r = o - A.qs[OPT_PAIR[o]]; return r < TH.kinds.length ? TH.kinds[r] : TH.muted; }
// The ink that reads on a fill: the darker or the lighter of the two themes' inks, whichever contrasts more.
function inkOn(fill) {
  const lum = (h) => { const v = hex(h).map((x) => { x /= 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); }); return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]; };
  const f = lum(fill), dark = TOK.paper.ink, light = "#ffffff";
  return (f + 0.05) / (lum(dark) + 0.05) > (1.05) / (f + 0.05) ? dark : light;
}
// ---- the trips' visual identity: one line style per kind of leg, one mark per kind of place, drawn by the same
// functions on the map, in the list's strips, in the legend and in the exports ----
// The line styles, each named by what the traveller does: drive, a solid line; track (a ride on rail, metro or tram)
// and bus (a ride on anything else), a thicker solid line; bike, long dashes; walk, round dots. The marks: A and B (origin, destination), dark discs; a stop, a
// small ring.
const LEG_STYLE_NAMES = { drive: "car", track: "transit", bus: "transit", bike: "bike", walk: "walk" };
const RAIL_KINDS = new Set(["rail", "metro", "tram"]);
// A ride's line, and its kind's style: track for rail, metro and tram, bus for the rest.
function lineOf(g) { return A.lline[g] === NONE ? "" : LINES[A.lline[g]]; }
function legStyle(g) {
  const k = A.lk[g];
  if (k === LEG.drive) return "drive";
  if (k === LEG.bike) return "bike";
  if (k === LEG.ride) return A.lline[g] !== NONE && RAIL_KINDS.has(KINDS[LINE_KINDS[A.lline[g]]]) ? "track" : "bus";
  return "walk";
}
// The strokes that draw a style `lw` px wide: [width, dash or null, cap].
function legStrokes(style, lw) {
  switch (style) {
    case "track": case "bus": return [[lw * 1.5, null, "round"]];
    case "bike": return [[lw, [2.4 * lw, 1.5 * lw], "butt"]];
    case "walk": return [[lw, [0.1, 1.9 * lw], "round"]];
    default: return [[lw, null, "round"]];
  }
}
// Stroke `path` (null: the context's current path) in a style: pass 0 its margin of the surface, pass 1 its strokes.
function strokeLeg(c2d, path, style, colour, lw, pass) {
  const go = () => (path ? c2d.stroke(path) : c2d.stroke());
  const strokes = legStrokes(style, lw);
  if (pass === 0) {
    const wide = Math.max(...strokes.map((x) => x[0])), walk = style === "walk";
    c2d.lineCap = "round"; c2d.setLineDash(walk ? strokes[0][1] : []); c2d.strokeStyle = TH.surface; c2d.lineWidth = wide + 2; go(); c2d.setLineDash([]);
    return;
  }
  for (const [w, dash, cap] of strokes) { c2d.lineCap = cap; c2d.setLineDash(dash || []); c2d.strokeStyle = colour; c2d.lineWidth = w; go(); }
  c2d.setLineDash([]); c2d.lineCap = "round";
}
// A short sample of a style, `len` px long from (x, y): the legend's and the exports' swatch.
function drawSwatch(c2d, x, y, len, style, colour) {
  for (const pass of [0, 1]) { c2d.beginPath(); c2d.moveTo(x, y); c2d.lineTo(x + len, y); strokeLeg(c2d, null, style, colour, 3, pass); }
}
// A place's mark at (x, y): "A", "B" or "stop".
function drawMark(c2d, kind, x, y) {
  const disc = (r, fill, line, lw) => { c2d.beginPath(); c2d.arc(x, y, r, 0, 6.2832); c2d.fillStyle = fill; c2d.fill(); c2d.strokeStyle = line; c2d.lineWidth = lw; c2d.stroke(); };
  if (kind === "stop") { disc(3.6, TH.surface, TH.ink2, 1.6); return; }
  disc(9, TH.ink, TH.surface, 1.6);
  c2d.fillStyle = TH.surface; c2d.font = "700 10px " + FONT; c2d.textAlign = "center"; c2d.textBaseline = "middle"; c2d.fillText(kind, x, y + 0.5);
}
// A drawing of a mark or a swatch as inline SVG, by the same functions (the legend's).
function svgOf(w, h, draw) { const rec = svgRecorder(); draw(rec); return '<svg width="' + w + '" height="' + h + '" viewBox="0 0 ' + w + " " + h + '" style="vertical-align:middle;margin-right:5px;overflow:visible">' + rec.out.join("") + "</svg>"; }
// A journey's style, by its main leg: its first ride's, else its drive's, else its bike ride's, else a walk's (an
// export's legend, one line per journey).
function journeyStyle(o) {
  const styles = []; for (let g = A.ol[o]; g < A.ol[o + 1]; g++) styles.push(legStyle(g));
  for (const st of ["track", "bus", "drive", "bike"]) if (styles.includes(st)) return st;
  return "walk";
}
// A leg's points in metres: along its links' shapes, through its stops, or straight between two stops; none for a
// walk between a door and the network or a stop, or a wait.
function legPoints(g) {
  const k = A.lk[g], out = [];
  if (k <= LEG.walk) {
    let wx = NaN, wy = NaN;
    for (let i = A.la[g]; i < A.lb[g]; i++) {
      const l = A.pl[i];
      for (let v = VS[l]; v < VS[l + 1]; v++) {
        if (v === VS[l] && X[v] === wx && Y[v] === wy) continue;
        out.push(X[v], Y[v]); wx = X[v]; wy = Y[v];
      }
    }
  } else if (k === LEG.ride) { for (let i = A.la[g]; i < A.lb[g]; i++) { const st = A.pst[i]; out.push(A.tsx[st] * 0.1, A.tsy[st] * 0.1); } }
  else if (k === LEG.connect && A.la[g] !== NONE) out.push(A.tsx[A.la[g]] * 0.1, A.tsy[A.la[g]] * 0.1, A.tsx[A.lb[g]] * 0.1, A.tsy[A.lb[g]] * 0.1);
  return out;
}
let LEGPTS = new Map();   // the selected pair's legs' points, made once per pair
function pairOptions(p) { const out = []; for (let o = A.qs[p]; o < A.qs[p + 1]; o++) out.push(o); return out; }
// A pair shows its most travelled journeys, MAX_SHOWN of them until asked for all.
const MAX_SHOWN = 6;
// The list holds every mode's journeys, whatever modes are drawn; the map draws those of the modes ticked.
function listedOption(o) { return S.allOpts || o - A.qs[OPT_PAIR[o]] < MAX_SHOWN; }
function shownOption(o) { return !S.hidden.has(A.om[o]) && listedOption(o); }
// A journey's n points in PX/PY moved `off` px to the right of travel: each straight piece parallel to its own line,
// joined by a mitre at a bend up to about 105 degrees and a bevel at a sharper one. Returns the new number of points.
function offsetJourney(n, off) {
  if (off === 0 || n < 2) return n;
  const seg = [];
  for (let i = 0; i < n - 1; i++) { const dx = PX[i + 1] - PX[i], dy = PY[i + 1] - PY[i], m = Math.hypot(dx, dy); if (m > 1e-6) seg.push([PX[i], PY[i], PX[i + 1], PY[i + 1], -dy / m, dx / m]); }
  if (!seg.length) return n;
  const ox = [], oy = [];
  for (let j = 0; j < seg.length; j++) {
    const [x, y, , , ux, uy] = seg[j];
    if (j === 0) { ox.push(x + ux * off); oy.push(y + uy * off); continue; }
    const vx = seg[j - 1][4], vy = seg[j - 1][5], d = vx * ux + vy * uy;
    if (d > -0.3) { const mx = vx + ux, my = vy + uy, k = 2 * off / (mx * mx + my * my); ox.push(x + mx * k); oy.push(y + my * k); }
    else { ox.push(x + vx * off, x + ux * off); oy.push(y + vy * off, y + uy * off); }
  }
  const last = seg[seg.length - 1]; ox.push(last[2] + last[4] * off); oy.push(last[3] + last[5] * off);
  ensure(ox.length); for (let i = 0; i < ox.length; i++) { PX[i] = ox[i]; PY[i] = oy[i]; }
  return ox.length;
}
// The screen path of points `pts` (metres), moved `off` px to the right of travel; its middle, ends and length.
function screenPath(pts, off) {
  const n0 = pts.length / 2; ensure(n0 + 2);
  let n = 0, lx = 0, ly = 0;
  for (let i = 0; i < n0; i++) {
    const x = sx(pts[2 * i]), y = sy(pts[2 * i + 1]);
    if (n === 0 || i === n0 - 1 || Math.abs(x - lx) + Math.abs(y - ly) >= 1.2) { PX[n] = x; PY[n] = y; n++; lx = x; ly = y; }
  }
  n = offsetJourney(n, off);
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
  ctx.fillStyle = inkOn(fill); ctx.textAlign = "center"; ctx.textBaseline = "middle"; ctx.fillText(text, x, y + 0.5);
}
function ring(x, y, r, fill, line) { ctx.beginPath(); ctx.arc(x, y, r, 0, 6.2832); ctx.fillStyle = fill; ctx.fill(); ctx.strokeStyle = line; ctx.lineWidth = 1.6; ctx.stroke(); }
// A car journey's number among the pair's car journeys, from 1.
function carRank(o) { let r = 0; for (let q = A.qs[OPT_PAIR[o]]; q < o; q++) if (A.om[q] === 0) r++; return r + 1; }
function drawTrips() {
  const p = S.pair, opts = pairOptions(p).filter(shownOption);
  // The order drawn: the most travelled first, so a journey along the same streets, a walk beside a bike route, stays
  // on top and in sight; the hovered on top of all.
  opts.sort((a, b) => (a === S.hoverOpt) - (b === S.hoverOpt) || A.on[b] - A.on[a] || a - b);
  // Every journey one width, a tenth of the room the roads leave on screen, 3 to 4 px.
  const room = roomPx(), width = Math.max(3, Math.min(4, 0.1 * room));
  // The journeys side by side, the most travelled nearest the line, each beside the one before by its width and 1.5 px,
  // one lane per journey whatever its mode; all of them within half the room (22 px at least), narrower in proportion
  // if not.
  const lane = new Map(); let acc = 0;
  for (const o of [...opts].sort((a, b) => A.on[b] - A.on[a] || a - b)) { lane.set(o, acc + width / 2 + 1.5); acc += width + 3; }
  const fit = Math.min(1, Math.max(22, 0.5 * room) / Math.max(acc, 1));
  const badges = [], stops = [];
  // Two passes: every journey's margin of the surface first, then their colours, so that a journey on top hides no
  // more of another than its own line; the hovered journey after all the others, alone on top.
  const groups = S.hoverOpt >= 0 && opts.includes(S.hoverOpt) ? [opts.filter((o) => o !== S.hoverOpt), [S.hoverOpt]] : [opts];
  for (const group of groups) for (const pass of [0, 1]) for (const o of group) {
    const hl = S.hoverOpt === o, dim = S.hoverOpt >= 0 && !hl;
    const w = width + (hl ? 1 : 0), f = lane.get(o), c = journeyColour(o);
    ctx.globalAlpha = dim ? 0.16 : 1;
    // From the door to where the journey meets the network or its first stop, and from its last to the door, when
    // they are apart: a thin dotted straight line.
    if (pass === 1) {
      let firstPt = null, lastPt = null;
      for (let g = A.ol[o]; g < A.ol[o + 1]; g++) { const pts = LEGPTS.get(g); if (pts && pts.length >= 2) { if (!firstPt) firstPt = [pts[0], pts[1]]; lastPt = [pts[pts.length - 2], pts[pts.length - 1]]; } }
      for (const [door, at] of [[[A.qox[p] * 0.1, A.qoy[p] * 0.1], firstPt], [[A.qdx[p] * 0.1, A.qdy[p] * 0.1], lastPt]]) {
        if (!at || Math.hypot(door[0] - at[0], door[1] - at[1]) < 25) continue;
        const path = new Path2D(); path.moveTo(sx(door[0]), sy(door[1])); path.lineTo(sx(at[0]), sy(at[1]));
        strokeLeg(ctx, path, "walk", c, w, 1);
      }
    }
    for (let g = A.ol[o]; g < A.ol[o + 1]; g++) {
      const k = A.lk[g], pts = LEGPTS.get(g); if (!pts || pts.length < 4) continue;
      const sp = screenPath(pts, k === LEG.connect ? 0 : f * fit); if (sp.n < 2) continue;
      // Every leg in the journey's one colour; what the leg is, by its line style (legStrokes).
      strokeLeg(ctx, sp.path, legStyle(g), c, w, pass);
      if (pass === 1 && k === LEG.ride) {
        stops.push({ at: sp.first, dim }, { at: sp.last, dim });
        if (!dim && sp.len > 60) badges.push([sp.mid, lineOf(g), c]);
      }
      // A car journey's number only on the journey hovered: the colour already ties a journey to its row.
      if (pass === 1 && k === LEG.drive && hl && sp.len > 40) badges.push([sp.mid, String(carRank(o)), c]);
    }
  }
  // The stops, each once where several journeys share it (a mark within 4 px of one kept is the same stop), then the
  // badges, then the origin and destination on top.
  const kept = []; for (const m of stops) { const had = kept.find((k) => Math.hypot(k.at[0] - m.at[0], k.at[1] - m.at[1]) < 4); if (!had) kept.push(m); else if (had.dim && !m.dim) had.dim = false; }
  for (const m of kept) { ctx.globalAlpha = m.dim ? 0.25 : 1; drawMark(ctx, "stop", m.at[0], m.at[1]); }
  ctx.globalAlpha = 1;
  // A line's name once where several journeys ride it side by side.
  const named = []; for (const b of badges) if (b[1] && !named.some((n) => n[1] === b[1] && Math.hypot(n[0][0] - b[0][0], n[0][1] - b[0][1]) < 80)) named.push(b);
  for (const [at, text, fill] of named) badge(at[0], at[1], text, fill);
  drawMark(ctx, "A", sx(A.qox[p] * 0.1), sy(A.qoy[p] * 0.1)); drawMark(ctx, "B", sx(A.qdx[p] * 0.1), sy(A.qdy[p] * 0.1));
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
  if (S.picking === 2) drawMark(ctx, "A", sx(S.pickFrom[0] * 0.1), sy(S.pickFrom[1] * 0.1));
}
function pickAt(px, py) {
  const places = pickPlaces(); let best = null, bd = 28;
  for (const e of places) { const d = Math.hypot(sx(e.x * 0.1) - px, sy(e.y * 0.1) - py); if (d < bd) { bd = d; best = e; } }
  if (!best) return;
  if (S.picking === 1) { S.picking = 2; S.pickFrom = [best.x, best.y]; pickHint(); redraw(false); return; }
  S.picking = 0; S.pickFrom = null; setCursor(null); selectPair(best.p, true);
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
