//! Loaders: the dynamic network loading behind one contract.
//!
//! A [`Loader`] takes the vehicles of an assignment iteration (each with its route, its
//! weight and its departure second: when it sets off, its leg's access time after the leg's
//! departure) and the network, moves them, and returns what happened
//! ([`Loaded`]): each vehicle's arrival second at the downstream end of its last link (or that
//! it had not arrived when the loading ended), and the **entry-time tables** the loading
//! recorded, from which the measure stage builds the link travel times by time of day that the
//! next iteration's searches read.
//!
//! **Level 0, free flow** ([`FreeFlow`]): no capacity and no interaction between vehicles.
//! Each vehicle arrives at its departure second plus the sum of its links' free-flow times
//! (control delay included), exactly, with no rounding of the clock. It records no table: every
//! link takes its free-flow time. A vehicle whose arrival would fall after the loading's end
//! ([`FreeFlow::end_seconds`], unbounded by default) is reported as unfinished.
//!
//! **Level 2, the point queue** ([`PointQueue`]): every link holds a vertical queue at its
//! downstream end that discharges at the link's capacity (times the green share on a
//! signalised approach), so a vehicle's time on a link is its free-flow time plus the wait in
//! that queue; queues take no road space and never block the link upstream (Vickrey 1969).
//! See [`PointQueue`] for what it records and how signals enter.

use std::collections::HashMap;

use openmobisim_core_demand::Trips;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{EntryTables, FidelityLevel, LinkTimes, Vehicle, run_ltm_recorded};
use openmobisim_core_types::ids::{EntityId, TripId, VehicleId};
use openmobisim_core_types::units::Duration;

/// What a loading returns.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// Per vehicle, in the order the vehicles were given: its arrival, in seconds after
    /// midnight, at the downstream end of its last link; `None` if it had not arrived when the
    /// loading ended (unfinished).
    pub arrival: Vec<Option<f64>>,
    /// The entry-time tables the loading recorded: per link and per bin of entry time, the
    /// mean time the vehicles that entered the link in that bin took to cross it, and per first
    /// link and bin of departure, the mean wait outside the network before getting onto it.
    /// `None` for a loading that records none (level 0), where every link takes its free-flow
    /// time.
    pub entry: Option<EntryTables>,
}

impl Loaded {
    /// The link travel times by time of day this loading measured: built from its entry-time
    /// tables ([`LinkTimes::from_tables`]: a bin nobody entered reads the last recorded time
    /// carried forward, less the time elapsed, down to free flow), or the free-flow times when
    /// it recorded none.
    #[must_use]
    pub fn link_times(&self, network: &RoadNetwork) -> LinkTimes {
        match &self.entry {
            Some(tables) => LinkTimes::from_tables(network, tables),
            None => LinkTimes::free_flow(network),
        }
    }
}

/// A dynamic network loading. See the [module docs](self). It is shared by the threads that
/// run an assignment (as the choice model is), so it holds nothing it changes.
pub trait Loader: Send + Sync {
    /// Move `vehicles` on `network` and say what happened.
    fn load(&self, vehicles: &[Vehicle], network: &RoadNetwork) -> Loaded;

    /// When the loading ends, in seconds after midnight: a trip whose last leg ends later (its
    /// vehicle's arrival plus the leg's egress time) is unfinished. Unbounded unless a loader
    /// says otherwise.
    fn end_seconds(&self) -> f64 {
        f64::INFINITY
    }
}

/// Level 0: every vehicle at free flow, independently. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreeFlow {
    /// When the loading ends, in seconds after midnight: a vehicle arriving later is unfinished.
    pub end_seconds: f64,
}

impl Default for FreeFlow {
    /// A loading that never ends: every vehicle arrives.
    fn default() -> Self {
        Self { end_seconds: f64::INFINITY }
    }
}

impl Loader for FreeFlow {
    fn load(&self, vehicles: &[Vehicle], network: &RoadNetwork) -> Loaded {
        let link_times = LinkTimes::free_flow(network);
        let arrival = vehicles
            .iter()
            .map(|v| {
                let links: Vec<u32> = v.route.iter().map(|l| l.raw()).collect();
                let departure = f64::from(v.departure.get());
                let at = departure + link_times.route_seconds(&links, departure);
                (at <= self.end_seconds).then_some(at)
            })
            .collect();
        Loaded { arrival, entry: None }
    }

    fn end_seconds(&self) -> f64 {
        self.end_seconds
    }
}

/// The loading step of [`PointQueue`] by default, in seconds: the step the existing loading
/// runs at. Nothing about a vehicle's timing depends on it; it bounds how often the loading's
/// bookkeeping runs.
pub const DEFAULT_STEP_SECONDS: f64 = 300.0;

/// The width of [`PointQueue`]'s entry-time bins by default, in seconds: the bins the link
/// travel times, and so the next iteration's route costs, are read in.
pub const DEFAULT_BIN_SECONDS: u32 = 300;

/// Level 2, the point queue: `core-loading`'s link transmission model at its point-queue
/// level, with the classic rules (no inflow headway at nodes or origins).
///
/// **What a vehicle does.** It enters its first link at its departure second (storage is
/// infinite, so nothing holds it outside the network), crosses each link at the free-flow
/// speed, and joins the queue at the link's downstream end, which lets vehicle fronts out no
/// closer together than `PCU / μ`, where `μ` is the link's capacity, times the green share
/// `g/C` on a **signalised approach** (a link whose downstream node has a signal); on such an
/// approach it then waits the signal's control delay at the stop line (Webster's uniform
/// delay, a constant of the network) before entering the next link. The next link takes it at
/// once. So on a link of free-flow time `t` and discharge rate `μ`, a vehicle's time is `t`
/// plus its wait in the queue, which for arrivals at a constant rate `λ > μ` over a duration
/// `T` grows at `λ − μ` and is gone at `T' = λT/μ`, a total delay of `½ (λ − μ) T T'`
/// (the deterministic queue; Newell 1982). Signals are read from the network and its turn
/// table as the loading reads them: there is no option for them, and a network without signals
/// is one where they are ignored.
///
/// **What it records.** Each vehicle's arrival, floored to the whole second, if it arrived by
/// the window's end ([`Self::end_seconds`]); and the entry-time tables in bins of
/// [`Self::bin_seconds`], every link traversal counted in the bin of its entry with its exact
/// time, the traversals still under way at the window's end counted with the time they had
/// taken so far (a lower bound). Vehicles departing at or after the window's end do not
/// travel and are unfinished.
///
/// The loading is deterministic: the same vehicles give the same result, to the bit.
#[derive(Clone, Copy, Debug)]
pub struct PointQueue<'a> {
    /// The network's turn table, which says which turns exist and each signalised approach's
    /// green share.
    pub turns: &'a TurnTable,
    /// When the loading ends, in seconds after midnight.
    pub end_seconds: f64,
    /// The loading step, in seconds ([`DEFAULT_STEP_SECONDS`]).
    pub step_seconds: f64,
    /// The width of the entry-time bins, in seconds ([`DEFAULT_BIN_SECONDS`]).
    pub bin_seconds: u32,
}

impl<'a> PointQueue<'a> {
    /// The point queue on a network with `turns`, ending at `end_seconds` after midnight, with
    /// the default step and bins.
    ///
    /// # Panics
    ///
    /// Panics if `end_seconds` is not a finite number of at least 0.
    #[must_use]
    pub fn new(turns: &'a TurnTable, end_seconds: f64) -> Self {
        assert!(
            end_seconds.is_finite() && end_seconds >= 0.0,
            "the loading must end at a finite, non-negative second, got {end_seconds}"
        );
        Self {
            turns,
            end_seconds,
            step_seconds: DEFAULT_STEP_SECONDS,
            bin_seconds: DEFAULT_BIN_SECONDS,
        }
    }

    /// The point queue ending `margin_seconds` after the last departure of `trips`.
    ///
    /// # Panics
    ///
    /// Panics if `margin_seconds` is not a finite number of at least 0.
    #[must_use]
    pub fn after_last_departure(turns: &'a TurnTable, trips: &Trips, margin_seconds: f64) -> Self {
        let last = (0..trips.len())
            .map(|t| trips.departure(TripId::new(t)).get())
            .max()
            .map_or(0.0, f64::from);
        Self::new(turns, last + margin_seconds)
    }
}

impl Loader for PointQueue<'_> {
    /// # Panics
    ///
    /// Panics if [`PointQueue::step_seconds`] is not positive or [`PointQueue::bin_seconds`]
    /// is 0.
    fn load(&self, vehicles: &[Vehicle], network: &RoadNetwork) -> Loaded {
        let (trajectories, _, tables) = run_ltm_recorded(
            network,
            self.turns,
            vehicles,
            Duration(self.end_seconds),
            Duration(self.step_seconds),
            FidelityLevel::PointQueue,
            self.bin_seconds,
        );
        let mut arrival = vec![None; vehicles.len()];
        // The loop numbers its vehicles 0, 1, 2, … in order, so a vehicle's number is its place;
        // other numberings are looked up.
        if vehicles.iter().enumerate().all(|(i, v)| v.id.index() == i) {
            for t in &trajectories {
                if let Some(slot) = arrival.get_mut(t.vehicle.index()) {
                    *slot = Some(f64::from(t.arrival().get()));
                }
            }
        } else {
            let position: HashMap<VehicleId, usize> =
                vehicles.iter().enumerate().map(|(i, v)| (v.id, i)).collect();
            for t in &trajectories {
                if let Some(&i) = position.get(&t.vehicle) {
                    arrival[i] = Some(f64::from(t.arrival().get()));
                }
            }
        }
        Loaded { arrival, entry: Some(tables) }
    }

    fn end_seconds(&self) -> f64 {
        self.end_seconds
    }
}
