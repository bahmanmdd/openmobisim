
// ---------------------------------------------------------------- export ----
// The view as it is, with its title, legend and provenance: a PNG at twice the screen's resolution, or an SVG drawn
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
    // A halo behind a label (the stops' ids).
    strokeText(t, x, y) {
      const anchor = { center: "middle", right: "end", end: "end" }[r.textAlign] || "start";
      out.push(`<text x="${x}" y="${y}" fill="none" stroke="${r.strokeStyle}" stroke-width="${r.lineWidth}" stroke-linejoin="round" text-anchor="${anchor}" dominant-baseline="central" style="font:${esc(r.font)}"${style(r)}>${esc(t)}</text>`);
    },
    measureText(t) { const c = document.createElement("canvas").getContext("2d"); c.font = r.font; return c.measureText(t); },
  };
  return r;
}
// The title above the map and the legend and provenance below it: the words and colours the page shows.
function exportFrame() {
  const legend = HAS_TRAFFIC && S.ly.road ? roadLegend() : null;
  // The other layers shown, each a title and its swatches [name, colour, style].
  const others = [];
  if (S.ly.transit && HAS_TRANSIT) others.push({ title: "Transit lines · width = passengers on board", items: transitKinds().map(([name, k]) => [name, TH.kinds[k], "drive"]) });
  if (S.ly.bike && FLOWS.bike) others.push({ title: "Cyclists · width = cyclists", items: [["cyclists", TH.modes[1], "bike"]] });
  if (S.ly.walk && FLOWS.walk) others.push({ title: "Walkers · width = walkers", items: [["walkers", TH.modes[2], "walk"]] });
  if (S.pair >= 0) others.unshift({ title: "Trips " + placeName(S.pair, 0) + " → " + placeName(S.pair, 1) + " · line = mode; colour = journey, the most travelled first", items: tripJourneyItems() }, { title: "Legs", items: tripLegendItems() }, { title: "Places", items: tripMarkItems().map(([name, m]) => [name, "", "mark:" + m]) });
  return { title: $("title").textContent, subtitle: $("subtitle").textContent, legend, others, provenance: $("provenance").textContent, source: $("source").textContent };
}
const HEAD = 64, FOOT = 46, ROW = 32;
// Where an export's legends below the map go: each group of items placed left to right, a new row when the next would
// pass the right edge; then the provenance and the wordmark on a line of their own. Returns each group's place, each
// item's x, and the foot's height. Measured in the viewer's font, the same for the PNG and the SVG.
function exportLayout(f) {
  const m = document.createElement("canvas").getContext("2d"), top = HEAD + H + 18, groups = [];
  let ox = 16, row = 0;
  for (const g of f.others) {
    m.font = "10px " + FONT; const tw = m.measureText(g.title).width; m.font = "9.5px " + FONT;
    const xs = []; let w = 0; for (const [name] of g.items) { xs.push(w); w += 32 + m.measureText(name).width; }
    const gw = Math.max(w, tw);
    if (ox > 16 && ox + gw > W - 16) { row++; ox = 16; }
    groups.push({ g, x: ox, y: top + row * ROW, xs }); ox += gw + 24;
  }
  const rows = groups.length ? row + 1 : 0, line = groups.length ? top + rows * ROW - 2 : top;
  return { groups, line, foot: groups.length ? 18 + rows * ROW + 30 : FOOT };
}
// One item of an export's legend at (x, y), 16 px wide: a place's mark ("mark:<kind>", drawn small) or a leg's style.
function exportItem(c2d, x, y, colour, style) {
  if (style.startsWith("mark:")) {
    if (c2d.save) { c2d.save(); c2d.translate(x + 8, y); c2d.scale(0.7, 0.7); drawMark(c2d, style.slice(5), 0, 0); c2d.restore(); }
    else { const inner = svgRecorder(); drawMark(inner, style.slice(5), 0, 0); c2d.out.push(`<g transform="translate(${x + 8} ${y}) scale(0.7)">${inner.out.join("")}</g>`); }
    return;
  }
  drawSwatch(c2d, x, y, 16, style || "drive", colour);
}
// An export has no cards: what the screen shows between them is centred in the export's whole width, at the same scale.
function viewWithMoving(drawInto) {
  const moving = MOVING, hover = S.hover, item = S.hoverItem, cx = S.cx; MOVING = false; S.hover = -1; S.hoverItem = null;
  S.cx -= panelWidth() / 2 * S.mpp; EXPORTING = true;
  try { drawInto(); } finally { MOVING = moving; S.hover = hover; S.hoverItem = item; S.cx = cx; EXPORTING = false; }
}
// Where an export's mark goes: before the wordmark (right-aligned at W - 16, baseline y + 20, bold 11 px), as measured
// in the viewer's font, and centred on its lowercase body (an "x"'s height).
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
  const lay = exportLayout(f), total = H + HEAD + lay.foot, parts = [];
  // The SVG namespace from the page's own compass (the page names no address anywhere).
  const ns = $("compass").querySelector("svg").namespaceURI;
  parts.push(`<svg xmlns="${ns}" width="${W}" height="${total}" viewBox="0 0 ${W} ${total}" font-family="${esc(FONT)}">`);
  parts.push(`<rect width="${W}" height="${total}" fill="${TH.surface}"/>`);
  parts.push(`<g transform="translate(0 ${HEAD})">${rec.out.join("")}</g>`);
  // The bands above and below the map, painted over whatever of it runs past the view.
  parts.push(`<rect width="${W}" height="${HEAD}" fill="${TH.surface}"/><rect y="${HEAD + H}" width="${W}" height="${lay.foot}" fill="${TH.surface}"/>`);
  parts.push(`<text x="16" y="26" font-size="17" font-weight="700" fill="${TH.ink}">${esc(f.title)}</text>`);
  parts.push(`<text x="16" y="48" font-size="11.5" fill="${TH.ink2}">${esc(f.subtitle)}</text>`);
  const y = lay.line;
  if (f.legend) {  // at the top right
    const n = f.legend.stops.length, w = 200, step = w / n, x0 = W - 16 - w, ly = 30;
    parts.push(`<text x="${x0 + w}" y="${ly - 8}" font-size="10.5" text-anchor="end" fill="${TH.ink2}">${esc(f.legend.label)}</text>`);  // right-aligned: a long title runs left, not off the page
    parts.push(f.legend.stops.map((c, i) => `<rect x="${(x0 + i * step).toFixed(2)}" y="${ly}" width="${(f.legend.bands ? step - 1.5 : step + 0.3).toFixed(2)}" height="7" fill="${c}"/>`).join(""));
    if (f.legend.bands) parts.push(f.legend.bands.map((t, i) => `<text x="${(x0 + (i + 0.5) * step).toFixed(2)}" y="${ly + 19}" font-size="9.5" text-anchor="middle" fill="${TH.ink2}">${esc(t)}</text>`).join(""));
    else parts.push(`<text x="${x0}" y="${ly + 19}" font-size="9.5" fill="${TH.ink2}">${esc(f.legend.low)}</text><text x="${x0 + w / 2}" y="${ly + 19}" font-size="9.5" text-anchor="middle" fill="${TH.ink2}">${esc(f.legend.mid)}</text><text x="${x0 + w}" y="${ly + 19}" font-size="9.5" text-anchor="end" fill="${TH.ink2}">${esc(f.legend.high)}</text>`);
  }
  for (const { g, x: gx, y: gy, xs } of lay.groups) {  // the other layers' legends below the map, in rows
    parts.push(`<text x="${gx}" y="${gy - 2}" font-size="10" fill="${TH.ink2}">${esc(g.title)}</text>`);
    g.items.forEach(([name, c, style], i) => {
      // Each item drawn by the map's own functions: a leg's style, or a place's mark.
      const x = gx + xs[i], rec = svgRecorder(); exportItem(rec, x, gy + 11, c, style);
      parts.push(rec.out.join("") + `<text x="${x + 21}" y="${gy + 14}" font-size="9.5" fill="${TH.ink2}">${esc(name)}</text>`);
    });
  }
  parts.push(`<text x="${W - 16}" y="${y + 4}" font-size="9.5" text-anchor="end" fill="${TH.ink2}">${esc([f.provenance, f.source].filter(Boolean).join(" · "))}</text>`);
  parts.push(`<text x="${W - 16}" y="${y + 20}" font-size="11" font-weight="700" text-anchor="end" fill="${TH.ink}">openmobisim</text>`);
  const m = markPlace(y);
  parts.push(`<path transform="translate(${m.x.toFixed(2)} ${m.y.toFixed(2)}) scale(${m.s})" d="${LOGO}" fill="${TH.mark}"/>`);
  parts.push("</svg>");
  return parts.join("");
}
function exportPNG() {
  const f = exportFrame(), scale = 2, off = document.createElement("canvas");
  const lay = exportLayout(f);
  off.width = Math.round(W * scale); off.height = Math.round((H + HEAD + lay.foot) * scale);
  const o = off.getContext("2d"), map = document.createElement("canvas");
  map.width = Math.round(W * scale); map.height = Math.round(H * scale);
  const keep = ctx, keepDpr = DPR; ctx = map.getContext("2d"); DPR = scale;
  try { viewWithMoving(draw); } finally { ctx = keep; DPR = keepDpr; draw(); }
  o.setTransform(scale, 0, 0, scale, 0, 0);
  o.fillStyle = TH.surface; o.fillRect(0, 0, W, H + HEAD + lay.foot);
  o.drawImage(map, 0, HEAD, W, H);
  o.fillStyle = TH.ink; o.font = "700 17px " + FONT; o.fillText(f.title, 16, 26);
  o.fillStyle = TH.ink2; o.font = "11.5px " + FONT; o.fillText(f.subtitle, 16, 48);
  const y = lay.line;
  if (f.legend) {  // at the top right
    const n = f.legend.stops.length, w = 200, step = w / n, x0 = W - 16 - w, ly = 30;
    o.textAlign = "right"; o.fillStyle = TH.ink2; o.font = "10.5px " + FONT; o.fillText(f.legend.label, x0 + w, ly - 8); o.textAlign = "left";
    if (f.legend.bands) {
      f.legend.stops.forEach((c, i) => { o.fillStyle = c; o.fillRect(x0 + i * step, ly, step - 1.5, 7); });
      o.fillStyle = TH.ink2; o.font = "9.5px " + FONT; o.textAlign = "center";
      f.legend.bands.forEach((t, i) => o.fillText(t, x0 + (i + 0.5) * step, ly + 19));
    } else {
      const g = o.createLinearGradient(x0, 0, x0 + w, 0);
      f.legend.stops.forEach((c, i) => g.addColorStop(i / Math.max(n - 1, 1), c));
      o.fillStyle = g; o.fillRect(x0, ly, w, 7);
      o.fillStyle = TH.ink2; o.font = "9.5px " + FONT; o.fillText(f.legend.low, x0, ly + 19); o.textAlign = "center"; o.fillText(f.legend.mid, x0 + w / 2, ly + 19); o.textAlign = "right"; o.fillText(f.legend.high, x0 + w, ly + 19);
    }
  }
  for (const { g, x: gx, y: gy, xs } of lay.groups) {  // the other layers' legends below the map, in rows
    o.textAlign = "left"; o.fillStyle = TH.ink2; o.font = "10px " + FONT; o.fillText(g.title, gx, gy - 2);
    g.items.forEach(([name, c, style], i) => {
      const x = gx + xs[i]; exportItem(o, x, gy + 11, c, style); o.textAlign = "left"; o.font = "9.5px " + FONT;
      o.fillStyle = TH.ink2; o.fillText(name, x + 21, gy + 14);
    });
  }
  o.textAlign = "right"; o.fillStyle = TH.ink2; o.font = "9.5px " + FONT;
  o.fillText([f.provenance, f.source].filter(Boolean).join(" · "), W - 16, y + 4);
  o.fillStyle = TH.ink; o.font = "700 11px " + FONT; o.fillText("openmobisim", W - 16, y + 20);
  const m = markPlace(y);
  o.save(); o.translate(m.x, m.y); o.scale(m.s, m.s); o.fillStyle = TH.mark; o.fill(new Path2D(LOGO)); o.restore();
  return off.toDataURL("image/png");
}
function download(name, url) { const a = document.createElement("a"); a.href = url; a.download = name; document.body.appendChild(a); a.click(); a.remove(); }
function exportName(ext) { return (META.title || "openmobisim map").replace(/[^\w\- ]+/g, "").trim().replace(/\s+/g, "_") + "." + ext; }
function exportAs(kind) {
  const data = kind === "svg" ? exportSVG() : exportPNG();
  download(exportName(kind), kind === "svg" ? URL.createObjectURL(new Blob([data], { type: "image/svg+xml" })) : data);
}
function wire() {
  $("theme").addEventListener("click", () => { setTheme(S.theme === "paper" ? "night" : "paper"); subtitle(); redraw(false); writeHash(); });
  $("colour").addEventListener("change", (e) => { S.colour = e.target.value; subtitle(); redraw(false); writeHash(); });
  $("allbins").addEventListener("change", (e) => { if (e.target.checked) setBins(0, NB); else setBins(+$("bin").value, +$("bin").value + 1); $("bin").disabled = e.target.checked; });
  $("bin").addEventListener("input", (e) => { $("allbins").checked = false; $("bin").disabled = false; setBins(+e.target.value, +e.target.value + 1); });
  $("play").addEventListener("click", () => {
    S.playing = !S.playing; $("play").textContent = S.playing ? "Pause" : "Play";
    if (S.playing) { $("allbins").checked = false; $("bin").disabled = false; const tick = () => { if (!S.playing) return; const nb = (S.bins[1] - S.bins[0] === NB || S.bins[0] + 1 >= NB ? 0 : S.bins[0] + 1); $("bin").value = nb; setBins(nb, nb + 1); setTimeout(tick, 450); }; tick(); }
  });
  document.querySelectorAll("#levels button").forEach((b, i) => b.addEventListener("click", () => goLevel(i)));
  $("zin").addEventListener("click", () => zoomAt(W / 2, H / 2, 0.5)); $("zout").addEventListener("click", () => zoomAt(W / 2, H / 2, 2));
  $("fit").addEventListener("click", () => { fitTo(EX0, EY0, EX1, EY1, 0.06); redraw(false); levelButtons(); writeHash(); });
  $("exportpng").addEventListener("click", () => exportAs("png")); $("exportsvg").addEventListener("click", () => exportAs("svg"));
  if (HAS_ROUTES) {
    // Every pair held, the busiest first; the first line says which pairs those are.
    const pick = $("pairpick"), first = fmt(NP) + " pairs, the busiest first";
    pick.innerHTML = '<option value="-1">' + first + "…</option>" + Array.from({ length: NP }, (_, k) => '<option value="' + k + '">' + escHtml(pairText(k)) + "</option>").join("");
    pick.addEventListener("change", (e) => selectPair(+e.target.value, true));
    $("prev").addEventListener("click", () => selectPair(S.pair <= 0 ? NP - 1 : S.pair - 1, true)); $("next").addEventListener("click", () => selectPair((S.pair + 1) % NP, true));
    $("pickmap").addEventListener("click", () => { S.picking = S.picking ? 0 : 1; S.pickFrom = null; setCursor(null); pickHint(); redraw(false); });
    $("clear").addEventListener("click", () => { S.picking = 0; setCursor(null); selectPair(-1, false); selectLink(-1); });
  }
  window.addEventListener("resize", () => { resize(); redraw(false); });
  window.addEventListener("keydown", (e) => { if (e.key === "Escape") { if (S.picking) { S.picking = 0; S.pickFrom = null; setCursor(null); pickHint(); redraw(false); return; } selectLink(-1); if (S.pair >= 0) selectPair(-1, false); } });
}

// ---------------------------------------------------------------- start ----
function start() {
  $("title").textContent = META.title;
  // The run's provenance and its network's source, from its record; the size note.
  $("provenance").textContent = META.provenance; $("provenance").title = META.provenance; $("source").textContent = META.source;
  let held = 0; for (let p = 0; p < NP; p++) held += A.qn[p];
  $("budget").textContent = fmt(NP) + " of " + count(META.pairs_total, "pair") + ", holding " + fmt(held) + " of " + count(META.trips, "trip") + " · " + count(NL, "link") + " · " + META.megabytes.toFixed(1) + " MB" +
    (META.left_out ? " · " + count(META.left_out, "trip") + " left out: their routes do not agree with the trips table" : "");
  document.title = META.title + " · openmobisim";
  // The tab icon: the mark, as a data URL made here (the page names no address).
  const icon = document.createElement("link"); icon.rel = "icon";
  icon.href = "data:image/svg+xml," + encodeURIComponent(`<svg xmlns="${$("compass").querySelector("svg").namespaceURI}" viewBox="0 0 64 64"><path d="${LOGO}" fill="${TOK.paper.mark}"/></svg>`);
  document.head.appendChild(icon);
  $("routegroup").hidden = !HAS_ROUTES; $("routehint").dataset.text = $("routehint").textContent;
  if (HAS_TRAFFIC) { $("bin").min = 0; $("bin").max = NB - 1; $("bin").value = 0; $("bin").disabled = true; }
  const p = readHash(); setTheme(S.theme); $("colour").value = S.colour; resize();
  if (HAS_TRAFFIC) { const all = S.bins[1] - S.bins[0] === NB; $("allbins").checked = all; $("bin").disabled = all; $("bin").value = all ? 0 : S.bins[0]; }
  fitTo(EX0, EY0, EX1, EY1, 0.06); FIT.mpp = S.mpp; MAX_MPP = FIT.mpp * 1.6; MIN_MPP = Math.max(0.25, Math.min(0.5, FIT.mpp / 200));
  if (HAS_TRAFFIC) aggregate(S.bins[0], S.bins[1]); subtitle();
  layerRows(); wire(); $("trafficgroup").hidden = !HAS_TRAFFIC || !S.ly.road;
  if (p.get("z")) { const [x, y, m] = p.get("z").split(",").map(Number); if (isFinite(m)) { S.cx = x; S.cy = y; S.mpp = Math.max(MIN_MPP, Math.min(MAX_MPP, m)); } }
  if (p.get("p") !== null && HAS_ROUTES && /^\d+$/.test(p.get("p"))) selectPair(+p.get("p"), !p.get("z"));
  if (/^\d+$/.test(p.get("l") || "")) selectLink(+p.get("l"));
  else if (/^[bwt]\d+$/.test(p.get("li") || "")) selectItem({ kind: { b: "bike", w: "walk", t: "transit" }[p.get("li")[0]], id: +p.get("li").slice(1), stop: false });
  levelButtons(); draw(); writeHash();
  // What the page read, for a check: the links, the rows of car traffic, the pairs, their journeys, the transit hops.
  const d = document.body.dataset;
  d.links = NL; d.rows = RL.length; d.pairs = NP; d.journeys = A.om.length; d.hops = A.tl.length;
  d.state = "ready";
}
start();
})();
