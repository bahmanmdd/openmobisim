//! What a dock is expected to cost a traveller, per time bin, as the fleet engine's days left
//! it.
//!
//! **Per dock and bin**, for a pickup: the share of the bin the dock was empty times the walk to
//! its nearest neighbour that was empty for at most half of that bin (or
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
