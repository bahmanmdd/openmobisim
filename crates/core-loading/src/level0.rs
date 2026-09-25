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
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

use crate::link_bins::{EntryTables, LinkBinRecorder, LinkBins};
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
    traverse_exact(vehicle, network, free_flow_of(network), |_, _, _| {})
}

/// [`traverse_free_flow`] at the link times in `seconds` (one per link of
/// `network`) instead of the network's free-flow times: a bike or walk trip on
/// its layer, whose times are static (S195).
#[must_use]
pub fn traverse_timed(vehicle: &Vehicle, network: &RoadNetwork, seconds: &[f64]) -> Trajectory {
    traverse_exact(vehicle, network, |l| seconds[l.index()], |_, _, _| {})
}

/// The network's own free-flow time of a link, in seconds.
fn free_flow_of(network: &RoadNetwork) -> impl Fn(LinkId) -> f64 + '_ {
    |link| free_flow_seconds(network.free_flow_time(link))
}

/// [`traverse_free_flow`], reporting each link's exact `(link, enter, exit)`
/// times, in seconds and unfloored, to `on_link` as it goes.
fn traverse_exact(
    vehicle: &Vehicle,
    network: &RoadNetwork,
    seconds_of: impl Fn(LinkId) -> f64,
    mut on_link: impl FnMut(LinkId, f64, f64),
) -> Trajectory {
    let mut links = Vec::with_capacity(vehicle.route.len());
    // The clock stays exact; only what is *recorded* is floored to a whole
    // second (S88). Flooring the clock itself would drop the sub-second
    // remainder of every link — about half a second each on average, so a
    // hundred-link trip would come out a minute short (S161, F1).
    let mut clock = f64::from(vehicle.departure.get());
    for (i, &link) in vehicle.route.iter().enumerate() {
        if i > 0 {
            let previous = vehicle.route[i - 1];
            debug_assert!(
                network.link_to(previous) == network.link_from(link),
                "route is discontinuous: {previous:?} does not lead to {link:?}"
            );
        }
        let end = clock + seconds_of(link);
        links.push(LinkTraversal { link, enter: floored(clock), exit: floored(end) });
        on_link(link, clock, end);
        clock = end;
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

/// [`load_level_0`], also returning the per-link, per-time-bin results (S163):
/// every link traversal that finishes inside `window` seconds, in bins of
/// `bin_seconds`, with each traversal's exact free-flow time.
///
/// Level 0 visits vehicles one at a time, so the traversals are collected and
/// put in exit-time order before they are binned: `16` bytes per link
/// crossing while it runs.
///
/// # Panics
///
/// Panics if `bin_seconds` is zero.
pub fn load_level_0_binned<'a>(
    vehicles: impl IntoIterator<Item = &'a Vehicle>,
    network: &RoadNetwork,
    window: f64,
    bin_seconds: u32,
) -> (Vec<Trajectory>, LinkBins) {
    binned(vehicles, network, free_flow_of(network), window, bin_seconds)
}

/// [`load_level_0_binned`] at the link times in `seconds` (S195): a bike or
/// walk layer's per-link results. Each vehicle's `pcu` is what the bins sum,
/// which on a static layer is its traveller weight.
///
/// # Panics
///
/// Panics if `bin_seconds` is zero.
pub fn load_timed_binned<'a>(
    vehicles: impl IntoIterator<Item = &'a Vehicle>,
    network: &RoadNetwork,
    seconds: &[f64],
    window: f64,
    bin_seconds: u32,
) -> (Vec<Trajectory>, LinkBins) {
    binned(vehicles, network, |l| seconds[l.index()], window, bin_seconds)
}

fn binned<'a>(
    vehicles: impl IntoIterator<Item = &'a Vehicle>,
    network: &RoadNetwork,
    seconds_of: impl Fn(LinkId) -> f64,
    window: f64,
    bin_seconds: u32,
) -> (Vec<Trajectory>, LinkBins) {
    let mut crossings: Vec<(f64, f64, u32, f64)> = Vec::new();
    let trajectories: Vec<Trajectory> = vehicles
        .into_iter()
        .map(|v| {
            let pcu = v.pcu.get();
            traverse_exact(v, network, &seconds_of, |link, enter, exit| {
                crossings.push((exit, enter, link.raw(), pcu));
            })
        })
        .collect();
    // Exit time first, then link and enter time: a total order, so the table
    // never depends on the order the vehicles were given.
    crossings.sort_by(|a, b| {
        a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)).then(a.1.total_cmp(&b.1)).then(a.3.total_cmp(&b.3))
    });
    let mut recorder = LinkBinRecorder::new(network.link_count() as usize, bin_seconds, window);
    for (exit, enter, link, pcu) in crossings {
        recorder.record(LinkId::from_index(link as usize), enter, exit, pcu);
    }
    (trajectories, recorder.finish())
}

/// [`load_level_0_binned`], also returning the traversals filed by the bin they
/// **entered** their link in (S170).
///
/// # Panics
///
/// Panics only if the recorder's entry tables were not built, which they always are
/// here.
#[must_use]
pub fn load_level_0_recorded<'a>(
    vehicles: impl IntoIterator<Item = &'a Vehicle>,
    network: &RoadNetwork,
    window: f64,
    bin_seconds: u32,
) -> (Vec<Trajectory>, LinkBins, EntryTables) {
    let mut crossings: Vec<(f64, f64, u32, f64)> = Vec::new();
    let trajectories: Vec<Trajectory> = vehicles
        .into_iter()
        .map(|v| {
            let pcu = v.pcu.get();
            traverse_exact(v, network, free_flow_of(network), |link, enter, exit| {
                crossings.push((exit, enter, link.raw(), pcu));
            })
        })
        .collect();
    crossings.sort_by(|a, b| {
        a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)).then(a.1.total_cmp(&b.1)).then(a.3.total_cmp(&b.3))
    });
    let mut recorder =
        LinkBinRecorder::new(network.link_count() as usize, bin_seconds, window).with_entry_bins();
    for (exit, enter, link, pcu) in crossings {
        recorder.record(LinkId::from_index(link as usize), enter, exit, pcu);
    }
    let (exit, tables) = recorder.finish_with_entry();
    (trajectories, exit, tables.expect("entry bins were asked for"))
}

/// A link's free-flow time in seconds, checked to be usable as a duration.
fn free_flow_seconds(duration: Duration) -> f64 {
    debug_assert!(
        duration.get().is_finite() && duration.get() >= 0.0,
        "a link's free-flow time must be finite and non-negative, got {}",
        duration.get()
    );
    duration.get()
}

/// An exact time floored to a whole second (S88's convention).
fn floored(seconds: f64) -> Second {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the clock is finite and non-negative (departures are unsigned, durations are                   checked); truncating is the deliberate floor to a whole second S88 requires,                   and f64-to-u32 `as` casts saturate rather than wrap on overflow"
    )]
    let whole_seconds = seconds as u32;
    Second(whole_seconds)
}
