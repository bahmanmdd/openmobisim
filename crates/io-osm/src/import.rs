//! From an extract's ways to the network: read each way's modes and car fields, split the ways
//! at their junctions, measure each piece, keep the car network's largest strongly connected
//! component, and hand the rows to the graph's builder.
//!
//! # Junctions
//!
//! A node is a junction when it is an end of a street way, when street ways pass it twice or
//! more (two ways, or one way twice), or when it is signalised (`highway=traffic_signals`). A
//! link runs from one junction to the next along one way; the nodes between are not network
//! nodes but the link's shape (for drawing only, in the link's own direction), and the link's
//! length is the sum of the WGS84 geodesics between them. A way cars,
//! bikes or walkers may use in both directions gives two links per piece, one each way, each
//! with the modes of its own direction. A piece that starts and ends at the same node (a closed
//! way no other street touches) cannot be a link and is dropped and counted.
//!
//! # The car network's largest strongly connected component
//!
//! Over the links cars may use, the largest set of nodes each reachable from every other by
//! car. A link with cars whose two ends are not both in it loses its car mode and its car
//! fields (lanes, speed), and is counted; it is kept for its bike and walk modes, and dropped
//! only when no mode is left. A link without cars is kept as it is.

use std::collections::{HashMap, HashSet};

use openmobisim_graph::geometry::{LonLat, Position};
use openmobisim_graph::network::{
    BuildValues, LINKS_OF_NO_LENGTH, LinkDrawing, LinkRow, Network, NetworkBuilder,
};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::repairs::Repairs;

use crate::access::{Directions, Unread, access};
use crate::car::{CarFields, CarNotes, car_fields};
use crate::class::class_of;
use crate::geodesic::geodesic_m;
use crate::source::{Osm, OsmError, OsmWay, tag_of};
use crate::strong::largest_component;

/// What a read made of an extract, counted, and what it set right instead of refusing.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Report {
    /// Nodes of the network.
    pub nodes: u32,
    /// Links of the network.
    pub links: u32,
    /// Links that lost their car mode (and car fields) because their ends are not both in the
    /// car network's largest strongly connected component; those with bike or walk are kept.
    pub links_unreachable: u32,
    /// Pieces of closed ways dropped because they start and end at the same node.
    pub loops_dropped: u32,
    /// Street ways that name nodes the extract does not hold (cut by its edge), or a node whose
    /// position is not finite or not projectable, or a segment whose geodesic cannot be
    /// measured: each run of nodes it holds is read, nothing is bridged across a gap.
    pub ways_cut: u32,
    /// What the reader set right instead of refusing, by kind (the crate's README lists them),
    /// the network builder's repairs included.
    pub repairs: Repairs,
}

/// What one direction of a way carries, and the way's class (the first `highway` value of its
/// row in the class table).
#[derive(Clone, Copy, Debug)]
struct Side {
    modes: ModeSet,
    car: Option<CarFields>,
    class: &'static str,
}

/// A way's two sides: along its node order, and against it (`None` for a side no mode uses).
type Sides = (Option<Side>, Option<Side>);

/// One directed link before the build.
#[derive(Clone, Debug)]
struct Piece {
    id: String,
    upstream: i64,
    downstream: i64,
    length_m: f64,
    side: Side,
    /// The positions (longitude, latitude) of the way's nodes between the two ends, in the
    /// link's direction.
    shape: Vec<(f64, f64)>,
}

/// The network of `osm`, built with `values`, and its counts and repairs.
///
/// # Errors
///
/// [`OsmError::NoStreets`] if no way is a street; [`OsmError::Build`] if no node is left to
/// build a network from (the builder's refusal).
pub fn network(osm: &Osm, values: BuildValues) -> Result<(Network, Report), OsmError> {
    let mut report = Report::default();
    let mut ways: Vec<&OsmWay> = osm.ways.iter().collect();
    ways.sort_by_key(|w| w.id);
    let mut streets = Vec::new();
    let mut last = None;
    for way in ways {
        if last == Some(way.id) {
            let what = "way ids given twice: the first kept";
            report.repairs.note("ways_repeated", what, way.id.to_string());
            continue;
        }
        last = Some(way.id);
        if let Some(sides) = sides(way, &mut report.repairs) {
            streets.push((way, sides));
        }
    }
    if streets.is_empty() {
        return Err(OsmError::NoStreets);
    }
    // A node whose position is not finite or not projectable is not held: the ways through it
    // are cut there, as at the extract's edge.
    let positions: HashMap<i64, (f64, f64)> = osm
        .nodes
        .iter()
        .filter(|n| LonLat::new(n.lon, n.lat).is_projectable())
        .map(|n| (n.id, (n.lon, n.lat)))
        .collect();
    let signalised: HashSet<i64> = osm
        .nodes
        .iter()
        .filter(|n| tag_of(&n.tags, "highway") == Some("traffic_signals"))
        .map(|n| n.id)
        .collect();

    let mut uses: HashMap<i64, u32> = HashMap::new();
    for (way, _) in &streets {
        for &n in &way.nodes {
            *uses.entry(n).or_insert(0) += 1;
        }
    }
    // A way's ends are junctions by construction of `split`; here, the nodes inside it.
    let junction = |n: i64| uses[&n] >= 2 || signalised.contains(&n);

    let mut pieces = Vec::new();
    for (way, (forward, backward)) in &streets {
        split(way, *forward, *backward, &positions, &junction, &mut pieces, &mut report);
    }
    let pieces = keep_car_component(pieces, &mut report);
    build(&pieces, &positions, &signalised, values, report)
}

/// The two sides of a way that is a street (along, against its node order), or `None` if it
/// is not a street of any mode; what its tags needed set right noted in `repairs`.
fn sides(way: &OsmWay, repairs: &mut Repairs) -> Option<Sides> {
    let class = way.tag("highway").and_then(class_of)?;
    if way.nodes.len() < 2 || way.tag("area") == Some("yes") {
        return None;
    }
    let mut unread = Unread::default();
    let a = access(way, class, &mut unread);
    let mut notes = CarNotes::default();
    let (car_forward, car_backward) = match class.car {
        Some(default) => car_fields(way, a.car, default, &mut notes),
        None => (None, None),
    };
    let id = || format!("way {}", way.id);
    if unread.0 || notes.unread {
        let what = "ways with an access, one-way or lane value of no known meaning: read as absent";
        repairs.note("tags_unread", what, id());
    }
    if notes.speed {
        let what = "ways whose maxspeed is not a speed: read as absent (the class's default where no other key gives one)";
        repairs.note("speeds_defaulted", what, id());
    }
    if notes.lanes {
        let what = "ways whose lane tags contradict or leave a driven direction no lane: the sides as tagged, at least 1 lane each";
        repairs.note("lanes_repaired", what, id());
    }
    let side = |along: fn(Directions) -> bool, car: Option<CarFields>| {
        let mut modes = ModeSet::EMPTY;
        for (mode, d) in [(Mode::Car, a.car), (Mode::Bike, a.bike), (Mode::Walk, a.walk)] {
            if along(d) {
                modes = modes.with(mode);
            }
        }
        (!modes.is_empty()).then_some(Side { modes, car, class: class.highway[0] })
    };
    let forward = side(|d| d.forward, car_forward);
    let backward = side(|d| d.backward, car_backward);
    (forward.is_some() || backward.is_some()).then_some((forward, backward))
}

/// Cut `way` at its junctions into pieces, one link per side that carries a mode. Nodes the
/// extract does not hold (a way cut by the extract's edge, or a node not placeable) cut the way
/// too, and so does a segment whose geodesic cannot be measured (at its far node): each run of
/// nodes it holds is read as a way of its own, and nothing is bridged across the gap.
fn split(
    way: &OsmWay,
    forward: Option<Side>,
    backward: Option<Side>,
    positions: &HashMap<i64, (f64, f64)>,
    junction: &dyn Fn(i64) -> bool,
    out: &mut Vec<Piece>,
    report: &mut Report,
) {
    let mut held: Vec<Option<(f64, f64)>> =
        way.nodes.iter().map(|n| positions.get(n).copied()).collect();
    // The length of the segment ending at each node.
    let mut segment_m = vec![0.0; held.len()];
    for j in 1..held.len() {
        if let (Some(a), Some(b)) = (held[j - 1], held[j]) {
            match geodesic_m(a, b) {
                Some(m) => segment_m[j] = m,
                None => held[j] = None,
            }
        }
    }
    if held.iter().any(Option::is_none) {
        report.ways_cut += 1;
    }
    let mut segment = 0;
    let mut i = 0;
    while i < way.nodes.len() {
        let run_end = (i..way.nodes.len()).find(|&j| held[j].is_none()).unwrap_or(way.nodes.len());
        let (mut start, mut length_m) = (i, 0.0);
        for j in i + 1..run_end {
            length_m += segment_m[j];
            if j + 1 != run_end && !junction(way.nodes[j]) {
                continue;
            }
            let (up, down) = (way.nodes[start], way.nodes[j]);
            if up == down {
                report.loops_dropped += 1;
            } else if length_m.is_nan() || length_m <= 0.0 {
                // Two nodes at one position: no length to give, left out here, before the car
                // network's strong component is taken, so that nothing is joined through it.
                let (key, what) = LINKS_OF_NO_LENGTH;
                let sides = [(forward, "f"), (backward, "b")];
                for (_, tag) in sides.iter().filter(|(side, _)| side.is_some()) {
                    report.repairs.note(key, what, format!("{}:{segment}:{tag}", way.id));
                }
            } else {
                let along: Vec<(f64, f64)> = held[start + 1..j].iter().flatten().copied().collect();
                let mut push = |side: Option<Side>, upstream, downstream, tag, shape| {
                    if let Some(side) = side {
                        let id = format!("{}:{segment}:{tag}", way.id);
                        out.push(Piece { id, upstream, downstream, length_m, side, shape });
                    }
                };
                push(forward, up, down, "f", along.clone());
                push(backward, down, up, "b", along.into_iter().rev().collect());
            }
            segment += 1;
            (start, length_m) = (j, 0.0);
        }
        i = run_end + 1;
    }
}

/// The pieces with the car taken off every car link outside the car network's largest strongly
/// connected component; a piece left with no mode is dropped.
fn keep_car_component(pieces: Vec<Piece>, report: &mut Report) -> Vec<Piece> {
    let has_car = |p: &Piece| p.side.modes.contains(Mode::Car);
    let mut nodes: Vec<i64> =
        pieces.iter().filter(|p| has_car(p)).flat_map(|p| [p.upstream, p.downstream]).collect();
    nodes.sort_unstable();
    nodes.dedup();
    let dense = |n: i64| nodes.binary_search(&n).ok();
    let edges: Vec<(usize, usize)> = pieces
        .iter()
        .filter(|p| has_car(p))
        .filter_map(|p| Some((dense(p.upstream)?, dense(p.downstream)?)))
        .collect();
    let inside = largest_component(nodes.len(), &edges);
    let reachable =
        |p: &Piece| [p.upstream, p.downstream].iter().all(|&n| dense(n).is_some_and(|i| inside[i]));
    let mut kept = Vec::with_capacity(pieces.len());
    for mut p in pieces {
        if has_car(&p) && !reachable(&p) {
            report.links_unreachable += 1;
            let rest = p.side.modes.iter().filter(|&m| m != Mode::Car);
            p.side = Side { car: None, modes: rest.fold(ModeSet::EMPTY, ModeSet::with), ..p.side };
        }
        if !p.side.modes.is_empty() {
            kept.push(p);
        }
    }
    kept
}

/// Hand the pieces, their drawings (shape, class) and the nodes they use to the graph's builder.
fn build(
    pieces: &[Piece],
    positions: &HashMap<i64, (f64, f64)>,
    signalised: &HashSet<i64>,
    values: BuildValues,
    mut report: Report,
) -> Result<(Network, Report), OsmError> {
    let mut used: Vec<i64> = pieces.iter().flat_map(|p| [p.upstream, p.downstream]).collect();
    used.sort_unstable();
    used.dedup();
    let mut builder = NetworkBuilder::new();
    for &n in &used {
        let (lon, lat) = positions[&n];
        builder.add_node(n.to_string(), Position::LonLat(LonLat { lon, lat }));
        if signalised.contains(&n) {
            builder.mark_signalised(n.to_string());
        }
    }
    for p in pieces {
        let row = LinkRow {
            modes: p.side.modes,
            lanes: p.side.car.map(|c| c.lanes),
            car_speed_km_h: p.side.car.map(|c| c.speed_km_h),
            length_m: Some(p.length_m),
            capacity_veh_h: None,
            signal_at_downstream_end: signalised.contains(&p.downstream),
        };
        builder.add_link(p.id.clone(), p.upstream.to_string(), p.downstream.to_string(), row);
        let shape = p.shape.iter().map(|&(lon, lat)| Position::LonLat(LonLat { lon, lat }));
        let road_class = Some(p.side.class.to_owned());
        builder.add_drawing(p.id.clone(), LinkDrawing { shape: shape.collect(), road_class });
    }
    let network = builder.build(values)?;
    report.repairs.extend(network.repairs());
    report.nodes = network.node_count();
    report.links = network.link_count();
    Ok((network, report))
}
