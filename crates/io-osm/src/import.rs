//! From an extract's ways to the network: read each way's modes and car fields, split the ways
//! at their junctions, measure each piece, keep the car network's largest strongly connected
//! component, and hand the rows to the graph's builder.
//!
//! # Junctions
//!
//! A node is a junction when it is an end of a street way, when street ways pass it twice or
//! more (two ways, or one way twice), or when it is signalised (`highway=traffic_signals`). A
//! link runs from one junction to the next along one way; the nodes between are not network
//! nodes, and the link's length is the sum of the WGS84 geodesics between them. A way cars,
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
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_types::mode::{Mode, ModeSet};

use crate::access::{Directions, access};
use crate::car::{CarFields, car_fields};
use crate::class::class_of;
use crate::geodesic::geodesic_m;
use crate::source::{Osm, OsmError, OsmWay, tag_of};
use crate::strong::largest_component;

/// What a read made of an extract, counted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
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
    /// Street ways that name nodes the extract does not hold (cut by its edge): each run of
    /// nodes it holds is read, nothing is bridged across a gap.
    pub ways_cut: u32,
}

/// What one direction of a way carries.
#[derive(Clone, Copy, Debug)]
struct Side {
    modes: ModeSet,
    car: Option<CarFields>,
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
}

/// The network of `osm`, built with `values`, and its counts.
///
/// # Errors
///
/// An [`OsmError`] naming the way for a tag that cannot be read or a segment with no
/// geodesic; [`OsmError::NoStreets`] if no way is a street;
/// [`OsmError::Build`] if the builder refuses a row (a link of zero length, for one).
pub fn network(osm: &Osm, values: BuildValues) -> Result<(Network, Report), OsmError> {
    let mut ways: Vec<&OsmWay> = osm.ways.iter().collect();
    ways.sort_unstable_by_key(|w| w.id);
    let mut streets = Vec::new();
    for way in ways {
        if let Some(sides) = sides(way)? {
            streets.push((way, sides));
        }
    }
    if streets.is_empty() {
        return Err(OsmError::NoStreets);
    }
    let positions: HashMap<i64, (f64, f64)> =
        osm.nodes.iter().map(|n| (n.id, (n.lon, n.lat))).collect();
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

    let mut report = Report::default();
    let mut pieces = Vec::new();
    for (way, (forward, backward)) in &streets {
        split(way, *forward, *backward, &positions, &junction, &mut pieces, &mut report)?;
    }
    let pieces = keep_car_component(pieces, &mut report);
    build(&pieces, &positions, &signalised, values, report)
}

/// The two sides of a way that is a street (along, against its node order), or `None` if it
/// is not a street of any mode.
fn sides(way: &OsmWay) -> Result<Option<Sides>, OsmError> {
    let Some(class) = way.tag("highway").and_then(class_of) else { return Ok(None) };
    if way.nodes.len() < 2 || way.tag("area") == Some("yes") {
        return Ok(None);
    }
    let a = access(way, class)?;
    let (car_forward, car_backward) = match class.car {
        Some(default) => car_fields(way, a.car, default)?,
        None => (None, None),
    };
    let side = |along: fn(Directions) -> bool, car: Option<CarFields>| {
        let mut modes = ModeSet::EMPTY;
        for (mode, d) in [(Mode::Car, a.car), (Mode::Bike, a.bike), (Mode::Walk, a.walk)] {
            if along(d) {
                modes = modes.with(mode);
            }
        }
        (!modes.is_empty()).then_some(Side { modes, car })
    };
    let forward = side(|d| d.forward, car_forward);
    let backward = side(|d| d.backward, car_backward);
    Ok((forward.is_some() || backward.is_some()).then_some((forward, backward)))
}

/// Cut `way` at its junctions into pieces, one link per side that carries a mode. Nodes the
/// extract does not hold (a way cut by the extract's edge) cut the way too: each run of nodes
/// it holds is read as a way of its own, and nothing is bridged across the gap.
fn split(
    way: &OsmWay,
    forward: Option<Side>,
    backward: Option<Side>,
    positions: &HashMap<i64, (f64, f64)>,
    junction: &dyn Fn(i64) -> bool,
    out: &mut Vec<Piece>,
    report: &mut Report,
) -> Result<(), OsmError> {
    let held: Vec<Option<(f64, f64)>> =
        way.nodes.iter().map(|n| positions.get(n).copied()).collect();
    if held.iter().any(Option::is_none) {
        report.ways_cut += 1;
    }
    let mut segment = 0;
    let mut i = 0;
    while i < way.nodes.len() {
        let run_end = (i..way.nodes.len()).find(|&j| held[j].is_none()).unwrap_or(way.nodes.len());
        let (mut start, mut length_m) = (i, 0.0);
        for j in i + 1..run_end {
            let (a, b) = (held[j - 1].unwrap_or_default(), held[j].unwrap_or_default());
            length_m += geodesic_m(a, b).ok_or(OsmError::Geodesic { way: way.id })?;
            if j + 1 != run_end && !junction(way.nodes[j]) {
                continue;
            }
            let (up, down) = (way.nodes[start], way.nodes[j]);
            if up == down {
                report.loops_dropped += 1;
            } else {
                let mut push = |side: Option<Side>, upstream, downstream, tag| {
                    if let Some(side) = side {
                        let id = format!("{}:{segment}:{tag}", way.id);
                        out.push(Piece { id, upstream, downstream, length_m, side });
                    }
                };
                push(forward, up, down, "f");
                push(backward, down, up, "b");
            }
            segment += 1;
            (start, length_m) = (j, 0.0);
        }
        i = run_end + 1;
    }
    Ok(())
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
            p.side = Side { modes: rest.fold(ModeSet::EMPTY, ModeSet::with), car: None };
        }
        if !p.side.modes.is_empty() {
            kept.push(p);
        }
    }
    kept
}

/// Hand the pieces and the nodes they use to the graph's builder.
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
    }
    let network = builder.build(values)?;
    report.nodes = network.node_count();
    report.links = network.link_count();
    Ok((network, report))
}
