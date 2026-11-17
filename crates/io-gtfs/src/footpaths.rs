//! Footpaths: for each stop, every other stop a walker reaches on the walk network within
//! `transfer_walk_max` metres, derived once, here; and `transfers.txt`'s minimum times.
//!
//! - **Linking.** Each stop goes to its nearest walk node by straight line (the demand
//!   crate's [`NodeIndex`]); the straight line is the stop's gap.
//! - **The walk.** From each stop's node, the bounded walk search the demand crate runs for
//!   its access lists ([`NearStops`]). The walk from stop `a` to stop `b` is `a`'s gap, the
//!   walk from `a`'s node to `b`'s node, and `b`'s gap, all at the network's walk speed; a pair
//!   is kept when that walk is at most `transfer_walk_max` metres, read to a part in a billion
//!   as the demand crate reads its limit. Its seconds are its metres over the walk speed. The
//!   bound is applied in the search and nothing is filtered afterwards.
//! - **`transfers.txt`.** A row naming two stops of a kept pair, with no route or trip, and
//!   `transfer_type` 2 (its `min_transfer_time`) or 1 (zero): the footpath's seconds are the
//!   larger of the walk's and the row's. Every other row is skipped and counted (a pair with
//!   no footpath, one stop to itself, a route or trip given, `transfer_type` 0, 3 or other).

use std::collections::HashMap;

use openmobisim_demand::index::{Joins, NodeIndex};
use openmobisim_demand::reach::NearStops;
use openmobisim_graph::network::Network;
use openmobisim_types::ids::{EntityId, NodeId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Duration;

use crate::source::Feed;
use crate::stop_times::Stops;
use crate::{GtfsError, Report};

/// One footpath: from stop, to stop (indices), seconds.
pub(crate) type Footpath = (u32, u32, f64);

/// The footpaths between the stops, by from stop then to stop, `transfers.txt` applied.
pub(crate) fn derive(
    feed: &mut Feed,
    network: &Network,
    stops: &Stops,
    transfer_walk_max: f64,
    report: &mut Report,
) -> Result<Vec<Footpath>, GtfsError> {
    let walk = NodeIndex::new(network, Mode::Walk, Joins::Either);
    let mut ends: Vec<(NodeId, f64)> = Vec::with_capacity(stops.xy.len());
    for &p in &stops.xy {
        let Some(end) = walk.nearest(p) else {
            let what = "networks with no link a walker may use: no footpath between stops";
            report.repairs.note("footpaths_none", what, "the network");
            return Ok(Vec::new());
        };
        ends.push(end);
    }
    let speed = network.walk_speed().get();
    let limit = transfer_walk_max / speed * (1.0 + 1e-9);
    let nodes: Vec<NodeId> = ends.iter().map(|e| e.0).collect();
    let near = NearStops::new(network, &nodes, Duration(limit));
    let mut out: Vec<Footpath> = Vec::new();
    for (b, &(node, gap_b)) in ends.iter().enumerate() {
        let b = u32::try_from(b).expect("stops fit u32");
        for &(a, seconds) in near.stops_reaching(node) {
            let total = ends[a.index()].1 / speed + seconds + gap_b / speed;
            if a.raw() != b && total <= limit {
                out.push((a.raw(), b, total));
            }
        }
    }
    out.sort_by_key(|f| (f.0, f.1));
    apply_transfers(feed, stops, &mut out, report)?;
    report.footpaths = out.len() as u64;
    Ok(out)
}

/// Raise each footpath to the minimum time `transfers.txt` gives its pair (see the module
/// documentation).
fn apply_transfers(
    feed: &mut Feed,
    stops: &Stops,
    footpaths: &mut [Footpath],
    report: &mut Report,
) -> Result<(), GtfsError> {
    let f = "transfers.txt";
    let Some(mut csv) = feed.optional_csv(f)? else { return Ok(()) };
    let columns = ["from_stop_id", "to_stop_id", "transfer_type"].map(|c| csv.column(c));
    let [Some(from), Some(to), Some(kind)] = columns else {
        let what = "optional files without one of their columns: not read";
        report.repairs.note("files_unread", what, f);
        return Ok(());
    };
    let seconds = csv.column("min_transfer_time");
    let specific: Vec<Option<usize>> =
        ["from_route_id", "to_route_id", "from_trip_id", "to_trip_id"]
            .iter()
            .map(|c| csv.column(c))
            .collect();
    let pair: HashMap<(u32, u32), usize> =
        footpaths.iter().enumerate().map(|(i, f)| ((f.0, f.1), i)).collect();
    while csv.next()? {
        let min = match csv.field(kind) {
            "2" => csv.get(seconds).parse::<u32>().ok(),
            "1" => Some(0),
            _ => None,
        };
        let general = specific.iter().all(|c| csv.get(*c).is_empty());
        let stop = |name: &str| stops.index.get(name).copied();
        let found = match (stop(csv.field(from)), stop(csv.field(to))) {
            (Some(a), Some(b)) => pair.get(&(a, b)),
            _ => None,
        };
        match (min, general, found) {
            (Some(min), true, Some(&i)) => {
                footpaths[i].2 = footpaths[i].2.max(f64::from(min));
                report.transfers_applied += 1;
            }
            _ => report.transfers_skipped += 1,
        }
    }
    Ok(())
}
