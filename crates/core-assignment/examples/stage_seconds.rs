//! The run time of each stage of the assignment, iteration by iteration, on two small cases
//! of the point queue (level 2), and a digest of every trip's choice set and chosen chain.
//!
//! The cases are those of the crate's level-2 tests: **two routes** from `o` to `d`, the short
//! one (2 × 1 440 m at 36 km/h) behind a bottleneck of 1 200 veh/h, the long one
//! (2 × 2 305 m) of 1 800 veh/h, with 1 800 cars an hour for an hour; and **the Braess
//! network** with its middle link (`s → a` and `b → t` 1 km at 1 000 veh/h, `a → t` and
//! `s → b` 10 km, `a → b` 1 km, all at 60 km/h), 1 500 cars an hour for three hours. Each runs
//! exactly 100 assignment iterations with the deterministic choice (`braess-logit`: the logit
//! instead), seed 2026.
//!
//! ```text
//! cargo run --release -p openmobisim-core-assignment --example stage_seconds -- <case> [digest file]
//! ```
//!
//! `<case>` is `two-routes`, `braess` or `braess-logit`. The digest file, if named, receives
//! one line per trip (its chosen chain's index, the identities of its set's chains in order,
//! the chosen chain's in-vehicle seconds as bits) and one line per iteration (the convergence
//! measures as bits), so two builds can be compared for identical results.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::print_stdout,
    reason = "an example: small counts, printed"
)]

use std::fmt::Write as _;
use std::time::Instant;

use openmobisim_core_assignment::{
    Coefficients, HighestUtility, Inputs, NoHooks, Params, PointQueue, assign, logit,
};
use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::rng::RngKey;
use openmobisim_core_types::time::Second;

/// A link: name, upstream node, downstream node, metres, km/h, veh/h, lanes.
type LinkDef = (&'static str, &'static str, &'static str, f64, f64, f64, u8);

fn network(nodes: &[(&str, f64, f64)], links: &[LinkDef]) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for &(n, x, y) in nodes {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for &(name, from, to, metres, km_h, veh_h, lanes) in links {
        let mut spec = LinkSpec::new(RoadClass::Secondary);
        spec.length_m = Some(metres);
        spec.maxspeed_km_h = Some(km_h);
        spec.capacity_veh_h = Some(veh_h);
        spec.lanes = Some(lanes);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn two_routes() -> RoadNetwork {
    network(
        &[("o", 0.0, 0.0), ("a", 1400.0, 300.0), ("b", 1400.0, -1700.0), ("d", 2800.0, 0.0)],
        &[
            ("oa", "o", "a", 1440.0, 36.0, 1200.0, 1),
            ("ad", "a", "d", 1440.0, 36.0, 100_000.0, 3),
            ("ob", "o", "b", 2305.0, 36.0, 1800.0, 1),
            ("bd", "b", "d", 2305.0, 36.0, 1800.0, 1),
        ],
    )
}

fn braess() -> RoadNetwork {
    network(
        &[("s", 0.0, 0.0), ("a", 500.0, 400.0), ("b", 500.0, -400.0), ("t", 1000.0, 0.0)],
        &[
            ("sa", "s", "a", 1000.0, 60.0, 1000.0, 1),
            ("bt", "b", "t", 1000.0, 60.0, 1000.0, 1),
            ("at", "a", "t", 10_000.0, 60.0, 6000.0, 3),
            ("sb", "s", "b", 10_000.0, 60.0, 6000.0, 3),
            ("ab", "a", "b", 1000.0, 60.0, 6000.0, 3),
        ],
    )
}

/// Cars leaving at `per_hour` a hour for `seconds`: the k-th at `⌊k · 3600 / per_hour⌋`.
fn trips(net: &RoadNetwork, from: &str, to: &str, per_hour: u64, seconds: u64) -> Vec<RawTrip> {
    let at = |name: &str| {
        net.node_lonlat(net.node_external_ids().typed_id_of(name).expect("a node of the case"))
    };
    (0..per_hour * seconds / 3600)
        .map(|k| RawTrip {
            traveller_id: format!("t{k:05}"),
            trip_seq: 0,
            origin: at(from),
            destination: at(to),
            departure_time: Second((k * 3600 / per_hour) as u32),
            user_class: "everyone".to_string(),
            weight: Some(1),
            mode: None,
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let case = args.get(1).map_or("two-routes", String::as_str);
    let (net, raw) = match case {
        "two-routes" => {
            let net = two_routes();
            let raw = trips(&net, "o", "d", 1800, 3600);
            (net, raw)
        }
        "braess" | "braess-logit" => {
            let net = braess();
            let raw = trips(&net, "s", "t", 1500, 3 * 3600);
            (net, raw)
        }
        other => panic!("unknown case {other}: two-routes, braess or braess-logit"),
    };
    let owners = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &owners, 1, &mut Diagnostics::new()).expect("demand");
    let turns = TurnTable::build(&net, net.signals());
    let money = vec![0.0; net.link_count() as usize];
    let classes = vec![Coefficients::time_based()];
    let loader = PointQueue::after_last_departure(&turns, &trips, 3600.0);
    let the_logit = logit();
    let model: &dyn ChoiceModel = if case == "braess-logit" { &the_logit } else { &HighestUtility };
    let inputs = Inputs {
        network: &net,
        turns: &turns,
        travellers: &travellers,
        trips: &trips,
        classes: &classes,
        money_per_link: &money,
        model,
        loader: &loader,
        seed: RngKey::from_seed(2026),
        initial: None,
    };
    let params = Params { min_iterations: 100, max_iterations: 100, ..Params::default() };
    let clock = Instant::now();
    let (state, reports) = assign(&inputs, &params, &mut NoHooks).expect("the run");
    let seconds = clock.elapsed().as_secs_f64();
    println!("iteration,search_s,choose_s,load_s,measure_s,assess_s");
    for r in &reports {
        let s = r.stage_seconds;
        println!(
            "{},{:.6},{:.6},{:.6},{:.6},{:.6}",
            r.iteration, s.search, s.choose, s.load, s.measure, s.assess
        );
    }
    let sum = |f: fn(&openmobisim_core_assignment::StageSeconds) -> f64| {
        reports.iter().map(|r| f(&r.stage_seconds)).sum::<f64>()
    };
    let stages = [
        ("search", sum(|s| s.search)),
        ("choose", sum(|s| s.choose)),
        ("load", sum(|s| s.load)),
        ("measure", sum(|s| s.measure)),
        ("assess", sum(|s| s.assess)),
    ];
    let staged: f64 = stages.iter().map(|s| s.1).sum();
    let mut line =
        format!("{case}: {} trips, {} iterations in {seconds:.4} s", trips.len(), reports.len());
    for (name, s) in stages {
        write!(line, "; {name} {s:.4} s ({:.1} %)", 100.0 * s / staged).expect("a string");
    }
    let last = reports.last().expect("reports");
    write!(
        line,
        "; last iteration: mean trip {:.3} s, duality gap {:.6}",
        last.mean_trip_seconds, last.duality_gap
    )
    .expect("a string");
    println!("{line}");
    if let Some(path) = args.get(2) {
        let mut out = String::new();
        for t in state.trips() {
            let ids: Vec<String> = t.set.iter().map(|c| format!("{:x}", c.identity())).collect();
            let seconds = t.current_chain().map_or(0, |c| c.total().in_vehicle_seconds.to_bits());
            writeln!(out, "trip {:?} [{}] {seconds:x}", t.current, ids.join(" "))
                .expect("a string");
        }
        for r in &reports {
            writeln!(
                out,
                "iteration {} found {} switch {:x} floor {:x} gap {:x} deterministic {:x} \
                 cheapest {:x} mean {:x} departed {:x} arrived {:x}",
                r.iteration,
                r.found,
                r.switch_share.to_bits(),
                r.switch_floor.to_bits(),
                r.duality_gap.to_bits(),
                r.gap_deterministic.to_bits(),
                r.on_cheapest_share.to_bits(),
                r.mean_trip_seconds.to_bits(),
                r.departed.to_bits(),
                r.arrived.to_bits(),
            )
            .expect("a string");
        }
        std::fs::write(path, out).expect("the digest written");
    }
}
