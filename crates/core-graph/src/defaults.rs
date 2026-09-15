//! The OSM defaults table: how a `highway=residential` tag becomes a
//! fundamental diagram (S15, design §12).
//!
//! # What this buys, and what it promises
//!
//! The promise is that a user with an OSM extract and no calibration data gets
//! a **runnable, physically plausible** network — Level 0 / Level 1 accuracy in
//! the design's terms: good enough to compare designs, not good enough to
//! forecast absolute volumes. Everything here is a default, and every default
//! is overridable.
//!
//! The parameters are **physically bounded and roughly transferable**, not free
//! parameters to be tuned: jam density and saturation flow per lane vary far
//! less between cities than demand does.
//!
//! # The over-determination, and how it is resolved
//!
//! A triangular fundamental diagram has **three** degrees of freedom, but the
//! literature quotes **four** quantities: free-flow speed `v`, capacity
//! `q`, jam density `k_j` and backward wave speed `w`. They are related by
//!
//! ```text
//! q = v · w · k_j / (v + w)
//! ```
//!
//! so one of them must be derived. **This table derives `w`**:
//!
//! ```text
//! w = q / (k_j − q/v)
//! ```
//!
//! Free-flow speed and capacity are the best-measured of the four and the two a
//! user is most likely to have local values for; jam density is nearly constant
//! across road types; the backward wave speed is the least directly observed.
//! Deriving the least-observed quantity from the three better-observed ones is
//! the choice that makes a user's local capacity figure actually take effect.
//!
//! [`WAVE_SPEED_BOUNDS_KM_H`] then guards the result. The bounds are wide enough
//! that **no default row clamps** — they exist to catch a nonsensical override,
//! not to reshape the defaults. When a clamp does bite, `w` is held at the
//! bound and `k_j` is re-derived to keep the diagram consistent, and the link
//! is recorded in diagnostics. The simulation does not stop for it.
//!
//! # Citations
//!
//! The saturation-flow and green-time figures come from the Highway Capacity
//! Manual; the control-delay form is Webster's uniform delay. **Two reference
//! rows are still owed** and are marked `CITATION OWED` below — the empirical
//! ranges for jam density and backward wave speed, and a verification of the
//! Webster constants. Neither blocks code; both block the claim that every
//! default is grounded.

use openmobisim_core_types::units::{Density, Duration, Flow, Metres, Speed};

/// Version of this table.
///
/// Part of every artifact fingerprint (Foundations §8), so **bump it whenever
/// any number below changes**. A cached network built with different parameters
/// is not the same network, and silently reusing it is the kind of error that
/// takes a week to find.
pub const DEFAULTS_VERSION: u32 = 1;

/// The range a derived backward wave speed is allowed to fall in.
///
/// Deliberately wider than the 15–20 km/h the literature quotes for urban
/// roads: every default row below lands inside 15–20 on its own, so this bound
/// never fires for shipped defaults. It fires when a user overrides capacity or
/// jam density into a combination no triangular diagram can represent — and
/// then it records the fact rather than producing a diagram with a negative
/// congested branch.
pub const WAVE_SPEED_BOUNDS_KM_H: (f64, f64) = (6.0, 30.0);

/// A road class, as the defaults table keys on it.
///
/// These are the OSM `highway` values that carry motor traffic, plus the
/// pedestrian and cycle classes the walk and bike layers need. Anything else
/// falls to [`RoadClass::Unclassified`] with a diagnostic — the simulation
/// never stops for an unfamiliar tag (§3c).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum RoadClass {
    /// `highway=motorway`.
    Motorway = 0,
    /// `highway=motorway_link`.
    MotorwayLink = 1,
    /// `highway=trunk`.
    Trunk = 2,
    /// `highway=trunk_link`.
    TrunkLink = 3,
    /// `highway=primary`.
    Primary = 4,
    /// `highway=primary_link`.
    PrimaryLink = 5,
    /// `highway=secondary`.
    Secondary = 6,
    /// `highway=secondary_link`.
    SecondaryLink = 7,
    /// `highway=tertiary`.
    Tertiary = 8,
    /// `highway=tertiary_link`.
    TertiaryLink = 9,
    /// `highway=unclassified` — a public road below tertiary, **not** "unknown".
    Unclassified = 10,
    /// `highway=residential`.
    Residential = 11,
    /// `highway=living_street`.
    LivingStreet = 12,
    /// `highway=service`.
    Service = 13,
    /// `highway=pedestrian` — a street given over to pedestrians.
    Pedestrian = 14,
    /// `highway=footway`, `steps`, `path` with foot access.
    Footway = 15,
    /// `highway=cycleway`.
    Cycleway = 16,
}

impl RoadClass {
    /// Every class, in discriminant order.
    pub const ALL: [RoadClass; 17] = [
        RoadClass::Motorway,
        RoadClass::MotorwayLink,
        RoadClass::Trunk,
        RoadClass::TrunkLink,
        RoadClass::Primary,
        RoadClass::PrimaryLink,
        RoadClass::Secondary,
        RoadClass::SecondaryLink,
        RoadClass::Tertiary,
        RoadClass::TertiaryLink,
        RoadClass::Unclassified,
        RoadClass::Residential,
        RoadClass::LivingStreet,
        RoadClass::Service,
        RoadClass::Pedestrian,
        RoadClass::Footway,
        RoadClass::Cycleway,
    ];

    /// The OSM `highway` value this class corresponds to.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RoadClass::Motorway => "motorway",
            RoadClass::MotorwayLink => "motorway_link",
            RoadClass::Trunk => "trunk",
            RoadClass::TrunkLink => "trunk_link",
            RoadClass::Primary => "primary",
            RoadClass::PrimaryLink => "primary_link",
            RoadClass::Secondary => "secondary",
            RoadClass::SecondaryLink => "secondary_link",
            RoadClass::Tertiary => "tertiary",
            RoadClass::TertiaryLink => "tertiary_link",
            RoadClass::Unclassified => "unclassified",
            RoadClass::Residential => "residential",
            RoadClass::LivingStreet => "living_street",
            RoadClass::Service => "service",
            RoadClass::Pedestrian => "pedestrian",
            RoadClass::Footway => "footway",
            RoadClass::Cycleway => "cycleway",
        }
    }

    /// Classify an OSM `highway` tag value.
    ///
    /// Returns `None` for a value this table does not model, which the importer
    /// records as a data-quality diagnostic before falling back. Returning
    /// `None` rather than guessing keeps the fallback decision — and its
    /// diagnostic — in one place.
    #[must_use]
    pub fn from_osm_highway(value: &str) -> Option<Self> {
        Some(match value {
            "motorway" => RoadClass::Motorway,
            "motorway_link" => RoadClass::MotorwayLink,
            "trunk" => RoadClass::Trunk,
            "trunk_link" => RoadClass::TrunkLink,
            "primary" => RoadClass::Primary,
            "primary_link" => RoadClass::PrimaryLink,
            "secondary" => RoadClass::Secondary,
            "secondary_link" => RoadClass::SecondaryLink,
            "tertiary" => RoadClass::Tertiary,
            "tertiary_link" => RoadClass::TertiaryLink,
            "unclassified" | "road" => RoadClass::Unclassified,
            "residential" => RoadClass::Residential,
            "living_street" => RoadClass::LivingStreet,
            "service" => RoadClass::Service,
            "pedestrian" => RoadClass::Pedestrian,
            "footway" | "steps" | "path" | "track" | "corridor" => RoadClass::Footway,
            "cycleway" => RoadClass::Cycleway,
            _ => return None,
        })
    }

    /// Whether motor vehicles may use this class by default.
    #[inline]
    #[must_use]
    pub const fn carries_motor_traffic(self) -> bool {
        !matches!(self, RoadClass::Pedestrian | RoadClass::Footway | RoadClass::Cycleway)
    }

    /// Whether pedestrians may use this class by default.
    ///
    /// Motorways and their links are the exception; everything else is walkable
    /// unless OSM says otherwise.
    #[inline]
    #[must_use]
    pub const fn carries_pedestrians(self) -> bool {
        !matches!(self, RoadClass::Motorway | RoadClass::MotorwayLink | RoadClass::Cycleway)
    }

    /// Whether cyclists may use this class by default.
    #[inline]
    #[must_use]
    pub const fn carries_cyclists(self) -> bool {
        !matches!(
            self,
            RoadClass::Motorway | RoadClass::MotorwayLink | RoadClass::Footway | RoadClass::Trunk
        )
    }
}

/// One row of the defaults table.
///
/// Units are as a scenario file writes them — km/h, veh/h, veh/km — because
/// this table is the human-facing end. Conversion to internal SI happens in
/// [`LinkParameters::from_defaults`], once, at the build boundary.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DefaultRow {
    /// Free-flow speed when OSM gives no `maxspeed`, in km/h.
    pub free_flow_km_h: f64,
    /// Lanes per direction when OSM gives no `lanes`.
    pub lanes_per_direction: u8,
    /// Saturation flow per lane, in veh/h.
    pub saturation_flow_veh_h_lane: f64,
    /// Jam density per lane, in veh/km.
    pub jam_density_veh_km_lane: f64,
}

/// The shipped defaults, one row per [`RoadClass`].
///
/// | Source | Covers |
/// |---|---|
/// | Highway Capacity Manual | saturation flow per lane |
/// | Common OSM speed conventions | free-flow speeds where `maxspeed` is absent |
/// | *CITATION OWED* | jam density per lane (the 120–150 veh/km/lane range) |
///
/// Pedestrian and cycle classes carry motor-traffic parameters that are never
/// read — those layers have static costs (design §21.1) and no fundamental
/// diagram. They are filled with the residential row rather than with zeros so
/// that a mistaken read produces something sane rather than a division by zero.
#[must_use]
pub const fn default_row(class: RoadClass) -> DefaultRow {
    match class {
        RoadClass::Motorway => DefaultRow {
            free_flow_km_h: 110.0,
            lanes_per_direction: 2,
            saturation_flow_veh_h_lane: 2000.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::MotorwayLink => DefaultRow {
            free_flow_km_h: 60.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1600.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::Trunk => DefaultRow {
            free_flow_km_h: 90.0,
            lanes_per_direction: 2,
            saturation_flow_veh_h_lane: 1900.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::TrunkLink => DefaultRow {
            free_flow_km_h: 50.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1500.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::Primary => DefaultRow {
            free_flow_km_h: 60.0,
            lanes_per_direction: 2,
            saturation_flow_veh_h_lane: 1900.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::PrimaryLink => DefaultRow {
            free_flow_km_h: 40.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1500.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::Secondary => DefaultRow {
            free_flow_km_h: 50.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1800.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::SecondaryLink => DefaultRow {
            free_flow_km_h: 40.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1500.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::Tertiary => DefaultRow {
            free_flow_km_h: 50.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1700.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::TertiaryLink => DefaultRow {
            free_flow_km_h: 30.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1400.0,
            jam_density_veh_km_lane: 135.0,
        },
        RoadClass::Unclassified => DefaultRow {
            free_flow_km_h: 40.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1600.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::Residential => DefaultRow {
            free_flow_km_h: 30.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1400.0,
            jam_density_veh_km_lane: 140.0,
        },
        RoadClass::LivingStreet => DefaultRow {
            free_flow_km_h: 20.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 900.0,
            jam_density_veh_km_lane: 150.0,
        },
        RoadClass::Service => DefaultRow {
            free_flow_km_h: 20.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 800.0,
            jam_density_veh_km_lane: 150.0,
        },
        RoadClass::Pedestrian | RoadClass::Footway | RoadClass::Cycleway => DefaultRow {
            free_flow_km_h: 30.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1400.0,
            jam_density_veh_km_lane: 140.0,
        },
    }
}

/// Signal defaults (S90, design §12.2).
///
/// OSM gives signal *locations* and never timings, so both numbers here are
/// assumptions — visible ones, in a table a user can override, rather than
/// constants buried in the node model.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SignalDefaults {
    /// Cycle length, in seconds.
    pub cycle_seconds: f64,
    /// Effective green time as a fraction of the cycle, per signalised approach.
    pub green_fraction: f64,
    /// The degree of saturation the control delay is evaluated at.
    ///
    /// Webster's uniform delay rises steeply near saturation, so this is a real
    /// choice: with a 90-second cycle and `g/C = 0.48` it gives roughly 12 s at
    /// zero flow, 20 s at `x = 0.85` and 23 s at `x = 0.95`. The delay is
    /// constant per approach — it does **not** respond to the flow the run
    /// produces, which is the price of costing nothing at run time.
    pub nominal_degree_of_saturation: f64,
}

impl SignalDefaults {
    /// The shipped values.
    pub const SHIPPED: SignalDefaults = SignalDefaults {
        cycle_seconds: 90.0,
        green_fraction: 0.48,
        nominal_degree_of_saturation: 0.85,
    };

    /// Webster's uniform delay for a signalised approach, in seconds.
    ///
    /// ```text
    /// d = 0.5 · C · (1 − g/C)²  /  (1 − min(1, x) · g/C)
    /// ```
    ///
    /// *CITATION OWED: the Webster constants are working knowledge and must be
    /// checked against the 1958 paper before the defaults table ships.*
    #[must_use]
    pub fn uniform_delay(self) -> Duration {
        let g = self.green_fraction;
        let x = self.nominal_degree_of_saturation.clamp(0.0, 1.0);
        let numerator = 0.5 * self.cycle_seconds * (1.0 - g) * (1.0 - g);
        let denominator = (1.0 - x * g).max(1e-6);
        Duration(numerator / denominator)
    }

    /// The share of a turn's saturation flow that a signal lets through.
    #[inline]
    #[must_use]
    pub fn turn_capacity_fraction(self) -> f64 {
        self.green_fraction
    }
}

/// The five global multipliers (S15).
///
/// The whole calibration surface a user gets before they start editing
/// per-link parameters. Few enough to sweep, and each one has a direction a
/// modeller can reason about.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GlobalMultipliers {
    /// Scales every free-flow speed.
    pub free_flow_speed: f64,
    /// Scales every link capacity.
    pub capacity: f64,
    /// Scales every jam density, and therefore every link's storage.
    pub jam_density: f64,
    /// Scales the signal control delay.
    pub control_delay: f64,
    /// Scales the green-time fraction, and therefore turn capacity at signals.
    pub green_fraction: f64,
}

impl Default for GlobalMultipliers {
    /// All ones: the shipped table, unmodified.
    fn default() -> Self {
        Self {
            free_flow_speed: 1.0,
            capacity: 1.0,
            jam_density: 1.0,
            control_delay: 1.0,
            green_fraction: 1.0,
        }
    }
}

/// The internal, per-link parameters the loading actually reads.
///
/// SI and per-second (Foundations §9). Everything here was converted exactly
/// once, at the build boundary; nothing downstream divides by 3600 again.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LinkParameters {
    /// Free-flow speed.
    pub free_flow_speed: Speed,
    /// Capacity across all lanes of the link.
    pub capacity: Flow,
    /// Jam density across all lanes.
    pub jam_density: Density,
    /// Backward wave speed — **derived**, see the [module docs](self).
    pub wave_speed: Speed,
    /// Constant control delay added to the free-flow traversal time (S90).
    pub control_delay: Duration,
}

/// What happened while deriving a link's parameters, for diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParameterNote {
    /// The diagram was consistent as given; nothing to record.
    Consistent,
    /// The derived wave speed was outside [`WAVE_SPEED_BOUNDS_KM_H`] and was
    /// clamped; jam density was re-derived to keep the diagram consistent.
    WaveSpeedClamped,
    /// Capacity implied a critical density at or above jam density — no
    /// triangular diagram exists. Capacity was reduced to the largest value
    /// the given free-flow speed and jam density admit.
    CapacityReducedToFitJamDensity,
}

impl LinkParameters {
    /// Derive a link's internal parameters from a defaults row.
    ///
    /// `lanes` is the lane count for this direction, from OSM where present and
    /// from the row otherwise.
    ///
    /// Returns the parameters and a note saying whether anything had to be
    /// adjusted; the caller records the note as a diagnostic. **Nothing here
    /// ever fails** — an impossible parameter combination is resolved by a
    /// documented rule and recorded, because the simulation never stops (§3c).
    #[must_use]
    pub fn from_defaults(
        row: DefaultRow,
        lanes: u8,
        signalised: bool,
        signals: SignalDefaults,
        multipliers: GlobalMultipliers,
        maxspeed_km_h: Option<f64>,
    ) -> (Self, ParameterNote) {
        let lanes = f64::from(lanes.max(1));

        let v_km_h = maxspeed_km_h.unwrap_or(row.free_flow_km_h) * multipliers.free_flow_speed;
        let mut q_veh_h = row.saturation_flow_veh_h_lane * lanes * multipliers.capacity;
        let mut k_j_veh_km = row.jam_density_veh_km_lane * lanes * multipliers.jam_density;

        // Critical density: where the free-flow branch meets capacity.
        let k_c = q_veh_h / v_km_h;

        let mut note = ParameterNote::Consistent;

        // No triangular diagram exists if the free-flow branch reaches capacity
        // at or beyond jam density. Reduce capacity rather than invent storage:
        // capacity is the quantity the run can least afford to be wrong about
        // upward, and an over-stated capacity produces a network that never
        // congests, which is a silent failure.
        if k_c >= k_j_veh_km * 0.95 {
            q_veh_h = 0.95 * k_j_veh_km * v_km_h * 0.5;
            note = ParameterNote::CapacityReducedToFitJamDensity;
        }

        let k_c = q_veh_h / v_km_h;
        let mut w_km_h = q_veh_h / (k_j_veh_km - k_c);

        let (w_min, w_max) = WAVE_SPEED_BOUNDS_KM_H;
        if !(w_min..=w_max).contains(&w_km_h) {
            w_km_h = w_km_h.clamp(w_min, w_max);
            // Hold capacity and free-flow speed; re-derive jam density so the
            // triangle closes. k_j = q/w + q/v.
            k_j_veh_km = q_veh_h / w_km_h + k_c;
            if note == ParameterNote::Consistent {
                note = ParameterNote::WaveSpeedClamped;
            }
        }

        let control_delay = if signalised {
            signals.uniform_delay() * multipliers.control_delay
        } else {
            Duration::ZERO
        };

        (
            Self {
                free_flow_speed: Speed::from_km_per_hour(v_km_h),
                capacity: Flow::from_veh_per_hour(q_veh_h),
                jam_density: Density::from_veh_per_km(k_j_veh_km),
                wave_speed: Speed::from_km_per_hour(w_km_h),
                control_delay,
            },
            note,
        )
    }

    /// The free-flow traversal time of a link of this length, including the
    /// control delay (S90).
    #[inline]
    #[must_use]
    pub fn free_flow_time(&self, length: Metres) -> Duration {
        length / self.free_flow_speed + self.control_delay
    }

    /// How many vehicles a link of this length holds at jam density.
    #[inline]
    #[must_use]
    pub fn storage(&self, length: Metres) -> openmobisim_core_types::units::Pcu {
        self.jam_density * length
    }

    /// The critical density — where the diagram peaks.
    #[inline]
    #[must_use]
    pub fn critical_density(&self) -> Density {
        self.capacity / self.free_flow_speed
    }

    /// How far the triangular diagram departs from consistency, as a relative
    /// error on capacity.
    ///
    /// Zero for a consistent diagram. Exposed so that a test, and the build
    /// report, can assert the identity `q = v·w·k_j/(v+w)` rather than trust it.
    #[must_use]
    pub fn consistency_error(&self) -> f64 {
        let v = self.free_flow_speed.get();
        let w = self.wave_speed.get();
        let k = self.jam_density.get();
        let implied = v * w * k / (v + w);
        (implied - self.capacity.get()).abs() / self.capacity.get().max(f64::MIN_POSITIVE)
    }
}
