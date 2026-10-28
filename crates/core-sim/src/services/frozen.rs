//! A fleet day without a run: trips given as a table, their service legs planned by a fixed
//! rule, and the rides and walks the day asks for on the static layers.
//!
//! **The frozen day** is what the fleet engine needs to run alone: the legs (who asks for which
//! service, when, from where to where, with what weight), the road times ([`super::roads`]), the
//! window and a seed. It holds the travellers' choices fixed, and the traffic: **an
//! approximation, labelled as such** in what it returns. A run's own choice reads the services'
//! expected costs and may change its mind; here nobody does.
//!
//! **Planning a leg** ([`plan_legs`]): a shared-bike trip takes the dock nearest its origin on
//! foot within the service's `walk_max_s`, and returns at the dock nearest its destination on
//! foot within the same; it asks for a bike when it reaches the first dock (its departure plus
//! the walk), rides between the two on the bike layer at the service's `ride_factor`, and walks
//! on. A trip with no dock within reach at either end, or with one dock for both, has no leg
//! and is counted. A taxi trip asks at its departure at its origin's nearest road node, for its
//! destination's.
//!
//! **Cost:** per trip two bounded walk searches and one ride (kept per dock pair); per
//! redirection one walk search.

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{NodeSnapper, Reach};
use openmobisim_core_types::ids::{EntityId, NodeId};

use super::engine::{Leg, ServiceLegs};
use super::roads::FrozenPaths;
use super::{ServiceParts, ServiceSetup};
use crate::layers::LayerSetup;

/// One trip as a table gives it: the service it uses, where, when and how many people.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TripRow {
    /// Its id, the key that orders legs at one second.
    pub trip: u32,
    /// The service, by its index in the day's services.
    pub service: u16,
    /// The people it stands for.
    pub weight: u32,
    /// Its departure, in seconds from midnight.
    pub departure_s: f64,
    /// Where it starts.
    pub origin: LonLat,
    /// Where it ends.
    pub destination: LonLat,
}

/// What planning the legs kept and left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanReport {
    /// Trips given.
    pub trips: u32,
    /// Legs planned.
    pub legs: u32,
    /// Left out: no dock within reach on foot of the origin or of the destination.
    pub no_dock: u32,
    /// Left out: one dock nearest both ends (a trip shorter than a ride).
    pub same_dock: u32,
    /// Left out: no ride on the bike layer between the two docks.
    pub no_ride: u32,
}

/// The nearest dock to `node` on the walk layer within `bound` seconds, and the walk; `back`
/// searches the walks *to* `node`. Ties go to the lower dock index.
fn nearest_dock(
    reach: &mut Reach<'_>,
    node: NodeId,
    bound: f64,
    has_dock: &[bool],
    walk_nodes: &[NodeId],
    back: bool,
) -> Option<(u32, f64)> {
    let found = if back {
        reach.backward(node, bound, has_dock)
    } else {
        reach.forward(node, bound, has_dock)
    };
    let (at, secs) = *found.first()?;
    let dock = walk_nodes.iter().position(|&w| w == at)?;
    Some((u32::try_from(dock).expect("docks fit u32"), secs))
}

/// The legs of `trips` for `setups` (see the [module docs](self)), in the trips' order, with
/// each leg's trip row index beside it.
///
/// # Panics
///
/// If a trip names a service `setups` does not have, or there are more than `u32::MAX` trips.
#[must_use]
pub fn plan_legs(
    setups: &[ServiceSetup],
    trips: &[TripRow],
    road: &RoadNetwork,
    walk: &LayerSetup,
    paths: &FrozenPaths<'_>,
) -> (Vec<(usize, Leg)>, PlanReport) {
    let mut report = PlanReport {
        trips: u32::try_from(trips.len()).expect("trips fit u32"),
        ..PlanReport::default()
    };
    let walk_graph = walk.network().network();
    let walk_snapper = NodeSnapper::every_node(walk_graph);
    let road_snapper = NodeSnapper::new(road);
    let mut reach = Reach::new(walk_graph, walk.seconds());
    let has_dock: Vec<Vec<bool>> = setups
        .iter()
        .map(|s| {
            let mut marks = vec![false; walk_graph.node_count() as usize];
            if let ServiceParts::BikeDocked(docks, _) = s.parts() {
                for w in &docks.walk_node {
                    marks[w.index()] = true;
                }
            }
            marks
        })
        .collect();
    let mut out = Vec::with_capacity(trips.len());
    for (i, t) in trips.iter().enumerate() {
        let setup = &setups[usize::from(t.service)];
        let leg = match setup.parts() {
            ServiceParts::BikeDocked(docks, d) => {
                let from = walk_snapper.nearest(walk_graph, t.origin);
                let to = walk_snapper.nearest(walk_graph, t.destination);
                let marks = &has_dock[usize::from(t.service)];
                let pick =
                    nearest_dock(&mut reach, from, d.walk_max_s, marks, &docks.walk_node, false);
                let ret = nearest_dock(&mut reach, to, d.walk_max_s, marks, &docks.walk_node, true);
                let (Some((p, walk_in)), Some((r, walk_out))) = (pick, ret) else {
                    report.no_dock += 1;
                    continue;
                };
                if p == r {
                    report.same_dock += 1;
                    continue;
                }
                let ride = paths.ride_s(docks.bike_node[p as usize], docks.bike_node[r as usize])
                    * d.ride_factor;
                if !ride.is_finite() {
                    report.no_ride += 1;
                    continue;
                }
                Leg {
                    service: t.service,
                    trip: t.trip,
                    weight: t.weight,
                    request_s: t.departure_s + walk_in,
                    origin: from,
                    destination: to,
                    pickup_dock: p,
                    return_dock: r,
                    ride_s: ride,
                    egress_s: walk_out,
                }
            }
            ServiceParts::Taxi(..) => Leg {
                service: t.service,
                trip: t.trip,
                weight: t.weight,
                request_s: t.departure_s,
                origin: road_snapper.nearest(road, t.origin),
                destination: road_snapper.nearest(road, t.destination),
                pickup_dock: u32::MAX,
                return_dock: u32::MAX,
                ride_s: f64::NAN,
                egress_s: f64::NAN,
            },
        };
        report.legs += 1;
        out.push((i, leg));
    }
    (out, report)
}

/// The rides and walks a frozen day's redirections ask for: between a service's docks on the
/// bike layer at its `ride_factor`, from a dock to a leg's destination on the walk layer.
#[derive(Debug)]
pub struct FrozenLegs<'a> {
    setups: &'a [ServiceSetup],
    paths: &'a FrozenPaths<'a>,
    /// Per leg of the day, its service and its destination's walk node (a taxi leg's is
    /// unused).
    destinations: Vec<(u16, NodeId)>,
}

impl<'a> FrozenLegs<'a> {
    /// For the day's `legs` of `setups`, on `paths`.
    #[must_use]
    pub fn new(setups: &'a [ServiceSetup], paths: &'a FrozenPaths<'a>, legs: &[Leg]) -> Self {
        Self {
            setups,
            paths,
            destinations: legs.iter().map(|l| (l.service, l.destination)).collect(),
        }
    }
}

impl ServiceLegs for FrozenLegs<'_> {
    fn ride_s(&self, service: u16, from: u32, to: u32) -> f64 {
        match self.setups[usize::from(service)].parts() {
            ServiceParts::BikeDocked(docks, d) => {
                self.paths.ride_s(docks.bike_node[from as usize], docks.bike_node[to as usize])
                    * d.ride_factor
            }
            ServiceParts::Taxi(..) => f64::INFINITY,
        }
    }

    fn walk_to_destination_s(&self, leg: usize, dock: u32) -> f64 {
        let (service, to) = self.destinations[leg];
        match self.setups[usize::from(service)].parts() {
            ServiceParts::BikeDocked(docks, _) => docks
                .walk_node
                .get(dock as usize)
                .map_or(f64::INFINITY, |&w| self.paths.walk_s(w, to)),
            ServiceParts::Taxi(..) => f64::INFINITY,
        }
    }
}
