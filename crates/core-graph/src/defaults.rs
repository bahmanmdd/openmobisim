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
//! Manual; the control-delay form is Webster's uniform delay. Jam density and the
//! backward wave speed are checked against Helbing's review (2001, *Reviews of Modern
//! Physics* 73(4) 1067–1141, <https://arxiv.org/abs/cond-mat/0012229>: jam densities estimated
//! "from 120 to 200 vehicles per kilometer and lane", 140–160 "probably most realistic", p. 9;
//! congested traffic's wave speed "C0 = 15 ± 5 km/h", p. 13); the uniform-delay form is
//! Webster's (1958), see [`SignalDefaults::uniform_delay`] (S259).

use openmobisim_core_types::units::{Density, Duration, Flow, Metres, Speed};

/// Version of this table.
///
/// Part of every artifact fingerprint (Foundations §8), so **bump it whenever
/// any number below changes**. A cached network built with different parameters
/// is not the same network, and silently reusing it is the kind of error that
/// takes a week to find.
///
/// Version 2 (S195): the bike and walk layers' speeds and the dedicated-bike
/// preference ([`crate::layers::StaticLayerDefaults`]) joined the table.
///
/// Version 3 (S197): ferry crossings on the bike and walk layers, their default
/// speed and expected wait.
///
/// Version 4 (S199): scheduled public transport joined the table (`core-transit`'s
/// `TransitDefaults`): the boarding slack, walks to and between stops, bus dwell,
/// PCU and the plausibility ratio; and the busway class's row.
///
/// Version 5 (M4, S201): parking joined the table (`core-sim`'s
/// `ParkingDefaults`: walks from parkings, the reach of park-and-ride and
/// bike-and-ride, the candidates, the parking times and their occupancy
/// response), and the itineraries' choice coefficients joined the built-in
/// logit's defaults (walking, waiting, transfers); car routes are unchanged.
///
/// Version 6 (M5, S205): mode choice. The nested logit's nest scale (`mu`, 0.5) and
/// park-and-ride's shortest trip in mode choice (`ParkingDefaults::pr_min_km`, 3 km); the
/// mode constants are 0. A run without mode choice is unchanged.
///
/// Version 7 (S209): mode choice offers a walk or a ride only up to 30 minutes
/// (`core-sim`'s `ModeDefaults`: `walk_max_s`, `bike_max_s`, design §21.1). A run without
/// mode choice is unchanged.
///
/// Version 8 (S210): `msa`'s itinerary gap sample (`core-sim`'s `Msa::itinerary_gap_sample`):
/// how many itinerary trips plan their whole choice set to measure the gap, the others that
/// keep an itinerary out through a parking re-costed there. Its default is every trip, as
/// before: no run changes.
///
/// Version 9 (S213): the loading's rules (`core-sim`'s `LoadingOptions`): priority at merges
/// and en-route rerouting (`reroute_after_s` 300, `reroute_max` 3, `reroute_min_gain` 0.1),
/// both off while they are measured: no run changes.
///
/// Version 10 (S215): **en-route rerouting on by default** (300 s, at most 3, 10% faster),
/// priority off. Runs where vehicles stand still for five minutes change; others do not.
///
/// Version 11 (S217): **turn pockets** on approaches of two lanes or more (`core-sim`'s
/// `LoadingOptions::pocket_length_m`, 50 m per lane): a vehicle waiting to turn into a full
/// street no longer holds up those behind it turning elsewhere, while the waiting ones fit in
/// their pockets. Runs where a multi-lane approach's front waits change; others do not.
///
/// Version 12 (S223): **the free-flow loading** (iteration 0) is built up in groups and loaded
/// with the point queue: `core-sim`'s `Equilibration::increments` (the Python layer's default
/// 5, uncalibrated: the user's suggestion) and `warmup` 1 for a run of one loading as for one
/// that iterates, at flow levels 2 to 4. Every such run changes; level-0 runs do not.
///
/// Version 13 (S225): **every network parameter by name** (`network_options`, the
/// [`crate::network_options`] module: the road-class table, the multipliers, the signal settings,
/// the bike and walk layers' speeds), kept by the network and used by its turn table and bike
/// costs; the green-fraction multiplier, which nothing read, now scales the green time. No run at
/// the shipped values changes.
///
/// Version 14 (S229): **the point queue is the default loading** (`flow_level` 2 in the Python
/// layer; the full model, with spillback, pockets and rerouting, at 4); `msa` stops when
/// converged (`gap_tolerance` 0.02, `route_growth_tolerance` 1, the most `iterations` 10);
/// en-route rerouting off at level 2, where nothing waits for room. Every default run changes.
///
/// Version 15 (S230): **a region clip keeps what can be kept** (`network_read_osm`'s
/// `region_stubs` and `region_buffer_m`, `io-osm`'s `ClipOptions`): each road's first node past
/// the edge, and motorways, trunk roads and their slip roads within 2 km, so a ring road just
/// outside a study area stays. Every network read with a region changes.
///
/// Version 16 (S235): **longer choice-set limits**: mode choice offers a ride up to 60 minutes
/// (`core-sim`'s `ModeDefaults::bike_max_s`, 30 before), and a walk to or from a stop is up to
/// 30 minutes (`core-transit`'s `TransitDefaults::access_walk_max_s`, 15 before), counted in
/// whole seconds; a traveller class may give its own (`core-sim`'s `ClassLimits`). Runs with
/// mode choice or a timetable change; others do not.
///
/// Version 17 (S236): a painted lane's own multiplier in the bike layer's dedicated cost
/// (`bike_lane_cost_factor`, 1: a lane counted as a track, as before), overridable by name in
/// `network_options`. No run at the shipped values changes.
///
/// Version 18 (S248): **prices** (`core-sim`'s `Prices`, a scenario's `price_options`): a car's
/// running cost per km (0.12 €), a bike's (0), a transit journey's flat fare (2.0 €, nothing per
/// km or transfer) and a parking's fee where its table gives none (0); every alternative carries
/// `cost_eur` and its parts. The built-in models weigh money only when given `beta_cost_eur`: no
/// run at the shipped values changes.
///
/// Version 19: **three parking candidates** for park-and-ride and bike-and-ride (`core-sim`'s
/// `ParkingDefaults::candidates`, five before): each trip weighs the three nearest and the three
/// best by estimate. On Amsterdam's morning it saved 12 % of the run's time and took
/// bike-and-ride from 0.76 % to 0.54 % of trips. Runs with parkings change; others do not.
///
/// Version 20 (S254, the pre-release rebuild): **money counts by default** (D-1): the built-in logit
/// and nested logit weigh `cost_eur` at −1.2 per euro (`beta_cost_eur`; 10 €/h at −0.2 per
/// minute); and **a bike leg's kilometre in mixed traffic weighs −0.16 on top of its time** (D-4,
/// S244: `beta_bike_mixed_km`, what the bike route choice already assumes). Every logit run with a
/// price changes (a car's running cost is one), and every run whose choices include a bike leg.
///
/// Version 21 (S256, Q-7): **no prices by default**: a car's running cost per km and a transit
/// journey's fare are 0 (`core-sim`'s `Prices`; 0.12 € and 2.0 € before), so every shipped price
/// is 0 — having a car, a bike, a pass or the means to pay a fare is access, which traveller
/// classes give, and no money is counted for it. `beta_cost_eur` stays −1.2 and weighs the prices
/// a scenario gives. Every logit run at the shipped prices changes back to one weighing no money.
///
/// Version 22 (S265, the user: "the congestion level is always low … are these numbers defensible?"):
/// **at a signal, a lane of an at-grade road discharges at most its speed band's saturation flow**
/// ([`SaturationBands`]: 1 000 veh/h at 30 km/h or less, 1 200 at 50, 1 500 at 70, per hour of
/// green), the French references for signalised junctions, below the Highway Capacity Manual's base
/// the class rows keep elsewhere; and a trunk road's lane 1 800 veh/h (1 900 before). Every network
/// read from OpenStreetMap with signalised at-grade approaches of 70 km/h or less changes; a link
/// table that states its capacities does not. With it, the reader's fix (S265) that **a signal
/// tagged `traffic_signals:direction` stops only its direction** (a stop line's signal; before,
/// both, so a car met two signals at most junctions of a two-way street) and that an emergency
/// signal stops nobody; the network keeps signals per approach ([`crate::RoadNetwork::is_link_signalised`]).
///
/// Version 23 (S268, the user's rulings of 2026-10-12): **a region clip joins a main road's cut
/// carriageways by a turnaround** (`io-osm`'s `ClipOptions`, with a buffer), so motorways and
/// trunk roads stay to the buffer's edge instead of vanishing back to their last interchange
/// (every network read with a region and a buffer changes); **a traveller is offered at most 5
/// routes**, the fastest at their departure (`core-sim`'s `Run::with_choice_max_routes`; every
/// route within the detour limit before), while a set still holds up to 10 and a full set is
/// still searched, a route a second faster than its best replacing one nobody chose; and the
/// Python layer's `msa` **stops at a gap of 0.05** (0.02 before). Every run with a choice among
/// more than five routes changes, and every iterating run.
///
/// Version 24 (S269): **a journey with transit is offered only up to two hours door to door**
/// (`core-transit`'s `TransitDefaults::journey_max_s`): a timetable's late service had offered
/// a morning traveller an 18-hour journey that ended after midnight. Every run with transit
/// where such journeys were found changes; and a trip that chose its mode and had none is
/// counted apart (`RunResult::no_mode`), not under the car. And the OSM reader's walk layer
/// **walks cycleways unless `foot=no`** (`io-osm`'s `walk_way`; before, only with `foot=yes`):
/// every network read from OpenStreetMap with cycleways gains walk links, most in the
/// Netherlands (Amsterdam's walk network had fallen apart into islands, Schiphol's among them).
///
/// Version 25 (S271): **the simulation window is the study period**: a run starts at the
/// window's start and stops at its end (before, it ran from midnight, and its end defaulted to
/// midnight). Without a window given, it starts at the earliest departure, rounded down to the
/// quarter hour, and lasts **three hours** (the starter mornings' departures span 06:30–09:30);
/// trips departing outside it are left out with a warning, and a timetable keeps only the calls
/// inside it. And **a trip with no alternative at all travels by car** (`core-sim`'s
/// `ModeDefaults::fallback_car`, W-1), counted and reported apart. Every run changes.
///
/// Version 26 (S276): **the car fallback is off by default** (`core-sim`'s
/// `ModeDefaults::fallback_car`, experimental): a trip with no alternative stays unsimulated,
/// as before version 25, and the run reports it with why every mode gave it none. Every run
/// where some trip had no alternative changes.
///
/// Version 27 (S279): **a car trip pays its access and egress** in mode choice (`core-sim`'s
/// `ModeDefaults::car_access_s` 120 s and `car_egress_s` 180 s): reaching the car, and parking
/// and walking from it, counted in a car alternative's door-to-door time, its walk and its
/// parking time, and in the drive to a park-and-ride car park; every other mode paid its walks
/// and waits already. The loading moves the car from the trip's departure as before. Every run
/// with mode choice changes. **Turn pockets are off by default** (`core-sim`'s
/// `LoadingOptions::pocket_length_m` 0; 50 m since S218): with them an iterated run under heavy
/// congestion mispriced the waiting movements (S227) and the fix (I-au) is not built; every run at
/// flow levels 3–4 on a multi-lane network changes. Also (no default moved): the **nested logit** is the choice model a
/// scenario takes when it offers more than one mode, and mode choice offers at most
/// `choice_max_routes` car routes, as stated car trips get; the fundamental diagram keeps the
/// capacity a table or a factor gives and raises jam density when the backward wave would be
/// faster than its bound.
/// Version 28 (S280, C1): **one default table**: the core's defaults are the Python layer's,
/// which no longer sets its own (`core-sim`'s `Msa` 5 increments, warm-up 1, `gap_tolerance`
/// 0.05, `route_growth_tolerance` 1; `FreeFlow` 5 increments, warm-up 1; `LoadingOptions::reroute`
/// settled by the level: on at 3 and 4, off at the point queue unless asked). A Python run
/// simulates what it did; a run of the Rust core at its defaults now does the same.
///
/// Version 29 (S281, U-41 and U-42): **a trip's end snaps to a street**, the nearest node with
/// a road a car can enter from — motorways, trunk roads and their ramps excluded — unless the
/// point lies within `core-sim`'s `ModeDefaults::car_snap_motorway_m` (50 m) of a node that has
/// only such roads (a through trip placed on the motorway by design); before, the nearest
/// drivable node of any class, so 1–7 % of a city's trips began on a carriageway. Every run
/// whose trips lie near a motorway changes; the benchmark networks have none. And **a transit
/// pass is held only where said**: an unsaid `has_transit_pass` is false for every class (it was
/// true for a class that may take transit), so a fare, if set, is paid by all but the classes
/// and persons that say they hold a pass; the pass never gates transit. Prices are 0 by default,
/// so no shipped result moves; every fingerprint does (ownership is hashed).
pub const DEFAULTS_VERSION: u32 = 29;

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
    /// `route=ferry` — a ferry crossing, on the bike and walk layers only
    /// (S197): not a `highway` value, so no road is ever of this class.
    Ferry = 17,
    /// `highway=busway`, or a road closed to other motor traffic but open to
    /// buses (`access=no`, or `motor_vehicle=no`, with `bus` or `psv`): **buses
    /// only** (S199). Cars never use it; a bus rides it among the other buses,
    /// which is design §18.2's "dedicated-lane links exempt".
    Busway = 18,
}

impl RoadClass {
    /// Every class, in discriminant order.
    pub const ALL: [RoadClass; 19] = [
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
        RoadClass::Ferry,
        RoadClass::Busway,
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
            RoadClass::Ferry => "ferry",
            RoadClass::Busway => "busway",
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
            "busway" | "bus_guideway" => RoadClass::Busway,
            _ => return None,
        })
    }

    /// Whether cars and other general motor traffic may use this class by
    /// default: what a car is routed on and what "drivable" means. A busway is
    /// not: it is buses' only ([`Self::carries_buses`]).
    #[inline]
    #[must_use]
    pub const fn carries_motor_traffic(self) -> bool {
        !matches!(
            self,
            RoadClass::Pedestrian
                | RoadClass::Footway
                | RoadClass::Cycleway
                | RoadClass::Ferry
                | RoadClass::Busway
        )
    }

    /// Whether this is a motorway or a trunk road, or one of their ramps: a carriageway a trip
    /// does not start or end on (U-41, S281: a trip's end snaps to a street unless it lies on
    /// such a node, `ModeDefaults::car_snap_motorway_m`).
    #[inline]
    #[must_use]
    pub const fn is_motorway_or_trunk(self) -> bool {
        matches!(
            self,
            RoadClass::Motorway | RoadClass::MotorwayLink | RoadClass::Trunk | RoadClass::TrunkLink
        )
    }

    /// Whether buses may use this class: every class cars may, and busways
    /// (S199).
    #[inline]
    #[must_use]
    pub const fn carries_buses(self) -> bool {
        self.carries_motor_traffic() || matches!(self, RoadClass::Busway)
    }

    /// Whether pedestrians may use this class by default.
    ///
    /// Motorways and their links are the exception; everything else is walkable
    /// unless OSM says otherwise.
    #[inline]
    #[must_use]
    pub const fn carries_pedestrians(self) -> bool {
        !matches!(
            self,
            RoadClass::Motorway | RoadClass::MotorwayLink | RoadClass::Cycleway | RoadClass::Busway
        )
    }

    /// Whether the class is an at-grade road, with junctions, crossings and frontage (S265):
    /// everything but motorways, trunk roads and their slip roads. At a signal its lanes discharge
    /// at most their speed band's saturation flow ([`SaturationBands`]).
    #[inline]
    #[must_use]
    pub const fn is_at_grade(self) -> bool {
        !matches!(
            self,
            RoadClass::Motorway | RoadClass::MotorwayLink | RoadClass::Trunk | RoadClass::TrunkLink
        )
    }

    /// Whether cyclists may use this class by default, when no tag says more.
    ///
    /// Not motorways, trunk roads and their links (a tag such as
    /// `bicycle=yes` overrides the class; the OSM reader reads it so, G-19),
    /// footways, pedestrian streets (a bike is let on them by a tag only) and
    /// busways. This is the single source of the class part of the bike rule,
    /// for the OSM reader and the bike layer derived from a road table alike.
    #[inline]
    #[must_use]
    pub const fn carries_cyclists(self) -> bool {
        !matches!(
            self,
            RoadClass::Motorway
                | RoadClass::MotorwayLink
                | RoadClass::Trunk
                | RoadClass::TrunkLink
                | RoadClass::Footway
                | RoadClass::Pedestrian
                | RoadClass::Busway
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
/// | Helbing (2001), p. 9 (module docs) | jam density per lane: 130–150 by class, inside the 120–200 estimated (140–160 called most realistic) |
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
        // 1 800 (S265): the lower end of the 1 800–2 200 veh/h French managers use on urban
        // expressways (CEREMA 2016, table 2; see [`SaturationBands`]).
        RoadClass::Trunk => DefaultRow {
            free_flow_km_h: 90.0,
            lanes_per_direction: 2,
            saturation_flow_veh_h_lane: 1800.0,
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
        // A busway (S199): a secondary road's row, since buses there run at urban
        // arterial speeds. *An assumption*; `maxspeed` overrides it where tagged.
        RoadClass::Busway => DefaultRow {
            free_flow_km_h: 50.0,
            lanes_per_direction: 1,
            saturation_flow_veh_h_lane: 1800.0,
            jam_density_veh_km_lane: 130.0,
        },
        RoadClass::Pedestrian | RoadClass::Footway | RoadClass::Cycleway | RoadClass::Ferry => {
            DefaultRow {
                free_flow_km_h: 30.0,
                lanes_per_direction: 1,
                saturation_flow_veh_h_lane: 1400.0,
                jam_density_veh_km_lane: 140.0,
            }
        }
    }
}

/// The most a lane of an at-grade road discharges at a signal, by the road's speed (S265).
///
/// The class rows' saturation flows are the Highway Capacity Manual's base, for a lane at its
/// best; at a city street's signal a lane discharges less: parked cars, bus stops, crossing
/// pedestrians and cyclists, turning traffic, narrow lanes. European references measure it at
/// signalised junctions, per hour of green, by the junction's setting, of which the speed limit is
/// the part a network holds:
///
/// | Band | Shipped (veh/h per lane) | CEREMA (2016), measured mean | French managers use | Dutch practice (CROW 328) |
/// |---|---|---|---|---|
/// | 30 km/h or less | 1 000 | 800–1 200 ("very urban") | 900–1 200 | — |
/// | 31–50 km/h | 1 200 | 1 200–1 400 ("urban") | 1 500–1 600 | 1 092–1 248 (50 km/h, 2 × 1 lanes) |
/// | 51–70 km/h | 1 500 | 1 600–1 800 ("less urban") | 1 800 | 1 400–1 600 (80 km/h) |
///
/// Each shipped value is in the lower half of its band's references (the user, S265: "when there
/// are ranges try to go for lower values"). It applies to an approach that ends at a signal: the
/// references are measured there, and the green fraction applies on top
/// ([`SignalDefaults::turn_capacity_fraction`]); a lane between or at unsignalised junctions keeps
/// the class row (a first version capped every link, and congestion overshot the cities' measured
/// levels, S265). Above 70 km/h, and on motorways and trunk roads at any speed, the class row
/// applies. The cap is per lane and before the global capacity factor. A link whose capacity is
/// given (a link table's column) is not capped.
///
/// *Sources:* CEREMA, *Débit de saturation aux carrefours à feux : note de synthèse* (DTer
/// Méditerranée, October 2016), table 2,
/// <https://cerema.fr/fr/system/files/documents/2017/10/cerema_pcitcv_oct2016_note_synthese_debit_saturation_cle76df75.pdf>;
/// CROW publication 328, *Handboek wegontwerp* (2013), as a municipal traffic study applies it
/// (Woerden, Van Zwietenweg, 1 092–1 248 pae/h per lane for a 50 km/h distributor road with
/// separate cycle tracks, 1 400–1 600 at 80 km/h),
/// <https://www.ruimtelijkeplannen.nl/documents/NL.IMRO.0632.Vanzwietenweg-bOW1/b_NL.IMRO.0632.Vanzwietenweg-bOW1_tb15.pdf>
/// (the handbook itself was not read).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SaturationBands {
    /// At 30 km/h or less, in veh/h per lane.
    pub at_30_km_h: f64,
    /// Above 30 and at most 50 km/h.
    pub at_50_km_h: f64,
    /// Above 50 and at most 70 km/h.
    pub at_70_km_h: f64,
}

impl SaturationBands {
    /// The shipped values.
    pub const SHIPPED: SaturationBands =
        SaturationBands { at_30_km_h: 1000.0, at_50_km_h: 1200.0, at_70_km_h: 1500.0 };

    /// The most a lane of an at-grade road of `speed_km_h` carries; infinite above 70 km/h.
    #[must_use]
    pub fn cap(self, speed_km_h: f64) -> f64 {
        if speed_km_h <= 30.0 + 1e-9 {
            self.at_30_km_h
        } else if speed_km_h <= 50.0 + 1e-9 {
            self.at_50_km_h
        } else if speed_km_h <= 70.0 + 1e-9 {
            self.at_70_km_h
        } else {
            f64::INFINITY
        }
    }

    /// `row` with its saturation flow capped for an at-grade road of `speed_km_h`.
    #[must_use]
    pub fn applied(self, row: DefaultRow, speed_km_h: f64) -> DefaultRow {
        DefaultRow {
            saturation_flow_veh_h_lane: row.saturation_flow_veh_h_lane.min(self.cap(speed_km_h)),
            ..row
        }
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
    /// *The uniform-delay term of Webster (1958, *Traffic signal settings*, Road Research
    /// Technical Paper 39, HMSO), the first of his expression's three terms as Akgungor and
    /// Bullen's review of delay models describes them
    /// (<https://nacto.org/wp-content/uploads/analytical_delay_models_for_signalized_intersections_akgungor.pdf>);
    /// the 1958 paper itself was not read (S259).*
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
    /// The derived wave speed was below [`WAVE_SPEED_BOUNDS_KM_H`] (a capacity low for the
    /// jam density) and was clamped up; jam density was re-derived (lowered) so the triangle
    /// closes with the capacity given.
    WaveSpeedClamped,
    /// The derived wave speed was above [`WAVE_SPEED_BOUNDS_KM_H`], or no triangle existed
    /// (the free-flow branch reached capacity at or past jam density); the wave speed was
    /// clamped down and jam density re-derived (raised) so the triangle closes with the
    /// capacity given: a table's, a factor's or a tag's capacity is trusted over a default
    /// jam density (S279).
    JamDensityRaisedToFitCapacity,
}

impl LinkParameters {
    /// Derive a link's internal parameters from a defaults row.
    ///
    /// `lanes` is the lane count for this direction, from OSM where present and
    /// from the row otherwise. `maxspeed_km_h` and `capacity_veh_h` each
    /// override the row's own free-flow speed and total capacity when the
    /// caller already knows them (I-y: a link table that states one or both
    /// directly, rather than through `highway` × `lanes`); either or both may
    /// be `None`, in which case that quantity still comes from the row. Jam
    /// density has no override yet — no source this table reads from states it
    /// (design §12.1: it is the quantity that varies least between road
    /// types), so it always comes from the row.
    ///
    /// Returns the parameters and a note saying whether anything had to be
    /// adjusted; the caller records the note as a diagnostic. **Nothing here
    /// ever fails** — an impossible parameter combination (an overridden
    /// capacity too high for the row's jam density, say) is resolved by the
    /// same documented rule as an inconsistent default row, because the
    /// simulation never stops (§3c).
    #[must_use]
    pub fn from_defaults(
        row: DefaultRow,
        lanes: u8,
        signalised: bool,
        signals: SignalDefaults,
        multipliers: GlobalMultipliers,
        maxspeed_km_h: Option<f64>,
        capacity_veh_h: Option<f64>,
    ) -> (Self, ParameterNote) {
        let lanes = f64::from(lanes.max(1));

        let v_km_h = maxspeed_km_h.unwrap_or(row.free_flow_km_h) * multipliers.free_flow_speed;
        let q_veh_h =
            capacity_veh_h.unwrap_or(row.saturation_flow_veh_h_lane * lanes) * multipliers.capacity;
        let mut k_j_veh_km = row.jam_density_veh_km_lane * lanes * multipliers.jam_density;

        // Critical density: where the free-flow branch meets capacity.
        let k_c = q_veh_h / v_km_h;

        // The backward wave speed closes the triangle: w = q / (k_j − k_c). Capacity and
        // free-flow speed are held (a table, a factor or a tag means them). When w falls
        // outside [`WAVE_SPEED_BOUNDS_KM_H`] — a capacity too high for the jam density, the
        // free-flow branch reaching capacity only at or past jam density (no triangle), or a
        // capacity too low — it is clamped and jam density re-derived so the triangle closes
        // again, k_j = q/w + k_c: storage is raised above the bound and lowered below it, and
        // the note says which. (Until S279 a capacity near or past jam density was halved and
        // the clamp then raised storage anyway, while the note reported only the reduction.)
        let (w_min, w_max) = WAVE_SPEED_BOUNDS_KM_H;
        let w_raw = if k_c < k_j_veh_km { q_veh_h / (k_j_veh_km - k_c) } else { f64::INFINITY };
        let w_km_h = w_raw.clamp(w_min, w_max);
        let note = if w_raw > w_max {
            k_j_veh_km = q_veh_h / w_km_h + k_c;
            ParameterNote::JamDensityRaisedToFitCapacity
        } else if w_raw < w_min {
            k_j_veh_km = q_veh_h / w_km_h + k_c;
            ParameterNote::WaveSpeedClamped
        } else {
            ParameterNote::Consistent
        };

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

    /// The same link with its capacity multiplied by `factor`, jam density and free-flow speed
    /// kept (S238: a bottleneck), the backward wave speed derived again from the triangle and,
    /// outside [`WAVE_SPEED_BOUNDS_KM_H`], clamped with jam density re-derived, as
    /// [`Self::from_defaults`] does; the capacity asked for is held, as there.
    #[must_use]
    pub fn with_capacity_times(&self, factor: f64) -> Self {
        let v = self.free_flow_speed.as_km_per_hour();
        let mut k_j = self.jam_density.as_veh_per_km();
        let q = self.capacity.as_veh_per_hour() * factor;
        let k_c = q / v;
        let (w_min, w_max) = WAVE_SPEED_BOUNDS_KM_H;
        let w_raw = if k_c < k_j { q / (k_j - k_c) } else { f64::INFINITY };
        let w = w_raw.clamp(w_min, w_max);
        if w_raw < w_min || w_raw > w_max {
            k_j = q / w + k_c;
        }
        Self {
            capacity: Flow::from_veh_per_hour(q),
            jam_density: Density::from_veh_per_km(k_j),
            wave_speed: Speed::from_km_per_hour(w),
            ..*self
        }
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
