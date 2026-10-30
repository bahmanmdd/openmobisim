//! What a service is expected to cost a traveller, fixed for the whole run
//! ([`super::ServiceExpectations`], S303): never learned from a loading.
//!
//! **A dock**, per dock and time bin ([`DockCosts`]): the extra walk expected at a pickup and at
//! a return, from the dock's start-of-day counts ([`DockCosts::initial`], a day with no legs):
//! for a pickup, the share of the bin the dock is empty times the walk to its nearest neighbour
//! that is empty for at most half of that bin (or [`DockDefaults::fail_s`](super::DockDefaults::fail_s)
//! if none); for a return, the same with "full" in place of "empty". The expectations then scale
//! them ([`DockCosts::expected`]): each side by its factor, or, in the worst case, every dock
//! empty and full all day (the walk to its nearest neighbour, `fail_s` with none).
//!
//! **A taxi** ([`TaxiExpectation`]): one expected wait and one expected share of requests with no
//! vehicle, the same everywhere and all day.
//!
//! **Times** are seconds from midnight, as the engine's clock; the docks' bins start at the
//! window's start ([`DockBins::start_s`]). A second before the window reads its first bin, one
//! after it its last.
//!
//! **Cost:** two `f64` per dock and bin; two per taxi fleet.

use std::sync::Arc;

use openmobisim_core_types::ids::NodeId;

use super::docks::DockBins;
use super::engine::{FleetDay, FleetEngine, NoLegs, NoRoads};
use super::{ServiceExpectations, ServiceParts, ServiceSetup};

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

    /// The start-of-day costs: those of a day with no legs over `window` (seconds from
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

    /// These costs as the run's travellers expect them under `e`: each side multiplied by its
    /// factor; with [`ServiceExpectations::dock_worst_case`], first every dock and bin set to
    /// the walk to the dock's nearest neighbour ([`DockDefaults::fail_s`](super::DockDefaults::fail_s)
    /// with none), at both ends.
    ///
    /// # Panics
    ///
    /// If `setup` is not the docked service these costs are of.
    #[must_use]
    pub fn expected(mut self, setup: &ServiceSetup, e: &ServiceExpectations) -> Self {
        let ServiceParts::BikeDocked(docks, defaults) = setup.parts() else {
            panic!("service `{}` has no docks", setup.name());
        };
        assert_eq!(self.pickup.len(), docks.len() * self.bins, "costs of this service's docks");
        if e.dock_worst() {
            for k in 0..docks.len() {
                let k32 = u32::try_from(k).expect("docks fit u32");
                let walk = docks.neighbours(k32).first().map_or(defaults.fail_s, |&(_, w)| w);
                for b in 0..self.bins {
                    self.pickup[k * self.bins + b] = walk;
                    self.ret[k * self.bins + b] = walk;
                }
            }
        }
        for v in &mut self.pickup {
            *v *= e.dock_pickup_factor;
        }
        for v in &mut self.ret {
            *v *= e.dock_return_factor;
        }
        self
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

/// What a taxi fleet is expected to cost a traveller (S303): the wait for a vehicle and the
/// share of requests with no vehicle within the fleet's longest wait, the same at every road
/// node and second ([`ServiceExpectations::taxi_wait_s`],
/// [`ServiceExpectations::taxi_fail_share`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiExpectation {
    /// The expected wait, in seconds, of a request that is served.
    pub wait_s: f64,
    /// The expected share of requests with no vehicle.
    pub fail_share: f64,
}

impl TaxiExpectation {
    /// The taxi part of `e`.
    #[must_use]
    pub fn of(e: &ServiceExpectations) -> Self {
        Self { wait_s: e.taxi_wait_s, fail_share: e.taxi_fail_share }
    }

    /// The expected wait, in seconds, of a request at road node `node` asked at second `t`, and
    /// the expected share of such requests with no vehicle: the same everywhere.
    #[must_use]
    pub fn at(&self, _node: NodeId, _t: f64) -> (f64, f64) {
        (self.wait_s, self.fail_share)
    }
}
