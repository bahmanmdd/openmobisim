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
//! and its options; the bin length of the per-link results.
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

use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::link_geometry::NetworkFingerprint;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::hash::{Fnv1a, fingerprint_hex};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TravellerId, TripId};
use openmobisim_core_types::time::Second;

use crate::run::FlowMotor;

/// What went into one run. See the [module docs](self).
#[derive(Clone, PartialEq, Debug)]
pub struct RunDescription {
    /// A hash of every input that decides the results; the same on every
    /// platform. Show it with [`RunDescription::fingerprint_hex`].
    pub fingerprint: u64,
    /// The scenario's master seed: the one number that starts every random
    /// stream (`RngKey`). No stochastic step draws from it yet, so today it
    /// changes nothing but the fingerprint; the choice layer is the first to
    /// use it.
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

    RunDescription {
        fingerprint: h.finish(),
        master_seed: inputs.master_seed,
        network_fingerprint,
        flow_level,
        flow_step_seconds,
        route_method: inputs.route_method.to_string(),
        route_descriptor: inputs.route_descriptor.to_string(),
        link_bin_seconds: inputs.link_bin_seconds,
    }
}

fn hash_network(h: &mut Fnv1a, network: &RoadNetwork, ids: u64) {
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
