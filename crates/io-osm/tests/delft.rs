//! The Delft extract (`fixtures/delft/network.osm.pbf`, 17 976 211 bytes, © OpenStreetMap
//! contributors, ODbL) read in full, and 20 of its ways against `fixtures/delft/ways.csv`,
//! written by `delft_ways.py` from libosmium (pyosmium 4.3.2: tags and node positions) and
//! pyproj 3.8.0 (each way's length, Karney's geodesic). Per way and direction: the modes with a
//! link along its first segment (a mode the tags admit may be missing only where its layer's
//! strong component dropped the whole neighbourhood: no link of the mode touches the way), the
//! car's lanes and free-flow speed there, and, where the links along the way hold nothing but the
//! way (contraction merges most ways with their neighbours), their summed length. The cases include both fixed bugs (`lanes=3` with one side
//! given; `maxspeed:forward`/`:backward`) and, for bikes, a one-way street open to bikes both
//! ways, `bicycle=no`, `use_sidepath`, a cycleway, a one-way cycleway, a footway with and
//! without `bicycle=yes`, a path (walkers alone: version 1's rule).
//!
//! The read time is printed. In a release build it must be under 3 s (version 1: 2.0 s), a budget
//! judged on the developer's machine only (where the environment variable CI is set, it is not
//! judged): `cargo test --release -p openmobisim-io-osm --test delft -- --ignored --nocapture`
//! (release only: ignored in the debug suite).

mod common;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use common::{VALUES, close};
use openmobisim_graph::geometry::{LonLat, Xy};
use openmobisim_graph::network::Network;
use openmobisim_io_osm::{Osm, network, read_pbf};
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;

/// Each link as the OpenStreetMap nodes it passes (traced back through the network's own
/// projection), the link of each mode along each directed segment, and the nodes each mode's
/// links touch.
type Traced = (HashMap<(Mode, i64, i64), LinkId>, HashMap<LinkId, Vec<i64>>, HashSet<(Mode, i64)>);

fn traced(net: &Network, osm: &Osm) -> Traced {
    let projection = net.projection().expect("longitude/latitude");
    let of: HashMap<(u64, u64), i64> = osm
        .nodes
        .iter()
        .map(|n| {
            let xy = projection.project(LonLat::new(n.lon, n.lat));
            ((xy.x.to_bits(), xy.y.to_bits()), n.id)
        })
        .collect();
    let node = |xy: Xy| of[&(xy.x.to_bits(), xy.y.to_bits())];
    let mut along: HashMap<(Mode, i64, i64), LinkId> = HashMap::new();
    let mut points: HashMap<LinkId, Vec<i64>> = HashMap::new();
    let mut touching: HashSet<(Mode, i64)> = HashSet::new();
    for l in LinkId::iter_space(net.link_count()) {
        let mut ids = vec![node(net.node_position(net.upstream(l)))];
        ids.extend(net.shape(l).iter().map(|&xy| node(xy)));
        ids.push(node(net.node_position(net.downstream(l))));
        for mode in
            [Mode::Car, Mode::Bike, Mode::Walk].into_iter().filter(|&m| net.modes(l).contains(m))
        {
            for s in ids.windows(2) {
                along.insert((mode, s[0], s[1]), l);
            }
            touching.extend(ids.iter().map(|&n| (mode, n)));
        }
        points.insert(l, ids);
    }
    (along, points, touching)
}

#[test]
#[ignore = "release only: about 7 s in debug; CI runs it in release with --ignored"]
fn delft_reads_in_full_and_twenty_ways_match_libosmium_and_pyproj() {
    let started = Instant::now();
    let osm = read_pbf(Path::new("tests/fixtures/delft/network.osm.pbf")).expect("Delft reads");
    let (net, report) = network(&osm, VALUES, None).expect("Delft builds");
    let seconds = started.elapsed().as_secs_f64();
    println!("Delft: {report:?}, read in {seconds:.2} s");
    assert!(report.nodes > 0 && report.links > 0);
    if !cfg!(debug_assertions) && std::env::var_os("CI").is_none() {
        assert!(seconds < 3.0, "Delft read in {seconds:.2} s, the budget is 3 s");
    }

    let (along, points, touching) = traced(&net, &osm);
    let ways: HashMap<i64, &Vec<i64>> = osm.ways.iter().map(|w| (w.id, &w.nodes)).collect();

    let csv = std::fs::read_to_string("tests/fixtures/delft/ways.csv").expect("the fixture");
    let (mut rows, mut lengths, mut islands, mut worst) = (0, 0, 0, 0.0f64);
    for line in csv.lines().skip(1) {
        let cell: Vec<&str> = line.split(',').collect();
        let way: i64 = cell[0].parse().unwrap();
        let pyproj: f64 = cell[2].parse().unwrap();
        let mut nodes: Vec<i64> = ways[&way].clone();
        for (tag, at) in [("f", 3), ("b", 6)] {
            let case = format!("way {way} ({}) {tag}", cell[1]);
            let segments: Vec<(i64, i64)> = nodes.windows(2).map(|s| (s[0], s[1])).collect();
            let link = |mode| along.get(&(mode, segments[0].0, segments[0].1)).copied();
            let got: String = [(Mode::Car, 'c'), (Mode::Bike, 'b'), (Mode::Walk, 'w')]
                .iter()
                .filter(|(m, _)| link(*m).is_some())
                .map(|&(_, c)| c)
                .collect();
            // A mode the tags admit and the network lacks: the way's layer dropped it with every
            // street around it (an island outside the layer's strong component): no link of
            // that mode touches any of the way's nodes.
            let want = cell[at].replace('-', "");
            assert!(got.chars().all(|c| want.contains(c)), "{case}: modes {got}, expected {want}");
            for (mode, c) in [(Mode::Car, 'c'), (Mode::Bike, 'b'), (Mode::Walk, 'w')] {
                if want.contains(c) && !got.contains(c) {
                    let near = nodes.iter().any(|n| touching.contains(&(mode, *n)));
                    assert!(!near, "{case}: no {mode} link, but its nodes are on {mode} links");
                    println!("{case}: {mode} dropped with every street around it (an island)");
                    islands += 1;
                }
            }
            if let Some(car) = link(Mode::Car) {
                assert_eq!(Some(net.lanes(car)), cell[at + 1].parse().ok(), "{case}: lanes");
                let speed = net.free_flow_speed(car, Mode::Car).0 * 3.6;
                let want: f64 = cell[at + 2].parse().unwrap();
                assert!(close(speed, want, 1e-9), "{case}: {speed} km/h, expected {want}");
            }
            // The length, where the links along the way hold nothing else.
            if let Some(mode) =
                [Mode::Car, Mode::Bike, Mode::Walk].into_iter().find(|&m| link(m).is_some())
            {
                let mut covering: Vec<LinkId> = segments
                    .iter()
                    .filter_map(|&(a, b)| along.get(&(mode, a, b)).copied())
                    .collect();
                covering.dedup();
                let inside = covering
                    .iter()
                    .all(|l| points[l].windows(2).all(|s| segments.contains(&(s[0], s[1]))));
                if inside && covering.len() <= segments.len() {
                    let length: f64 = covering.iter().map(|&l| net.length(l).0).sum();
                    worst = worst.max((length - pyproj).abs());
                    assert!(close(length, pyproj, 1e-6), "{case}: {length} m, pyproj {pyproj} m");
                    lengths += 1;
                }
            }
            nodes.reverse();
        }
        rows += 1;
    }
    assert_eq!(rows, 20);
    println!(
        "20 ways: {lengths} of 40 directions held by links of their own; largest length difference \
         from pyproj {worst:.2e} m; {islands} mode(s) dropped as islands"
    );
}
