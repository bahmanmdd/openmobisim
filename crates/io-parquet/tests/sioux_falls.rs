//! Sioux Falls's run written and timed: the loop's regular test case (test 6's regular tier:
//! TNTP's network with the capacities at a tenth, three hours of every OD pair at 0.045 of the
//! TNTP demand per hour, the loop's fixtures read in place), its three tables written; the row
//! counts must be the run's, the write under 1 s in a release build. The sizes and the time are
//! printed:
//!
//! ```text
//! cargo test --release -p openmobisim-io-parquet --test sioux_falls -- --ignored --nocapture
//! ```

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "trip counts and seconds of the window, far below 2^53 and above zero"
)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{LonLat, Position, Projection, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_io_parquet::write;
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// The loop's Sioux Falls fixtures.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../loop/tests/fixtures/sioux_falls")
}

/// The fields of the lines of `file` split at `separator`, `;` removed.
fn lines(file: &str, separator: Option<char>) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(fixtures().join(file)).expect("a fixture");
    let split = |l: &str| -> Vec<String> {
        let l = l.replace(';', "");
        match separator {
            Some(c) => l.split(c).map(str::to_string).collect(),
            None => l.split_whitespace().map(str::to_string).collect(),
        }
    };
    text.lines().map(split).collect()
}

/// TNTP's rows: at least `fields` fields, the first a number.
fn tntp(file: &str, fields: usize) -> Vec<Vec<String>> {
    let rows = lines(file, None);
    rows.into_iter().filter(|f| f.len() >= fields && f[0].parse::<u32>().is_ok()).collect()
}

/// As the loop's test 6: a free-flow minute as a kilometre at 60 km/h, the capacity at a tenth,
/// the nodes projected to metres.
fn network() -> Network {
    let nodes: Vec<(String, LonLat)> = tntp("SiouxFalls_node.tntp", 3)
        .into_iter()
        .map(|f| (f[0].clone(), LonLat::new(f[1].parse().expect("x"), f[2].parse().expect("y"))))
        .collect();
    let projection = Projection::for_extent(nodes.iter().map(|n| n.1)).expect("a zone");
    let mut b = NetworkBuilder::new();
    for (id, p) in &nodes {
        b.add_node(id.as_str(), Position::Metres(projection.project(*p)));
    }
    for f in tntp("SiouxFalls_net.tntp", 10) {
        let (cap, min): (f64, f64) = (f[2].parse().expect("cap"), f[4].parse().expect("min"));
        let row = LinkRow {
            modes: ModeSet::of(&[Mode::Car]),
            lanes: Some(1),
            car_speed_km_h: Some(60.0),
            length_m: Some(min * 1_000.0),
            capacity_veh_h: Some(cap * 0.1),
            signal_at_downstream_end: false,
        };
        b.add_link(format!("{}-{}", f[0], f[1]), f[0].as_str(), f[1].as_str(), row);
    }
    let values = BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };
    b.build(values).expect("a network")
}

/// As the loop's test 6: every OD pair of `equilibrium.csv` at its rate for three hours from 0,
/// spaced regularly, the ends at the zones' nodes.
fn demand(network: &Network) -> Demand {
    let node = |id: &str| {
        let n = network.node_ids().typed_id_of(id).expect("a node");
        let p = network.node_position(n);
        Position::Metres(Xy::new(p.x, p.y))
    };
    let mut rows = Vec::new();
    for f in lines("equilibrium.csv", Some(',')).into_iter().filter(|f| f[0] == "od") {
        let rate: f64 = f[3].parse().expect("a rate");
        for i in 0..(rate * 3.0).round() as usize {
            rows.push(TripRow {
                id: format!("{}-{}-{i:06}", f[1], f[2]),
                origin: node(&f[1]),
                destination: node(&f[2]),
                departure: Second((i as f64 * 3_600.0 / rate).floor() as u32),
                class: "default".to_string(),
                modes: ModeSet::of(&[Mode::Car]),
            });
        }
    }
    Demand::build(&rows, &["default".to_string()], network, None, Metres(1_500.0))
        .expect("a demand")
}

#[test]
#[ignore = "a run of about 11 s in debug, outside the 2-minute suite; CI runs it in release"]
fn sioux_falls_s_run_is_written_in_under_1_s() {
    let net = network();
    let dem = demand(&net);
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 100,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![[
            1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ]],
        ..Parameters::defaults(1)
    };
    let started = Instant::now();
    let r = run(&net, &dem, None, (Second(0), Second(4 * 3_600)), &parameters).expect("a run");
    let ran = started.elapsed().as_secs_f64();
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join("io-parquet-sioux-falls");
    let started = Instant::now();
    let w = write(&r, &net, &dem, &folder).expect("written");
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "{} trips, {} iterations: run in {ran:.1} s, written in {seconds:.3} s",
        r.trips.class.len(),
        r.iterations.len()
    );
    for t in [&w.trips, &w.links, &w.iterations] {
        let bytes = std::fs::metadata(&t.path).expect("a file").len();
        println!("  {}: {} rows, {bytes} bytes", t.path.display(), t.rows);
    }
    assert_eq!(w.trips.rows, dem.trip_count() as usize);
    assert_eq!(w.links.rows, r.links.len());
    assert_eq!(w.iterations.rows, r.iterations.len());
    if !cfg!(debug_assertions) {
        assert!(seconds < 1.0, "written in {seconds:.3} s, the budget is 1 s");
    }
}
