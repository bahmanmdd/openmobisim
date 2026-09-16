//! Performance floor for the level-4 loading (a loose budget on
//! every hot path, enforced in release only — see `core-types`' `tests/perf.rs`).
//!
//! The budget is an order of magnitude above what a laptop needs, so it catches
//! a regression to per-link sweeps or per-step allocation, not noise.

use std::time::{Duration as Wall, Instant};

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, Vehicle, run_ltm};
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

#[test]
fn a_city_sized_grid_loads_an_hour_within_budget() {
    let n = 100u32;
    let (network, _) = manhattan_grid(n, 200.0, true);
    let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
    let link = |a: (u32, u32), z: (u32, u32)| {
        let ext = format!("l_{}__{}", node_name(a.0, a.1), node_name(z.0, z.1));
        network.link_external_ids().typed_id_of::<LinkId>(&ext).expect("grid link")
    };
    // 80 000 trips of up to 16 blocks, departing over 45 minutes.
    let vehicles: Vec<Vehicle> = (0..80_000u32)
        .map(|i| {
            let (r0, c0) = ((i * 7) % (n - 1), (i * 13) % n);
            let (r1, c1) = ((r0 + 1 + i % 8).min(n - 1), (c0 + i % 9).min(n - 1));
            let mut route = Vec::new();
            let (mut r, mut c) = (r0, c0);
            while c != c1 {
                route.push(link((r, c), (r, c + 1)));
                c += 1;
            }
            while r != r1 {
                route.push(link((r, c), (r + 1, c)));
                r += 1;
            }
            Vehicle::new(VehicleId::new(i), route, Pcu(1.0), Second(i % 2700))
        })
        .collect();
    let crossings: usize = vehicles.iter().map(|v| v.route.len()).sum();

    let start = Instant::now();
    let done = run_ltm(
        &network,
        &turns,
        &vehicles,
        Duration(3600.0),
        Duration(300.0),
        FidelityLevel::Full,
    );
    let elapsed = start.elapsed();
    println!(
        "{} links, {} vehicles, {crossings} crossings: {elapsed:?}, {} completed",
        network.link_count(),
        vehicles.len(),
        done.len()
    );
    assert!(!done.is_empty());
    if cfg!(debug_assertions) {
        return;
    }
    let budget = Wall::from_secs(10);
    assert!(elapsed <= budget, "loading took {elapsed:?}, over its {budget:?} budget");
}
