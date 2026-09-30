//! Prints a fingerprint of a full run through the Phase 1 pipeline — the
//! N3 fixture (`manhattan_grid`) through `core-demand`, `core-sim` and
//! `io-parquet`'s writers.
//!
//! CI runs this twice and compares the two outputs byte for byte: this is
//! the run-twice bit-identity gate in the form S111 said it would take once
//! `core-sim` existed — "the gate becomes a comparison of two
//! `kpis.parquet` files" — done here by writing all four artifacts for
//! real and then printing a digest of what they contain, the same
//! bits-not-decimals convention `core-types`' `determinism_probe` already
//! established (`determinism_probe` itself stays, as the cheap, fast check
//! of the mechanisms below `core-sim`).
//!
//! Run it yourself with:
//!
//! ```text
//! cargo run -p openmobisim-io-parquet --release --example kpis_determinism_probe
//! ```

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::{FlowMotor, Run};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;
use openmobisim_io_parquet::manifest::Manifest;
use openmobisim_io_parquet::{write_diagnostics, write_events, write_kpis, write_manifest};

/// The N3 fixture's own parameters:
/// `manhattan_grid(n=5, block_metres=200, signals=True)`.
const GRID_N: u32 = 5;
const BLOCK_METRES: f64 = 200.0;

/// Comfortably longer than any trip below except the one deliberately
/// timed to miss it.
const WINDOW_S: u32 = 50_000;

/// A deterministic demand set spanning every completion bucket
/// `core-sim::TripCompletionStats` reports: some travellers own a car and
/// complete every trip, one owns no car at all (`no_vehicle_available`),
/// and the window is set to strand one traveller's second trip
/// (`truncated`). Small and hand-authored, matching every other crate's
/// fixture convention this session (S133) — this is a determinism check,
/// not a demand-realism one.
fn demand(node: impl Fn(u32, u32) -> LonLat) -> Vec<RawTrip> {
    let trip = |traveller: &str,
                seq: u32,
                from: (u32, u32),
                to: (u32, u32),
                dep: u32,
                class: &str,
                weight: Option<u32>| RawTrip {
        traveller_id: traveller.to_string(),
        trip_seq: seq,
        origin: node(from.0, from.1),
        destination: node(to.0, to.1),
        departure_time: Second(dep),
        user_class: class.to_string(),
        weight,
        mode: None,
    };

    let mut trips = Vec::new();

    // Five car-owning commuters, crossing the grid on the diagonal and back.
    for i in 0..5u32 {
        trips.push(trip(
            &format!("commuter_{i}"),
            0,
            (i, 0),
            (i, GRID_N - 1),
            i * 100,
            "commuter",
            None,
        ));
        trips.push(trip(
            &format!("commuter_{i}"),
            1,
            (i, GRID_N - 1),
            (i, 0),
            3600 + i * 100,
            "commuter",
            None,
        ));
    }
    // A traveller with a heavier weight, to exercise the population scaling.
    trips.push(trip("heavy", 0, (0, 0), (GRID_N - 1, GRID_N - 1), 50, "commuter", Some(10)));
    // A pedestrian: no declared class default, so no vehicle is ever
    // available — exercises `no_vehicle_available`.
    trips.push(trip("pedestrian_0", 0, (2, 2), (2, 4), 10, "pedestrian", None));
    // A commuter whose first trip completes normally and whose second
    // departs close enough to the window's end that it cannot possibly
    // arrive before it — exercises `truncated`, regardless of the exact
    // free-flow speed the defaults table gives this road class.
    trips.push(trip("late_0", 0, (4, 0), (4, 4), 0, "commuter", None));
    trips.push(trip("late_0", 1, (4, 4), (0, 4), WINDOW_S - 50, "commuter", None));

    trips
}

/// A demand that jams the grid's middle: 2 000 travellers, each crossing from one side to the
/// other (pairs from a fixed generator, so it never changes), leaving within a minute.
fn jammed_demand(node: impl Fn(u32, u32) -> LonLat) -> Vec<RawTrip> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |bound: u32| -> u32 {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        u32::try_from((state >> 33) % u64::from(bound)).expect("a small bound")
    };
    (0..2_000u32)
        .map(|i| {
            let row = next(GRID_N);
            let (from, to) = ((row, 0), (GRID_N - 1 - row, GRID_N - 1));
            RawTrip {
                traveller_id: format!("jam_{i}"),
                trip_seq: 0,
                origin: node(from.0, from.1),
                destination: node(to.0, to.1),
                departure_time: Second(next(60)),
                user_class: "commuter".to_string(),
                weight: None,
                mode: None,
            }
        })
        .collect()
}

fn main() {
    let (network, build_diagnostics) = manhattan_grid(GRID_N, BLOCK_METRES, true);
    assert!(build_diagnostics.is_empty(), "a clean synthetic grid must produce no diagnostics");
    let network = Arc::new(network);

    let node = |r: u32, c: u32| -> LonLat {
        network.node_lonlat(
            network.node_external_ids().typed_id_of(&node_name(r, c)).expect("known node"),
        )
    };

    let class_defaults =
        ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE });
    let mut demand_diagnostics = Diagnostics::new();
    let (travellers, trips) = build_travellers(
        demand(node),
        Vec::new(),
        &class_defaults,
        /* default_weight */ 1,
        &mut demand_diagnostics,
    )
    .expect("the hand-authored demand above is well-formed");
    let travellers = Arc::new(travellers);
    let trips = Arc::new(trips);

    let window = Second(WINDOW_S);

    let mut run = Run::new(network.clone(), travellers.clone(), trips.clone(), window);
    let description = run.description();
    let mut run_diagnostics = Diagnostics::new();
    let result = run.execute(&mut run_diagnostics);

    let mut diagnostics = demand_diagnostics;
    diagnostics.merge(&run_diagnostics);

    let dir = std::env::temp_dir().join("openmobisim-kpis-determinism-probe");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let run_id = "n3-probe";
    write_kpis(dir.join("kpis.parquet"), run_id, "default", 0, 0, &result).expect("write kpis");
    write_diagnostics(dir.join("diagnostics.parquet"), run_id, &diagnostics)
        .expect("write diagnostics");
    write_events(
        dir.join("events.parquet"),
        run_id,
        &result.events,
        openmobisim_io_parquet::events::DEFAULT_SAMPLE_RATE,
    )
    .expect("write events");
    let manifest = Manifest::for_run(&travellers, &result, window, 1, &description);
    write_manifest(dir.join("manifest.json"), &manifest).expect("write manifest");

    // The same demand again, each trip choosing its route from the logit (S169): the
    // draws are keyed on (traveller, trip, iteration, route), so the choices must be
    // the same on every run and at any thread count.
    let sampled =
        Arc::from(openmobisim_core_choice::model("logit", &Default::default()).expect("built in"));
    let trips2 = trips.clone();
    let mut sampled_run = Run::new(network.clone(), travellers.clone(), trips, window)
        .with_master_seed(20_260_921)
        .with_choice_model(sampled);
    let sampled_description = sampled_run.description();
    let sampled_result = sampled_run.execute(&mut Diagnostics::new());
    let mut choice_digest = 0u64;
    for r in &sampled_result.route_choices.as_ref().expect("recorded").route {
        choice_digest = choice_digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(u64::from(*r));
    }

    // And iterated (S170): four loadings under the method of successive averages, on the same
    // demand. Who chooses again is a keyed draw, the link times are read back from each
    // loading, and the gap is measured against a fresh sample: every one of those must be
    // the same on every run and at any thread count.
    let iterated =
        Arc::from(openmobisim_core_choice::model("logit", &Default::default()).expect("built in"));
    let msa = Arc::from(
        openmobisim_core_sim::equilibration::strategy(
            "msa",
            &[("iterations".to_string(), 4.0)].into_iter().collect(),
        )
        .expect("built in"),
    );
    let mut iterated_run = Run::new(network.clone(), travellers.clone(), trips2, window)
        .with_master_seed(20_260_921)
        .with_choice_model(iterated)
        .with_equilibration(msa);
    let iterated_description = iterated_run.description();
    let iterated_result = iterated_run.execute(&mut Diagnostics::new());
    let mut iterated_digest = 0u64;
    for report in &iterated_result.iterations {
        for x in [
            report.reselected_share,
            report.changed_share,
            report.total_travel_time_s,
            report.time_change,
            report.gap,
            report.gap_network,
            report.gap_flow,
            report.gap_flow_floor,
        ] {
            iterated_digest =
                iterated_digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(x.to_bits());
        }
    }

    // And adapting (S176): the link transmission model on a jammed grid, four loadings, the
    // route sets growing between them by best responses to the congested times. The searches
    // run in parallel over pairs and are merged in key order: the routes added, and so every
    // number after them, must be the same on every run and at any thread count.
    let jam = jammed_demand(node);
    let (jam_travellers, jam_trips) = build_travellers(
        jam,
        Vec::new(),
        &class_defaults,
        /* default_weight */ 1,
        &mut Diagnostics::new(),
    )
    .expect("the generated demand is well-formed");
    let turns = Arc::new(TurnTable::build(&network, SignalDefaults::SHIPPED));
    let mut adapting_run =
        Run::new(network.clone(), Arc::new(jam_travellers), Arc::new(jam_trips), window)
            .with_flow_motor(FlowMotor::Ltm {
                turns,
                step: Duration(30.0),
                level: FidelityLevel::Full,
            })
            .with_master_seed(20_260_921)
            .with_choice_model(Arc::from(
                openmobisim_core_choice::model("logit", &Default::default()).expect("built in"),
            ))
            .with_equilibration(Arc::from(
                openmobisim_core_sim::equilibration::strategy(
                    "msa",
                    &[("iterations".to_string(), 4.0)].into_iter().collect(),
                )
                .expect("built in"),
            ))
            .with_route_update(Arc::from(
                openmobisim_core_sim::route_update::update("best_response", &Default::default())
                    .expect("built in"),
            ));
    let adapting_description = adapting_run.description();
    let adapting_result = adapting_run.execute(&mut Diagnostics::new());
    let step = |digest: u64, x: u64| digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(x);
    let mut adapting_digest = 0u64;
    for report in &adapting_result.iterations {
        for x in [report.total_travel_time_s, report.gap, report.gap_network, report.time_change] {
            adapting_digest = step(adapting_digest, x.to_bits());
        }
        adapting_digest = step(adapting_digest, u64::from(report.routes_added));
        adapting_digest = step(adapting_digest, u64::from(report.route_searches));
    }
    let grown = adapting_result.route_sets.as_ref().expect("recorded");
    let mut sets_digest = 0u64;
    for r in 0..grown.route_count() {
        for &l in grown.route(r).links {
            sets_digest = step(sets_digest, u64::from(l));
        }
        sets_digest = step(sets_digest, u64::from(grown.stamps()[r]));
    }
    for r in &adapting_result.route_choices.as_ref().expect("recorded").route {
        sets_digest = step(sets_digest, u64::from(*r));
    }
    let routes_added: u32 = adapting_result.iterations.iter().map(|it| it.routes_added).sum();
    assert!(routes_added > 0, "the probe must exercise the update: nothing was added");

    // --- The report ----------------------------------------------------
    // Floats as raw bits: the property under test is bit-identity, and a
    // decimal rendering would hide exactly the difference the gate exists
    // to catch (the same convention `determinism_probe` uses).
    println!("grid                  {GRID_N}x{GRID_N}, {BLOCK_METRES} m blocks");
    println!(
        "network               {} nodes, {} links",
        network.node_count(),
        network.link_count()
    );
    // The run's fingerprint is a pure function of its inputs: it must be the
    // same on every run and at any thread count (S168).
    println!("run_fingerprint       {}", description.fingerprint_hex());
    println!("travellers            {}", travellers.len());
    println!("total_trips           {}", result.completion.total_trips);
    println!("completed             {}", result.completion.completed);
    println!("truncated             {}", result.completion.truncated);
    println!("no_vehicle_available  {}", result.completion.no_vehicle_available);
    println!("no_feasible_path      {}", result.completion.no_feasible_path);
    println!("completion_rate       {:016x}", result.completion.completion_rate().to_bits());
    println!("total_travel_time_s   {:016x}", result.total_travel_time.get().to_bits());
    println!("events                {}", result.events.len());
    println!("choice_fingerprint    {}", sampled_description.fingerprint_hex());
    println!("choice_routes_digest  {choice_digest:016x}");
    println!("msa_fingerprint       {}", iterated_description.fingerprint_hex());
    println!("msa_reports_digest    {iterated_digest:016x}");
    println!("msa_iterations        {}", iterated_result.iterations.len());
    println!("choice_total_time_s   {:016x}", sampled_result.total_travel_time.get().to_bits());
    println!("adapt_fingerprint     {}", adapting_description.fingerprint_hex());
    println!("adapt_reports_digest  {adapting_digest:016x}");
    println!("adapt_routes_added    {routes_added}");
    println!("adapt_routes          {}", grown.route_count());
    println!("adapt_sets_digest     {sets_digest:016x}");
    println!("adapt_sets_identity   {:016x}", grown.identity());

    let mut event_digest = 0u64;
    for e in &result.events {
        event_digest =
            event_digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(u64::from(e.second.get()));
        event_digest = event_digest
            .wrapping_mul(0x0100_0000_01b3)
            .wrapping_add(e.event_type.as_str().len() as u64);
        event_digest =
            event_digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(u64::from(e.entity_id));
    }
    println!("events_digest         {event_digest:016x}");

    let mut diag_digest = 0u64;
    for row in diagnostics.rows() {
        diag_digest = diag_digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(row.count);
        diag_digest = diag_digest
            .wrapping_mul(0x0100_0000_01b3)
            .wrapping_add(u64::from(row.key.element.id().unwrap_or(u32::MAX)));
    }
    println!("diagnostics           {} rows, digest {diag_digest:016x}", diagnostics.rows().len());

    transit_probe(&dir);
    parking_probe(&dir);
}

/// Transit (S199): the toy network with its tram and bus, the bus on the roads among
/// cars, and enough transit trips that routing them is split across threads. Every
/// realised time, boarding and trip outcome must be the same on every run and at any
/// thread count, and so must the `transit_calls.parquet` written.
fn transit_probe(dir: &std::path::Path) {
    use openmobisim_core_demand::Mode;
    use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
    use openmobisim_core_sim::{LayerSetup, StaticLayers, TransitSetup};

    let (road, _) = openmobisim_core_graph::toy_network();
    let road = Arc::new(road);
    let (bike, walk) = openmobisim_core_graph::toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let layers = Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(Arc::new(bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d)),
    });
    let transit = Arc::new(
        TransitSetup::new(
            Arc::new(openmobisim_core_transit::examples::toy_transit()),
            layers.walk.as_ref().expect("made above"),
            layers.bike.as_ref(),
            openmobisim_core_transit::TransitDefaults::SHIPPED,
        )
        .with_roads(road.clone()),
    );
    let names = ["W", "N1", "S", "N2", "M", "X0", "D1", "N3", "D2", "R2"];
    let at = |name: &str| {
        road.node_lonlat(road.node_external_ids().typed_id_of(name).expect("a toy node"))
    };
    let mut rows = Vec::new();
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = |bound: usize| -> usize {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % bound as u64).expect("small")
    };
    for i in 0..460u32 {
        let (from, to) = (names[next(names.len())], names[next(names.len())]);
        let car = i >= 400;
        rows.push(RawTrip {
            traveller_id: format!("t{i:03}"),
            trip_seq: 0,
            origin: at(if car { "W" } else { from }),
            destination: at(if car { "M" } else { to }),
            departure_time: Second(u32::try_from(next(7200)).expect("small")),
            user_class: "commuter".to_string(),
            weight: None,
            mode: Some(if car { Mode::Car } else { Mode::Transit }),
        });
    }
    let defaults =
        ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE });
    let (travellers, trips) =
        build_travellers(rows, Vec::new(), &defaults, 1, &mut Diagnostics::new()).expect("valid");
    let mut run = Run::new(road.clone(), Arc::new(travellers), Arc::new(trips), Second(86_400))
        .with_flow_motor(FlowMotor::Ltm {
            turns: Arc::new(TurnTable::build(&road, SignalDefaults::SHIPPED)),
            step: Duration(300.0),
            level: FidelityLevel::Full,
        })
        .with_layers(layers)
        .with_transit(transit.clone());
    let description = run.description();
    let result = run.execute(&mut Diagnostics::new());
    let t = result.transit.as_ref().expect("the run has a timetable");
    let mut digest = 0u64;
    let mut step = |v: u64| digest = digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(v);
    for (a, dep) in t.times.arrival.iter().zip(&t.times.departure) {
        step(u64::from(*a));
        step(u64::from(*dep));
    }
    for b in t.boardings.iter().chain(&t.alightings) {
        step(b.to_bits());
    }
    for e in &result.events {
        step(u64::from(e.second.get()));
        step(u64::from(e.entity_id));
    }
    let transit_trips = result.by_mode[Mode::Transit.index()];
    openmobisim_io_parquet::write_transit_calls(
        dir.join("transit_calls.parquet"),
        "toy-transit-probe",
        transit.timetable(),
        t,
        &description,
    )
    .expect("write transit calls");
    println!("transit_fingerprint   {}", description.fingerprint_hex());
    println!(
        "transit_trips         {} completed of {}",
        transit_trips.completion.completed, transit_trips.completion.total_trips
    );
    println!("transit_time_s        {:016x}", transit_trips.total_travel_time.get().to_bits());
    println!("transit_digest        {digest:016x}");
}

/// Park-and-ride and bike-and-ride (M4): the toy network with its tram and bus and its
/// parkings, round trips out and back, a logit choosing the parkings and journeys under
/// `msa`, cars in the loading. Every choice, probability, parking tally and trip outcome
/// must be the same on every run and at any thread count, and so must the
/// `parking_bins.parquet` written.
fn parking_probe(dir: &std::path::Path) {
    use openmobisim_core_demand::Mode;
    use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
    use openmobisim_core_sim::{
        LayerSetup, ParkingDefaults, ParkingSetup, StaticLayers, TransitSetup,
    };

    let (road, _) = openmobisim_core_graph::toy_network();
    let road = Arc::new(road);
    let (bike, walk) = openmobisim_core_graph::toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let layers = Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(Arc::new(bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d)),
    });
    let transit = Arc::new(
        TransitSetup::new(
            Arc::new(openmobisim_core_transit::examples::toy_transit()),
            layers.walk.as_ref().expect("made above"),
            layers.bike.as_ref(),
            openmobisim_core_transit::TransitDefaults::SHIPPED,
        )
        .with_roads(road.clone()),
    );
    let parking = Arc::new(
        ParkingSetup::new(
            &openmobisim_core_graph::toy_network_parkings(),
            road.clone(),
            layers.bike.as_ref(),
            &transit,
            ParkingDefaults::SHIPPED,
        )
        .expect("the toy's parkings"),
    );
    let at = |name: &str| {
        road.node_lonlat(road.node_external_ids().typed_id_of(name).expect("a toy node"))
    };
    let trip = |who: String, seq: u32, from: &str, to: &str, t: u32, mode: Mode| RawTrip {
        traveller_id: who,
        trip_seq: seq,
        origin: at(from),
        destination: at(to),
        departure_time: Second(t),
        user_class: "commuter".to_string(),
        weight: None,
        mode: Some(mode),
    };
    let mut rows = Vec::new();
    for i in 0..40u32 {
        // Out by car and back, spread over an hour; out by bike; transit alone.
        rows.push(trip(format!("p{i:03}"), 0, "W", "N1", 60 * i, Mode::CarTransit));
        rows.push(trip(format!("p{i:03}"), 1, "N1", "D2", 3600 + 60 * i, Mode::CarTransit));
        rows.push(trip(format!("b{i:03}"), 0, "S", "N1", 45 * i, Mode::BikeTransit));
        rows.push(trip(format!("t{i:03}"), 0, "N1", "D2", 30 * i, Mode::Transit));
    }
    let defaults = ClassDefaults::new()
        .with_default("commuter", Ownership { car: true, bike: true, ..Ownership::NONE });
    let (travellers, trips) =
        build_travellers(rows, Vec::new(), &defaults, 1, &mut Diagnostics::new()).expect("valid");
    let logit =
        Arc::from(openmobisim_core_choice::model("logit", &Default::default()).expect("built in"));
    let msa = Arc::from(
        openmobisim_core_sim::equilibration::strategy(
            "msa",
            &[("iterations".to_string(), 3.0)].into_iter().collect(),
        )
        .expect("built in"),
    );
    let mut run = Run::new(road.clone(), Arc::new(travellers), Arc::new(trips), Second(86_400))
        .with_flow_motor(FlowMotor::Ltm {
            turns: Arc::new(TurnTable::build(&road, SignalDefaults::SHIPPED)),
            step: Duration(300.0),
            level: FidelityLevel::Full,
        })
        .with_master_seed(20_260_930)
        .with_choice_model(logit)
        .with_equilibration(msa)
        .with_layers(layers)
        .with_transit(transit)
        .with_parking(parking.clone());
    let description = run.description();
    let result = run.execute(&mut Diagnostics::new());
    let it = result.itineraries.as_ref().expect("itinerary trips");
    let p = result.parking.as_ref().expect("parkings");
    let mut digest = 0u64;
    let mut step = |v: u64| digest = digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(v);
    for i in 0..it.trip.len() {
        step(u64::from(it.parking[i]));
        step(it.probability[i].to_bits());
        step(u64::from(it.rides[i]));
    }
    for v in p.bins.arrivals.iter().chain(&p.bins.occupancy_mean).chain(&p.bins.full_s) {
        step(v.to_bits());
    }
    for e in &result.events {
        step(u64::from(e.second.get()));
        step(u64::from(e.entity_id));
    }
    for r in &result.iterations {
        for g in r.itinerary_gap {
            step(g.to_bits());
        }
        step(r.hub_mismatch_s.to_bits());
    }
    openmobisim_io_parquet::write_parking_bins(
        dir.join("parking_bins.parquet"),
        "toy-parking-probe",
        &parking,
        p,
        &description,
    )
    .expect("write parking bins");
    println!("parking_fingerprint   {}", description.fingerprint_hex());
    println!("parking_arrivals      {} car, {} bike", p.by_kind[0].arrivals, p.by_kind[1].arrivals);
    println!("parking_digest        {digest:016x}");
}
