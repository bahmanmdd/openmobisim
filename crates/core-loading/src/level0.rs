//! Level 0: free-flow traversal — "none; static times" (design §10, the
//! debug rung of the fidelity ladder). No curves, no capacity, no
//! interaction between vehicles: each vehicle's trajectory is computed
//! independently, purely from the network's free-flow times (S90's control
//! delay is already folded into [`RoadNetwork::free_flow_time`], so summing
//! it per link is "free-flow time plus control delay" in one number).
//!
//! This is deliberately the trivial case of the nested fidelity ladder
//! (design §10: "Fidelity levels are nested — the same code with parameters
//! changed"): what it exercises is the [`Vehicle`]/[`Trajectory`] plumbing
//! levels 1–4 will reuse, not a numerical method — there isn't one yet.

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

use crate::vehicle::Vehicle;

/// One vehicle's time on one link of its route.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LinkTraversal {
    /// The link.
    pub link: LinkId,
    /// When the vehicle entered it.
    pub enter: Second,
    /// When the vehicle left it, floored to a whole second — the same
    /// convention S88 sets for the curve-interpolated exit times levels 1–4
    /// will produce, so a caller never needs to know which level produced a
    /// [`Trajectory`].
    pub exit: Second,
}

/// One vehicle's whole path through the network, in time.
#[derive(Clone, Debug)]
pub struct Trajectory {
    /// The vehicle this trajectory belongs to.
    pub vehicle: VehicleId,
    /// When the vehicle was scheduled to depart. Its first link's entry can be
    /// later, if it waited at the origin for room (levels 2–4).
    pub departure: Second,
    /// One entry per link of the vehicle's route, in route order. Never
    /// empty: [`Vehicle::route`] is never empty, and this has one entry per
    /// route link.
    pub links: Vec<LinkTraversal>,
}

impl Trajectory {
    /// When the vehicle left the last link of its route — its arrival time.
    ///
    /// # Panics
    ///
    /// Panics if the trajectory has no links, which cannot happen for a
    /// [`Trajectory`] this crate produced.
    #[must_use]
    pub fn arrival(&self) -> Second {
        self.links.last().expect("a trajectory always has at least one link").exit
    }

    /// When the vehicle was scheduled to depart — the start of its travel
    /// time, including any wait at the origin.
    #[must_use]
    pub fn departure(&self) -> Second {
        self.departure
    }

    /// Total travel time from departure to arrival.
    #[must_use]
    pub fn total_travel_time(&self) -> Duration {
        Duration::from_clock(self.arrival()) - Duration::from_clock(self.departure())
    }
}

/// Advance `vehicle` along its route at free-flow time, with no interaction
/// with any other vehicle — level 0's whole mechanism.
#[must_use]
pub fn traverse_free_flow(vehicle: &Vehicle, network: &RoadNetwork) -> Trajectory {
    let mut links = Vec::with_capacity(vehicle.route.len());
    let mut clock = vehicle.departure;
    for (i, &link) in vehicle.route.iter().enumerate() {
        if i > 0 {
            let previous = vehicle.route[i - 1];
            debug_assert!(
                network.link_to(previous) == network.link_from(link),
                "route is discontinuous: {previous:?} does not lead to {link:?}"
            );
        }
        let exit = floored_exit(clock, network.free_flow_time(link));
        links.push(LinkTraversal { link, enter: clock, exit });
        clock = exit;
    }
    Trajectory { vehicle: vehicle.id, departure: vehicle.departure, links }
}

/// Every vehicle's trajectory, independently — level 0 has no interaction
/// between vehicles, so this is a pure per-vehicle map, not a stepped
/// simulation. Levels 1–4 replace this function, not [`Vehicle`] or
/// [`Trajectory`].
pub fn load_level_0<'a>(
    vehicles: impl IntoIterator<Item = &'a Vehicle>,
    network: &RoadNetwork,
) -> Vec<Trajectory> {
    vehicles.into_iter().map(|v| traverse_free_flow(v, network)).collect()
}

/// `base + duration`, floored to a whole second (S88's convention).
fn floored_exit(base: Second, duration: Duration) -> Second {
    debug_assert!(
        duration.get().is_finite() && duration.get() >= 0.0,
        "a link's free-flow time must be finite and non-negative, got {}",
        duration.get()
    );
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "duration is finite and non-negative (asserted above); truncating is the \
                  deliberate floor to a whole second S88 requires, and f64-to-u32 `as` casts \
                  saturate rather than wrap on overflow"
    )]
    let whole_seconds = duration.get() as u32;
    base.saturating_add(whole_seconds)
}
