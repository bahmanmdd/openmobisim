//! A cold-run benchmark of the level 2–4 loading on a real `.osm.pbf` extract
//! (S91: every performance number is for a cold run).
//!
//! The demand is a **load fixture, not a forecast**: random origins, each
//! routed along the shortest free-flow path (respecting the turn table, over
//! links that carry motor traffic) to a random link reachable within
//! `max_route_seconds`, departing uniformly over the first three quarters of
//! the window. It exists to measure cost and to
//! exercise the loading on real link-length distributions — short links,
//! signals, one-way streets — not to say anything about a place.
//!
//! ```text
//! OPENMOBISIM_TEST_PBF=luxembourg-latest.osm.pbf \
//!   cargo run -p openmobisim-core-loading --release --example extract_benchmark -- \
//!   [trips_per_link_per_hour=0.25] [traveller_weight=1] [level=full|spatial|point] [max_route_seconds=300]
//! ```

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::Instant;

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, Vehicle, run_ltm};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};
use openmobisim_io_osm::{ImportOptions, PbfSource, import};

const WINDOW_S: f64 = 3600.0;
const STEP_S: f64 = 300.0;

/// A small deterministic generator: the fixture must be the same every run.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, bound: usize) -> usize {
        self.0 =
            self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        #[allow(clippy::cast_possible_truncation, reason = "the modulus is below `bound`, a usize")]
        let r = ((self.0 >> 33) % bound as u64) as usize;
        r
    }
}

fn arg<T: std::str::FromStr>(n: usize, default: T) -> T {
    std::env::args().nth(n).and_then(|a| a.parse().ok()).unwrap_or(default)
}

#[allow(clippy::too_many_lines, reason = "a linear benchmark script")]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts and percentiles for printing"
)]
fn main() {
    let Ok(path) = std::env::var("OPENMOBISIM_TEST_PBF") else {
        eprintln!("set OPENMOBISIM_TEST_PBF to a .osm.pbf file");
        return;
    };
    let trips_per_link_hour: f64 = arg(1, 0.25);
    let weight: u32 = arg(2, 1);
    let level = match std::env::args().nth(3).as_deref() {
        Some("point") => FidelityLevel::PointQueue,
        Some("spatial") => FidelityLevel::SpatialQueue,
        _ => FidelityLevel::Full,
    };
    let max_route: f64 = arg(4, 300.0);

    let start = Instant::now();
    let (network, _, _) =
        import(&PbfSource::new(&path), ImportOptions::default(), &mut Diagnostics::new())
            .expect("the extract imports");
    let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
    let free_flow: Vec<f64> = network.free_flow_times().iter().map(|d| d.get()).collect();
    println!(
        "{path}: {} links, {} nodes, imported in {:.1?}",
        network.link_count(),
        network.node_count(),
        start.elapsed()
    );

    let mut sorted = free_flow.clone();
    sorted.sort_by(f64::total_cmp);
    let share_below =
        |t: f64| 100.0 * sorted.partition_point(|&x| x < t) as f64 / sorted.len() as f64;
    println!(
        "  link free-flow time: median {:.1} s; under 1 s {:.1}%, under 5 s {:.1}%",
        sorted[sorted.len() / 2],
        share_below(1.0),
        share_below(5.0)
    );

    let start = Instant::now();
    let trips = (trips_per_link_hour * f64::from(network.link_count())).round() as usize;
    let vehicles =
        fixture(&network, &turns, &free_flow, trips / weight.max(1) as usize, weight, max_route);
    let crossings: usize = vehicles.iter().map(|v| v.route.len()).sum();
    println!(
        "  {} vehicles of weight {weight}, {crossings} link crossings, built in {:.1?}",
        vehicles.len(),
        start.elapsed()
    );

    let start = Instant::now();
    let done = run_ltm(&network, &turns, &vehicles, Duration(WINDOW_S), Duration(STEP_S), level);
    let elapsed = start.elapsed();

    let mut delay: Vec<f64> = done
        .iter()
        .map(|t| {
            let v = &vehicles[t.vehicle.raw() as usize];
            f64::from(t.arrival().get() - t.departure().get())
                - v.route.iter().map(|l| free_flow[l.index()]).sum::<f64>()
        })
        .collect();
    delay.sort_by(f64::total_cmp);
    let pct =
        |p: f64| delay.get(((delay.len() as f64 - 1.0) * p) as usize).copied().unwrap_or(f64::NAN);
    println!(
        "  {level:?}: loaded {WINDOW_S} s in {elapsed:.2?} ({:.0} ns per crossing)",
        elapsed.as_secs_f64() * 1e9 / crossings.max(1) as f64
    );
    println!(
        "  completed {:.1}%; delay over free flow p50 {:.1} s, p90 {:.1} s, p99 {:.1} s",
        100.0 * done.len() as f64 / vehicles.len().max(1) as f64,
        pct(0.5),
        pct(0.9),
        pct(0.99)
    );
}

/// Shortest free-flow routes over the turn table and motor-traffic links, from
/// random nodes to random reachable links.
fn fixture(
    network: &RoadNetwork,
    turns: &TurnTable,
    free_flow: &[f64],
    count: usize,
    weight: u32,
    max_route: f64,
) -> Vec<Vehicle> {
    let n_links = network.link_count() as usize;
    let mut rng = Lcg(42);
    let mut dist = vec![f64::INFINITY; n_links];
    let mut pred = vec![u32::MAX; n_links];
    let mut touched = Vec::new();
    let mut vehicles = Vec::with_capacity(count);
    let mut attempts = 0;
    while vehicles.len() < count && attempts < count * 20 {
        attempts += 1;
        for &t in &touched {
            dist[t] = f64::INFINITY;
            pred[t] = u32::MAX;
        }
        touched.clear();
        let origin = NodeId::from_index(rng.below(network.node_count() as usize));
        let mut heap = BinaryHeap::new();
        for &l in network.out_links(origin) {
            if !network.link_class(l).carries_motor_traffic() {
                continue;
            }
            dist[l.index()] = free_flow[l.index()];
            touched.push(l.index());
            heap.push(Reverse((free_flow[l.index()].to_bits(), l.index())));
        }
        let mut settled = Vec::new();
        while let Some(Reverse((bits, u))) = heap.pop() {
            let d = f64::from_bits(bits);
            if d > dist[u] {
                continue;
            }
            settled.push(u);
            for &t in turns.turns_from(LinkId::from_index(u)) {
                let v = turns.outgoing(t).index();
                if !network.link_class(LinkId::from_index(v)).carries_motor_traffic() {
                    continue;
                }
                let nd = d + free_flow[v];
                if nd <= max_route && nd < dist[v] {
                    if dist[v].is_infinite() {
                        touched.push(v);
                    }
                    dist[v] = nd;
                    pred[v] = u32::try_from(u).expect("link ids are u32");
                    heap.push(Reverse((nd.to_bits(), v)));
                }
            }
        }
        if settled.len() < 2 {
            continue;
        }
        let half = settled.len() / 2;
        let mut link = settled[half + rng.below(settled.len() - half)];
        let mut route = vec![LinkId::from_index(link)];
        while pred[link] != u32::MAX {
            link = pred[link] as usize;
            route.push(LinkId::from_index(link));
        }
        route.reverse();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a draw below 10^6, scaled into the window"
        )]
        let departure = Second((rng.below(1_000_000) as f64 / 1e6 * WINDOW_S * 0.75) as u32);
        let id = VehicleId::new(u32::try_from(vehicles.len()).expect("vehicle count fits u32"));
        vehicles.push(Vehicle::new(id, route, Pcu(f64::from(weight)), departure));
    }
    vehicles
}
