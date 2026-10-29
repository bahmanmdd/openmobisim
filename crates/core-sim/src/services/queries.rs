//! What a policy may ask of its view at a step, besides the state: travel times on the roads as
//! they are at the step, the legs to expect per dock or zone and time bin, the docks or vehicles
//! nearest a node; and the zones.
//!
//! **Travel times** ([`Queries::travel_times`]): from each source road node to each target, in
//! seconds, leaving at the step's second on the day's road times (time-dependent, turn rules
//! kept, only links that carry motor traffic), by one bounded earliest-arrival search per source
//! that stops once every target is found; sources in parallel (each with its own search), the
//! matrix the same at any number of threads. Infinite past `max_s` (and past the longest drive
//! a vehicle's search looks for, [`super::roads::DRIVE_BOUND_S`]). Every service's vehicles
//! drive the roads (a docked service's vans, a taxi fleet's cars), so the times are the same
//! for each.
//!
//! **Expected legs** ([`Queries::expected_legs`]): the people asking for a service, per dock
//! (a docked service: pickups at the dock each leg planned) or per zone (a taxi fleet: requests
//! by the zone of their origin node), per time bin of the window, from the **world's legs**:
//! the day's legs before any variation a seed draws (a fleet environment's `legs_keep` and
//! `departure_jitter_s`), so the expectation over the days the seeds draw, never one day's own
//! variation. **In a frozen day without variation, the world's legs are the day's own legs:
//! there the expectation equals what the day will ask for.**
//!
//! **Nearest** ([`Queries::nearest`]): a service's docks (by their road nodes) or vehicles (by
//! the nodes they are at, or driving to) nearest a road node by free-flow time on the roads,
//! nearest first, ties by index; those no road reaches are left out.
//!
//! **Zones** ([`Zones`]): a square grid of [`ZONE_GRID_M`] metres (an assumption, not
//! calibrated; a world's option, [`super::world::FrozenWorld::with_zone_grid_m`]) laid over the
//! road network's nodes in the network's projected metres, from the south-west corner of their
//! extent; a zone is a cell holding at least one road node, numbered by row (south to north)
//! then column (west to east), so the same network always gives the same zones.
//!
//! **Cached within the step:** a query asked twice at one step with the same arguments is
//! answered once. **Cost:** a travel-time query is one search per source, bounded by `max_s`;
//! a nearest query one free-flow search over the network per source node and step; expected
//! legs one pass over the legs per (service, bin length) and step; zones built once per world
//! (one pass over the nodes, 4 bytes per node).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use openmobisim_core_graph::geometry::{LonLat, Projected, Projection};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, NodeId};

use super::engine::FleetDay;
use super::world::ServiceWorld;
use super::{ServiceParts, ServiceSetup};

/// The side of a zone's square, in metres, unless a world is given another (an assumption, not
/// calibrated: about a walk of a quarter of an hour across).
pub const ZONE_GRID_M: f64 = 1000.0;

/// A square grid of zones over a road network's nodes: see the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct Zones {
    grid_m: f64,
    projection: Projection,
    origin: Projected,
    /// Per zone, its cell `(row, column)`, ascending.
    cells: Vec<(i64, i64)>,
    lon: Vec<f64>,
    lat: Vec<f64>,
    /// Per road node, its zone.
    node_zone: Vec<u32>,
}

impl Zones {
    /// The grid of `grid_m` metres over `road`'s nodes; `None` for a network without nodes or
    /// one that cannot be projected.
    ///
    /// # Errors
    ///
    /// A message for a side that is not a positive number of metres.
    ///
    /// # Panics
    ///
    /// Never in practice: the zones of a network fit `u32`.
    pub fn grid(road: &RoadNetwork, grid_m: f64) -> Result<Option<Self>, String> {
        if !(grid_m.is_finite() && grid_m > 0.0) {
            return Err(format!("zone_grid_m must be a positive number of metres, got {grid_m}"));
        }
        let n = road.node_count() as usize;
        if n == 0 {
            return Ok(None);
        }
        let points: Vec<LonLat> = (0..n).map(|i| road.node_lonlat(NodeId::from_index(i))).collect();
        let Ok(projection) = Projection::for_extent(points.iter().copied()) else {
            return Ok(None);
        };
        let xy: Vec<Projected> = points.iter().map(|&p| projection.project(p)).collect();
        let origin = Projected::new(
            xy.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
            xy.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
        );
        let mut zones = Self {
            grid_m,
            projection,
            origin,
            cells: Vec::new(),
            lon: Vec::new(),
            lat: Vec::new(),
            node_zone: Vec::new(),
        };
        let node_cell: Vec<(i64, i64)> = xy.iter().map(|&p| zones.cell(p)).collect();
        let mut cells = node_cell.clone();
        cells.sort_unstable();
        cells.dedup();
        for &(row, col) in &cells {
            #[allow(clippy::cast_precision_loss, reason = "a cell of a city's grid")]
            let centre = Projected::new(
                origin.x + (col as f64 + 0.5) * grid_m,
                origin.y + (row as f64 + 0.5) * grid_m,
            );
            let p = projection.unproject(centre);
            zones.lon.push(p.lon);
            zones.lat.push(p.lat);
        }
        zones.cells = cells;
        zones.node_zone = node_cell
            .iter()
            .map(|c| {
                u32::try_from(zones.cells.binary_search(c).expect("a node's own cell"))
                    .expect("zones fit u32")
            })
            .collect();
        Ok(Some(zones))
    }

    fn cell(&self, p: Projected) -> (i64, i64) {
        #[allow(clippy::cast_possible_truncation, reason = "a cell of a city's grid")]
        let at = |v: f64| (v / self.grid_m).floor() as i64;
        (at(p.y - self.origin.y), at(p.x - self.origin.x))
    }

    /// The side of a zone's square, in metres.
    #[must_use]
    pub fn grid_m(&self) -> f64 {
        self.grid_m
    }

    /// How many zones.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Per zone, its centre's longitude.
    #[must_use]
    pub fn lon(&self) -> &[f64] {
        &self.lon
    }

    /// Per zone, its centre's latitude.
    #[must_use]
    pub fn lat(&self) -> &[f64] {
        &self.lat
    }

    /// Road node `node`'s zone (`None`: not a node of the network).
    #[must_use]
    pub fn of_node(&self, node: NodeId) -> Option<u32> {
        self.node_zone.get(node.index()).copied()
    }

    /// The zone of point `p` (`None`: its cell holds no road node).
    ///
    /// # Panics
    ///
    /// Never in practice: the zones of a network fit `u32`.
    #[must_use]
    pub fn of_point(&self, p: LonLat) -> Option<u32> {
        let c = self.cell(self.projection.project(p));
        self.cells.binary_search(&c).ok().map(|k| u32::try_from(k).expect("zones fit u32"))
    }
}

/// What [`Queries::nearest`] looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NearestKind {
    /// A docked service's docks, by their road nodes.
    Docks,
    /// A taxi fleet's vehicles, by their nodes now.
    Vehicles,
}

/// People asking per row (a dock or a zone) and time bin: see [`Queries::expected_legs`].
#[derive(Clone, Debug, PartialEq)]
pub struct ExpectedLegs {
    /// Rows: docks (a docked service) or zones (a taxi fleet).
    pub rows: usize,
    /// Time bins of the window, the first at its start.
    pub bins: usize,
    /// The bins' length, in seconds.
    pub bin_s: f64,
    /// Whether the rows are zones.
    pub by_zone: bool,
    /// People, row by row: `values[row * bins + bin]`.
    pub values: Vec<f64>,
}

impl ExpectedLegs {
    /// People of row `row` in bin `bin`.
    #[must_use]
    pub fn at(&self, row: usize, bin: usize) -> f64 {
        self.values[row * self.bins + bin]
    }
}

type TimesKey = (Vec<u32>, Vec<u32>, u64);

#[derive(Debug, Default)]
struct Cache {
    times: HashMap<TimesKey, Arc<Vec<f64>>>,
    reach: HashMap<u32, Arc<HashMap<u32, f64>>>,
    legs: HashMap<(u16, u64), Arc<ExpectedLegs>>,
}

/// The queries of one step: see the [module docs](self). Owned (shared parts by `Arc`), so a
/// binding's view can keep it for its step; copies share one cache.
#[derive(Clone)]
pub struct Queries {
    world: Arc<dyn ServiceWorld>,
    setups: Arc<Vec<ServiceSetup>>,
    day: Arc<FleetDay>,
    t: f64,
    /// Per service, its vehicles' nodes at the step (empty for a docked service).
    vehicles: Arc<Vec<Vec<NodeId>>>,
    cache: Arc<Mutex<Cache>>,
}

impl std::fmt::Debug for Queries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queries").field("t", &self.t).finish_non_exhaustive()
    }
}

impl Queries {
    pub(super) fn new(
        world: Arc<dyn ServiceWorld>,
        setups: Arc<Vec<ServiceSetup>>,
        day: Arc<FleetDay>,
        t: f64,
        vehicles: Vec<Vec<NodeId>>,
    ) -> Self {
        Self {
            world,
            setups,
            day,
            t,
            vehicles: Arc::new(vehicles),
            cache: Arc::new(Mutex::new(Cache::default())),
        }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The step's second, from midnight: the queries' departure time.
    #[must_use]
    pub fn t(&self) -> f64 {
        self.t
    }

    /// The zones (`None`: a world without a road network, or none that can be projected).
    #[must_use]
    pub fn zones(&self) -> Option<&Zones> {
        self.world.zones()
    }

    /// Seconds from each of `from` to each of `to`, leaving at [`Self::t`] on the road times,
    /// row by row (`out[i * to.len() + j]`), infinite where no road gets there within `max_s`
    /// (see the [module docs](self)).
    ///
    /// # Errors
    ///
    /// A message for a `max_s` that is not a number of seconds of 0 or more.
    ///
    /// # Panics
    ///
    /// If a node is not of the world's network.
    pub fn travel_times(
        &self,
        from: &[NodeId],
        to: &[NodeId],
        max_s: f64,
    ) -> Result<Arc<Vec<f64>>, String> {
        if max_s.is_nan() || max_s < 0.0 {
            return Err(format!("max_s must be 0 seconds or more, got {max_s}"));
        }
        let key: TimesKey = (
            from.iter().map(|n| n.raw()).collect(),
            to.iter().map(|n| n.raw()).collect(),
            max_s.to_bits(),
        );
        if let Some(m) = self.cache().times.get(&key) {
            return Ok(Arc::clone(m));
        }
        let mut out = Vec::new();
        self.world.with_roads(&mut |roads| out = roads.travel_times(from, to, self.t, max_s));
        let out = Arc::new(out);
        self.cache().times.insert(key, Arc::clone(&out));
        Ok(out)
    }

    /// The people asking for service `service` per dock or zone and bin of `bin_s` seconds of
    /// the window, from the world's legs (see the [module docs](self)): a leg asking before
    /// the window counts in its first bin (it asks at the window's start), one after it not at
    /// all.
    ///
    /// # Errors
    ///
    /// A message for a service the day does not have, a bin length that is not a positive
    /// number of seconds, or a taxi fleet in a world without zones.
    ///
    /// # Panics
    ///
    /// Never in practice: the window's bins fit `usize`.
    pub fn expected_legs(&self, service: u16, bin_s: f64) -> Result<Arc<ExpectedLegs>, String> {
        let setup = self
            .setups
            .get(usize::from(service))
            .ok_or_else(|| format!("no service {service}, of {}", self.setups.len()))?;
        if !(bin_s.is_finite() && bin_s > 0.0) {
            return Err(format!("bin_s must be a positive number of seconds, got {bin_s}"));
        }
        let key = (service, bin_s.to_bits());
        if let Some(m) = self.cache().legs.get(&key) {
            return Ok(Arc::clone(m));
        }
        let (start, end) = self.day.window;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "bins")]
        let bins = (((end - start) / bin_s).ceil() as usize).max(1);
        let zones = self.zones();
        let (rows, by_zone) = match setup.parts() {
            ServiceParts::BikeDocked(docks, _) => (docks.len(), false),
            ServiceParts::Taxi(..) => {
                let z = zones.ok_or_else(|| {
                    format!("{}: a taxi fleet's legs are by zone, and there are none", setup.name())
                })?;
                (z.len(), true)
            }
        };
        let mut values = vec![0.0; rows * bins];
        let legs = self.world.base_legs().unwrap_or(&self.day.legs);
        for leg in legs.iter().filter(|l| l.service == service && l.request_s <= end) {
            let row = if by_zone {
                match zones.and_then(|z| z.of_node(leg.origin)) {
                    Some(z) => z as usize,
                    None => continue,
                }
            } else {
                leg.pickup_dock as usize
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
            let bin = (((leg.request_s.max(start) - start) / bin_s) as usize).min(bins - 1);
            if row < rows {
                values[row * bins + bin] += f64::from(leg.weight);
            }
        }
        let out = Arc::new(ExpectedLegs { rows, bins, bin_s, by_zone, values });
        self.cache().legs.insert(key, Arc::clone(&out));
        Ok(out)
    }

    /// Service `service`'s `k` docks or vehicles nearest road node `from` by free-flow time:
    /// `(index, seconds)`, nearest first, ties by index; those no road reaches left out.
    ///
    /// # Errors
    ///
    /// A message for a service the day does not have, a kind the service does not have, or a
    /// docked service set up without roads.
    pub fn nearest(
        &self,
        service: u16,
        kind: NearestKind,
        from: NodeId,
        k: usize,
    ) -> Result<Vec<(u32, f64)>, String> {
        let setup = self
            .setups
            .get(usize::from(service))
            .ok_or_else(|| format!("no service {service}, of {}", self.setups.len()))?;
        let nodes: &[NodeId] = match (setup.parts(), kind) {
            (ServiceParts::BikeDocked(docks, _), NearestKind::Docks) => {
                if docks.road_node.is_empty() && !docks.is_empty() {
                    return Err(format!("{}: docks set up without roads", setup.name()));
                }
                &docks.road_node
            }
            (ServiceParts::Taxi(..), NearestKind::Vehicles) => &self.vehicles[usize::from(service)],
            _ => {
                return Err(format!(
                    "{}: a {kind:?} query of the wrong kind of service",
                    setup.name()
                ));
            }
        };
        let cached = self.cache().reach.get(&from.raw()).cloned();
        let reach = if let Some(r) = cached {
            r
        } else {
            let mut found = Vec::new();
            self.world.with_roads(&mut |roads| found = roads.reach_from(from, f64::INFINITY));
            let r: Arc<HashMap<u32, f64>> =
                Arc::new(found.into_iter().map(|(n, s)| (n.raw(), s)).collect());
            self.cache().reach.insert(from.raw(), Arc::clone(&r));
            r
        };
        let mut out: Vec<(u32, f64)> = (0_u32..)
            .zip(nodes)
            .filter_map(|(i, n)| reach.get(&n.raw()).map(|&s| (i, s)))
            .filter(|p| p.1.is_finite())
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        out.truncate(k);
        Ok(out)
    }
}
