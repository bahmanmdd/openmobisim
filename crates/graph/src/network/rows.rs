//! The rows a network is built from, each one used as given, repaired, or left out, and every
//! repair noted in the build's [`Repairs`] by kind (the key, then what it completes "N …" with):
//!
//! | Key | Rows | What is done |
//! |---|---|---|
//! | `nodes_repeated` | a node id given twice | the first usable row kept (one whose position can be placed), the others left out |
//! | `nodes_unplaced` | a node whose position is not finite, or in longitude/latitude not projectable (beyond ±180°, ±84°), in every row of its id | the node left out, and every link naming it (`links_to_no_node`) |
//! | `signals_unplaced` | a signal on a node not held | ignored |
//! | `links_repeated` | a link id given twice | the first usable row kept (one no fault below leaves out), with its drawing; the others left out |
//! | `links_to_no_node` | a link naming a node not held | left out |
//! | `links_transit_removed` | a link allowing transit | transit taken out of its modes (transit runs on its timetable) |
//! | `car_mode_removed` | a car link without lanes, with 0 lanes, or without a car speed that is a finite number above zero | the car mode taken off (no speed is guessed); the link kept for its other modes |
//! | `car_fields_ignored` | a link without cars giving lanes, a car speed or a capacity | the field ignored |
//! | `links_without_mode` | a link left with no mode | left out |
//! | `lengths_defaulted` | a length given that is not finite or not above zero | the straight line between its nodes taken |
//! | `links_of_no_length` | a link whose length is 0 m (a node to itself, or two nodes at one position, without a length) | left out |
//! | `capacities_defaulted` | a car link's capacity given that is NaN or not above zero | lanes × `capacity_per_lane` taken |
//!
//! Rows are sorted by external id, stably, so that "the first row" is the first given; then ids
//! follow sorted external id, as before. Each fault is noted once: of an id whose every row is
//! left out, the first row's fault, and each other row as repeated; of an id with a usable row,
//! that row's repairs and each other row as repeated.

use openmobisim_types::ids::{EntityId, ExternalIdTable, ExternalIdTableBuilder, NodeId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::repairs::Repairs;
use openmobisim_types::units::{Flow, Metres, Speed};

use super::{BuildError, LinkDrawing, LinkRow};
use crate::geometry::{Position, Projection, Xy};

/// A repair's key and phrase.
type Note = (&'static str, &'static str);

/// A link row as given: its id, its two node ids, the row, and its drawing.
#[derive(Debug)]
pub(crate) struct GivenLink {
    pub id: String,
    pub upstream: String,
    pub downstream: String,
    pub row: LinkRow,
    pub drawing: Option<LinkDrawing>,
}

/// The nodes held, placed in metres, in sorted external order.
pub(super) struct Nodes {
    pub ids: ExternalIdTable,
    pub xy: Vec<Xy>,
    pub signalised: Vec<bool>,
    pub projection: Option<Projection>,
}

/// Whether a position can be placed: finite, and in longitude/latitude projectable.
fn placeable(p: &Position) -> bool {
    match p {
        Position::LonLat(at) => at.is_projectable(),
        Position::Metres(xy) => xy.x.is_finite() && xy.y.is_finite(),
    }
}

/// Sort the nodes; of each id keep the first row that can be placed (the module's table),
/// choose the projection if they are in longitude/latitude, and mark the signalised ones.
///
/// # Errors
///
/// [`BuildError::MixedPositions`] for nodes kept of both kinds; [`BuildError::NoNodes`] when no
/// node is left.
pub(super) fn place_nodes(
    mut nodes: Vec<(String, Position)>,
    signalised: Vec<String>,
    repairs: &mut Repairs,
) -> Result<Nodes, BuildError> {
    nodes.sort_by(|a, b| a.0.cmp(&b.0));
    let mut placed: Vec<(String, Position)> = Vec::with_capacity(nodes.len());
    let mut i = 0;
    while i < nodes.len() {
        let j = (i..nodes.len()).find(|&j| nodes[j].0 != nodes[i].0).unwrap_or(nodes.len());
        let id = &nodes[i].0;
        match (i..j).find(|&k| placeable(&nodes[k].1)) {
            Some(k) => placed.push(nodes[k].clone()),
            None => {
                let what = "nodes whose position is not finite or not projectable: left out, with their links";
                repairs.note("nodes_unplaced", what, id.clone());
            }
        }
        for _ in i + 1..j {
            let what = "node ids given twice: the first usable row kept, the others left out";
            repairs.note("nodes_repeated", what, id.clone());
        }
        i = j;
    }
    let first_lon_lat = placed.iter().find(|(_, p)| matches!(p, Position::LonLat(_)));
    let first_metres = placed.iter().find(|(_, p)| matches!(p, Position::Metres(_)));
    if let (Some((a, _)), Some((b, _))) = (first_lon_lat, first_metres) {
        return Err(BuildError::MixedPositions { lon_lat: a.clone(), metres: b.clone() });
    }
    if placed.is_empty() {
        return Err(BuildError::NoNodes);
    }
    let lon_lat: Vec<_> = placed
        .iter()
        .filter_map(|(_, p)| match p {
            Position::LonLat(at) => Some(*at),
            Position::Metres(_) => None,
        })
        .collect();
    let projection = (!lon_lat.is_empty()).then(|| {
        Projection::for_extent(lon_lat.iter().copied()).expect("every point is projectable")
    });
    let xy = placed
        .iter()
        .map(|(_, p)| match (p, projection) {
            (Position::LonLat(at), Some(proj)) => proj.project(*at),
            (Position::Metres(xy), _) => *xy,
            (Position::LonLat(_), None) => {
                unreachable!("a projection exists for points in degrees")
            }
        })
        .collect();
    let mut ids = ExternalIdTableBuilder::new();
    ids.extend(placed.iter().map(|(id, _)| id.as_str()));
    let ids = ids.build();
    let mut flags = vec![false; placed.len()];
    let mut signalised = signalised;
    signalised.sort_unstable();
    signalised.dedup();
    for external in signalised {
        match ids.id_of(&external) {
            Some(id) => flags[id as usize] = true,
            None => {
                repairs.note("signals_unplaced", "signals on a node not held: ignored", external)
            }
        }
    }
    Ok(Nodes { ids, xy, signalised: flags, projection })
}

/// The link columns, in sorted external order, with their ids and drawings.
pub(super) struct Links {
    pub ids: ExternalIdTable,
    pub upstream: Vec<NodeId>,
    pub downstream: Vec<NodeId>,
    pub length: Vec<Metres>,
    pub car_speed: Vec<Speed>,
    pub capacity: Vec<Flow>,
    pub lanes: Vec<u8>,
    pub modes: Vec<ModeSet>,
    pub signal_downstream: Vec<bool>,
    /// Per link, its id and the drawing of the row kept.
    pub drawings: Vec<(String, Option<LinkDrawing>)>,
}

/// One link as the network holds it, and the repairs its row needed.
struct Usable {
    upstream: NodeId,
    downstream: NodeId,
    length: Metres,
    car_speed: Speed,
    lanes: u8,
    capacity: Flow,
    modes: ModeSet,
    notes: Vec<Note>,
}

/// The links of `rows` on `nodes`, each used as given, repaired or left out (the module's
/// table); `per_lane` the capacity of one lane, in vehicles per hour.
pub(super) fn links(
    mut rows: Vec<GivenLink>,
    nodes: &Nodes,
    per_lane: f64,
    repairs: &mut Repairs,
) -> Links {
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    let n = rows.len();
    let mut out = Links {
        ids: ExternalIdTable::default(),
        upstream: Vec::with_capacity(n),
        downstream: Vec::with_capacity(n),
        length: Vec::with_capacity(n),
        car_speed: Vec::with_capacity(n),
        capacity: Vec::with_capacity(n),
        lanes: Vec::with_capacity(n),
        modes: Vec::with_capacity(n),
        signal_downstream: Vec::with_capacity(n),
        drawings: Vec::with_capacity(n),
    };
    let mut ids = ExternalIdTableBuilder::new();
    let mut rows = rows.into_iter().peekable();
    while let Some(first) = rows.next() {
        let mut group = vec![first];
        while rows.peek().is_some_and(|r| r.id == group[0].id) {
            group.extend(rows.next());
        }
        let mut fault = None;
        let mut kept = None;
        for (k, row) in group.iter().enumerate() {
            match usable(row, nodes, per_lane) {
                Ok(link) => {
                    kept = Some((k, link));
                    break;
                }
                Err(notes) => {
                    fault.get_or_insert(notes);
                }
            }
        }
        let id = group[0].id.clone();
        for _ in 1..group.len() {
            let what = "link ids given twice: the first usable row kept, the others left out";
            repairs.note("links_repeated", what, id.clone());
        }
        let Some((k, link)) = kept else {
            for (key, what) in fault.expect("a row left out has its fault") {
                repairs.note(key, what, id.clone());
            }
            continue;
        };
        for (key, what) in link.notes {
            repairs.note(key, what, id.clone());
        }
        let row = group.swap_remove(k);
        ids.insert(id.as_str());
        out.upstream.push(link.upstream);
        out.downstream.push(link.downstream);
        out.length.push(link.length);
        out.car_speed.push(link.car_speed);
        out.capacity.push(link.capacity);
        out.lanes.push(link.lanes);
        out.modes.push(link.modes);
        out.signal_downstream.push(row.row.signal_at_downstream_end);
        out.drawings.push((id, row.drawing));
    }
    out.ids = ids.build();
    out
}

/// The link of one row, with the repairs it needs; else the repairs made and, last, the fault
/// that leaves it out (a car-only row without a speed: the car taken off, then no mode left).
fn usable(given: &GivenLink, nodes: &Nodes, per_lane: f64) -> Result<Usable, Vec<Note>> {
    let node = |id: &str| nodes.ids.typed_id_of::<NodeId>(id);
    let (Some(upstream), Some(downstream)) = (node(&given.upstream), node(&given.downstream))
    else {
        return Err(vec![("links_to_no_node", "links naming a node not held: left out")]);
    };
    let row = &given.row;
    let mut notes = Vec::new();
    let (modes, car_speed, lanes, capacity) = match modes_and_car(row, per_lane, &mut notes) {
        Ok(car) => car,
        Err(fault) => {
            notes.push(fault);
            return Err(notes);
        }
    };
    let straight = nodes.xy[upstream.index()].distance_to(nodes.xy[downstream.index()]);
    let given_m = row.length_m.filter(|m| m.is_finite() && *m > 0.0);
    let metres = given_m.unwrap_or(straight);
    if metres.is_nan() || metres <= 0.0 {
        notes.push(super::LINKS_OF_NO_LENGTH);
        return Err(notes);
    }
    if given_m.is_none() && row.length_m.is_some() {
        notes.push((
            "lengths_defaulted",
            "lengths not finite or not above zero: the straight line taken",
        ));
    }
    Ok(Usable {
        upstream,
        downstream,
        length: Metres(metres),
        car_speed,
        lanes,
        capacity,
        modes,
        notes,
    })
}

/// A row's modes (transit taken out) and its car speed, lanes and capacity: a car link without
/// lanes, with 0, or without a car speed above zero loses its car mode; a link without cars
/// ignores its car fields and gets no speed (NaN, never read), 0 lanes and an infinite capacity.
/// The repairs made are pushed to `notes`; a row left with no mode is the fault returned.
fn modes_and_car(
    row: &LinkRow,
    per_lane: f64,
    notes: &mut Vec<Note>,
) -> Result<(ModeSet, Speed, u8, Flow), Note> {
    let without = |modes: ModeSet, gone: Mode| {
        modes.iter().filter(|&m| m != gone).fold(ModeSet::EMPTY, ModeSet::with)
    };
    let mut modes = row.modes;
    if modes.contains(Mode::Transit) {
        notes.push((
            "links_transit_removed",
            "links allowing transit: transit taken out (it runs on its timetable)",
        ));
        modes = without(modes, Mode::Transit);
    }
    let lanes = row.lanes.filter(|&l| l > 0);
    let speed = row.car_speed_km_h.filter(|k| k.is_finite() && *k > 0.0);
    let car = modes.contains(Mode::Car);
    if car && (lanes.is_none() || speed.is_none()) {
        notes.push((
            "car_mode_removed",
            "car links without lanes or a car speed above zero: the car mode taken off",
        ));
        modes = without(modes, Mode::Car);
    } else if !car
        && (row.lanes.is_some() || row.car_speed_km_h.is_some() || row.capacity_veh_h.is_some())
    {
        notes.push((
            "car_fields_ignored",
            "links without cars giving a car field (lanes, speed or capacity): the field ignored",
        ));
    }
    if modes.is_empty() {
        return Err(("links_without_mode", "links allowing no mode: left out"));
    }
    let (Some(lanes), Some(km_h), true) = (lanes, speed, modes.contains(Mode::Car)) else {
        return Ok((modes, Speed(f64::NAN), 0, Flow::INFINITE));
    };
    // Positive infinity is an unlimited capacity, kept as `Flow::INFINITE` (∞ / 3 600 = ∞).
    let by_lanes = f64::from(lanes) * per_lane;
    let veh_h = match row.capacity_veh_h {
        Some(c) if c.is_nan() || c <= 0.0 => {
            notes.push(("capacities_defaulted", "car links whose capacity is NaN or not above zero: lanes × capacity_per_lane taken"));
            by_lanes
        }
        Some(c) => c,
        None => by_lanes,
    };
    Ok((modes, Speed::from_km_per_hour(km_h), lanes, Flow::from_veh_per_hour(veh_h)))
}
