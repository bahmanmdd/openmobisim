//! A run's choices must not depend on the ids of transit stops, lines, runs and parkings (S312).
//!
//! **The case.** The toy network with its tram `T1` between the stops at `N1` and `D2` and its
//! three parkings (a car park and a bike parking at the tram's stop at `D2`, a car park `P2` at
//! `R2`, 225 s on foot from the stop), built twice: under the toy's names, and with every stop,
//! line, run and parking renamed so that their ids, which follow the sorted names, come in
//! another order. The coordinates, the times and the capacities are the same. Each trip must
//! make the same choice under both namings, read back by where things are: its mode, the point
//! of the parking it uses, and its arrival.
//!
//! - **Mode choice, logit** (`modes_and_parkings_do_not_follow_the_names`): 1 000 travellers
//!   `W → N1`, one a second from 0, choose among the bike, the walk, park-and-ride at either car
//!   park and bike-and-ride (the five alternatives of `toy_modes.rs`). Each traveller's choice
//!   is a random draw per alternative, addressed by the alternative's identity; before S312 the
//!   identity hashed the parking's index and the stops' and the line's ids, so renaming redrew
//!   the errors of every transit alternative: 134 of the 1 000 chose otherwise.
//! - **A tie, deterministic** (`a_tie_between_two_car_parks_does_not_follow_the_names`):
//!   `W → N1` by park-and-ride at 0 reaches `N1` at 1 200 s through either car park (PR3 of
//!   `toy_parking.rs`); the tie went to the parking of the lower index, which followed the
//!   parkings' and hubs' ids. It now goes to the car park first by where it is (the one further
//!   west, then south).

#![allow(clippy::float_cmp, reason = "arrivals are whole seconds")]

use std::sync::Arc;

use openmobisim_core_choice::{Logit, Options};
use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::hubs::{ParkingKind, ParkingRow};
use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::{LonLat, toy_network, toy_network_layers};
use openmobisim_core_sim::{
    LayerSetup, ModeDefaults, NO_PARKING, ParkingDefaults, ParkingSetup, Run, RunResult,
    StaticLayers, TransitSetup,
};
use openmobisim_core_transit::{
    ALIGHT, BOARD, CallSpec, RouteSpec, ServiceDate, StopSpec, Timetable, TimetableBuilder,
    TransitDefaults,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;

/// The names of one naming: the stops at `N1` and `D2`, the line, a run's (out or back, its
/// number), and the parkings at `D2` (car, bike) and `R2` with their hubs.
struct Names {
    stop_n1: &'static str,
    stop_d2: &'static str,
    line: &'static str,
    run: fn(bool, u32) -> String,
    parking: [(&'static str, &'static str); 3],
}

/// The toy's own names.
const TOY: Names = Names {
    stop_n1: "N1",
    stop_d2: "H",
    line: "T1",
    run: |out, k| format!("T1:{}:{k:02}", if out { "out" } else { "back" }),
    parking: [("H-car", "H"), ("H-bike", "H"), ("P2", "P2")],
};

/// Every name changed, so every sorted order is reversed: the stops (`s1` at `N1` before `s9`
/// at `D2`, where the toy has `H` before `N1`), the runs (counted down), the parkings (`P2`'s
/// first) and their hubs (the stop's renamed with it).
const RENAMED: Names = Names {
    stop_n1: "s1",
    stop_d2: "s9",
    line: "x",
    run: |out, k| format!("r{:02}{}", 99 - k, if out { "b" } else { "a" }),
    parking: [("q1", "s9"), ("q2", "s9"), ("q0", "h0")],
};

struct Toy {
    road: Arc<RoadNetwork>,
    layers: Arc<StaticLayers>,
    transit: Arc<TransitSetup>,
    parking: Arc<ParkingSetup>,
}

fn at(road: &RoadNetwork, node: &str) -> LonLat {
    road.node_lonlat(road.node_external_ids().typed_id_of(node).expect("a toy node"))
}

/// The toy tram (`openmobisim_core_transit::examples::toy_tram`) under `names`.
fn tram(road: &RoadNetwork, names: &Names) -> Timetable {
    let mut b = TimetableBuilder::new(ServiceDate::parse("20261009").expect("a date"));
    let stop = |b: &mut TimetableBuilder, id: &str, node: &str| {
        b.add_stop(StopSpec {
            external_id: id.to_string(),
            name: String::new(),
            position: at(road, node),
            parent: None,
        })
    };
    let n1 = stop(&mut b, names.stop_n1, "N1");
    let h = stop(&mut b, names.stop_d2, "D2");
    let line = b.add_route(RouteSpec {
        external_id: names.line.into(),
        short_name: String::new(),
        route_type: 0,
    });
    let call =
        |stop: u32, t: u32| CallSpec { stop, arrival: t, departure: t, flags: BOARD | ALIGHT };
    for k in 0..18 {
        let out = k * 600;
        b.add_run((names.run)(true, k), line, &[call(n1, out), call(h, out + 300)]);
        let back = out + 300;
        b.add_run((names.run)(false, k), line, &[call(h, back), call(n1, back + 300)]);
    }
    b.build().0
}

/// The toy's parkings (`toy_network_parkings`) under `names`.
fn parkings(road: &RoadNetwork, names: &Names) -> Vec<ParkingRow> {
    let row = |(id, hub): (&str, &str), node: &str, kind: ParkingKind, capacity: u32| ParkingRow {
        parking_id: id.to_string(),
        name: None,
        hub_id: Some(hub.to_string()),
        position: at(road, node),
        kind,
        capacity,
        initial_occupancy: 0,
        fee_eur: None,
    };
    vec![
        row(names.parking[0], "D2", ParkingKind::Car, 6),
        row(names.parking[1], "D2", ParkingKind::Bike, 3),
        row(names.parking[2], "R2", ParkingKind::Car, 20),
    ]
}

fn toy(names: &Names) -> Toy {
    let (road, _) = toy_network();
    let road = Arc::new(road);
    let (bike, walk) = toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let layers = Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(Arc::new(bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d)),
    });
    let transit = Arc::new(TransitSetup::new(
        Arc::new(tram(&road, names)),
        layers.walk.as_ref().unwrap(),
        layers.bike.as_ref(),
        TransitDefaults { access_walk_max_s: 900.0, ..TransitDefaults::SHIPPED },
    ));
    let defaults = ParkingDefaults { pr_min_km: 0.0, ..ParkingDefaults::SHIPPED };
    let parking = Arc::new(
        ParkingSetup::new(
            &parkings(&road, names),
            road.clone(),
            layers.bike.as_ref(),
            &transit,
            defaults,
        )
        .expect("the toy's parkings"),
    );
    Toy { road, layers, transit, parking }
}

/// Each trip's choice read back by where things are: its mode, its parking's point (none
/// without one), its arrival in whole seconds.
type Choice = (u8, Option<(u64, u64)>, u32);

fn choices(t: &Toy, r: &RunResult) -> Vec<Choice> {
    let it = r.itineraries.as_ref().expect("itineraries");
    let mut arrival: Vec<(u32, u32)> =
        r.events.iter().map(|e| (e.entity_id, e.second.get())).collect();
    arrival.sort_unstable();
    (0..it.trip.len())
        .map(|i| {
            let p = it.parking[i];
            let place = (p != NO_PARKING).then(|| {
                let at = t.parking.position(p);
                (at.lon.to_bits(), at.lat.to_bits())
            });
            let a = arrival.iter().find(|x| x.0 == it.trip[i]).map_or(0, |x| x.1);
            (it.mode[i], place, a)
        })
        .collect()
}

fn run(names: &Names, trips: &dyn Fn(&RoadNetwork) -> Vec<RawTrip>, logit: bool) -> Vec<Choice> {
    let t = toy(names);
    let defaults = ClassDefaults::new()
        .with_default("everyone", Ownership { car: true, bike: true, transit_pass: false });
    let (travellers, trips) =
        build_travellers(trips(&t.road), Vec::new(), &defaults, 1, &mut Diagnostics::new())
            .unwrap();
    // The logit's case is `toy_modes.rs`'s (no access or egress times, its five alternatives);
    // the tie is PR3's (the shipped times: both car parks reach `N1` at 1 200 s).
    let modes = if logit {
        ModeDefaults {
            car_access_s: 0.0,
            car_egress_s: 0.0,
            bike_access_s: 0.0,
            bike_egress_s: 0.0,
            ..ModeDefaults::SHIPPED
        }
    } else {
        ModeDefaults::SHIPPED
    };
    let mut run = Run::new(t.road.clone(), Arc::new(travellers), Arc::new(trips), Second(20_000))
        .with_layers(t.layers.clone())
        .with_mode_defaults(modes)
        .with_choice_detour_limit(1.5)
        .with_mode_choice(&Mode::BASE)
        .with_transit(t.transit.clone())
        .with_parking(t.parking.clone());
    if logit {
        let money: Options = [("beta_cost_eur".to_string(), 0.0)].into();
        run = run.with_choice_model(Arc::new(Logit::from_options(&money).unwrap()));
    }
    let r = run.execute(&mut Diagnostics::new());
    choices(&t, &r)
}

fn trip(road: &RoadNetwork, who: String, t: u32, mode: Option<Mode>) -> RawTrip {
    RawTrip {
        traveller_id: who,
        trip_seq: 0,
        origin: at(road, "W"),
        destination: at(road, "N1"),
        departure_time: Second(t),
        user_class: "everyone".to_string(),
        weight: None,
        mode,
    }
}

#[test]
fn modes_and_parkings_do_not_follow_the_names() {
    let trips = |road: &RoadNetwork| -> Vec<RawTrip> {
        (0..1_000).map(|i| trip(road, format!("p{i:04}"), i, None)).collect()
    };
    let (a, b) = (run(&TOY, &trips, true), run(&RENAMED, &trips, true));
    assert_eq!(a.len(), 1_000);
    // The choice is a real one: more than one mode is taken.
    let modes: std::collections::BTreeSet<u8> = a.iter().map(|c| c.0).collect();
    assert!(modes.len() > 1, "{modes:?}");
    let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    assert_eq!(differ, 0, "{differ} of 1 000 travellers choose otherwise under other names");
}

#[test]
fn a_tie_between_two_car_parks_does_not_follow_the_names() {
    let trips = |road: &RoadNetwork| vec![trip(road, "a".to_string(), 0, Some(Mode::CarTransit))];
    let (a, b) = (run(&TOY, &trips, false), run(&RENAMED, &trips, false));
    assert_eq!(a[0].2, 1_200, "both car parks reach N1 at 1 200 s");
    assert_eq!(a, b, "the car park taken follows the names");
    // The one first by where it is: the further west, then south, of D2 and R2.
    let (road, _) = toy_network();
    let (d2, r2) = (at(&road, "D2"), at(&road, "R2"));
    let first = if openmobisim_core_graph::places::point_order(d2, r2).is_lt() { d2 } else { r2 };
    assert_eq!(a[0].1, Some((first.lon.to_bits(), first.lat.to_bits())));
}
