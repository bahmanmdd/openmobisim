//! Phase 2 item 2's cold-run CPU benchmark, on the N3 fixture (S84, S91).
//!
//! The roadmap asks for two things before the iterative LTM prototype
//! (`crate::ltm`) counts as de-risked: that vehicle hand-over survives
//! crossing several links within one loading step on a real-shaped network
//! (`crate::ltm`'s own tests cover that on hand-built fixtures), and a
//! cold-run CPU number against N3 (`06_INTERFACE_V0.md` §3:
//! `manhattan_grid(n=5, block_metres=200, signals=True)`, the same
//! parameters `io-parquet`'s `kpis_determinism_probe` uses). This is that
//! second thing.
//!
//! S91: **every published performance number is for a cold run** — this
//! process does the one run and exits; nothing here warm-starts.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p openmobisim-core-loading --release --example n3_ltm_benchmark
//! ```

use std::time::Instant;

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, Vehicle, run_ltm};
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

/// N3's own parameters.
const GRID_N: u32 = 5;
const BLOCK_METRES: f64 = 200.0;

/// The loading step S84/S89 default.
const STEP_S: f64 = 300.0;

/// Long enough that every vehicle below has room to complete even under
/// congestion.
const WINDOW_S: f64 = 7200.0;

/// One vehicle per second of a two-hour morning peak, spread across every
/// corner-to-corner route the grid offers — enough load that the node
/// model's proportional-distribution and full-blocking paths (S48/S77) are
/// actually exercised, not just free-flow traversal.
const VEHICLE_COUNT: u32 = 2000;

fn main() {
    let (network, _diagnostics) = manhattan_grid(GRID_N, BLOCK_METRES, true);
    let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);

    // Two corner-to-corner "L" routes (right-then-down, down-then-right),
    // hand-built the way every core-loading fixture is (S133: this crate
    // never routes).
    let routes = corner_routes(&network);

    let vehicles: Vec<Vehicle> = (0..VEHICLE_COUNT)
        .map(|i| {
            let route = routes[i as usize % routes.len()].clone();
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "i and VEHICLE_COUNT are small and non-negative by construction"
            )]
            let departure = Second((f64::from(i) * (WINDOW_S / f64::from(VEHICLE_COUNT))) as u32);
            Vehicle::new(VehicleId::new(i), route, Pcu(1.0), departure)
        })
        .collect();

    let start = Instant::now();
    let trajectories = run_ltm(
        &network,
        &turns,
        &vehicles,
        Duration(WINDOW_S),
        Duration(STEP_S),
        FidelityLevel::Full,
    );
    let elapsed = start.elapsed();

    let link_count = network.link_count();
    let node_count = network.node_count();
    println!("N3 cold-run LTM benchmark (S84/S91)");
    println!("  grid: {GRID_N}x{GRID_N}, {node_count} nodes, {link_count} links");
    println!("  vehicles departed: {VEHICLE_COUNT}, window: {WINDOW_S}s, step: {STEP_S}s");
    println!("  completed trajectories: {}", trajectories.len());
    #[allow(clippy::cast_precision_loss, reason = "a trajectory count in the low thousands")]
    let completed = trajectories.len() as f64;
    println!("  completion rate: {:.1}%", 100.0 * completed / f64::from(VEHICLE_COUNT));
    println!("  elapsed: {elapsed:?}");
    #[allow(clippy::cast_precision_loss, reason = "vehicle count is small")]
    let per_vehicle_us = elapsed.as_secs_f64() * 1e6 / f64::from(VEHICLE_COUNT);
    println!("  per departed vehicle: {per_vehicle_us:.1} us");
}

/// Two "L"-shaped corner-to-corner routes: right along row 0 then down the
/// last column, and down the first column then right along the last row.
fn corner_routes(network: &openmobisim_core_graph::network::RoadNetwork) -> Vec<Vec<LinkId>> {
    let n = GRID_N;
    let link = |a: String, z: String| -> LinkId {
        let external = format!("l_{a}__{z}");
        network.link_external_ids().typed_id_of::<LinkId>(&external).expect("known link")
    };

    let mut right_then_down = Vec::new();
    for c in 0..n - 1 {
        right_then_down.push(link(node_name(0, c), node_name(0, c + 1)));
    }
    for r in 0..n - 1 {
        right_then_down.push(link(node_name(r, n - 1), node_name(r + 1, n - 1)));
    }

    let mut down_then_right = Vec::new();
    for r in 0..n - 1 {
        down_then_right.push(link(node_name(r, 0), node_name(r + 1, 0)));
    }
    for c in 0..n - 1 {
        down_then_right.push(link(node_name(n - 1, c), node_name(n - 1, c + 1)));
    }

    vec![right_then_down, down_then_right]
}
