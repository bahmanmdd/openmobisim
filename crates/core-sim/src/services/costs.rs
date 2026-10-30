//! What a dock is expected to cost a traveller, per time bin, as the fleet engine's days left
//! it.
//!
//! **Per dock and bin**, what a day's legs met there: the extra seconds of the pickups planned at
//! the dock in the bin, and of the returns, each the mean over those legs weighted by the people
//! they stand for ([`DockCosts::with_legs`]). **Where no leg went**, an estimate from the
//! dock's counts ([`DockCosts::from_bins`]): for a pickup, the share of the bin the dock was
//! empty times the walk to its nearest neighbour that was empty for at most half of that bin (or
//! [`DockDefaults::fail_s`](super::DockDefaults::fail_s) if none); for a return, the same with
//! "full" in place of "empty". **Choice reads the expected cost**, the average over the
//! loadings so far (the method of successive averages: each loading's realised costs averaged
//! in with weight `1 / (loadings seen + 1)`); at the start, the costs of a day with no legs, from
//! the start-of-day counts alone. **A traveller pays the realised one**, on the engine.
//!
//! **Times** are seconds from midnight, as the engine's clock; the bins start at the window's
//! start ([`DockBins::start_s`]). A second before the window reads its first bin, one after it
//! its last.
//!
//! **Cost:** two `f64` per dock and bin.

use std::sync::Arc;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::Reach;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

#[cfg(doc)]
use super::TaxiDefaults;
use super::docks::DockBins;
use super::engine::{FleetDay, FleetEngine, NoLegs, NoRoads};
use super::{ServiceParts, ServiceSetup};

/// What a docked service's docks cost per time bin: per dock and bin, the expected extra
/// seconds of a pickup and of a return. See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct DockCosts {
    start_s: f64,
    bin_s: f64,
    bins: usize,
    /// `dock * bins + bin`.
    pickup: Vec<f64>,
    ret: Vec<f64>,
}

impl DockCosts {
    /// The costs a day's bins imply.
    ///
    /// # Panics
    ///
    /// If `setup` is not a docked service, or `bins` were not kept for its docks.
    #[must_use]
    pub fn from_bins(setup: &ServiceSetup, bins: &DockBins) -> Self {
        let ServiceParts::BikeDocked(docks, defaults) = setup.parts() else {
            panic!("service `{}` has no docks", setup.name());
        };
        let n = docks.len();
        let nb = bins.bins;
        assert_eq!(bins.empty_s.len(), n * nb, "bins for service `{}`'s docks", setup.name());
        let mut pickup = vec![0.0; n * nb];
        let mut ret = vec![0.0; n * nb];
        let fail = defaults.fail_s;
        for k in 0..n {
            let k32 = u32::try_from(k).expect("docks fit u32");
            for b in 0..nb {
                let len = bins.bin_len(b);
                let cell = k * nb + b;
                let empty = bins.empty_s[cell] / len;
                if empty > 0.0 {
                    let walk = docks
                        .neighbours(k32)
                        .iter()
                        .find(|&&(o, _)| bins.empty_s[o as usize * nb + b] / len <= 0.5)
                        .map_or(fail, |&(_, w)| w);
                    pickup[cell] = empty * walk;
                }
                let full = bins.full_s[cell] / len;
                if full > 0.0 {
                    let walk = docks
                        .neighbours(k32)
                        .iter()
                        .find(|&&(o, _)| bins.full_s[o as usize * nb + b] / len <= 0.5)
                        .map_or(fail, |&(_, w)| w);
                    ret[cell] = full * walk;
                }
            }
        }
        Self { start_s: bins.start_s, bin_s: f64::from(bins.bin_s), bins: nb, pickup, ret }
    }

    /// The realised costs of a day whose legs met `pickups` and `returns` (each: dock, second
    /// from midnight, extra seconds met, people): in a dock and bin some leg met, the mean of
    /// what those legs met, weighted by people; elsewhere the cost `self` says (the bins').
    #[must_use]
    pub fn with_legs(
        mut self,
        pickups: &[(u32, f64, f64, f64)],
        returns: &[(u32, f64, f64, f64)],
    ) -> Self {
        for (side, met) in [(0, pickups), (1, returns)] {
            let mut sum = vec![0.0; self.pickup.len()];
            let mut people = vec![0.0; self.pickup.len()];
            for &(dock, t, extra, w) in met {
                let c = self.cell(dock, t);
                sum[c] += w * extra;
                people[c] += w;
            }
            let v = if side == 0 { &mut self.pickup } else { &mut self.ret };
            for ((v, s), p) in v.iter_mut().zip(&sum).zip(&people) {
                if *p > 0.0 {
                    *v = s / p;
                }
            }
        }
        self
    }

    /// Before any loading: the costs of a day with no legs over `window` (seconds from
    /// midnight), from the start-of-day counts alone (a dock empty or full at the start stays
    /// so).
    ///
    /// # Panics
    ///
    /// If `setup` is not a docked service.
    #[must_use]
    pub fn initial(setup: &ServiceSetup, window: (f64, f64)) -> Self {
        let day = FleetDay { window, legs: Vec::new(), seed: 0 };
        let out = FleetEngine::new(
            Arc::new(vec![setup.clone()]),
            Arc::new(day),
            Arc::new(NoLegs),
            Arc::new(NoRoads),
            None,
        )
        .run_day()
        .expect("no policy, no policy failure");
        let bins = out.docks[0]
            .as_ref()
            .unwrap_or_else(|| panic!("service `{}` has no docks", setup.name()));
        Self::from_bins(setup, bins)
    }

    /// Average in a later loading's realised costs: the method of successive averages over
    /// `seen + 1` loadings.
    pub fn absorb(&mut self, realised: &DockCosts, seen: u32) {
        let w = 1.0 / f64::from(seen + 1);
        for (v, r) in self.pickup.iter_mut().zip(&realised.pickup) {
            *v += w * (r - *v);
        }
        for (v, r) in self.ret.iter_mut().zip(&realised.ret) {
            *v += w * (r - *v);
        }
    }

    fn cell(&self, dock: u32, t: f64) -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
        let b = (((t - self.start_s).max(0.0) / self.bin_s) as usize).min(self.bins - 1);
        dock as usize * self.bins + b
    }

    /// The expected extra seconds of taking a bike at `dock` at second `t` (from midnight).
    #[must_use]
    pub fn pickup_s(&self, dock: u32, t: f64) -> f64 {
        self.pickup[self.cell(dock, t)]
    }

    /// The expected extra seconds of returning a bike to `dock` at second `t` (from midnight).
    #[must_use]
    pub fn return_s(&self, dock: u32, t: f64) -> f64 {
        self.ret[self.cell(dock, t)]
    }
}

/// What a taxi fleet is expected to cost a traveller, per zone and time bin (S302): the wait for
/// a vehicle of a request that is served, and the share of requests that fail (no vehicle sent
/// within the fleet's longest wait, [`TaxiDefaults::max_wait_s`]).
///
/// **Per zone and bin**, what a day's requests met there (by the zone of their pickup and the
/// bin of their request): the mean wait of the served ones and the share that failed, each
/// weighted by people ([`TaxiCosts::with_requests`]). **Where no request went**, an estimate from
/// the start of the day ([`TaxiCosts::initial`]): per zone, over its road nodes, the free-flow
/// drive from the nearest vehicle where it stands at the start (the mean over the nodes some
/// vehicle reaches within the longest wait), and the share of its nodes none reaches. The
/// estimate ignores that vehicles move and get busy; it only starts the averaging, and stays
/// where no traveller asked. **Choice reads the expected cost**, the average over the loadings
/// so far (the method of successive averages, [`TaxiCosts::absorb`]); a traveller pays the
/// realised one, on the engine.
///
/// **Zones** are the engine's ([`super::queries::Zones`]): squares of
/// [`super::queries::ZONE_GRID_M`] metres over the road network's nodes. **Bins** start at the
/// window's start, [`super::taxis::FLEET_BIN_S`] seconds long; a second before the window reads
/// its first bin, one after it its last.
///
/// **Cost:** two `f64` per zone and bin; at the start one bounded free-flow search per distinct
/// node a vehicle stands at.
#[derive(Clone, Debug, PartialEq)]
pub struct TaxiCosts {
    start_s: f64,
    bin_s: f64,
    bins: usize,
    /// Per road node, its zone.
    node_zone: Arc<Vec<u32>>,
    /// `zone * bins + bin`.
    wait: Vec<f64>,
    fail: Vec<f64>,
}

impl TaxiCosts {
    /// The start-of-day estimate for the taxi fleet `setup` over `window` (seconds from
    /// midnight), on `road` at free flow (the links motor traffic may use), with `zones` over its
    /// nodes (`node_zone`: per road node, its zone; `zones`: how many).
    ///
    /// # Panics
    ///
    /// If `setup` is not a taxi fleet.
    #[must_use]
    pub fn initial(
        setup: &ServiceSetup,
        road: &RoadNetwork,
        (node_zone, zones): (Arc<Vec<u32>>, usize),
        window: (f64, f64),
    ) -> Self {
        let ServiceParts::Taxi(fleet, d) = setup.parts() else {
            panic!("service `{}` is not a taxi fleet", setup.name());
        };
        let free: Vec<f64> = (0..road.link_count())
            .map(|i| {
                let link = LinkId::new(i);
                if road.link_class(link).carries_motor_traffic() {
                    road.free_flow_time(link).get()
                } else {
                    f64::INFINITY
                }
            })
            .collect();
        let every = vec![true; road.node_count() as usize];
        let mut nearest = vec![f64::INFINITY; road.node_count() as usize];
        let mut starts: Vec<NodeId> = fleet.vehicle_node.clone();
        starts.sort_unstable_by_key(|n| n.raw());
        starts.dedup();
        let mut reach = Reach::new(road, &free);
        for from in starts {
            nearest[from.index()] = 0.0;
            for (node, s) in reach.forward(from, d.max_wait_s, &every) {
                let n = &mut nearest[node.index()];
                *n = n.min(s);
            }
        }
        let (mut sum, mut reached, mut nodes) =
            (vec![0.0; zones], vec![0.0_f64; zones], vec![0.0_f64; zones]);
        for (node, &z) in node_zone.iter().enumerate() {
            if z == u32::MAX {
                continue;
            }
            let z = z as usize;
            nodes[z] += 1.0;
            if nearest[node] <= d.max_wait_s {
                sum[z] += nearest[node];
                reached[z] += 1.0;
            }
        }
        let window_s = (window.1 - window.0).max(0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin count")]
        let bins = ((window_s / f64::from(super::taxis::FLEET_BIN_S)).ceil() as usize).max(1);
        let (mut wait, mut fail) = (vec![0.0; zones * bins], vec![0.0; zones * bins]);
        for z in 0..zones {
            let (w, f) = if nodes[z] > 0.0 && reached[z] > 0.0 {
                (sum[z] / reached[z], 1.0 - reached[z] / nodes[z])
            } else {
                (d.max_wait_s, 1.0)
            };
            for b in 0..bins {
                wait[z * bins + b] = w;
                fail[z * bins + b] = f;
            }
        }
        Self {
            start_s: window.0,
            bin_s: f64::from(super::taxis::FLEET_BIN_S),
            bins,
            node_zone,
            wait,
            fail,
        }
    }

    /// The realised costs of a day whose requests met `met` (each: pickup road node, second of
    /// the request, seconds waited if served (`None`: failed), people): in a zone and bin some
    /// request asked from, the mean wait of the served ones (where one was) and the share that
    /// failed, weighted by people; elsewhere what `self` says.
    #[must_use]
    pub fn with_requests(mut self, met: &[(NodeId, f64, Option<f64>, f64)]) -> Self {
        let n = self.wait.len();
        let (mut waited, mut served, mut people, mut failed) =
            (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for &(node, t, wait, w) in met {
            let Some(c) = self.cell(node, t) else { continue };
            people[c] += w;
            match wait {
                Some(s) => {
                    waited[c] += w * s;
                    served[c] += w;
                }
                None => failed[c] += w,
            }
        }
        for c in 0..n {
            if served[c] > 0.0 {
                self.wait[c] = waited[c] / served[c];
            }
            if people[c] > 0.0 {
                self.fail[c] = failed[c] / people[c];
            }
        }
        self
    }

    /// Average in a later loading's realised costs: the method of successive averages over
    /// `seen + 1` loadings.
    pub fn absorb(&mut self, realised: &TaxiCosts, seen: u32) {
        let w = 1.0 / f64::from(seen + 1);
        for (v, r) in self.wait.iter_mut().zip(&realised.wait) {
            *v += w * (r - *v);
        }
        for (v, r) in self.fail.iter_mut().zip(&realised.fail) {
            *v += w * (r - *v);
        }
    }

    fn cell(&self, node: NodeId, t: f64) -> Option<usize> {
        let z = *self.node_zone.get(node.index())?;
        if z == u32::MAX {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
        let b = (((t - self.start_s).max(0.0) / self.bin_s) as usize).min(self.bins - 1);
        Some(z as usize * self.bins + b)
    }

    /// The expected wait, in seconds, of a request served at road node `node` asked at second
    /// `t`, and the expected share of such requests that fail; `None` for a node of no zone.
    #[must_use]
    pub fn at(&self, node: NodeId, t: f64) -> Option<(f64, f64)> {
        self.cell(node, t).map(|c| (self.wait[c], self.fail[c]))
    }
}
