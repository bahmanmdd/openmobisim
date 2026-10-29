//! The roads and paths a fleet day uses, without a run: stored road times and the searches on
//! them.
//!
//! **Road times** ([`RoadTimes`]): per link, its mean traversal time in each time bin of a day,
//! where it has one, and its free-flow time elsewhere. Read from a run's per-link results
//! (`link_bins.parquet`: `pcu_seconds / pcu` per link and bin) or stored in a frozen day's
//! binary file. **An approximation, labelled as such:** those results are by the bin a vehicle
//! *left* the link, and a bin nobody left reads free flow; a run's own loading prices a link by
//! the bin a vehicle *enters* it, and carries a standing queue forward.
//!
//! **The searches** ([`FrozenRoads`]): a vehicle's drive, by the time-dependent fastest-route
//! search on the road times (turn rules kept); the bounded free-flow searches a matching uses
//! (the nodes within a few minutes of a pickup, to and from it). Only links that carry motor
//! traffic are driven. **A shared bike's rides and walks** ([`FrozenPaths`]): between docks on
//! the bike layer at the service's speed, from a dock to a destination on the walk layer, each
//! the static layer's shortest time.
//!
//! **Cost:** road times 8 bytes per (link, bin) row plus 8 per link, a lookup one binary
//! search over the link's few rows; a drive one bounded time-dependent search; a ride or walk
//! one static search, kept per pair.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Mutex;

use openmobisim_core_graph::binary::{self, Content, Reader, Writer, invalid};
use openmobisim_core_graph::layers::StaticNetwork;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{Reach, Search, SearchContext};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

use super::engine::ServiceRoads;

/// The longest drive a vehicle's search looks for, in seconds: a city crossed with room.
pub const DRIVE_BOUND_S: f64 = 4.0 * 3600.0;

/// Per link, its mean time by time bin where it has one; free flow elsewhere (see the
/// [module docs](self)).
#[derive(Clone, Debug, PartialEq)]
pub struct RoadTimes {
    bin_s: f64,
    free_flow: Vec<f64>,
    /// `link_count + 1` offsets into the rows; a link's rows by bin, ascending.
    start: Vec<u32>,
    bin: Vec<u32>,
    seconds: Vec<f64>,
}

impl RoadTimes {
    /// Free flow everywhere on `network`.
    #[must_use]
    pub fn free_flow(network: &RoadNetwork) -> Self {
        Self {
            bin_s: 900.0,
            free_flow: (0..network.link_count())
                .map(|i| network.free_flow_time(LinkId::new(i)).get())
                .collect(),
            start: vec![0; network.link_count() as usize + 1],
            bin: Vec::new(),
            seconds: Vec::new(),
        }
    }

    /// From rows `(link index, bin, mean seconds)` in bins of `bin_s` seconds counted from
    /// midnight; a row's time is never taken below the link's free-flow time, and a repeated
    /// (link, bin) keeps the last.
    ///
    /// # Errors
    ///
    /// A message for a link index out of `network`, a time that is not finite and positive, or
    /// a bin length not above 0.
    ///
    /// # Panics
    ///
    /// Never in practice: the rows of a day fit `u32`.
    pub fn from_rows(
        network: &RoadNetwork,
        bin_s: f64,
        rows: &[(u32, u32, f64)],
    ) -> Result<Self, String> {
        if !(bin_s.is_finite() && bin_s > 0.0) {
            return Err(format!("the bin length must be above 0 seconds, got {bin_s}"));
        }
        let mut out = Self { bin_s, ..Self::free_flow(network) };
        let links = network.link_count();
        let mut per: Vec<Vec<(u32, f64)>> = vec![Vec::new(); links as usize];
        for &(link, bin, s) in rows {
            if link >= links {
                return Err(format!("link index {link} is not of this network ({links} links)"));
            }
            if !(s.is_finite() && s > 0.0) {
                return Err(format!("link {link}, bin {bin}: a time of {s} s"));
            }
            per[link as usize].push((bin, s.max(out.free_flow[link as usize])));
        }
        for (l, mut list) in per.into_iter().enumerate() {
            list.sort_by_key(|r| r.0);
            // The last of a repeated (link, bin): kept by the reverse dedup's first.
            list.reverse();
            list.dedup_by_key(|r| r.0);
            list.reverse();
            out.bin.extend(list.iter().map(|r| r.0));
            out.seconds.extend(list.iter().map(|r| r.1));
            out.start[l + 1] = u32::try_from(out.bin.len()).expect("rows fit u32");
        }
        Ok(out)
    }

    /// The bin length, in seconds.
    #[must_use]
    pub fn bin_s(&self) -> f64 {
        self.bin_s
    }

    /// How many (link, bin) rows it holds.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.bin.len()
    }

    /// The time to cross `link` entering it at second `at` (from midnight): its bin's mean if
    /// it has one, else its free-flow time.
    #[must_use]
    pub fn link_seconds(&self, link: u32, at: f64) -> f64 {
        let l = link as usize;
        let (a, z) = (self.start[l] as usize, self.start[l + 1] as usize);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
        let wanted = (at.max(0.0) / self.bin_s) as u32;
        self.bin[a..z].binary_search(&wanted).map_or(self.free_flow[l], |k| self.seconds[a + k])
    }

    /// Bytes held.
    #[must_use]
    pub fn bytes(&self) -> usize {
        (self.free_flow.len() + self.seconds.len()) * 8 + (self.start.len() + self.bin.len()) * 4
    }

    /// Its binary form ([`openmobisim_core_graph::binary`], magic `OMSRTM`): [`Self::read_binary`]
    /// gives back a table equal to this one, bit for bit.
    #[must_use]
    pub fn write_binary(&self) -> Vec<u8> {
        let mut w = Writer::of(Content::RoadTimes);
        w.f64(self.bin_s);
        w.f64s(self.free_flow.iter().copied());
        w.u32s(self.start.iter().copied());
        w.u32s(self.bin.iter().copied());
        w.f64s(self.seconds.iter().copied());
        w.finish()
    }

    /// Read what [`Self::write_binary`] wrote.
    ///
    /// # Errors
    ///
    /// [`binary::FormatError`] for bytes that are not road times, or are damaged.
    pub fn read_binary(bytes: &[u8]) -> binary::Result<Self> {
        let mut r = Reader::of(bytes, Content::RoadTimes)?;
        let bin_s = r.f64()?;
        let free_flow = r.f64s()?;
        let start = r.u32s()?;
        let bin = r.u32s()?;
        let seconds = r.f64s()?;
        if start.len() != free_flow.len() + 1
            || bin.len() != seconds.len()
            || start.last().is_none_or(|&e| e as usize != bin.len())
        {
            return invalid("road times whose rows do not match their links");
        }
        Ok(Self { bin_s, free_flow, start, bin, seconds })
    }
}

/// A fleet day's roads: timed drives on [`RoadTimes`], bounded free-flow reaches (see the
/// [module docs](self)).
#[derive(Debug)]
pub struct FrozenRoads<'a> {
    road: &'a RoadNetwork,
    times: Cow<'a, RoadTimes>,
    /// Free-flow seconds of the links motor traffic may use, infinite on the others.
    free: Vec<f64>,
    every: Vec<bool>,
    search: Mutex<Search<'a>>,
}

impl<'a> FrozenRoads<'a> {
    /// The roads of `ctx`'s network at `times`.
    #[must_use]
    pub fn new(road: &'a RoadNetwork, ctx: &'a SearchContext<'a>, times: RoadTimes) -> Self {
        Self::with_times(road, ctx, Cow::Owned(times))
    }

    /// The roads of `ctx`'s network at `times`, borrowed or owned.
    #[must_use]
    pub fn with_times(
        road: &'a RoadNetwork,
        ctx: &'a SearchContext<'a>,
        times: Cow<'a, RoadTimes>,
    ) -> Self {
        let free = (0..road.link_count())
            .map(|i| {
                let link = LinkId::new(i);
                if road.link_class(link).carries_motor_traffic() {
                    road.free_flow_time(link).get()
                } else {
                    f64::INFINITY
                }
            })
            .collect();
        Self {
            road,
            times,
            free,
            every: vec![true; road.node_count() as usize],
            search: Mutex::new(Search::new(ctx)),
        }
    }

    /// The road times it drives on.
    #[must_use]
    pub fn times(&self) -> &RoadTimes {
        &self.times
    }

    /// The drive from `from` to `to` leaving at second `t`: its seconds and metres on the
    /// road times, infinite where there is no road within [`DRIVE_BOUND_S`].
    ///
    /// # Panics
    ///
    /// If an earlier search panicked while holding the search.
    #[must_use]
    pub fn drive(&self, from: NodeId, to: NodeId, t: f64) -> (f64, f64) {
        if from == to {
            return (0.0, 0.0);
        }
        let seconds = |l: u32, at: f64| {
            if self.free[l as usize].is_finite() {
                self.times.link_seconds(l, at)
            } else {
                f64::INFINITY
            }
        };
        self.search
            .lock()
            .expect("a search not poisoned")
            .fastest_route(from, to, t, DRIVE_BOUND_S, &|_, _| 0.0, &seconds)
            .map_or((f64::INFINITY, f64::INFINITY), |(s, links)| {
                (s, links.iter().map(|&l| self.road.link_length(l).get()).sum())
            })
    }

    /// The nodes from which `to` is reached within `bound` seconds at free flow, with those
    /// seconds, nearest first (ties by node id).
    #[must_use]
    pub fn reach_to(&self, to: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        self.reach(to, bound, true)
    }

    /// The nodes reached from `from` within `bound` seconds at free flow, with those seconds,
    /// nearest first (ties by node id).
    #[must_use]
    pub fn reach_from(&self, from: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        self.reach(from, bound, false)
    }

    fn reach(&self, node: NodeId, bound: f64, back: bool) -> Vec<(NodeId, f64)> {
        let mut reach = Reach::new(self.road, &self.free);
        let mut out = if back {
            reach.backward(node, bound, &self.every)
        } else {
            reach.forward(node, bound, &self.every)
        };
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.raw().cmp(&b.0.raw())));
        out
    }
}

impl ServiceRoads for FrozenRoads<'_> {
    fn drive(&self, from: NodeId, to: NodeId, t: f64) -> (f64, f64) {
        FrozenRoads::drive(self, from, to, t)
    }
    fn reach_to(&self, to: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        FrozenRoads::reach_to(self, to, bound)
    }
    fn reach_from(&self, from: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        FrozenRoads::reach_from(self, from, bound)
    }
}

/// A shared bike's rides and walks on the static layers: see the [module docs](self).
#[derive(Debug)]
pub struct FrozenPaths<'a> {
    bike: &'a StaticNetwork,
    bike_seconds: &'a [f64],
    walk: &'a StaticNetwork,
    walk_seconds: &'a [f64],
    rides: Mutex<HashMap<(u32, u32), f64>>,
}

impl<'a> FrozenPaths<'a> {
    /// Paths on the bike layer `bike` and the walk layer `walk`, their links taking
    /// `bike_seconds` and `walk_seconds`.
    #[must_use]
    pub fn new(
        bike: &'a StaticNetwork,
        bike_seconds: &'a [f64],
        walk: &'a StaticNetwork,
        walk_seconds: &'a [f64],
    ) -> Self {
        Self { bike, bike_seconds, walk, walk_seconds, rides: Mutex::new(HashMap::new()) }
    }

    fn shortest(network: &RoadNetwork, seconds: &[f64], from: NodeId, to: NodeId) -> f64 {
        if from == to {
            return 0.0;
        }
        let mut wanted = vec![false; network.node_count() as usize];
        wanted[to.index()] = true;
        Reach::new(network, seconds)
            .forward(from, f64::INFINITY, &wanted)
            .first()
            .map_or(f64::INFINITY, |&(_, s)| s)
    }

    /// The ride from bike-layer node `from` to `to`, in seconds of the layer (infinite with no
    /// path), kept per pair.
    ///
    /// # Panics
    ///
    /// If an earlier call panicked while holding the kept rides.
    #[must_use]
    pub fn ride_s(&self, from: NodeId, to: NodeId) -> f64 {
        let key = (from.raw(), to.raw());
        if let Some(&s) = self.rides.lock().expect("not poisoned").get(&key) {
            return s;
        }
        let s = Self::shortest(self.bike.network(), self.bike_seconds, from, to);
        self.rides.lock().expect("not poisoned").insert(key, s);
        s
    }

    /// The walk from walk-layer node `from` to `to`, in seconds (infinite with no path).
    #[must_use]
    pub fn walk_s(&self, from: NodeId, to: NodeId) -> f64 {
        Self::shortest(self.walk.network(), self.walk_seconds, from, to)
    }
}

/// The turn rules a drive keeps, for a [`SearchContext`]: the network's own.
#[must_use]
pub fn search_context<'a>(road: &'a RoadNetwork, turns: &'a TurnTable) -> SearchContext<'a> {
    SearchContext::new(road, turns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmobisim_core_graph::examples::manhattan_grid;

    #[test]
    fn a_row_s_bin_reads_its_mean_and_every_other_moment_free_flow() {
        let (road, _) = manhattan_grid(3, 100.0, false);
        let ff = road.free_flow_time(LinkId::new(0)).get();
        let t =
            RoadTimes::from_rows(&road, 900.0, &[(0, 30, ff * 3.0), (0, 31, ff * 2.0)]).unwrap();
        assert!((t.link_seconds(0, 30.0 * 900.0 + 1.0) - ff * 3.0).abs() < 1e-9);
        assert!((t.link_seconds(0, 31.0 * 900.0) - ff * 2.0).abs() < 1e-9, "a bin starts with it");
        assert!((t.link_seconds(0, 32.0 * 900.0) - ff).abs() < 1e-9, "no row: free flow");
        assert!(
            (t.link_seconds(1, 30.0 * 900.0) - road.free_flow_time(LinkId::new(1)).get()).abs()
                < 1e-9
        );
        let slow = RoadTimes::from_rows(&road, 900.0, &[(0, 1, ff / 2.0)]).unwrap();
        assert!((slow.link_seconds(0, 900.0) - ff).abs() < 1e-9, "never below free flow");
        assert!(RoadTimes::from_rows(&road, 900.0, &[(99_999, 1, 5.0)]).is_err());
        assert!(RoadTimes::from_rows(&road, 900.0, &[(0, 1, f64::NAN)]).is_err());
        let back = RoadTimes::read_binary(&t.write_binary()).unwrap();
        assert_eq!(back, t, "the binary form gives it back");
        let mut damaged = t.write_binary();
        let k = damaged.len() / 2;
        damaged[k] ^= 1;
        assert!(RoadTimes::read_binary(&damaged).is_err(), "a damaged file is refused");
    }
}
