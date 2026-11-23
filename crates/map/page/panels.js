
// ---------------------------------------------------------- interaction ----
// The distance (m) from (mx, my) to link l's line moved `off` metres to the right of its travel: each direction of a
// two-way street is picked on its own side.
function distToLink(l, mx, my, off) { return distToPoly(VS[l], VS[l + 1], X, Y, mx, my, off); }
// The same for any polyline: the points v0 to v1 - 1 of xs, ys (metres).
function distToPoly(v0, v1, xs, ys, mx, my, off) {
  let best = Infinity;
  for (let v = v0; v < v1 - 1; v++) {
    let ax = xs[v], ay = ys[v], bx = xs[v + 1], by = ys[v + 1]; const dx = bx - ax, dy = by - ay, L = dx * dx + dy * dy;
    if (off && L > 0) { const m = Math.sqrt(L), nx = dy / m * off, ny = -dx / m * off; ax += nx; ay += ny; bx += nx; by += ny; }
    let t = L > 0 ? ((mx - ax) * dx + (my - ay) * dy) / L : 0; t = Math.max(0, Math.min(1, t));
    const d = Math.hypot(mx - (ax + t * dx), my - (ay + t * dy)); if (d < best) best = d;
  }
  return best;
}
function segDist(px, py, ax, ay, bx, by) {
  const dx = bx - ax, dy = by - ay, t = Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy || 1)));
  return Math.hypot(px - ax - t * dx, py - ay - t * dy);
}
// What the pointer is on: of the things drawn under it in the layers ticked (a road ribbon, a cyclists' or walkers'
// line, a transit hop or stop), the one drawn on top (in that order of drawing; the nearer centre of two of one
// layer); when it is on none, the nearest within 4 px of its edge; when nothing drawn is in reach, the nearest link
// drawn as a base line within 7 px, else the nearest drawn faint. Distances are on screen, in px, each line on its
// own side; a distance at or below 0 is the pointer on the line.
function pickItem(px, py) {
  const mx = S.cx + (px - W / 2) * S.mpp, my = S.cy - (py - H / 2) * S.mpp, lim = classLimit();
  const reach = (7 + GAP + FLOW_GAP + 16 + ribbonWidthOf(1)) * S.mpp;
  const cand = candidates(mx - reach, my - reach, mx + reach, my + reach);
  let best = null, bd = Infinity, on = null, onRank = -1, onD = Infinity;
  // `rank`: the order of drawing, ribbons 0, cyclists 1, walkers 2, hops 3, stops 4.
  const take = (item, d, rank) => {
    if (d <= 0 && (rank > onRank || (rank === onRank && d < onD))) { on = item; onRank = rank; onD = d; }
    if (d <= 4 && d < bd) { bd = d; best = item; }
  };
  for (const l of cand) {
    if (ribbonAt(l, lim)) { const w = ribbonWidth(l); take({ kind: "road", id: l }, distToLink(l, mx, my, ribbonOffset(l, w) * S.mpp) / S.mpp - w / 2, 0); }
    for (const [kind, rank] of [["bike", 1], ["walk", 2]]) {
      const f = FLOWS[kind];
      if (!S.ly[kind] || !f || !(LM[l] & f.bit) || !(f.v[l] > 0)) continue;
      const w = Math.round(flowWidth(f, l) * 2) / 2;
      take({ kind, id: l }, distToLink(l, mx, my, flowOffset(f, l, w) * S.mpp) / S.mpp - w / 2, rank);
    }
  }
  if (S.ly.transit && HAS_TRANSIT) {
    let most = 0; for (let i = 0; i < A.tsb.length; i++) most = Math.max(most, A.tsb[i]); most = most || 1;
    for (let i = 0; i < A.tsx.length; i++) take({ kind: "transit", stop: true, id: i }, Math.hypot(sx(A.tsx[i] * 0.1) - px, sy(A.tsy[i] * 0.1) - py) - (2 + 4 * Math.sqrt(A.tsb[i] / most)), 4);
    for (let i = 0; i < A.ta.length; i++) { const w = hopWidth(i), [x0, y0, x1, y1] = hopScreen(i, w); take({ kind: "transit", stop: false, id: i }, segDist(px, py, x0, y0, x1, y1) - w / 2, 3); }
  }
  if (on) return on;
  if (best) return best;
  let rank = 2, l0 = -1;
  bd = 7;
  for (const l of cand) {
    const r = faintAt(l, lim) ? 1 : 0, d = distToLink(l, mx, my, 0) / S.mpp;
    if (d <= 7 && (r < rank || (r === rank && d < bd))) { rank = r; bd = d; l0 = l; }
  }
  return l0 >= 0 ? { kind: "road", id: l0 } : null;
}
// A hover card as HTML, every string from the run's data (stop ids) escaped.
function escHtml(x) { return String(x).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"); }
function tipHtml(head, rows) { return "<b>" + escHtml(head) + "</b>" + rows.map((r) => "<div><span>" + escHtml(r[0]) + "</span>" + escHtml(r[1]) + "</div>").join(""); }
function sameItem(a, b) { return a === b || (!!a && !!b && a.kind === b.kind && a.id === b.id && !!a.stop === !!b.stop); }
function showTip(head, rows, ev) {
  const t = $("tip"); t.innerHTML = tipHtml(head, rows);
  t.hidden = false; t.style.left = Math.min(W - 250, ev.clientX + 14) + "px"; t.style.top = Math.min(H - 150, ev.clientY + 14) + "px";
}
// The travellers of the pairs the page holds whose journeys drive, ride or walk on link l.
function linkTravellers(l) { let n = 0; for (let i = LOPT_START[l]; i < LOPT_START[l + 1]; i++) n += A.on[LOPTS[i]]; return n; }
function tipItem(it, ev) {
  if (!it || it.kind === "road") { tip(it ? it.id : -1, ev); return; }
  const i = it.id, rows = [];
  if (it.kind === "bike" || it.kind === "walk") {
    const f = FLOWS[it.kind], who = it.kind === "bike" ? "cyclist" : "walker";
    rows.push(["Length", fmt(LEN[i]) + " m"]);
    // Which direction, as the road's hover names it.
    rows.push(["Direction", "from node " + nodeId(LU[i]) + " to node " + nodeId(LD[i])]);
    rows.push([it.kind === "bike" ? "Cyclists" : "Walkers", fmt(f.v[i]) + " over the run"]);
    if (f.twin[i] >= 0) rows.push(["Other direction", count(f.v[f.twin[i]], who)]);
    showTip((it.kind === "bike" ? "bike" : "walk") + " link", rows, ev);
  } else if (it.stop) {
    showTip("stop " + STOPS[i], [["Boardings", fmt(A.tsb[i]) + " over the run"]], ev);
  } else {
    const a = A.ta[i], b = A.tb[i];
    rows.push(["From", STOPS[a]]); rows.push(["To", STOPS[b]]);
    rows.push(["Passengers", fmt(A.tl[i]) + " on board, over the run"]);
    let back = -1; for (let j = 0; j < A.ta.length; j++) if (A.ta[j] === b && A.tb[j] === a && A.tk[j] === A.tk[i]) { back = j; break; }
    if (back >= 0) rows.push(["Other direction", fmt(A.tl[back]) + " on board"]);
    showTip(KINDS[A.tk[i]] + " between two stops", rows, ev);
  }
}
// The hovered element of a layer other than the road, outlined.
function highlightItem(it) {
  if (!it || it.kind === "road") return;
  const p = new Path2D(); let at = null;
  // Outlined on its own side, so the direction picked shows.
  if (it.kind === "bike" || it.kind === "walk") { const f = FLOWS[it.kind], l = it.id; addLine(p, l, f.v[l] > 0 ? flowOffset(f, l, Math.round(flowWidth(f, l) * 2) / 2) : 0, 0); }
  else if (!it.stop) { const [x0, y0, x1, y1] = hopScreen(it.id, hopWidth(it.id)); p.moveTo(x0, y0); p.lineTo(x1, y1); }
  else at = [sx(A.tsx[it.id] * 0.1), sy(A.tsy[it.id] * 0.1)];
  if (at) { ctx.beginPath(); ctx.arc(at[0], at[1], 9, 0, 6.2832); ctx.strokeStyle = TH.ink; ctx.lineWidth = 2; ctx.stroke(); return; }
  ctx.strokeStyle = TH.surface; ctx.lineWidth = 5; ctx.stroke(p); ctx.strokeStyle = TH.ink; ctx.lineWidth = 2.2; ctx.stroke(p);
}
function fmt(x) { return Number(x).toLocaleString(undefined, { maximumFractionDigits: 0 }); }
// A time in seconds: to 0.01 s under 10 s, to 0.1 s from 10 s on (9.996 s, rounded to 10 s, is "10.0 s").
function secs(t) { return (Math.round(t * 100) < 1000 ? t.toFixed(2) : t.toFixed(1)) + " s"; }
// The seconds each car crossing link l loses: its mean time less its free-flow time, never below 0.
function lost(l) { return Math.max(meanT[l] - FF[l], 0); }
// A count and its noun, singular for one: "1 cyclist", "3 cyclists".
function count(n, one) { return fmt(n) + " " + one + (n === 1 ? "" : "s"); }
// A car link's lanes and free-flow speed; its facts, those and its length.
function lanesSpeed(l) { return LANES[l] + (LANES[l] === 1 ? " lane" : " lanes") + " · " + fmt(SPD[l]) + " km/h"; }
function carFacts(l) { return lanesSpeed(l) + " · " + fmt(LEN[l]) + " m"; }
function tip(l, ev) {
  if (l < 0) { $("tip").hidden = true; return; }
  // With road classes, the class and the length, then a car link's lanes and speed; without them, a car link's facts
  // or another link's length.
  const rows = CLASSED ? [["Class", className(l) + " · " + fmt(LEN[l]) + " m"]].concat(LM[l] & CAR ? [["Lanes, speed", lanesSpeed(l)]] : [])
    : [[LM[l] & CAR ? "Lanes, speed" : "Length", LM[l] & CAR ? carFacts(l) : fmt(LEN[l]) + " m"]];
  rows.push(["Direction", "from node " + nodeId(LU[l]) + " to node " + nodeId(LD[l])]);
  if (TWIN[l] >= 0) rows.push(["Other direction", "link " + linkId(TWIN[l]) + (HAS_TRAFFIC ? ", " + fmt(vol[TWIN[l]]) + " veh/h" : "")]);
  if (HAS_TRAFFIC) { rows.push(["Volume", fmt(vol[l]) + " veh/h"]); if (vol[l] > 0) {
    // The colour's quantities: the delay over free flow, mean time / free flow - 1, and the seconds each car loses
    // (under FLOOR, the link is drawn as free flow).
    rows.push(["Delay", fmt(delay[l] * 100) + "% over free flow · " + secs(lost(l)) + " per car"]);
    rows.push(["Mean time", secs(meanT[l])]);
    rows.push(["Free flow", secs(FF[l])]); } }
  if (CAP[l] > 0) rows.push(["Capacity", fmt(CAP[l]) + " veh/h" + (vol[l] > 0 ? " (" + fmt(vol[l] / CAP[l] * 100) + "% used" + (S.bins[1] - S.bins[0] > 1 ? ", " + fmt(peak[l] / CAP[l] * 100) + "% in its busiest " + peakWords() : "") + ")" : "")]);
  const n = linkTravellers(l); if (n) rows.push(["Trips shown", count(n, "traveller") + " whose journey uses it (click for their pairs)"]);
  showTip("link " + linkId(l), rows, ev);
}
let raf = 0, idle = 0;
function redraw(moving) {
  MOVING = !!moving; if (raf) return;
  raf = requestAnimationFrame(() => { raf = 0; draw(); });
  clearTimeout(idle); if (moving) idle = setTimeout(() => { MOVING = false; draw(); writeHash(); }, 160);
}
let drag = null;
canvas.addEventListener("pointerdown", (e) => { drag = { x: e.clientX, y: e.clientY, cx: S.cx, cy: S.cy, moved: false }; canvas.setPointerCapture(e.pointerId); });
function setCursor(it) { canvas.style.cursor = drag && drag.moved ? "grabbing" : S.picking ? "crosshair" : it ? "pointer" : "default"; }
canvas.addEventListener("pointermove", (e) => {
  if (drag) { const dx = e.clientX - drag.x, dy = e.clientY - drag.y; if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true; S.cx = drag.cx - dx * S.mpp; S.cy = drag.cy + dy * S.mpp; setCursor(null); redraw(true); return; }
  const it = pickItem(e.offsetX, e.offsetY), l = it && it.kind === "road" ? it.id : -1; setCursor(it);
  if (l !== S.hover || !sameItem(it, S.hoverItem)) { S.hover = l; S.hoverItem = it; redraw(false); }
  tipItem(it, e);
});
canvas.addEventListener("pointerup", (e) => {
  if (drag && !drag.moved && S.picking) pickAt(e.offsetX, e.offsetY);
  // A click on a link, a cyclists' or walkers' line or a hop selects it; on a stop or the empty map it closes the link
  // panel when no pair is shown, and closes nothing while one is (×, Clear and Esc close).
  else if (drag && !drag.moved) { const it = pickItem(e.offsetX, e.offsetY); if (it && !it.stop) selectItem(it); else if (S.pair < 0) selectItem(null); }
  drag = null; setCursor(null); writeHash();
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
function levelButtons() { const cur = currentLevel(); document.querySelectorAll("#levels button").forEach((b, i) => { b.classList.toggle("on", i === cur); b.disabled = i > 0 && levelMpp(i) <= MIN_MPP && levelMpp(i - 1) <= MIN_MPP; }); }

// ---------------------------------------------------------- selection ----
function minutes(sec) { const m = sec / 60; return m < 10 ? m.toFixed(1) : String(Math.round(m)); }
// A pair's straight line from door to door, km.
function pairKm(p) { return Math.hypot(A.qdx[p] - A.qox[p], A.qdy[p] - A.qoy[p]) * 0.1 / 1000; }
// A pair's place: its origin (0) or destination (1), named as the encoder names it.
function placeName(p, end) { const n = PLACES[end ? A.qpd[p] : A.qpo[p]]; return n || (end ? "destination" : "origin"); }
function pairText(p) { return "#" + (p + 1) + " · " + placeName(p, 0) + " → " + placeName(p, 1) + " · " + fmt(A.qn[p]) + (A.qn[p] === 1 ? " traveller" : " travellers"); }
// Show pair k (-1: none), the view fitted to it unless `fit` is false; a link selected stays. A pair the page does not
// hold is ignored.
function selectPair(k, fit) {
  if (!HAS_ROUTES || !(Number.isInteger(k) && k >= -1 && k < NP)) return;
  S.pair = k; S.hoverOpt = -1; LEGPTS = new Map();
  if (k >= 0) {
    let x0 = Math.min(A.qox[k], A.qdx[k]) * 0.1, x1 = Math.max(A.qox[k], A.qdx[k]) * 0.1, y0 = Math.min(A.qoy[k], A.qdy[k]) * 0.1, y1 = Math.max(A.qoy[k], A.qdy[k]) * 0.1;
    for (const o of pairOptions(k)) for (let g = A.ol[o]; g < A.ol[o + 1]; g++) {
      const pts = legPoints(g); LEGPTS.set(g, pts);
      for (let i = 0; i < pts.length; i += 2) { x0 = Math.min(x0, pts[i]); x1 = Math.max(x1, pts[i]); y0 = Math.min(y0, pts[i + 1]); y1 = Math.max(y1, pts[i + 1]); }
    }
    if (fit !== false) { fitTo(x0, y0, x1, y1, 0.22); S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, S.mpp)); }
  }
  $("pairpick").value = String(k); tripPanel(); renderLegend(); if (S.sel) linkPanel(); redraw(false); levelButtons(); writeHash();
}
// A journey's legs as a strip, each as wide as its share of the time, the mean over its travellers who finished;
// none when a leg's seconds are unknown (a walk without its seconds in the run's files): its time alone is shown.
function strip(o) {
  const legs = []; let total = 0;
  for (let g = A.ol[o]; g < A.ol[o + 1]; g++) { const sec = A.lsec[g]; if (Number.isNaN(sec)) return ""; if (sec > 0) { legs.push(g); total += sec; } }
  if (!total) return "";
  const word = { 0: "drive", 1: "ride a bike", 2: "walk", 3: "ride", 4: "walk", 5: "wait" };
  return '<div class="strip">' + legs.map((g) => {
    // The journey's colour, each leg in its style as on the map (legStyle); waiting hatched.
    const k = A.lk[g], sec = A.lsec[g], cls = k === LEG.wait ? "wait" : legStyle(g);
    const style = "flex:" + (sec / total).toFixed(4) + " 1 0;--c:" + journeyColour(o) + ";";
    const what = k === LEG.ride ? "ride " + lineOf(g) : word[k];
    return '<span class="' + cls + '" style="' + style + '" title="' + escHtml(what) + " · " + minutes(sec) + ' min"></span>';
  }).join("") + "</div>";
}
// A journey's label: its mode's name, or a transit journey's lines in order ("1 → 19"), "Transit" without them.
function label(o) {
  const m = A.om[o];
  if (m !== 3) return MODES[m][0].toUpperCase() + MODES[m].slice(1);
  const names = []; for (let g = A.ol[o]; g < A.ol[o + 1]; g++) if (A.lk[g] === LEG.ride) names.push(lineOf(g));
  return names.every(Boolean) && names.length ? names.join(" → ") : "Transit";
}
function optionTitle(o) {
  if (A.om[o] === 0) return "Car " + carRank(o);
  // Journeys of one name in a pair (two bike routes, two ways by line 62) are numbered in the list's order.
  const same = pairOptions(OPT_PAIR[o]).filter((q) => A.om[q] !== 0 && label(q) === label(o));
  const k = same.indexOf(o) + 1; return label(o) + (same.length > 1 ? (/\d$/.test(label(o)) ? " (" + k + ")" : " " + k) : "");
}
function tripPanel() {
  const box = $("tripcard");
  if (S.pair < 0) { box.innerHTML = ""; $("routehint").hidden = false; pickHint(); return; }
  $("routehint").hidden = true;
  const p = S.pair, total = A.qn[p], opts = pairOptions(p), byMode = new Map(), all = total || 1;
  for (const o of opts) byMode.set(A.om[o], (byMode.get(A.om[o]) || 0) + A.on[o]);
  let h = '<div class="od"><span class="mk">A</span><b>' + escHtml(placeName(p, 0)) + '</b> → <span class="mk">B</span><b>' + escHtml(placeName(p, 1)) + "</b></div>";
  h += '<div class="odsub">' + fmt(total) + (total === 1 ? " traveller" : " travellers") + " · " + pairKm(p).toFixed(1) + " km</div>";
  const modes = [...byMode.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0]);
  h += '<div class="chips">' + modes.map(([m, n]) => '<label class="chip' + (S.hidden.has(m) ? " off" : "") + '" title="Draw this mode\'s journeys on the map, or not"><input type="checkbox" data-m="' + m + '"' + (S.hidden.has(m) ? "" : " checked") + "> " + MODES[m] + " " + Math.round(100 * n / all) + "%</label>").join("") + "</div>";
  h += '<div class="opthead"><span>Journey</span><span>share</span><span>time</span></div>';
  for (const o of opts) {
    if (!listedOption(o)) continue;
    const n = A.on[o], drawn = !S.hidden.has(A.om[o]);
    // One line a journey: its colour, its name, its legs' strip, its share of the pair and its time; the rest on hover.
    const more = MODES[A.om[o]] + " · " + fmt(n) + (n === 1 ? " traveller" : " travellers") + (drawn ? "" : " · not drawn");
    h += '<div class="opt' + (drawn ? "" : " off") + (S.hoverOpt === o ? " hl" : "") + '" data-o="' + o + '" title="' + escHtml(more) + '"><i class="dot" style="background:' + journeyColour(o) + '"></i><b>' + escHtml(optionTitle(o)) + "</b>" +
      strip(o) + '<span class="pc">' + Math.max(1, Math.round(n / all * 100)) + '%</span><span class="t">' + (isFinite(A.ot[o]) ? minutes(A.ot[o]) + " min" : "") + "</span></div>";
  }
  const links = [];
  if (opts.length > MAX_SHOWN) links.push('<a id="toggleall">' + (S.allOpts ? "Top " + MAX_SHOWN + " only" : "All " + opts.length + " journeys") + "</a>");
  if (A.qr[p] > 0) links.push("<span>" + fmt(A.qr[p]) + " without an alternative</span>");
  if (links.length) h += '<div class="more">' + links.join(" · ") + "</div>";
  box.title = "Times are door to door as the travellers made them, the mean over those who reached their door within the window. Hover a journey to see it alone; click it to zoom to it.";
  box.innerHTML = h;
  box.querySelectorAll(".chip input").forEach((c) => c.addEventListener("change", () => { const m = +c.dataset.m; if (c.checked) S.hidden.delete(m); else S.hidden.add(m); S.hoverOpt = -1; tripPanel(); renderLegend(); redraw(false); writeHash(); }));
  box.querySelectorAll(".opt").forEach((row) => {
    const o = +row.dataset.o;
    row.addEventListener("pointerenter", () => { if (S.hidden.has(A.om[o])) return; S.hoverOpt = o; redraw(false); });
    row.addEventListener("pointerleave", () => { S.hoverOpt = -1; redraw(false); });
    row.addEventListener("click", () => {
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      for (let g = A.ol[o]; g < A.ol[o + 1]; g++) { const pts = LEGPTS.get(g) || []; for (let i = 0; i < pts.length; i += 2) { x0 = Math.min(x0, pts[i]); x1 = Math.max(x1, pts[i]); y0 = Math.min(y0, pts[i + 1]); y1 = Math.max(y1, pts[i + 1]); } }
      if (isFinite(x0)) { fitTo(x0, y0, x1, y1, 0.25); S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, S.mpp)); redraw(false); levelButtons(); writeHash(); }
    });
  });
  const ta = $("toggleall"); if (ta) ta.addEventListener("click", () => { S.allOpts = !S.allOpts; S.hoverOpt = -1; tripPanel(); redraw(false); writeHash(); });
}
// ---- link analysis: the pairs whose journeys use a link, by car, bike and walk together, or a transit hop between two
// stops (ridden, by its kind of service). The hops' index is made the first time it is asked. ----
let HOP_USERS = null;
function hopUsers() {
  if (HOP_USERS) return HOP_USERS;
  HOP_USERS = new Map();
  for (let o = 0; o < A.om.length; o++) for (let g = A.ol[o]; g < A.ol[o + 1]; g++) {
    if (A.lk[g] !== LEG.ride) continue;
    const kind = A.lline[g] === NONE ? KINDS.length - 1 : LINE_KINDS[A.lline[g]];
    for (let i = A.la[g]; i < A.lb[g] - 1; i++) { const key = kind + ":" + A.pst[i] + ":" + A.pst[i + 1]; let s = HOP_USERS.get(key); if (!s) HOP_USERS.set(key, s = new Set()); s.add(o); }
  }
  return HOP_USERS;
}
function linkUsers(it) {
  if (it.kind !== "transit") return LOPTS.subarray(LOPT_START[it.id], LOPT_START[it.id + 1]);
  return hopUsers().get(A.tk[it.id] + ":" + A.ta[it.id] + ":" + A.tb[it.id]) || [];
}
const LINK_ROWS = 8;
function selectLink(l) { selectItem(l >= 0 ? { kind: "road", id: l } : null); }
// Select a link (road, bike or walk) or a hop; one the page does not hold is ignored.
// A bike or walk link is one the mode may use, on a page with that layer.
function selectItem(it) {
  if (it && !(Number.isInteger(it.id) && it.id >= 0 && it.id < (it.kind === "transit" ? A.ta.length : NL))) return;
  if (it && (it.kind === "bike" || it.kind === "walk") && !(FLOWS[it.kind] && (LM[it.id] & FLOWS[it.kind].bit))) return;
  S.sel = it; S.link = it && it.kind === "road" ? it.id : -1; S.allPairs = false; linkPanel(); redraw(false); writeHash();
}
function linkPanel() {
  const it = S.sel, box = $("linkinfo");
  box.hidden = !it; $("linkhint").hidden = !!it;
  if (!it) return;
  const i = it.id;
  let name, facts = [];
  if (it.kind === "road") {
    name = "Link " + linkId(i) + " · " + (CLASSED ? className(i) + " · " + fmt(LEN[i]) + " m" : linkKind(i) + " · " + (LM[i] & CAR ? carFacts(i) : fmt(LEN[i]) + " m"));
    if (HAS_TRAFFIC && vol[i] > 0) facts = [fmt(vol[i]) + " veh/h", "delay " + fmt(delay[i] * 100) + "% (" + secs(lost(i)) + " per car)"].concat(CAP[i] > 0 ? [fmt(vol[i] / CAP[i] * 100) + "% of capacity"] : []);
  } else if (it.kind === "transit") {
    name = KINDS[A.tk[i]] + " · " + STOPS[A.ta[i]] + " → " + STOPS[A.tb[i]];
    facts = [fmt(A.tl[i]) + " passengers on board"];
  } else {
    const f = FLOWS[it.kind];
    name = (it.kind === "bike" ? "Bike" : "Walk") + " link · " + fmt(LEN[i]) + " m";
    facts = [count(f.v[i], it.kind === "bike" ? "cyclist" : "walker")];
  }
  let h = '<div class="lname"><b>' + escHtml(name) + '</b><a id="linkclose" title="Close">×</a></div>' + (facts.length ? '<div class="lfacts">' + facts.join(" · ") + "</div>" : "");
  if (HAS_ROUTES) {
    const byPair = new Map(); let n = 0;
    for (const o of linkUsers(it)) { byPair.set(OPT_PAIR[o], (byPair.get(OPT_PAIR[o]) || 0) + A.on[o]); n += A.on[o]; }
    const rows = [...byPair.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0]), top = rows.length ? rows[0][1] : 1;
    h += '<div class="lsum" title="Counted over the ' + fmt(NP) + ' busiest pairs, the ones this page holds">' + (rows.length ? "<b>" + fmt(n) + "</b> travellers · <b>" + fmt(rows.length) + "</b> pair" + (rows.length === 1 ? "" : "s") : "No pairs") + "</div>";
    for (const [k, c] of rows.slice(0, S.allPairs ? rows.length : LINK_ROWS))
      h += '<div class="prow' + (S.pair === k ? " on" : "") + '" data-k="' + k + '" title="' + escHtml(pairText(k)) + '"><b>Pair ' + (k + 1) + '</b><i><u style="width:' + (100 * c / top).toFixed(0) + '%"></u></i><span>' + fmt(c) + "</span></div>";
    if (rows.length > LINK_ROWS) h += '<div class="more"><a id="allpairs">' + (S.allPairs ? "Top " + LINK_ROWS + " only" : "All " + rows.length + " pairs") + "</a></div>";
  }
  box.innerHTML = h;
  $("linkclose").addEventListener("click", () => selectItem(null));
  // A row shows its pair's journeys where the view is, the link staying selected; the open pair's row closes it.
  box.querySelectorAll(".prow").forEach((r) => r.addEventListener("click", () => { const k = +r.dataset.k; selectPair(S.pair === k ? -1 : k, false); }));
  const ap = $("allpairs"); if (ap) ap.addEventListener("click", () => { S.allPairs = !S.allPairs; linkPanel(); });
}

// ------------------------------------------------------------ controls ----
// A time of day, "08:15", from seconds of the clock.
function clock(seconds) { const m = Math.round(seconds / 60), h = Math.floor(m / 60); return String(h).padStart(2, "0") + ":" + String(m % 60).padStart(2, "0"); }
function binLabel() {
  const [a, b] = S.bins, s = META.bin_seconds;
  return (b - a === NB && NB > 1 ? "all bins, " : "") + clock(START + a * s) + "–" + clock(START + b * s);
}
function setBins(a, b) { S.bins = [a, b]; aggregate(a, b); subtitle(); redraw(false); writeHash(); }
// Play steps road traffic's bins, one every 0.45 s; Pause, the whole period ticked and road traffic hidden stop it, so
// the box, the slider and what is drawn agree.
let PLAY = 0;
function stopPlay() { S.playing = false; clearTimeout(PLAY); $("play").textContent = "Play"; }
const COLOUR_WORDS = { delay: "delay over free flow", volume: "volume", vc: "volume over capacity" };
// The subtitle is road traffic's caption, written only while road traffic is drawn, as each other layer's words are;
// a run without car traffic says so there, its network drawn plain.
function subtitle() {
  $("subtitle").textContent = !HAS_TRAFFIC ? "No car traffic in this run" : S.ly.road ? binLabel() + " · one ribbon per direction · width = road capacity, colour = " + COLOUR_WORDS[S.colour] : "";
  renderLegend();
}
// What the road's colour shows now: its title, its bands or ramp.
function roadLegend() {
  // The delay's title says the floor: a link whose cars lose under FLOOR seconds each is in the first band.
  if (S.colour === "delay") return { label: "Delay over free flow: mean time ÷ free-flow time − 1; a link whose cars lose under " + FLOOR + " s each is drawn as " + BAND_LABELS[0], bands: BAND_LABELS, stops: TH.bands };
  if (S.colour === "vc") return { label: "Volume over capacity" + (S.bins[1] - S.bins[0] > 1 ? ", the busiest " + peakWords() : ""), bands: VC_LABELS, stops: TH.bands };
  // The colour is the square root of the volume over VMAX: the middle of the bar is a quarter of VMAX.
  return { label: "Volume per direction (veh/h)", low: "0", mid: fmt(VMAX / 4), high: fmt(VMAX) + "+", stops: TABLES.volume };
}
function bandGradient(c) { const n = c.length; return "linear-gradient(90deg," + c.map((x, i) => x + " " + (100 * i / n).toFixed(2) + "% " + (100 * (i + 1) / n).toFixed(2) + "%").join(",") + ")"; }
// What each leg's line means: its style, in the ink; the colour is the journey's, as in the list. Only the kinds of
// leg the journeys drawn have.
function tripLegendItems() {
  const has = new Set();
  for (const o of pairOptions(S.pair).filter(shownOption)) for (let g = A.ol[o]; g < A.ol[o + 1]; g++) if (A.lk[g] !== LEG.wait) has.add(legStyle(g));
  return Object.keys(LEG_STYLE_NAMES).filter((st) => has.has(st)).map((st) => [LEG_STYLE_NAMES[st], TH.ink2, st]);
}
// The marks of the places the journeys drawn pass, as [name, mark]: the origin and destination always.
function tripMarkItems() {
  const items = [["origin", "A"], ["destination", "B"]];
  if (pairOptions(S.pair).filter(shownOption).some((o) => { for (let g = A.ol[o]; g < A.ol[o + 1]; g++) if (A.lk[g] === LEG.ride) return true; return false; })) items.push(["stop", "stop"]);
  return items;
}
// The journeys drawn, each with its colour and its style (an export's legend: it has no list beside it).
function tripJourneyItems() { return pairOptions(S.pair).filter(shownOption).map((o) => { const t = optionTitle(o); return [t.length > 26 ? t.slice(0, 25) + "…" : t, journeyColour(o), journeyStyle(o)]; }); }
// The legend: a section for each layer that is on, and the trips' marks while a pair is shown.
// The kinds of transit service the hops have, by name and index.
function transitKinds() { const seen = new Set(A.tk); return KINDS.map((name, k) => [name, k]).filter(([, k]) => seen.has(k)); }
function renderLegend() {
  const line = (c, style) => '<i style="border-top:' + (style === "dash" ? "3px dashed " : style === "dot" ? "3px dotted " : "4px solid ") + c + '"></i>';
  const parts = [];
  if (S.pair >= 0) {
    // The legs' styles and the places' marks, drawn by the map's own functions.
    const swatch = (st) => svgOf(26, 10, (rec) => drawSwatch(rec, 2, 5, 22, st, TH.ink2)), mark = (m) => svgOf(24, 22, (rec) => drawMark(rec, m, 12, 11));
    parts.push('<div class="lg"><h3>Trips between two places</h3><div class="kinds">' + tripLegendItems().map(([name, , st]) => "<span>" + swatch(st) + name + "</span>").join("") +
      '</div><div class="kinds">' + tripMarkItems().map(([name, m]) => "<span>" + mark(m) + name + "</span>").join("") + '</div><div class="note">line = mode; colour = journey, the most travelled first, as in the list</div></div>');
  }
  if (HAS_TRAFFIC && S.ly.road) {
    const r = roadLegend();
    const ramp = r.bands
      ? '<div class="bar" style="background:' + bandGradient(r.stops) + '"></div><div class="bands">' + r.bands.map((t) => "<span>" + t + "</span>").join("") + "</div>"
      : '<div class="bar" style="background:linear-gradient(90deg,' + r.stops.join(",") + ')"></div><div class="ends"><span>' + r.low + "</span><span>" + r.mid + "</span><span>" + r.high + "</span></div>";
    parts.push('<div class="lg"><h3>Road traffic</h3><div class="sub">' + r.label + "</div>" + ramp + '<div class="note">' + (RR ? "width = road capacity per direction; volume in the hover" : "width: one for every road") + "</div></div>");
  }
  if (S.ly.transit && HAS_TRANSIT) parts.push('<div class="lg"><h3>Transit lines</h3><div class="kinds">' + transitKinds().map(([name, k]) => "<span>" + line(TH.kinds[k]) + name + "</span>").join("") + '</div><div class="note">width = passengers on board; ○ stops, size = boardings</div></div>');
  if (S.ly.bike && FLOWS.bike) parts.push('<div class="lg"><h3>Cyclists</h3><div class="kinds"><span>' + line(TH.modes[1], "dash") + 'cyclists</span></div><div class="note">width = cyclists, full at ' + fmt(FLOW_FULL) + ' and more over the run; faint = the bike network</div></div>');
  if (S.ly.walk && FLOWS.walk) parts.push('<div class="lg"><h3>Walkers</h3><div class="kinds"><span>' + line(TH.modes[2], "dot") + 'walkers</span></div><div class="note">width = walkers, full at ' + fmt(FLOW_FULL) + ' and more over the run (as cyclists); faint = the walk network</div></div>');
  $("legendbody").innerHTML = parts.join("");
  // The card always holds the compass and the scale, the legend above them when a layer is on.
  $("legendbody").hidden = parts.length === 0;
}
function writeHash() {
  const p = new URLSearchParams(); p.set("t", S.theme); p.set("c", S.colour);
  p.set("b", S.bins[1] - S.bins[0] === NB ? "all" : String(S.bins[0]));
  if (S.pair >= 0) p.set("p", S.pair); if (S.link >= 0) p.set("l", S.link); else if (S.sel) p.set("li", S.sel.kind[0] + S.sel.id);
  if (S.hidden.size) p.set("hm", [...S.hidden].join(".")); if (S.allOpts) p.set("ao", "1");
  const on = Object.keys(LAYER_KEY).filter((k) => S.ly[k]).map((k) => LAYER_KEY[k]).join(""); if (on !== "r") p.set("ly", on || "-");
  p.set("z", [S.cx.toFixed(1), S.cy.toFixed(1), S.mpp.toFixed(3)].join(","));
  history.replaceState(null, "", "#" + p.toString());
}
function readHash() {
  const p = new URLSearchParams(location.hash.slice(1));
  if (p.get("t") && TOK[p.get("t")]) S.theme = p.get("t");
  if (["delay", "volume", "vc"].includes(p.get("c"))) S.colour = p.get("c");
  if (p.get("ly") !== null) for (const k of Object.keys(LAYER_KEY)) S.ly[k] = p.get("ly").includes(LAYER_KEY[k]);
  if (p.get("hm")) for (const m of p.get("hm").split(".")) if (/^[0-3]$/.test(m)) S.hidden.add(+m);
  if (p.get("ao") === "1") S.allOpts = true;
  // A bin the window does not have is ignored.
  const b = p.get("b"); if (b && b !== "all" && HAS_TRAFFIC && /^\d+$/.test(b) && +b < NB) S.bins = [+b, +b + 1];
  return p;
}
