//! Cumulative vehicle curves: the aggregate half of S85's split (design
//! §10.3–10.4, S84's iterative LTM).
//!
//! One [`LinkCurves`] per link holds two monotone step functions — `N_up(t)`,
//! the cumulative PCU that have entered the link, and `N_dn(t)`, the
//! cumulative PCU that have left it — sampled at the loading step boundaries.
//! Sending and receiving flow (Yperman's link transmission model, the
//! mechanism behind Himpe/Corthout/Tampère 2016's iterative form, S84) read
//! these curves **shifted by the link's free-flow and wave travel time**, so
//! Δt never has to be small enough for a vehicle to stay on one link for a
//! whole step — the "no stability limit" property S84 records.
//!
//! Nothing here knows about individual vehicles; [`crate::ltm`] is what maps
//! vehicles onto positions on these curves (S85).

use openmobisim_core_graph::defaults::LinkParameters;
use openmobisim_core_types::units::{Duration, Flow, Metres, Pcu};

/// One link's cumulative curves, sampled at every loading step boundary.
///
/// `times[0] == Duration::ZERO`, `up[0] == dn[0] == Pcu::ZERO`: every curve
/// starts empty at the run's origin.
#[derive(Clone, Debug)]
pub struct LinkCurves {
    times: Vec<Duration>,
    up: Vec<Pcu>,
    dn: Vec<Pcu>,
}

impl LinkCurves {
    /// A link with nothing on it yet.
    #[must_use]
    pub fn new() -> Self {
        Self { times: vec![Duration::ZERO], up: vec![Pcu::ZERO], dn: vec![Pcu::ZERO] }
    }

    /// The most recently committed sample time.
    ///
    /// # Panics
    ///
    /// Never: a [`LinkCurves`] always has at least the origin sample.
    #[inline]
    #[must_use]
    pub fn now(&self) -> Duration {
        *self.times.last().expect("a LinkCurves always has at least the origin sample")
    }

    /// `N_up` at the most recently committed sample.
    ///
    /// # Panics
    ///
    /// Never: a [`LinkCurves`] always has at least the origin sample.
    #[inline]
    #[must_use]
    pub fn up_now(&self) -> Pcu {
        *self.up.last().expect("a LinkCurves always has at least the origin sample")
    }

    /// `N_dn` at the most recently committed sample.
    ///
    /// # Panics
    ///
    /// Never: a [`LinkCurves`] always has at least the origin sample.
    #[inline]
    #[must_use]
    pub fn dn_now(&self) -> Pcu {
        *self.dn.last().expect("a LinkCurves always has at least the origin sample")
    }

    /// `N_up(t)`, linearly interpolated between committed samples.
    ///
    /// Flat before the origin (nothing has happened yet) and flat after the
    /// last committed sample (the curve is not known beyond "now" — callers
    /// only ever query the past or present, never the future).
    #[must_use]
    pub fn up_at(&self, t: Duration) -> Pcu {
        interpolate(&self.times, &self.up, t)
    }

    /// `N_dn(t)`, linearly interpolated between committed samples.
    #[must_use]
    pub fn dn_at(&self, t: Duration) -> Pcu {
        interpolate(&self.times, &self.dn, t)
    }

    /// Sending flow for the step `[now, now + dt)`: how much this link could
    /// discharge, bounded by what free-flow travel has already delivered to
    /// its downstream end and by capacity.
    ///
    /// `S(t) = min( (N_up(t − τ_f) − N_dn(t)) / dt, capacity )`
    #[must_use]
    pub fn sending_flow(&self, dt: Duration, params: LinkParameters, length: Metres) -> Flow {
        let tau_f = length / params.free_flow_speed;
        let sendable = (self.up_at(self.now() - tau_f) - self.dn_now()).non_negative();
        (sendable / dt).min(params.capacity)
    }

    /// Receiving flow for the step `[now, now + dt)`: how much this link
    /// could accept, bounded by the storage a backward-moving wave has
    /// already freed and by capacity.
    ///
    /// `R(t) = min( capacity, (N_dn(t − τ_w) + storage − N_up(t)) / dt )`
    #[must_use]
    pub fn receiving_flow(
        &self,
        dt: Duration,
        params: LinkParameters,
        length: Metres,
        storage: Pcu,
    ) -> Flow {
        let tau_w = length / params.wave_speed;
        let space = (self.dn_at(self.now() - tau_w) + storage - self.up_now()).non_negative();
        (space / dt).min(params.capacity)
    }

    /// Commit the result of a step: `entered`/`exited` PCU add to `N_up`/
    /// `N_dn`, and a new sample lands at `now() + dt`.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if either curve would stop being
    /// non-decreasing — the one invariant every caller must preserve.
    pub fn commit(&mut self, dt: Duration, entered: Pcu, exited: Pcu) {
        debug_assert!(entered.get() >= 0.0, "entered PCU must be non-negative, got {entered:?}");
        debug_assert!(exited.get() >= 0.0, "exited PCU must be non-negative, got {exited:?}");
        let up = self.up_now() + entered;
        let dn = self.dn_now() + exited;
        debug_assert!(
            dn.get() <= up.get() + 1e-6,
            "a link cannot discharge more than it has received: dn={dn:?} up={up:?}"
        );
        self.times.push(self.now() + dt);
        self.up.push(up);
        self.dn.push(dn);
    }

    /// Add PCU to the current `N_up` sample without advancing time — a
    /// vehicle departing mid-curve (its first link) or arriving on a link by
    /// hand-over within a step, rather than through a committed step.
    ///
    /// Safe because [`Self::up_at`]/[`Self::sending_flow`] only ever look
    /// *backward* from `now()`: a value bumped in place at `now()` cannot
    /// retroactively change a flow already computed for an earlier step.
    ///
    /// # Panics
    ///
    /// Never: a [`LinkCurves`] always has at least the origin sample.
    pub fn inject_up(&mut self, pcu: Pcu) {
        *self.up.last_mut().expect("a LinkCurves always has at least the origin sample") += pcu;
    }
}

impl Default for LinkCurves {
    fn default() -> Self {
        Self::new()
    }
}

/// Linear interpolation of a monotone step-sampled curve, flat outside the
/// sampled range.
fn interpolate(times: &[Duration], values: &[Pcu], t: Duration) -> Pcu {
    let last = times.len() - 1;
    if t <= times[0] {
        return values[0];
    }
    if t >= times[last] {
        return values[last];
    }
    let idx = times.partition_point(|&x| x <= t).saturating_sub(1).min(last - 1);
    let (t0, t1) = (times[idx], times[idx + 1]);
    let (v0, v1) = (values[idx], values[idx + 1]);
    let span = (t1 - t0).get();
    let frac = if span > 0.0 { (t - t0).get() / span } else { 0.0 };
    v0 + (v1 - v0) * frac
}

#[cfg(test)]
mod tests {
    use openmobisim_core_types::units::{Density, Duration, Speed};

    use super::*;

    /// A plausible triangular diagram: 72 km/h free flow, 1800 veh/h
    /// capacity, 18 km/h backward wave speed, no control delay.
    fn diagram() -> LinkParameters {
        LinkParameters {
            free_flow_speed: Speed::from_km_per_hour(72.0),
            capacity: Flow::from_veh_per_hour(1800.0),
            jam_density: Density::from_veh_per_km(130.0),
            wave_speed: Speed::from_km_per_hour(18.0),
            control_delay: Duration::ZERO,
        }
    }

    #[test]
    fn a_fresh_curve_is_zero_everywhere() {
        let c = LinkCurves::new();
        assert_eq!(c.up_at(Duration(-10.0)), Pcu::ZERO);
        assert_eq!(c.up_at(Duration::ZERO), Pcu::ZERO);
        assert_eq!(c.up_at(Duration(1000.0)), Pcu::ZERO);
    }

    #[test]
    fn interpolation_is_linear_between_samples() {
        let mut c = LinkCurves::new();
        c.commit(Duration(10.0), Pcu(20.0), Pcu(0.0));
        assert_eq!(c.up_at(Duration(5.0)), Pcu(10.0));
        assert_eq!(c.up_at(Duration(10.0)), Pcu(20.0));
        // Beyond the last sample: flat, not extrapolated.
        assert_eq!(c.up_at(Duration(50.0)), Pcu(20.0));
    }

    #[test]
    fn commit_is_cumulative_and_monotone() {
        let mut c = LinkCurves::new();
        c.commit(Duration(10.0), Pcu(5.0), Pcu(1.0));
        c.commit(Duration(10.0), Pcu(3.0), Pcu(4.0));
        assert_eq!(c.up_now(), Pcu(8.0));
        assert_eq!(c.dn_now(), Pcu(5.0));
        assert_eq!(c.now(), Duration(20.0));
    }

    /// S76's first exact assertion: **level 4 with storage → ∞ reproduces
    /// level 2** (design §10.1's fidelity table: "point queue ... LTM with
    /// infinite storage"). A jammed link (`up_now` far exceeds any
    /// realistic storage) has zero remaining receiving flow under a real,
    /// finite storage — but exactly `capacity` under infinite storage: with
    /// nothing left to bound `space`, only the link's own discharge
    /// capacity limits it, precisely the point-queue definition — "flow
    /// capacity + vertical queue at exit, loses queue length / physical
    /// extent".
    #[test]
    fn infinite_storage_reproduces_the_point_queue_exactly() {
        let params = diagram();
        let mut c = LinkCurves::new();
        // 1000 PCU already on a link with 50 PCU of real storage: fully jammed.
        c.commit(Duration(10.0), Pcu(1000.0), Pcu::ZERO);
        let dt = Duration(10.0);
        let length = Metres(200.0);

        let finite = c.receiving_flow(dt, params, length, Pcu(50.0));
        assert_eq!(finite, Flow::ZERO, "a jammed link with real storage has no room left");

        let infinite = c.receiving_flow(dt, params, length, Pcu::INFINITE);
        assert_eq!(
            infinite, params.capacity,
            "with no storage bound, only the link's own capacity limits it — the point-queue \
             definition, exactly"
        );
    }

    /// S76's second exact assertion: **level 4 with backward wave speed →
    /// ∞ reproduces level 3** (design §10.1: "+ storage capacity →
    /// spillback ... LTM with infinite backward wave speed", "loses
    /// discharge timing accuracy"). A real, finite wave speed reports
    /// receiving flow computed from a *stale* downstream count — the
    /// physical lag a shockwave takes to travel back up the link; infinite
    /// wave speed collapses that lag to zero, reading the *current* count
    /// instead. The two must differ by exactly the PCU the downstream
    /// discharged during the lag window — no more, no less.
    #[test]
    fn infinite_wave_speed_reproduces_the_spatial_queue_exactly() {
        // A high capacity, deliberately: this test isolates the wave-speed
        // term, so capacity must not be the binding constraint on either side.
        let params = LinkParameters { capacity: Flow::from_veh_per_hour(200_000.0), ..diagram() };
        // 100 m at 18 km/h = 5 m/s -> tau_w = 20 s exactly.
        let length = Metres(100.0);
        let mut c = LinkCurves::new();
        c.commit(Duration(10.0), Pcu(20.0), Pcu::ZERO); // now = 10, dn = 0
        c.commit(Duration(10.0), Pcu(0.0), Pcu(20.0)); // now = 20, dn = 20: a burst discharges
        c.commit(Duration(10.0), Pcu(0.0), Pcu(0.0)); // now = 30, dn stays 20
        assert_eq!(c.now(), Duration(30.0));
        let storage = Pcu(100.0);
        let dt = Duration(10.0);

        let finite = c.receiving_flow(dt, params, length, storage);
        let infinite_wave = LinkParameters { wave_speed: Speed::INFINITE, ..params };
        let infinite = c.receiving_flow(dt, infinite_wave, length, storage);

        // Finite: reads dn(30 - 20) = dn(10) = 0 -> space = 0 + 100 - up_now(20) = 80.
        // Infinite: reads dn(30 - 0) = dn(30) = 20 -> space = 20 + 100 - 20 = 100.
        // The 20 PCU difference is exactly the burst the finite wave hasn't "heard about" yet.
        let expected_finite = (Pcu(80.0) / dt).min(params.capacity);
        let expected_infinite = (Pcu(100.0) / dt).min(params.capacity);
        assert_eq!(finite, expected_finite);
        assert_eq!(infinite, expected_infinite);
        assert!(
            (infinite - finite).get() > 0.0,
            "the infinite-wave-speed reading must be strictly ahead of the lagged one"
        );
    }
}
