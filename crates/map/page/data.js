(async function () {
"use strict";
// The page's script, in four files joined in order into one function: the data and the view here (data.js); the
// drawing (draw.js); the pointer, the panels and the legend (panels.js); the exports, the controls and the start
// (start.js).
const $ = (id) => document.getElementById(id);
const fail = (e) => { document.body.dataset.state = "error"; const b = $("error"); b.hidden = false; b.textContent = "openmobisim map: " + (e && e.message ? e.message : e); console.error(e); };
window.addEventListener("error", (ev) => fail(ev.error || ev.message));
window.addEventListener("unhandledrejection", (ev) => fail(ev.reason));

const META = JSON.parse($("meta").textContent);
const LOGO = "__LOGO__";
// The two themes (the surface, the inks, the network's grey, the mark, the journeys' colours, the modes' colours,
// the delay's ramp and its four bands, the volume's ramp), as the encoder defines them for every page of a run.
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

// The network: each node's position (metres from the network's corner), each link's two ends and its shape.
const NL = META.n_links, NN = META.n_nodes, NB = META.n_bins, START = META.window_start;
const LU = A.lu, LD = A.ld, LM = A.lm, CLS = A.cls, LEN = A.len, FF = A.ff, CAP = A.cap, LANES = A.lanes, SPD = A.spd;
// Each link's vertices, VS[l] to VS[l + 1] - 1 of X, Y: its upstream end, its shape's points in order of travel, its
// downstream end (the shape's points are each in decimetres from the one before).
const VS = new Uint32Array(NL + 1), NV = 2 * NL + A.dx.length, X = new Float64Array(NV), Y = new Float64Array(NV);
{
  let cx = 0, cy = 0, v = 0;
  const end = (n) => { X[v] = A.nx[n] * 0.1; Y[v] = A.ny[n] * 0.1; v++; };
  for (let l = 0; l < NL; l++) {
    VS[l] = v; end(LU[l]);
    for (let i = A.vs[l]; i < A.vs[l + 1]; i++) { cx += A.dx[i]; cy += A.dy[i]; X[v] = cx * 0.1; Y[v] = cy * 0.1; v++; }
    end(LD[l]);
  }
  VS[NL] = v;
}
const CAR = 1, BIKE = 2, WALK = 4;
// A link's name in the hover and the panel, by the modes it allows.
function linkKind(l) { const m = LM[l]; return m & CAR ? "car link" : (m & BIKE) && (m & WALK) ? "bike and walk link" : m & BIKE ? "bike link" : "walk link"; }
// The road hierarchy, by each link's drawing class (CLS). With a road class on every link (CLASSED): version 1's 19
// classes in order, 0 motorway (the encoder's rank); the classes shown at each zoom up to 5, 9, 13 and 16 (every road
// and path); 0 to 13 the roads, the rest (paths, cycleways) drawn only zoomed in; three fewer while moving. Without
// them: the encoder's classes by capacity, 0 the largest roads, 3 the other car links, 4 a link without cars; shown up
// to 1, 2, 3 and 4; one fewer while moving.
const CLASSED = !!META.road_classes, ROAD_CLASSES = META.road_classes, RC = A.rc;
const ZOOM = CLASSED ? { limits: [5, 9, 13, 16], roads: 13, moving: 3 } : { limits: [1, 2, 3, 4], roads: 3, moving: 1 };
const ALL_CLASSES = ZOOM.limits[3];
function isRoad(l) { return CLS[l] <= ZOOM.roads; }
// A link's road class as shown ("living street").
function className(l) { return ROAD_CLASSES[RC[l]].replace(/_/g, " "); }
// Each link's other direction among the links of a mode (`bit`): the link between the same two nodes the other way
// (-1: one way). A two-way street is two links on one line, each drawn on its own side.
function twins(bit) {
  const at = new Map(), twin = new Int32Array(NL).fill(-1);
  for (let l = 0; l < NL; l++) if (LM[l] & bit) { const k = LU[l] + ":" + LD[l]; if (!at.has(k)) at.set(k, l); }
  for (let l = 0; l < NL; l++) if (LM[l] & bit) { const r = at.get(LD[l] + ":" + LU[l]); if (r !== undefined && r !== l) twin[l] = r; }
  return twin;
}
const TWIN = twins(CAR);
// The bikes and the walkers per link over the window: two views of the same links, each drawn when its box is ticked.
function flowLayer(v, bit) {
  let any = false; for (let l = 0; l < NL; l++) if (v[l] > 0) { any = true; break; }
  return any ? { v, bit, twin: twins(bit) } : null;
}
const FLOWS = { bike: flowLayer(A.bv, BIKE), walk: flowLayer(A.wv, WALK) };
// Transit: the stops' names, the kinds of service, the lines ridden and each one's kind.
const HAS_TRANSIT = A.tl.length > 0, STOPS = META.stops, KINDS = META.kinds, LINES = META.lines, LINE_KINDS = META.line_kinds;
const LAYER_ROWS = [
  ["road", "Road traffic", "ribbons: width = road capacity, colour as below"],
  ["transit", "Transit lines", "width = passengers on board, colour = kind of service"],
  ["bike", "Cyclists", "on the bike network, the whole run"],
  ["walk", "Walkers", "on the walk network, the whole run"],
];
// A node's and a link's external id, read from the data when shown.
const TEXT = new TextDecoder();
const NODE_IDS = TEXT.decode(A.nid).split("\0"), LINK_IDS = TEXT.decode(A.lid).split("\0");
const nodeId = (n) => NODE_IDS[n], linkId = (l) => LINK_IDS[l];
const LAYER_KEY = { road: "r", transit: "t", bike: "b", walk: "w" };

// link bounding boxes and a uniform grid over them
const BX0 = new Float32Array(NL), BY0 = new Float32Array(NL), BX1 = new Float32Array(NL), BY1 = new Float32Array(NL);
let EX0 = Infinity, EY0 = Infinity, EX1 = -Infinity, EY1 = -Infinity;
for (let l = 0; l < NL; l++) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (let v = VS[l]; v < VS[l + 1]; v++) { if (X[v] < x0) x0 = X[v]; if (X[v] > x1) x1 = X[v]; if (Y[v] < y0) y0 = Y[v]; if (Y[v] > y1) y1 = Y[v]; }
  BX0[l] = x0; BY0[l] = y0; BX1[l] = x1; BY1[l] = y1;
  if (x0 < EX0) EX0 = x0; if (y0 < EY0) EY0 = y0; if (x1 > EX1) EX1 = x1; if (y1 > EY1) EY1 = y1;
}
if (!NL) { EX0 = EY0 = 0; EX1 = EY1 = 1; }
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

// per-bin traffic: the cars that entered each link in each bin of the window, and the sum of their crossing times
const BS = A.bs, RL = A.rl, RP = A.rp, RS = A.rs;
const HAS_TRAFFIC = NB > 0 && RL.length > 0;
const vol = new Float32Array(NL), peak = new Float32Array(NL), delay = new Float32Array(NL), meanT = new Float32Array(NL);
let VMAX = 1;
function percentile(values, p) { if (!values.length) return 1; const s = Float32Array.from(values).sort(); return s[Math.min(s.length - 1, Math.floor(p * s.length))]; }
let GLOBAL_VMAX = 1;
if (HAS_TRAFFIC) { const v = []; for (let r = 0; r < RL.length; r++) v.push(RP[r] / META.bin_seconds * 3600); GLOBAL_VMAX = Math.max(1, percentile(v, 0.99)); }
function aggregate(b0, b1) {
  const tot = new Float32Array(NL), sec = new Float32Array(NL);
  for (let b = b0; b < b1; b++) for (let r = BS[b]; r < BS[b + 1]; r++) { tot[RL[r]] += RP[r]; sec[RL[r]] += RS[r]; }
  // The hours the volumes are per: the bins asked for.
  const span = Math.max(1, b1 - b0) * META.bin_seconds, pos = [];
  // The busiest period's volume per hour: volume over capacity is coloured from it, so a road full for an hour of a
  // two-hour window reads full, not half used. The period is k consecutive bins, as near 15 min as whole bins come.
  const k = Math.max(1, Math.min(b1 - b0, Math.round(META.peak_seconds / META.bin_seconds)));
  PEAK = k * META.bin_seconds;
  peak.fill(0);
  const run = new Float32Array(NL), perBin = (b, f) => { for (let r = BS[b]; r < BS[b + 1]; r++) f(RL[r], RP[r]); };
  for (let b = b0; b < b1; b++) {
    perBin(b, (l, p) => { run[l] += p; });
    if (b - k >= b0) perBin(b - k, (l, p) => { run[l] -= p; });
    if (b - b0 + 1 >= k) for (let l = 0; l < NL; l++) peak[l] = Math.max(peak[l], run[l] / (k * META.bin_seconds) * 3600);
  }
  for (let l = 0; l < NL; l++) {
    vol[l] = tot[l] / span * 3600;
    // The delay over free flow, as the encoder defines it (rules::delay): the mean crossing time over the free-flow time,
    // less one (1 = twice free flow), never below 0.
    if (tot[l] > 0) { meanT[l] = sec[l] / tot[l]; delay[l] = FF[l] > 0 ? Math.max(meanT[l] / FF[l] - 1, 0) : 0; pos.push(vol[l]); } else { meanT[l] = 0; delay[l] = 0; }
  }
  VMAX = (b1 - b0 === 1) ? GLOBAL_VMAX : Math.max(1, percentile(pos, 0.99));
  VHIGH = pos.length ? percentile(pos, 0.95) : 0;
}
let VHIGH = 0, PEAK = 0;
// The busiest period's length in words: "quarter hour" at 15 min, else "20 min", "1 h".
function peakWords() { return PEAK === 900 ? "quarter hour" : PEAK % 3600 === 0 ? PEAK / 3600 + " h" : PEAK % 60 === 0 ? PEAK / 60 + " min" : PEAK + " s"; }

// trips between two places: each pair's journeys, by every mode; each pair's two places' names
const NP = META.n_pairs, HAS_ROUTES = NP > 0, PLACES = META.place_names;
const MODES = ["car", "bike", "walk", "transit"];
const LEG = { drive: 0, bike: 1, walk: 2, ride: 3, connect: 4, wait: 5 }, NONE = 4294967295;
const OPT_PAIR = new Uint32Array(A.om.length);
for (let p = 0; p < NP; p++) for (let o = A.qs[p]; o < A.qs[p + 1]; o++) OPT_PAIR[o] = p;
// Which journeys drive, ride or walk on each link: a clicked link lists the pairs whose journeys use it, by every mode.
const LOPT_START = new Uint32Array(NL + 1);
const LOPTS = (() => {
  const each = (f) => { for (let o = 0; o < A.om.length; o++) for (let g = A.ol[o]; g < A.ol[o + 1]; g++) if (A.lk[g] <= LEG.walk) for (let i = A.la[g]; i < A.lb[g]; i++) f(A.pl[i], o); };
  each((l) => LOPT_START[l + 1]++);
  for (let i = 1; i <= NL; i++) LOPT_START[i] += LOPT_START[i - 1];
  const fill = LOPT_START.slice(), out = new Uint32Array(LOPT_START[NL]);
  each((l, o) => { out[fill[l]++] = o; });
  return out;
})();

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
// The delay's bands: the upper edges of each, as META gives them, and their labels; volume over capacity's four bands
// in the same four colours, so the two read alike.
const BANDS = META.delay_bands, BAND_LABELS = META.delay_band_labels, VC_BANDS = META.vc_bands, VC_LABELS = META.vc_band_labels;
function band(d, edges) { const e = edges || BANDS; let b = 0; while (b < e.length && d >= e[b]) b++; return b; }
// A car link's delay band, as the encoder defines it (rules::delay_band): its band of the delay over free flow, the
// first when its cars lose under FLOOR seconds each (the mean time less the free-flow time), whatever the delay.
const FLOOR = META.delay_floor_s;
function delayBand(l) { return meanT[l] - FF[l] < FLOOR ? 0 : band(delay[l]); }
// The road's colours now: the four bands' (delay, volume over capacity) or the volume's ramp of NCOL steps.
function palette() { return S.colour === "volume" ? TABLES.volume : TH.bands; }
const S = { theme: "paper", colour: "vc", bins: [0, NB], playing: false, pair: -1, link: -1, sel: null, hover: -1, hoverItem: null, hoverOpt: -1, allOpts: false, allPairs: false, hidden: new Set(), picking: 0, pickFrom: null, cx: 0, cy: 0, mpp: 1, ly: { road: true, transit: false, bike: false, walk: false } };
let TH = TOK[S.theme], TABLES = {};
function setTheme(name) {
  S.theme = name; TH = TOK[name];
  const r = document.documentElement.style;
  r.setProperty("--surface", TH.surface); r.setProperty("--ink", TH.ink); r.setProperty("--ink2", TH.ink2); r.setProperty("--muted", TH.muted); r.setProperty("--base", TH.base); r.setProperty("--mark", TH.mark);
  TABLES = { volume: rampTable(TH.ramp_volume, NCOL) };
  renderLegend();
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
// The cards over the map (the legend on the left, the controls on the right): fitTo leaves their width free on each side.
function panelWidth() { return W > 760 ? 372 : 0; }  // a card (340 px), its 16 px margin and 16 px more, on each side
function fitTo(x0, y0, x1, y1, pad) {
  // Fit into the part of the window the cards, the title above and the footer below leave free.
  const top = 78, bottom = 34;
  const w = Math.max(x1 - x0, 60), h = Math.max(y1 - y0, 60), reserve = panelWidth();
  S.mpp = Math.max(w / ((W - 2 * reserve) * (1 - pad)), h / ((H - top - bottom) * (1 - pad)));
  S.cx = (x0 + x1) / 2; S.cy = (y0 + y1) / 2 + (top - bottom) / 2 * S.mpp;
}
function levelMpp(i) { return Math.max(MIN_MPP, FIT.mpp / Math.pow(3.2, i)); }
function currentLevel() { let best = 0, bd = 1e9; for (let i = 0; i < LEVELS.length; i++) { const d = Math.abs(Math.log(S.mpp / levelMpp(i))); if (d < bd) { bd = d; best = i; } } return best; }
// The drawing classes shown at this zoom, up to the limit (the rest are drawn faint): ZOOM's four limits, from an
// overview in; a small network shows every class at every zoom.
function classLimit() { const z = ZOOM.limits; return NL < 5000 ? ALL_CLASSES : S.mpp > 25 ? z[0] : S.mpp > 8 ? z[1] : S.mpp > 2.5 ? z[2] : z[3]; }
const sx = (x) => (x - S.cx) / S.mpp + W / 2, sy = (y) => H / 2 - (y - S.cy) / S.mpp;
