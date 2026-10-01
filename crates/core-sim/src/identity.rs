//! What a run was, in one line: its master seed and its fingerprint (S168).
//!
//! A [`RunDescription`] is made from a [`crate::Run`] *before* it executes and
//! says what went in: the network's content, the demand, every setting that
//! changes results, the route method with its options, and the master seed.
//! Its [`fingerprint`](RunDescription::fingerprint) is a hash of exactly those
//! inputs, so two runs with the same fingerprint were given the same inputs;
//! on one platform they then give the same results (S60). It is what a figure's
//! footer and a results file's header carry, so a picture or a table can be
//! traced to the run that made it.
//!
//! **What is in the hash.** The code version; the master seed; every node
//! (position, signal) and every link (ends, class, lanes, roundabout, length,
//! free-flow time, storage, and the whole diagram: speed, capacity, jam
//! density, wave speed, control delay); every traveller (id, class, weight,
//! what they own) and every trip (traveller, departure, origin, destination);
//! the window; the loading engine, its level and its step; the route method
//! and its options; the choice model and its options; the equilibration
//! strategy and its options; the choice-set detour limit (S178); the route update and its options, if there is one (S176);
//! the bin length of the per-link results; and, if any trip is not a car trip,
//! every trip's mode and each static layer a trip uses (its graph, speeds and
//! search costs, S195) — a car-only run hashes as it did before modes existed.
//!
//! **What is not.** The platform and the crate version (the manifest carries
//! them next to the fingerprint: a fingerprint says *what was run*, the
//! manifest says *what ran it*); the shape of the streets, which only pictures
//! use; the turn table, which is a function of the network and the shipped
//! signal defaults, covered by the code version.
//!
//! **Cost.** One pass over the inputs, a few array reads per link, node and
//! trip, at about a nanosecond per byte hashed. Measured on Luxembourg: 57 ms
//! for 384 000 links, 152 000 nodes and 19 000 trips; 152 ms with a million
//! trips; under 1% of an 8-second run.

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::layers::StaticLayer;
use openmobisim_core_graph::link_geometry::NetworkFingerprint;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::hash::{Fnv1a, fingerprint_hex};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TravellerId, TripId};
use openmobisim_core_types::time::Second;

use crate::layers::{StaticLayers, static_layer_of};
use crate::parking::ParkingSetup;
use crate::run::FlowMotor;
use crate::transit::TransitSetup;

/// What went into one run. See the [module docs](self).
#[derive(Clone, PartialEq, Debug)]
pub struct RunDescription {
    /// A hash of every input that decides the results; the same on every
    /// platform. Show it with [`RunDescription::fingerprint_hex`].
    pub fingerprint: u64,
    /// The scenario's master seed: the one number that starts every random
    /// stream (`RngKey`). Under a sampled choice model it decides who takes
    /// which route; under the all-or-nothing default nothing draws from it and
    /// it changes only the fingerprint.
    pub master_seed: u64,
    /// A hash of the network's ids, as stored in every artifact's header
    /// (`NetworkFingerprint`): an internal id means something only next to it.
    pub network_fingerprint: u64,
    /// The loading engine's level: 0 for free flow, 2, 3 or 4 for the link
    /// transmission model (design §10.1).
    pub flow_level: u32,
    /// The loading step in seconds, under the link transmission model.
    pub flow_step_seconds: Option<f64>,
    /// The route method's name.
    pub route_method: String,
    /// The route method with every option and its default, as a canonical
    /// string.
    pub route_descriptor: String,
    /// The bin length of the per-link results, if asked for.
    pub link_bin_seconds: Option<u32>,
    /// The choice model's name (S169).
    pub choice_model: String,
    /// The choice model with every option and default, canonical.
    pub choice_descriptor: String,
    /// The random streams the run draws from: `"choice"` under a sampled choice
    /// model, `"msa_reselection"` under an equilibration that iterates.
    pub live_streams: Vec<String>,
    /// The equilibration strategy's name (S170).
    pub equilibration: String,
    /// The strategy with every option and default, canonical.
    pub equilibration_descriptor: String,
    /// The most loadings the run makes.
    pub max_iterations: u32,
    /// How far above the best route's expected time a route may be and still be offered to a
    /// traveller (S178); 0 offers every route.
    pub choice_detour_limit: f64,
    /// The route update's name (S176): `"none"` if the sets stay as generated.
    pub route_update: String,
    /// The update with every option and default, canonical.
    pub route_update_descriptor: String,
}

impl RunDescription {
    /// The fingerprint as 16 lower-case hex digits.
    #[must_use]
    pub fn fingerprint_hex(&self) -> String {
        fingerprint_hex(self.fingerprint)
    }

    /// The network fingerprint as 16 lower-case hex digits.
    #[must_use]
    pub fn network_fingerprint_hex(&self) -> String {
        fingerprint_hex(self.network_fingerprint)
    }
}

/// The inputs a [`crate::Run`] hands over to be described.
pub(crate) struct Inputs<'a> {
    pub network: &'a RoadNetwork,
    pub travellers: &'a Travellers,
    pub trips: &'a Trips,
    pub window: Second,
    pub flow_motor: &'a FlowMotor,
    pub link_bin_seconds: Option<u32>,
    pub route_method: &'a str,
    pub route_descriptor: &'a str,
    pub master_seed: u64,
    pub choice_model: &'a str,
    pub choice_descriptor: &'a str,
    pub choice_sampled: bool,
    pub equilibration: &'a str,
    pub equilibration_descriptor: &'a str,
    pub equilibration_draws: bool,
    pub max_iterations: u32,
    pub route_update: &'a str,
    pub route_update_descriptor: &'a str,
    pub route_update_active: bool,
    pub choice_detour_limit: f64,
    pub layers: &'a StaticLayers,
    pub transit: Option<&'a TransitSetup>,
    pub parking: Option<&'a ParkingSetup>,
    /// The modes a trip without a stated mode chooses among (M5), if the run offers a choice.
    pub mode_choice: Option<&'a [Mode]>,
}

pub(crate) fn describe(inputs: &Inputs<'_>) -> RunDescription {
    let network_fingerprint = NetworkFingerprint::of(inputs.network).value();
    let (flow_level, flow_step_seconds) = match inputs.flow_motor {
        FlowMotor::Level0 => (0, None),
        FlowMotor::Ltm { step, level, .. } => (level.number(), Some(step.get())),
    };

    let mut h = Fnv1a::new();
    h.write_str("openmobisim-run");
    h.write_u32(openmobisim_core_types::CODE_VERSION);
    h.write_u64(inputs.master_seed);
    hash_network(&mut h, inputs.network, network_fingerprint);
    hash_demand(&mut h, inputs.travellers, inputs.trips);
    h.write_u32(inputs.window.get());
    h.write_u32(flow_level);
    h.write_bool(flow_step_seconds.is_some());
    h.write_f64(flow_step_seconds.unwrap_or(0.0));
    h.write_bool(inputs.link_bin_seconds.is_some());
    h.write_u32(inputs.link_bin_seconds.unwrap_or(0));
    h.write_str(inputs.route_method);
    h.write_str(inputs.route_descriptor);
    h.write_str(inputs.choice_model);
    h.write_str(inputs.choice_descriptor);
    h.write_str(inputs.equilibration);
    h.write_str(inputs.equilibration_descriptor);
    h.write_f64(inputs.choice_detour_limit);
    // Off means absent (S176): a run without an update hashes as it did before there was one.
    if inputs.route_update_active {
        h.write_str("route-update");
        h.write_str(inputs.route_update);
        h.write_str(inputs.route_update_descriptor);
    }
    hash_modes(&mut h, inputs.trips, inputs.layers);
    // Off means absent (M5): a run without mode choice hashes as it did before it.
    if let Some(modes) = inputs.mode_choice {
        hash_mode_choice(&mut h, inputs.trips, inputs.layers, modes);
    }
    // Off means absent: a run without a timetable hashes as it did before transit.
    if let Some(transit) = inputs.transit {
        hash_transit(&mut h, transit);
    }
    // Likewise without parkings (M4).
    if let Some(parking) = inputs.parking {
        hash_parking(&mut h, parking);
    }

    RunDescription {
        fingerprint: h.finish(),
        master_seed: inputs.master_seed,
        network_fingerprint,
        flow_level,
        flow_step_seconds,
        route_method: inputs.route_method.to_string(),
        route_descriptor: inputs.route_descriptor.to_string(),
        link_bin_seconds: inputs.link_bin_seconds,
        choice_model: inputs.choice_model.to_string(),
        choice_descriptor: inputs.choice_descriptor.to_string(),
        live_streams: {
            let mut streams = Vec::new();
            if inputs.choice_sampled {
                streams.push("choice".to_string());
            }
            if inputs.equilibration_draws {
                streams.push("msa_reselection".to_string());
            }
            streams
        },
        equilibration: inputs.equilibration.to_string(),
        equilibration_descriptor: inputs.equilibration_descriptor.to_string(),
        max_iterations: inputs.max_iterations,
        choice_detour_limit: inputs.choice_detour_limit,
        route_update: inputs.route_update.to_string(),
        route_update_descriptor: inputs.route_update_descriptor.to_string(),
    }
}

pub(crate) fn hash_network(h: &mut Fnv1a, network: &RoadNetwork, ids: u64) {
    h.write_u64(ids);
    for raw in 0..network.node_count() {
        let node = NodeId::new(raw);
        let at = network.node_lonlat(node);
        h.write_f64(at.lon);
        h.write_f64(at.lat);
        h.write_bool(network.is_signalised(node));
    }
    for raw in 0..network.link_count() {
        let link = LinkId::new(raw);
        h.write_u32(network.link_from(link).raw());
        h.write_u32(network.link_to(link).raw());
        h.write_u8(network.link_class(link) as u8);
        h.write_u8(network.link_lanes(link));
        h.write_bool(network.is_roundabout(link));
        h.write_f64(network.link_length(link).get());
        h.write_f64(network.free_flow_time(link).get());
        h.write_f64(network.storage(link).get());
        let p = network.link_parameters(link);
        h.write_f64(p.free_flow_speed.get());
        h.write_f64(p.capacity.get());
        h.write_f64(p.jam_density.get());
        h.write_f64(p.wave_speed.get());
        h.write_f64(p.control_delay.get());
    }
}

/// Every trip's mode and the static layers the trips use (S195); nothing at
/// all if every trip is a car trip.
fn hash_modes(h: &mut Fnv1a, trips: &Trips, layers: &StaticLayers) {
    let mut used = [false; 2];
    let mut any = false;
    // (Mode choice hashes the layers it offers itself: `hash_mode_choice`.)
    for raw in 0..trips.len() {
        let mode = trips.mode(TripId::new(raw));
        any |= mode != Mode::Car;
        match static_layer_of(mode) {
            Some(StaticLayer::Bike) => used[0] = true,
            Some(StaticLayer::Walk) => used[1] = true,
            None => {}
        }
        // A transit trip walks to and from its stops (S199).
        used[1] |= mode == Mode::Transit;
    }
    if !any {
        return;
    }
    h.write_str("modes");
    for raw in 0..trips.len() {
        h.write_u8(trips.mode(TripId::new(raw)) as u8);
    }
    for (layer, used) in [StaticLayer::Bike, StaticLayer::Walk].into_iter().zip(used) {
        if used {
            hash_layer(h, layers, layer);
        }
    }
}

/// A static layer the run has: its network, seconds and costs.
fn hash_layer(h: &mut Fnv1a, layers: &StaticLayers, layer: StaticLayer) {
    let Some(setup) = layers.get(layer) else { return };
    let graph = setup.network().network();
    h.write_str(layer.as_str());
    hash_network(h, graph, NetworkFingerprint::of(graph).value());
    for &s in setup.seconds() {
        h.write_f64(s);
    }
    for &c in setup.costs() {
        h.write_f64(c);
    }
}

/// Mode choice (M5): the modes offered, which trips choose, and the layers they may use.
fn hash_mode_choice(h: &mut Fnv1a, trips: &Trips, layers: &StaticLayers, modes: &[Mode]) {
    h.write_str("mode-choice");
    h.write_u32(u32::try_from(modes.len()).expect("few modes"));
    for &m in modes {
        h.write_u8(m as u8);
    }
    for raw in 0..trips.len() {
        h.write_bool(trips.mode_given(TripId::new(raw)));
    }
    for layer in [StaticLayer::Bike, StaticLayer::Walk] {
        hash_layer(h, layers, layer);
    }
}

/// The timetable, its defaults, and how its stops are linked to the walk and
/// bike layers (S199).
fn hash_transit(h: &mut Fnv1a, transit: &TransitSetup) {
    let t = transit.timetable();
    h.write_str("transit");
    h.write_u64(u64::from(t.date().day_number().unsigned_abs()));
    h.write_bool(t.date().day_number() < 0);
    h.write_u32(t.stop_count());
    for raw in 0..t.stop_count() {
        let stop = NodeId::new(raw);
        h.write_str(t.stop_ids().external(raw));
        let at = t.stop_position(stop);
        h.write_f64(at.lon);
        h.write_f64(at.lat);
        h.write_u32(transit.stop_walk_node(stop).map_or(u32::MAX, |n| n.raw()));
        h.write_u32(transit.stop_bike_node(stop).map_or(u32::MAX, |n| n.raw()));
    }
    for raw in 0..t.route_count() {
        h.write_str(t.route_ids().external(raw));
        h.write_u32(u32::from(t.route_type(raw)));
    }
    h.write_u32(t.run_count());
    for raw in 0..t.run_count() {
        let run = openmobisim_core_types::ids::TransitRunId::new(raw);
        h.write_str(t.run_ids().external(raw));
        h.write_u32(t.run_route(run));
        for c in t.run_calls(run) {
            h.write_u32(t.call_stop(c).raw());
            h.write_u8(t.call_flags(c));
            h.write_u32(t.scheduled().arrival[c]);
            h.write_u32(t.scheduled().departure[c]);
        }
    }
    for tr in t.transfers() {
        h.write_u32(tr.from.raw());
        h.write_u32(tr.to.raw());
        h.write_u32(tr.seconds);
    }
    let d = transit.defaults();
    h.write_u32(d.board_slack_s);
    h.write_u32(d.max_rides);
    for v in [
        d.access_walk_max_s,
        d.transfer_walk_max_s,
        d.stop_walk_snap_m,
        d.stop_transfer_s,
        d.bus_dwell_s,
        d.bus_pcu,
        d.bus_plausibility_ratio,
        d.bus_stop_snap_m,
    ] {
        h.write_f64(v);
    }
    let walk = transit.walk().network();
    hash_network(h, walk, NetworkFingerprint::of(walk).value());
}

/// The parkings, where they are linked to the layers, and parking's defaults (M4).
fn hash_parking(h: &mut Fnv1a, parking: &ParkingSetup) {
    h.write_str("parking");
    let n = u32::try_from(parking.count()).expect("parkings fit u32");
    h.write_u32(n);
    for p in 0..n {
        h.write_str(parking.external_id(p));
        h.write_str(parking.hub_external_id(p));
        h.write_u8(parking.kind(p) as u8);
        h.write_u32(parking.capacity(p));
        h.write_u32(parking.initial_occupancy(p));
        let at = parking.position(p);
        h.write_f64(at.lon);
        h.write_f64(at.lat);
        h.write_u32(parking.node(p).raw());
        h.write_u32(parking.walk_node(p).raw());
        for &(stop, walk) in parking.stops(p) {
            h.write_u32(stop.raw());
            h.write_u32(walk);
        }
    }
    let d = parking.defaults();
    for v in [
        d.walk_max_s,
        d.reach_car_s,
        d.reach_bike_s,
        d.candidates,
        d.rank_speed_km_h,
        d.floor_car_s,
        d.slope_car_s,
        d.floor_bike_s,
        d.slope_bike_s,
        d.snap_m,
        d.bin_s,
        d.pr_min_km,
    ] {
        h.write_f64(v);
    }
}

fn hash_demand(h: &mut Fnv1a, travellers: &Travellers, trips: &Trips) {
    h.write_u32(travellers.len());
    for raw in 0..travellers.len() {
        let traveller = TravellerId::new(raw);
        h.write_str(travellers.external_ids().external(raw));
        h.write_str(
            travellers.class_external_ids().external(travellers.user_class(traveller).raw()),
        );
        h.write_u32(travellers.weight(traveller));
        let own = travellers.ownership(traveller);
        h.write_bool(own.car);
        h.write_bool(own.bike);
        h.write_bool(own.transit_pass);
    }
    h.write_u32(trips.len());
    for raw in 0..trips.len() {
        let trip = TripId::new(raw);
        h.write_u32(trips.traveller(trip).raw());
        h.write_u32(trips.departure(trip).get());
        for at in [trips.origin(trip), trips.destination(trip)] {
            h.write_f64(at.lon);
            h.write_f64(at.lat);
        }
    }
}
