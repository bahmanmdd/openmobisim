//! From an extract's ways to the network, in three layers, one mode each: the car links, the
//! bike links and the walk links, each built from the ways its rules admit ([`crate::tags`]).
//! Per layer: split the ways at the layer's junctions, measure each piece, keep the largest
//! strongly connected component, merge chains of links through nodes that are no junction
//! (contraction), and hand the rows to the graph's builder. The layers share the nodes where
//! their links meet (an OpenStreetMap node is one network node).
//!
//! **Junctions.** On a layer, a node is a junction when the layer's ways use it twice or more
//! (two ways, or one way twice; the car layer counts every road, [`road_class`], footways and
//! cycleways among them), when it splits every way ([`splits`]: a signal, a stop, a barrier),
//! or when it ends a way's run of held nodes. A link runs from one junction to the next along
//! one way, its length the sum of the WGS84 geodesics between its nodes. A piece that starts and
//! ends at one node is dropped and counted; one of 0 m is left out and noted.
//!
//! **Strong component.** The largest set of nodes each reachable from every other on the
//! layer's links (the most nodes; on a tie, the first Tarjan's search completes). A link whose
//! two ends are not both in it is dropped and counted.
//!
//! **Contraction** (version 1's): a node is merged away when it splits no way, has one link in
//! and one out (a one-way chain) or two in and two out that are the two directions of one
//! street, the links merged carry the same road class and car fields, and the merge makes no
//! link from a node to itself. Lengths add; shapes join; the link keeps its first piece's id.
//! (Version 1 paired two links in and two out by the order it met them, so where a one-way
//! street met a two-way one it could merge the node away and lose a movement; here such a node
//! stays.)

use std::collections::{HashMap, HashSet};

use openmobisim_graph::geometry::{LonLat, Position};
use openmobisim_graph::network::{
    BuildValues, LINKS_OF_NO_LENGTH, LinkDrawing, LinkRow, Network, NetworkBuilder,
};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::repairs::Repairs;

use crate::geodesic::geodesic_m;
use crate::region::{Region, clip};
use crate::source::{Osm, OsmError, OsmWay};
use crate::strong::largest_component;
use crate::tags::{
    Car, CarNotes, RoadClass, bike_way, car_fields, direction, road_class, signalised, splits,
    walk_way,
};

/// What a read made of an extract, counted, what it set right instead of refusing, and the
/// region it was cut to.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Report {
    /// Nodes of the network.
    pub nodes: u32,
    /// Links of the network, of the three layers.
    pub links: u32,
    /// Links dropped because their ends are not both in their layer's largest strongly
    /// connected component, counted before contraction.
    pub links_unreachable: u32,
    /// Pieces dropped because they start and end at one node (a closed way no other way of the
    /// layer touches), of the three layers.
    pub loops_dropped: u32,
    /// Street ways holding some nodes and missing others: cut by the edge of the extract or of
    /// the region, or at a node not placeable or a segment whose geodesic does not converge.
    /// Each run of nodes it holds is read; nothing is bridged across a gap.
    pub ways_cut: u32,
    /// What the reader set right, by kind (the crate's README), the graph builder's included.
    pub repairs: Repairs,
    /// The region the extract was cut to; `None`: the whole extract.
    pub region: Option<Region>,
}

/// What one direction of a way carries on a layer: the car's fields on the car layer, none on
/// the bike and walk layers.
type Side = Option<Car>;

/// What a way gives a layer: its class, and each direction the layer uses.
struct Spec {
    class: RoadClass,
    forward: Option<Side>,
    backward: Option<Side>,
}

/// A way as read: its node ids and, per node, its position if held (inside the region, in the
/// extract, placeable, and reached by a measurable segment), with the length of the segment
/// ending there.
struct Held<'a> {
    way: &'a OsmWay,
    points: Vec<Option<(f64, f64)>>,
    segment_m: Vec<f64>,
}

impl Held<'_> {
    /// The runs of two or more consecutive held nodes, as index ranges.
    fn runs(&self) -> Vec<std::ops::Range<usize>> {
        let mut runs = Vec::new();
        let mut start = 0;
        for k in 0..=self.points.len() {
            if k == self.points.len() || self.points[k].is_none() {
                if k >= start + 2 {
                    runs.push(start..k);
                }
                start = k + 1;
            }
        }
        runs
    }
}

/// One directed link of a layer before the build.
#[derive(Clone, Debug)]
struct Link {
    mode: Mode,
    /// `<mode>:<way>:<piece>:<f|b>`: the first piece's way, its number along the way on this
    /// layer (from 0), and its direction along the way's node order or against it.
    id: String,
    from: i64,
    to: i64,
    class: RoadClass,
    car: Side,
    length_m: f64,
    /// The positions (longitude, latitude) from `from` to `to`, both included.
    points: Vec<(f64, f64)>,
}

/// The network of `osm` (cut to `region`, if given), built with `values`, and its counts and
/// repairs.
///
/// # Errors
///
/// [`OsmError::NoStreets`] if no way is a street of any mode; [`OsmError::Build`] if the
/// builder refuses (no node left to build a network from, a build value out of range).
pub fn network(
    osm: &Osm,
    values: BuildValues,
    region: Option<&Region>,
) -> Result<(Network, Report), OsmError> {
    let mut report = Report { region: region.cloned(), ..Report::default() };
    let mut ways: Vec<&OsmWay> = osm.ways.iter().collect();
    ways.sort_by_key(|w| w.id);
    let mut last = None;
    ways.retain(|way| {
        let repeated = last == Some(way.id);
        if repeated {
            let what = "way ids given twice: the first kept";
            report.repairs.note("ways_repeated", what, way.id.to_string());
        }
        last = Some(way.id);
        !repeated
    });
    let street = |w: &OsmWay| {
        let n = w.nodes.len();
        road_class(&w.tags, n).is_some()
            || bike_way(&w.tags, n).is_some()
            || walk_way(&w.tags, n).is_some()
    };
    if !ways.iter().any(|w| street(w)) {
        return Err(OsmError::NoStreets);
    }
    // A node not placeable is not held: the ways through it are cut there.
    let positions: HashMap<i64, (f64, f64)> = osm
        .nodes
        .iter()
        .filter(|n| LonLat::new(n.lon, n.lat).is_projectable())
        .map(|n| (n.id, (n.lon, n.lat)))
        .collect();
    let node_set = |rule: fn(&[(String, String)]) -> bool| -> HashSet<i64> {
        osm.nodes.iter().filter(|n| rule(&n.tags)).map(|n| n.id).collect()
    };
    let (split_nodes, signal_nodes) = (node_set(splits), node_set(signalised));

    // The region is cut on every way (a road no layer uses still makes a carriageway's cut
    // end no dead end); then the streets alone are read.
    let clipped = region.map(|r| clip(&ways, &positions, r));
    let mut held: Vec<Held<'_>> = Vec::with_capacity(ways.len());
    for (i, &way) in ways.iter().enumerate().filter(|(_, w)| street(w)) {
        let keep = clipped.as_ref().map(|c| c.keep[i].as_slice());
        held.push(hold(way, keep, &positions, &mut report));
    }
    for turnaround in clipped.iter().flat_map(|c| &c.turnarounds) {
        held.push(hold(turnaround, None, &positions, &mut report));
    }

    // A way's tags are repaired, counted and warned only if the way gives a piece: a way the
    // region (or the extract) leaves without two nodes in a row is read with its repairs dropped.
    let mut links = Vec::new();
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        let specs: Vec<(Option<Spec>, bool)> = held
            .iter()
            .map(|h| {
                let mut dropped = Repairs::default();
                let kept = !h.runs().is_empty();
                spec(mode, h.way, if kept { &mut report.repairs } else { &mut dropped })
            })
            .collect();
        links.extend(layer(mode, &held, &specs, &split_nodes, &mut report));
    }
    build(&links, &signal_nodes, values, report)
}

/// `way` as held: each node's position where `keep` (the region's, else every node) keeps it,
/// the extract holds it, and the segment reaching it can be measured; a way holding some nodes
/// and missing others counted as cut.
fn hold<'a>(
    way: &'a OsmWay,
    keep: Option<&[bool]>,
    positions: &HashMap<i64, (f64, f64)>,
    report: &mut Report,
) -> Held<'a> {
    let kept = |k: usize| keep.is_none_or(|keep| keep[k]);
    let mut points: Vec<Option<(f64, f64)>> = way
        .nodes
        .iter()
        .enumerate()
        .map(|(k, n)| positions.get(n).copied().filter(|_| kept(k)))
        .collect();
    let mut segment_m = vec![0.0; points.len()];
    for k in 1..points.len() {
        if let (Some(a), Some(b)) = (points[k - 1], points[k]) {
            match geodesic_m(a, b) {
                Some(m) => segment_m[k] = m,
                None => points[k] = None,
            }
        }
    }
    if points.iter().any(Option::is_some) && points.iter().any(Option::is_none) {
        report.ways_cut += 1;
    }
    Held { way, points, segment_m }
}

/// What `way` gives the layer of `mode`, and whether it counts in the layer's junctions (on
/// the car layer, every road; on the others, every way the layer uses). A car way's tags that
/// needed setting right are noted in `repairs`.
fn spec(mode: Mode, way: &OsmWay, repairs: &mut Repairs) -> (Option<Spec>, bool) {
    let n = way.nodes.len();
    let plain = |(class, d): (RoadClass, crate::tags::Direction)| Spec {
        class,
        forward: d.has_forward().then_some(None),
        backward: d.has_backward().then_some(None),
    };
    match mode {
        Mode::Bike => {
            let s = bike_way(&way.tags, n).map(plain);
            let counted = s.is_some();
            (s, counted)
        }
        Mode::Walk => {
            let s = walk_way(&way.tags, n).map(plain);
            let counted = s.is_some();
            (s, counted)
        }
        _ => {
            let Some(class) = road_class(&way.tags, n) else { return (None, false) };
            let Some(defaults) = class.car() else { return (None, true) };
            let mut notes = CarNotes::default();
            let (f, b) = car_fields(&way.tags, direction(&way.tags, class), defaults, &mut notes);
            note(&notes, way.id, repairs);
            (Some(Spec { class, forward: f.map(Some), backward: b.map(Some) }), true)
        }
    }
}

/// The repairs of one car way's tags.
fn note(notes: &CarNotes, way: i64, repairs: &mut Repairs) {
    let id = || format!("way {way}");
    if notes.unread {
        let what = "ways with a one-way or lane value of no known meaning: read as absent";
        repairs.note("tags_unread", what, id());
    }
    if notes.speed {
        let what = "ways cars drive whose maxspeed is not a speed: read as absent (the next key, else the class's speed)";
        repairs.note("speeds_defaulted", what, id());
    }
    if notes.lanes {
        let what = "ways whose lane tags contradict or leave a driven direction no lane: the sides as tagged, at least 1 lane each";
        repairs.note("lanes_repaired", what, id());
    }
}

/// The links of the layer of `mode`: split at its junctions, trimmed to its largest strongly
/// connected component, contracted.
fn layer(
    mode: Mode,
    held: &[Held<'_>],
    specs: &[(Option<Spec>, bool)],
    split_nodes: &HashSet<i64>,
    report: &mut Report,
) -> Vec<Link> {
    let mut uses: HashMap<i64, u32> = HashMap::new();
    for (h, _) in held.iter().zip(specs).filter(|(_, (_, counted))| *counted) {
        for k in h.runs().into_iter().flatten() {
            *uses.entry(h.way.nodes[k]).or_insert(0) += 1;
        }
    }
    let junction = |n: i64| uses.get(&n).is_some_and(|&u| u >= 2) || split_nodes.contains(&n);
    let mut links = Vec::new();
    for (h, (s, _)) in held.iter().zip(specs) {
        if let Some(s) = s {
            split(mode, h, s, &junction, &mut links, report);
        }
    }
    let links = keep_strong_component(links, report);
    contract(links, split_nodes)
}

/// Cut a held way at its junctions into links, one per direction the layer uses. Each run of
/// held nodes is read as a way of its own, and nothing is bridged across a gap.
fn split(
    mode: Mode,
    h: &Held<'_>,
    s: &Spec,
    junction: &dyn Fn(i64) -> bool,
    out: &mut Vec<Link>,
    report: &mut Report,
) {
    let nodes = &h.way.nodes;
    let mut piece = 0;
    for run in h.runs() {
        let (mut start, mut length_m) = (run.start, 0.0);
        for j in run.start + 1..run.end {
            length_m += h.segment_m[j];
            if j + 1 != run.end && !junction(nodes[j]) {
                continue;
            }
            let (up, down) = (nodes[start], nodes[j]);
            let id = |tag: &str| format!("{mode}:{}:{piece}:{tag}", h.way.id);
            if up == down {
                report.loops_dropped += 1;
            } else if length_m.is_nan() || length_m <= 0.0 {
                // Two nodes at one position: left out before the strong component is taken,
                // so that nothing is joined through it; no length is invented.
                let (key, what) = LINKS_OF_NO_LENGTH;
                for (side, tag) in [(s.forward, "f"), (s.backward, "b")] {
                    if side.is_some() {
                        report.repairs.note(key, what, id(tag));
                    }
                }
            } else {
                let points: Vec<(f64, f64)> =
                    h.points[start..=j].iter().flatten().copied().collect();
                let link = |car, from, to, tag, points| Link {
                    mode,
                    id: id(tag),
                    from,
                    to,
                    class: s.class,
                    car,
                    length_m,
                    points,
                };
                if let Some(car) = s.forward {
                    out.push(link(car, up, down, "f", points.clone()));
                }
                if let Some(car) = s.backward {
                    out.push(link(car, down, up, "b", points.into_iter().rev().collect()));
                }
            }
            piece += 1;
            (start, length_m) = (j, 0.0);
        }
    }
}

/// The links whose two ends lie in the largest strongly connected component of the layer;
/// the others dropped and counted.
fn keep_strong_component(links: Vec<Link>, report: &mut Report) -> Vec<Link> {
    let mut nodes: Vec<i64> = links.iter().flat_map(|l| [l.from, l.to]).collect();
    nodes.sort_unstable();
    nodes.dedup();
    let dense = |n: i64| nodes.binary_search(&n).expect("a link's end is a node");
    let edges: Vec<(usize, usize)> = links.iter().map(|l| (dense(l.from), dense(l.to))).collect();
    let inside = largest_component(nodes.len(), &edges);
    let before = links.len();
    let kept: Vec<Link> =
        links.into_iter().filter(|l| inside[dense(l.from)] && inside[dense(l.to)]).collect();
    report.links_unreachable += u32::try_from(before - kept.len()).expect("links fit u32");
    kept
}

/// Merge chains of links through nodes that are no junction (the module's documentation), pass
/// after pass until none merges; nodes in `blocked` are never merged away.
fn contract(mut links: Vec<Link>, blocked: &HashSet<i64>) -> Vec<Link> {
    loop {
        let mut incoming: HashMap<i64, Vec<usize>> = HashMap::new();
        let mut outgoing: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, l) in links.iter().enumerate() {
            incoming.entry(l.to).or_default().push(i);
            outgoing.entry(l.from).or_default().push(i);
        }
        // Nodes in a fixed order, so the merges made do not depend on a hash's order.
        let mut candidates: Vec<i64> = incoming.keys().copied().collect();
        candidates.sort_unstable();
        let mut removed = vec![false; links.len()];
        let mut merged = Vec::new();
        for node in candidates.into_iter().filter(|n| !blocked.contains(n)) {
            let (Some(ins), Some(outs)) = (incoming.get(&node), outgoing.get(&node)) else {
                continue;
            };
            if ins.iter().chain(outs).any(|&i| removed[i]) {
                continue;
            }
            let Some(pairs) = pairs(&links, ins, outs) else { continue };
            for (i, o) in pairs {
                let (a, b) = (&links[i], &links[o]);
                let mut points = a.points.clone();
                points.extend_from_slice(&b.points[1..]);
                let length_m = a.length_m + b.length_m;
                merged.push(Link { to: b.to, length_m, points, ..a.clone() });
                (removed[i], removed[o]) = (true, true);
            }
        }
        if merged.is_empty() {
            return links;
        }
        let mut next: Vec<Link> =
            links.into_iter().zip(removed).filter(|(_, r)| !r).map(|(l, _)| l).collect();
        next.extend(merged);
        next.sort_by(|a, b| (a.from, a.to, &a.id).cmp(&(b.from, b.to, &b.id)));
        links = next;
    }
}

/// How the links through a node pair up when it is merged away: one in and one out (a one-way
/// chain), or two in and two out that are one street's two directions (each link in continues
/// to where the other came from); each pair carrying the same class and car fields and making
/// no link from a node to itself. Any other node is a junction and stays.
fn pairs(links: &[Link], ins: &[usize], outs: &[usize]) -> Option<Vec<(usize, usize)>> {
    let pair = |i: usize, o: usize| {
        let (a, b) = (&links[i], &links[o]);
        (a.class == b.class && a.car == b.car && a.from != b.to).then_some((i, o))
    };
    match (ins, outs) {
        (&[i], &[o]) => Some(vec![pair(i, o)?]),
        (&[i, j], &[o, p]) => {
            let (u, w) = (links[i].from, links[j].from);
            match (links[o].to, links[p].to) {
                _ if u == w => None,
                (x, y) if (x, y) == (w, u) => Some(vec![pair(i, o)?, pair(j, p)?]),
                (x, y) if (x, y) == (u, w) => Some(vec![pair(i, p)?, pair(j, o)?]),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Hand the links, their drawings (shape, class) and the nodes they use to the graph's builder.
fn build(
    links: &[Link],
    signal_nodes: &HashSet<i64>,
    values: BuildValues,
    mut report: Report,
) -> Result<(Network, Report), OsmError> {
    let mut builder = NetworkBuilder::new();
    let mut ends: HashMap<i64, (f64, f64)> = HashMap::new();
    for l in links {
        ends.insert(l.from, l.points[0]);
        ends.insert(l.to, l.points[l.points.len() - 1]);
    }
    let mut used: Vec<(i64, (f64, f64))> = ends.into_iter().collect();
    used.sort_unstable_by_key(|&(n, _)| n);
    for (n, (lon, lat)) in used {
        builder.add_node(n.to_string(), Position::LonLat(LonLat { lon, lat }));
        if signal_nodes.contains(&n) {
            builder.mark_signalised(n.to_string());
        }
    }
    for l in links {
        let row = LinkRow {
            modes: ModeSet::EMPTY.with(l.mode),
            lanes: l.car.map(|c| c.lanes),
            car_speed_km_h: l.car.map(|c| c.speed_km_h),
            length_m: Some(l.length_m),
            capacity_veh_h: l.car.map(|c| c.capacity_veh_h),
            signal_at_downstream_end: signal_nodes.contains(&l.to),
        };
        builder.add_link(l.id.clone(), l.from.to_string(), l.to.to_string(), row);
        let inner = &l.points[1..l.points.len() - 1];
        let shape = inner.iter().map(|&(lon, lat)| Position::LonLat(LonLat { lon, lat }));
        let road_class = Some(l.class.name().to_owned());
        builder.add_drawing(l.id.clone(), LinkDrawing { shape: shape.collect(), road_class });
    }
    let network = builder.build(values)?;
    report.repairs.extend(network.repairs());
    report.nodes = network.node_count();
    report.links = network.link_count();
    Ok((network, report))
}
