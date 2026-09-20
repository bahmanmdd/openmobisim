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
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_sim::Run;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_io_parquet::manifest::Manifest;
use openmobisim_io_parquet::{write_diagnostics, write_events, write_kpis, write_manifest};

/// The N3 fixture's own parameters (`06_INTERFACE_V0.md` §3):
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
    let mut sampled_run = Run::new(network.clone(), travellers.clone(), trips, window)
        .with_master_seed(20_260_921)
        .with_choice_model(sampled);
    let sampled_description = sampled_run.description();
    let sampled_result = sampled_run.execute(&mut Diagnostics::new());
    let mut choice_digest = 0u64;
    for r in &sampled_result.route_choices.as_ref().expect("recorded").route {
        choice_digest = choice_digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(u64::from(*r));
    }

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
    println!("choice_total_time_s   {:016x}", sampled_result.total_travel_time.get().to_bits());

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
}
