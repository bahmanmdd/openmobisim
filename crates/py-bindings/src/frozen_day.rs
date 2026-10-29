//! A run's day frozen (`Run.freeze`), and read back as a fleet day's legs (`fleet_day(...,
//! frozen=folder)`, `fleet_environment(..., frozen=folder)`).
//!
//! **What is kept.** A run with a service keeps, beside its summary, its last loading's
//! shared-bike day ([`ShareDay`]: the legs as the engine played them, and their outcome), the
//! services as the run built them, that loading's road times and the run's window, seed and
//! fingerprints. A run without a service keeps nothing.
//!
//! **The folder.** `legs.parquet` ([`openmobisim_io_parquet::frozen_legs`]: one row per leg, every
//! number bit for bit), `road_times.bin` (the road times in the network cache's binary form,
//! [`RoadTimes::write_binary`]) and `setup.json` (written by `run.py` from [`ShareFreeze::setup`]).
//!
//! **Read back**, the legs become the engine's legs again, in the run's order, so a replay with
//! no policy plays what the run played. The folder is refused with a network or services other
//! than the run's (their fingerprints).
//!
//! **Cost:** the day kept is the run's own (about 120 bytes a leg), plus the road times (8 bytes
//! per link and per (link, bin) row with traffic).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::link_geometry::NetworkFingerprint;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::LinkBins;
use openmobisim_core_sim::identity::services_fingerprint;
use openmobisim_core_sim::services::engine::Leg;
use openmobisim_core_sim::services::frozen::PlanReport;
use openmobisim_core_sim::services::in_run::ShareDay;
use openmobisim_core_sim::services::queries::ZONE_GRID_M;
use openmobisim_core_sim::services::roads::RoadTimes;
use openmobisim_core_sim::services::{ServiceParts, ServiceSetup};
use openmobisim_core_types::hash::fingerprint_hex;
use openmobisim_core_types::ids::{EntityId, NodeId};
use openmobisim_io_parquet::{FrozenLegRow, read_frozen_legs, write_frozen_legs};

use crate::network::PyNetwork;
use crate::services::{
    Frozen, Planned, ServiceTuple, TripTuple, assemble, layers_of, outcome_dict, setups_of,
};

/// The legs' file in a frozen day's folder.
pub(crate) const LEGS_FILE: &str = "legs.parquet";
/// The road times' file.
pub(crate) const ROAD_TIMES_FILE: &str = "road_times.bin";

/// What a run with a service keeps to freeze its last loading's day (see the
/// [module docs](self)).
pub(crate) struct ShareFreeze {
    setups: Arc<Vec<ServiceSetup>>,
    /// The last loading's day; `None` if no trip took a shared bike in it.
    day: Option<ShareDay>,
    /// Per leg of the day, its traveller's id and the trip's place in the traveller's day.
    who: Vec<(String, u32)>,
    times: RoadTimes,
    window: (f64, f64),
    seed: u64,
    run_fingerprint: String,
    network_fingerprint: String,
}

impl ShareFreeze {
    /// What a run on `net` with `setups` keeps: its last loading's `day`, its trips' ids from
    /// `trips` and `travellers`, the road times of that loading's `link_bins` (each link's mean
    /// time in each bin, `pcu_seconds / pcu`, as `road_times_read` reads `link_bins.parquet`;
    /// free flow without them), its `window`, `seed` and fingerprint.
    ///
    /// # Errors
    ///
    /// A message if the link bins do not make road times (a link index not of `net`).
    #[allow(clippy::too_many_arguments, reason = "what a run keeps, named")]
    pub(crate) fn new(
        setups: Arc<Vec<ServiceSetup>>,
        day: Option<ShareDay>,
        trips: &Trips,
        travellers: &Travellers,
        link_bins: Option<&LinkBins>,
        net: &RoadNetwork,
        window: (f64, f64),
        seed: u64,
        run_fingerprint: String,
    ) -> Result<Self, String> {
        let who = day
            .as_ref()
            .map(|d| {
                d.legs
                    .iter()
                    .map(|l| {
                        let traveller = trips.traveller(l.trip);
                        let first = travellers.first_trip(traveller);
                        (
                            travellers.external_ids().external(traveller.raw()).to_string(),
                            l.trip.raw() - first.raw(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let times = match link_bins {
            Some(b) => {
                let rows: Vec<(u32, u32, f64)> = b
                    .links()
                    .iter()
                    .zip(b.bins())
                    .zip(b.pcu().iter().zip(b.pcu_seconds()))
                    .filter(|(_, (p, s))| **p > 0.0 && **s > 0.0)
                    .map(|((&l, &bin), (p, s))| (l, bin, s / p))
                    .collect();
                RoadTimes::from_rows(net, f64::from(b.bin_seconds()), &rows)?
            }
            None => RoadTimes::free_flow(net),
        };
        Ok(Self {
            setups,
            day,
            who,
            times,
            window,
            seed,
            run_fingerprint,
            network_fingerprint: fingerprint_hex(NetworkFingerprint::of(net).value()),
        })
    }

    /// The day's legs as `legs.parquet`'s rows, in the order the run played them.
    fn rows(&self) -> Vec<FrozenLegRow> {
        let Some(day) = &self.day else { return Vec::new() };
        day.legs
            .iter()
            .zip(&self.who)
            .map(|(l, (traveller, seq))| {
                let setup = &self.setups[usize::from(l.leg.service)];
                let dock = |d: u32| match setup.parts() {
                    ServiceParts::BikeDocked(docks, _) => docks.external[d as usize].clone(),
                    ServiceParts::Taxi(..) => String::new(),
                };
                FrozenLegRow {
                    trip: l.leg.trip,
                    traveller_id: traveller.clone(),
                    trip_seq: *seq,
                    service: setup.name().to_string(),
                    weight: l.leg.weight,
                    request_s: l.leg.request_s,
                    origin_node: l.leg.origin.raw(),
                    destination_node: l.leg.destination.raw(),
                    dock_from: dock(l.leg.pickup_dock),
                    dock_to: dock(l.leg.return_dock),
                    ride_s: l.leg.ride_s,
                    egress_s: l.leg.egress_s,
                    expected_dock_s: l.expected_dock_s,
                }
            })
            .collect()
    }

    /// Write `legs.parquet` and `road_times.bin` into `folder` (which exists).
    ///
    /// # Errors
    ///
    /// A `ValueError` if a file cannot be written.
    pub(crate) fn write(&self, folder: &Path) -> PyResult<()> {
        let bad = |e: String| PyValueError::new_err(e);
        let metadata = [
            ("openmobisim.run_fingerprint".to_string(), self.run_fingerprint.clone()),
            ("openmobisim.network_fingerprint".to_string(), self.network_fingerprint.clone()),
            ("openmobisim.master_seed".to_string(), self.seed.to_string()),
        ];
        write_frozen_legs(folder.join(LEGS_FILE), &self.rows(), &metadata)
            .map_err(|e| bad(e.to_string()))?;
        std::fs::write(folder.join(ROAD_TIMES_FILE), self.times.write_binary())
            .map_err(|e| bad(format!("{}: {e}", folder.join(ROAD_TIMES_FILE).display())))
    }

    /// What `setup.json` records: the fingerprints (the run's, its network's, its services'),
    /// the window, the seed, the services, the legs and the road times' size.
    pub(crate) fn setup<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("run_fingerprint", &self.run_fingerprint)?;
        d.set_item("network_fingerprint", &self.network_fingerprint)?;
        d.set_item("services_fingerprint", fingerprint_hex(services_fingerprint(&self.setups)))?;
        d.set_item("window", (self.window.0, self.window.1))?;
        d.set_item("seed", self.seed)?;
        let services = self
            .setups
            .iter()
            .map(|s| {
                let one = PyDict::new(py);
                one.set_item("name", s.name())?;
                one.set_item("kind", s.kind().as_str())?;
                if let ServiceParts::BikeDocked(docks, _) = s.parts() {
                    one.set_item("docks", docks.len())?;
                }
                let options = PyDict::new(py);
                for (k, v) in s.option_values() {
                    options.set_item(k, v)?;
                }
                one.set_item("options", options)?;
                Ok(one)
            })
            .collect::<PyResult<Vec<_>>>()?;
        d.set_item("services", services)?;
        let legs = self.day.as_ref().map_or(0, |day| day.legs.len());
        let people: u64 = self
            .day
            .as_ref()
            .map_or(0, |day| day.legs.iter().map(|l| u64::from(l.leg.weight)).sum());
        d.set_item("legs", legs)?;
        d.set_item("people", people)?;
        d.set_item("road_times_bin_s", self.times.bin_s())?;
        d.set_item("road_times_rows", self.times.rows())?;
        Ok(d)
    }

    /// The run's last loading's day in `fleet_day`'s tables (its trips, the docks' bins, the
    /// plan, the counts), on `network`, the run's: what a replay of the frozen day is compared
    /// with.
    ///
    /// # Errors
    ///
    /// A `ValueError` if the legs, written as rows and read back in memory, are not the run's
    /// legs bit for bit (a bug), or for another network.
    pub(crate) fn outcome<'py>(
        &self,
        py: Python<'py>,
        network: &PyNetwork,
    ) -> PyResult<Bound<'py, PyDict>> {
        let mine = fingerprint_hex(NetworkFingerprint::of(&network.inner).value());
        if mine != self.network_fingerprint {
            return Err(PyValueError::new_err("the run's day is of another network"));
        }
        let (walk, bike) = layers_of(network)?;
        let rows = self.rows();
        let planned = planned_of(&self.setups, walk.network().network(), &rows)?;
        let legs: Vec<Leg> =
            self.day.as_ref().map_or_else(Vec::new, |d| d.legs.iter().map(|l| l.leg).collect());
        let same = planned.planned.len() == legs.len()
            && planned.planned.iter().zip(&legs).all(|((_, a), b)| same_leg(a, b));
        if !same {
            return Err(PyValueError::new_err("the day's legs do not read back as the run's"));
        }
        let names = self.setups.iter().map(|s| s.name().to_string()).collect();
        let f = assemble(
            network,
            Arc::clone(&self.setups),
            walk,
            bike,
            self.times.clone(),
            planned,
            names,
            ZONE_GRID_M,
        )?;
        let Some(day) = &self.day else {
            return Err(PyValueError::new_err(
                "no trip took a shared bike in the run's last loading",
            ));
        };
        let outcome = &day.outcome;
        outcome_dict(py, &f, outcome)
    }
}

/// Two legs equal bit for bit.
fn same_leg(a: &Leg, b: &Leg) -> bool {
    a.service == b.service
        && a.trip == b.trip
        && a.weight == b.weight
        && a.request_s.to_bits() == b.request_s.to_bits()
        && a.origin == b.origin
        && a.destination == b.destination
        && a.pickup_dock == b.pickup_dock
        && a.return_dock == b.return_dock
        && a.ride_s.to_bits() == b.ride_s.to_bits()
        && a.egress_s.to_bits() == b.egress_s.to_bits()
}

/// The engine's legs of `rows` for `setups` on the walk layer `walk`, each its own trip row.
fn planned_of(
    setups: &[ServiceSetup],
    walk: &RoadNetwork,
    rows: &[FrozenLegRow],
) -> PyResult<Planned> {
    let bad = |e: String| PyValueError::new_err(e);
    let services: HashMap<&str, usize> =
        setups.iter().enumerate().map(|(i, s)| (s.name(), i)).collect();
    let docks: Vec<HashMap<&str, u32>> = setups
        .iter()
        .map(|s| match s.parts() {
            ServiceParts::BikeDocked(docks, _) => docks
                .external
                .iter()
                .enumerate()
                .map(|(i, d)| (d.as_str(), u32::try_from(i).expect("docks fit u32")))
                .collect(),
            ServiceParts::Taxi(..) => HashMap::new(),
        })
        .collect();
    let nodes = walk.node_count();
    let mut planned = Vec::with_capacity(rows.len());
    let mut trips: Vec<TripTuple> = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        let &s = services
            .get(r.service.as_str())
            .ok_or_else(|| bad(format!("leg {i}: no service {:?} here", r.service)))?;
        if !matches!(setups[s].parts(), ServiceParts::BikeDocked(..)) {
            return Err(bad(format!("leg {i}: {:?} is not a docked bike service", r.service)));
        }
        let dock = |id: &str| {
            docks[s]
                .get(id)
                .copied()
                .ok_or_else(|| bad(format!("leg {i}: no dock {id:?} in {:?}", r.service)))
        };
        if r.origin_node >= nodes || r.destination_node >= nodes {
            return Err(bad(format!("leg {i}: a walk node not of this network")));
        }
        let (origin, destination) = (NodeId::new(r.origin_node), NodeId::new(r.destination_node));
        let leg = Leg {
            service: u16::try_from(s).expect("services fit u16"),
            trip: r.trip,
            weight: r.weight,
            request_s: r.request_s,
            origin,
            destination,
            pickup_dock: dock(&r.dock_from)?,
            return_dock: dock(&r.dock_to)?,
            ride_s: r.ride_s,
            egress_s: r.egress_s,
        };
        let (o, d) = (walk.node_lonlat(origin), walk.node_lonlat(destination));
        trips.push((
            r.trip.to_string(),
            leg.service,
            r.weight,
            r.request_s,
            o.lon,
            o.lat,
            d.lon,
            d.lat,
        ));
        planned.push((i, leg));
    }
    let n = u32::try_from(rows.len()).expect("legs fit u32");
    let report = PlanReport { trips: n, legs: n, ..PlanReport::default() };
    Ok(Planned { planned, report, trips })
}

/// A frozen day as `fleet.py` hands it: the folder, and the services' and network's
/// fingerprints its `setup.json` records.
pub(crate) type FrozenArg = (String, String, String);

/// A frozen day's inputs from the folder `frozen` (see the [module docs](self)), for `services`
/// on `network`.
///
/// # Errors
///
/// A `ValueError` naming what differs for another network or other services, or for a folder
/// whose files cannot be read.
pub(crate) fn frozen_read(
    network: &PyNetwork,
    services: &[ServiceTuple<'_>],
    frozen: &FrozenArg,
    zone_grid_m: f64,
) -> PyResult<Frozen> {
    let bad = |e: String| PyValueError::new_err(e);
    let (folder, services_fp, network_fp) = frozen;
    let net = &network.inner;
    let mine = fingerprint_hex(NetworkFingerprint::of(net).value());
    if &mine != network_fp {
        return Err(bad(format!(
            "the frozen day {folder:?} is of network {network_fp}, not this one ({mine})"
        )));
    }
    let (walk, bike) = layers_of(network)?;
    let setups = setups_of(network, services, &walk, &bike)?;
    let theirs = fingerprint_hex(services_fingerprint(&setups));
    if &theirs != services_fp {
        return Err(bad(format!(
            "the frozen day {folder:?} was made with other services (fingerprint {services_fp}; \
             these give {theirs}): their names, docks, bikes or options differ"
        )));
    }
    let dir = Path::new(folder);
    let bytes = std::fs::read(dir.join(ROAD_TIMES_FILE))
        .map_err(|e| bad(format!("{}: {e}", dir.join(ROAD_TIMES_FILE).display())))?;
    let times = RoadTimes::read_binary(&bytes)
        .map_err(|e| bad(format!("{}: {e}", dir.join(ROAD_TIMES_FILE).display())))?;
    let rows = read_frozen_legs(dir.join(LEGS_FILE)).map_err(bad)?;
    let planned = planned_of(&setups, walk.network().network(), &rows)?;
    let names = services.iter().map(|s| s.1.clone()).collect();
    assemble(network, Arc::new(setups), walk, bike, times, planned, names, zone_grid_m)
}
