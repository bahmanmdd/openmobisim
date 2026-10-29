//! A shared bike to a transit stop's hub, then transit (`bike_share_transit`): trips choosing
//! their mode are offered a ride on a shared bike from a dock near their origin to a dock at a
//! stop's hub, the bike returned there, then the timetable from that stop; the shared bike is a
//! leg of the fleet engine's day, and the transit part starts when the engine has it returned.
//!
//! The network: a 5 × 5 two-way grid of 400 m blocks. A line runs from stop `A` (row 0,
//! column 4) to stop `B` (row 4, column 4) in 240 s, every 600 s from second 0. Docks of four
//! slots, two bikes each: `o` at the origin's corner (row 0, column 0), `a` at stop `A` (0 m: it
//! joins `A`'s hub) and `w` at row 4, column 0, at no stop. Travellers own no bike and no car.
//! From the origin's corner to `B`'s, 3 200 m, walking is beyond the shipped 1 800 s; walking to
//! `A` (1 600 m, within the 1 800 s to a stop) and riding is transit; riding a shared bike from
//! `o` to `a` and the line from there is the chain.

#![allow(clippy::float_cmp, reason = "seconds of a timetable are whole numbers held exactly")]

use std::collections::BTreeMap;
use std::sync::Arc;

use openmobisim_core_choice::{ChoiceModel, Deterministic};
use openmobisim_core_demand::{
    ClassDefaults, Mode, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::examples::node_name;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::manhattan_grid;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_sim::services::{DockDefaults, DockRow, ServiceSetup};
use openmobisim_core_sim::{
    LayerSetup, NoAlternativeReason, Run, RunResult, StaticLayers, TransitSetup,
};
use openmobisim_core_transit::TransitDefaults;
use openmobisim_core_transit::date::ServiceDate;
use openmobisim_core_transit::timetable::{
    ALIGHT, BOARD, CallSpec, RouteSpec, StopSpec, TimetableBuilder,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::EntityId;
use openmobisim_core_types::time::Second;

/// The line's ride from `A` to `B`, in seconds.
const RIDE_S: u32 = 240;
/// Its headway, in seconds.
const HEADWAY_S: u32 = 600;

fn at(road: &RoadNetwork, row: u32, col: u32) -> LonLat {
    let id = road.node_external_ids().id_of(&node_name(row, col)).expect("a grid node");
    road.node_lonlat(EntityId::new(id))
}

struct Toy {
    road: Arc<RoadNetwork>,
    layers: Arc<StaticLayers>,
    transit: Arc<TransitSetup>,
}

fn toy(n: u32, block_m: f64, line: &[(u32, u32)]) -> Toy {
    let road = Arc::new(manhattan_grid(n, block_m, false).0);
    let d = StaticLayerDefaults::SHIPPED;
    let derive = |layer| {
        Arc::new(StaticNetwork::derive(&road, layer, d, &mut Diagnostics::new()).expect("derives"))
    };
    let layers = Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(derive(StaticLayer::Bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(derive(StaticLayer::Walk), BikeCost::Dedicated, d)),
    });
    let mut b = TimetableBuilder::new(ServiceDate::parse("20261029").expect("a date"));
    let stops: Vec<u32> = line
        .iter()
        .enumerate()
        .map(|(i, &(r, c))| {
            b.add_stop(StopSpec {
                external_id: format!("S{i}"),
                name: format!("stop {i}"),
                position: at(&road, r, c),
                parent: None,
            })
        })
        .collect();
    let route =
        b.add_route(RouteSpec { external_id: "L".into(), short_name: "L".into(), route_type: 3 });
    for k in 0..40 {
        let calls: Vec<CallSpec> = stops
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let t = k * HEADWAY_S + u32::try_from(i).expect("few stops") * RIDE_S;
                CallSpec { stop: s, arrival: t, departure: t, flags: BOARD | ALIGHT }
            })
            .collect();
        b.add_run(format!("L:{k:02}"), route, &calls);
    }
    let transit = Arc::new(TransitSetup::new(
        Arc::new(b.build().0),
        layers.walk.as_ref().unwrap(),
        layers.bike.as_ref(),
        TransitDefaults::SHIPPED,
    ));
    Toy { road, layers, transit }
}

fn dock(road: &RoadNetwork, id: &str, (r, c): (u32, u32), capacity: u32, bikes: u32) -> DockRow {
    DockRow {
        dock_id: id.to_string(),
        name: None,
        hub_id: None,
        position: at(road, r, c),
        capacity,
        bikes: Some(bikes),
    }
}

fn service(t: &Toy, rows: &[DockRow]) -> ServiceSetup {
    ServiceSetup::bike_docked(
        "bike_share",
        rows,
        t.layers.walk.as_ref().unwrap(),
        t.layers.bike.as_ref().unwrap(),
        Some(&t.road),
        DockDefaults::SHIPPED,
    )
    .expect("set up")
}

fn trip(who: &str, from: LonLat, to: LonLat, s: u32) -> RawTrip {
    RawTrip {
        traveller_id: who.to_string(),
        trip_seq: 0,
        origin: from,
        destination: to,
        departure_time: Second(s),
        user_class: "everyone".to_string(),
        weight: None,
        mode: None,
    }
}

fn demand(rows: Vec<RawTrip>) -> (Arc<Travellers>, Arc<Trips>) {
    let classes = ClassDefaults::new()
        .with_default("everyone", Ownership { car: false, bike: false, transit_pass: false });
    let (t, trips) =
        build_travellers(rows, Vec::new(), &classes, 1, &mut Diagnostics::new()).expect("builds");
    (Arc::new(t), Arc::new(trips))
}

/// The 5 × 5 toy of the module docs.
fn line_toy() -> Toy {
    toy(5, 400.0, &[(0, 4), (4, 4)])
}

/// The docks of the module docs; `at_stop`: dock `a` at stop `A`, else 400 m south of it (row
/// 1, column 4: beyond the 150 m that join a stop's hub).
fn docks(t: &Toy, at_stop: bool) -> Vec<DockRow> {
    vec![
        dock(&t.road, "o", (0, 0), 4, 2),
        dock(&t.road, "a", if at_stop { (0, 4) } else { (1, 4) }, 4, 2),
        dock(&t.road, "w", (4, 0), 4, 2),
    ]
}

fn run_on(
    t: &Toy,
    rows: Vec<RawTrip>,
    docks: Option<Vec<DockRow>>,
    modes: &[Mode],
    model: Arc<dyn ChoiceModel>,
    class: Option<[bool; Mode::COUNT]>,
) -> RunResult {
    let (travellers, trips) = demand(rows);
    let mut run = Run::new(Arc::clone(&t.road), travellers, trips, Second(86_400))
        .with_layers(Arc::clone(&t.layers))
        .with_transit(Arc::clone(&t.transit))
        .with_choice_model(model)
        .with_mode_choice(modes);
    if let Some(rows) = docks {
        run = run.with_services(Arc::new(vec![service(t, &rows)]));
    }
    if let Some(allowed) = class {
        run = run.with_class_modes(vec![allowed]);
    }
    run.execute(&mut Diagnostics::new())
}

const MODES: [Mode; 3] = [Mode::Walk, Mode::Transit, Mode::BikeShareTransit];

fn completed(r: &RunResult, mode: Mode) -> u32 {
    r.by_mode[mode.index()].completion.completed
}

#[test]
fn a_shared_bike_to_the_stop_is_chosen_over_walking_to_it() {
    let t = line_toy();
    let rows = || vec![trip("t", at(&t.road, 0, 0), at(&t.road, 4, 4), 100)];
    let det: Arc<dyn ChoiceModel> = Arc::new(Deterministic);
    let r = run_on(&t, rows(), Some(docks(&t, true)), &MODES, Arc::clone(&det), None);
    assert_eq!(completed(&r, Mode::BikeShareTransit), 1, "{:?}", r.by_mode);
    assert!(r.modes_listed().contains(&Mode::BikeShareTransit));
    let it = r.itineraries.as_ref().expect("itineraries");
    assert_eq!(it.rides, [1]);
    // The leg on the engine: from `o` to `a` (docks by sorted id: `a` 0, `o` 1, `w` 2), served
    // as planned.
    let services = r.services.as_ref().expect("a service");
    let day = services.last.as_ref().expect("a shared bike was ridden");
    let o = day.outcome.legs[0];
    assert_eq!((o.picked, o.returned, o.redirections), (Some(1), Some(0), 0));
    // The closed form: at `o` at 100 (no walk), a minute to take the bike, the ride of 1 600 m
    // on the bike layer (about 384 s, as the shared-bike toy's), half a minute to dock it at
    // `a`; at the stop beside it once the stop's transfer time has passed, and on the first run
    // it can board from there (here the 1 200 one: at the stop at 574, too late for the 600 one
    // by the minute's boarding slack); at `B` a ride later. Walking to `A` (1 600 m, about 1 200 s) catches a run
    // two headways later at best.
    let d = DockDefaults::SHIPPED;
    let returned = o.arrived_s - 100.0;
    assert!(
        returned > 384.0 + d.pickup_s + d.return_s - 1.0
            && returned < 400.0 + d.pickup_s + d.return_s,
        "{returned}"
    );
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "seconds")]
    let at_stop = o.arrived_s.floor() as u32 + TransitDefaults::SHIPPED.stop_transfer_s as u32;
    // On board a run that leaves at least the boarding slack (a minute) after.
    let board = (at_stop + TransitDefaults::SHIPPED.board_slack_s).div_ceil(HEADWAY_S) * HEADWAY_S;
    let arrival = r
        .events
        .iter()
        .find(|e| e.event_type == openmobisim_core_sim::EventType::TripCompleted)
        .map(|e| e.second.get());
    assert_eq!(arrival, Some(board + RIDE_S), "at the stop at {at_stop}");
    // Its expected arrival, as the choice saw it, is the realised one: the docks had bikes and
    // slots, and the expectation at the first loading is the start-of-day counts'.
    assert_eq!(it.expected_s[0], f64::from(board + RIDE_S - 100));
    assert_eq!(day.mismatch_s, 0.0);
    // Transit alone, for the comparison: later.
    let alone = run_on(&t, rows(), None, &[Mode::Walk, Mode::Transit], Arc::clone(&det), None);
    let walked = alone
        .events
        .iter()
        .find(|e| e.event_type == openmobisim_core_sim::EventType::TripCompleted)
        .map(|e| e.second.get())
        .expect("transit");
    println!("by shared bike and the line: {}; on foot and the line: {walked}", board + RIDE_S);
    assert!(walked > board + RIDE_S);

    // Dock `a` 400 m from the stop joins no stop's hub: the chain is not offered, the trip walks
    // to the stop.
    let r = run_on(&t, rows(), Some(docks(&t, false)), &MODES, det, None);
    assert_eq!(completed(&r, Mode::BikeShareTransit), 0);
    assert_eq!(completed(&r, Mode::Transit), 1, "{:?}", r.by_mode);
    assert!(r.services.as_ref().unwrap().last.is_none(), "nobody took a shared bike");
}

#[test]
fn walking_to_the_stop_wins_when_it_is_beside_the_origin() {
    // From the corner beside stop `A`'s (row 0, column 3: 400 m from `A`): walking there (about
    // 300 s) beats walking to dock `o` (1 200 m) or riding from the nearest dock, which is `a`
    // itself.
    let t = line_toy();
    let rows = vec![trip("t", at(&t.road, 0, 3), at(&t.road, 4, 4), 100)];
    let r = run_on(&t, rows, Some(docks(&t, true)), &MODES, Arc::new(Deterministic), None);
    assert_eq!(completed(&r, Mode::Transit), 1, "{:?}", r.by_mode);
}

#[test]
fn a_class_without_shared_bikes_never_takes_one_to_transit() {
    let t = line_toy();
    let mut allowed = [true; Mode::COUNT];
    allowed[Mode::BikeShare.index()] = false;
    allowed[Mode::BikeShareTransit.index()] = false;
    let rows = vec![trip("t", at(&t.road, 0, 0), at(&t.road, 4, 4), 100)];
    let r = run_on(&t, rows, Some(docks(&t, true)), &MODES, Arc::new(Deterministic), Some(allowed));
    assert_eq!(completed(&r, Mode::BikeShareTransit), 0);
    assert_eq!(completed(&r, Mode::Transit), 1);
}

#[test]
fn no_dock_within_the_walk_is_the_reason_given() {
    // Only the hub's dock `a`, 1 600 m from the origin (beyond the 600 s a traveller walks to a
    // dock), and a walk beyond 1 800 s: the trip has no alternative, and the chain says why.
    let t = line_toy();
    let rows = vec![trip("t", at(&t.road, 0, 0), at(&t.road, 4, 4), 100)];
    let only_a = vec![dock(&t.road, "a", (0, 4), 4, 2)];
    let r = run_on(
        &t,
        rows,
        Some(only_a),
        &[Mode::Walk, Mode::BikeShareTransit],
        Arc::new(Deterministic),
        None,
    );
    let none = r.no_alternative.first().expect("the trip had no alternative");
    assert_eq!(none.reasons[Mode::BikeShareTransit.index()], NoAlternativeReason::NoDockInReach);
    assert_eq!(none.reasons[Mode::Walk.index()], NoAlternativeReason::WalkTooLong);
}

#[test]
fn without_a_service_the_chain_is_neither_offered_nor_listed() {
    let t = line_toy();
    let rows = vec![trip("t", at(&t.road, 0, 0), at(&t.road, 4, 4), 100)];
    let r = run_on(&t, rows, None, &[Mode::Walk, Mode::Transit], Arc::new(Deterministic), None);
    assert_eq!(completed(&r, Mode::Transit), 1);
    assert!(r.services.is_none());
    assert!(!r.modes_listed().contains(&Mode::BikeShareTransit));
}

/// Twelve travellers a minute apart, the logit, four loadings: the two bikes at `o` run out, so
/// later travellers redirect or walk on; two runs give the same day.
#[test]
fn same_seed_same_day_and_an_empty_dock_walks_on_to_the_stop() {
    let t = line_toy();
    let rows: Vec<RawTrip> = (0..12)
        .map(|k| trip(&format!("t{k:02}"), at(&t.road, 0, 0), at(&t.road, 4, 4), 100 + 60 * k))
        .collect();
    let logit = || -> Arc<dyn ChoiceModel> {
        Arc::from(openmobisim_core_choice::model("logit", &BTreeMap::new()).expect("logit"))
    };
    let go = || {
        let (travellers, trips) = demand(rows.clone());
        let options =
            BTreeMap::from([("iterations".to_string(), 4.0), ("gap_tolerance".to_string(), 0.0)]);
        Run::new(Arc::clone(&t.road), travellers, trips, Second(86_400))
            .with_layers(Arc::clone(&t.layers))
            .with_transit(Arc::clone(&t.transit))
            .with_choice_model(logit())
            .with_mode_choice(&MODES)
            .with_services(Arc::new(vec![service(&t, &docks(&t, true))]))
            .with_equilibration(Arc::from(
                openmobisim_core_sim::equilibration::strategy("msa", &options).expect("msa"),
            ))
            .execute(&mut Diagnostics::new())
    };
    let (a, b) = (go(), go());
    assert_eq!(a.events, b.events);
    let (sa, sb) = (a.services.unwrap(), b.services.unwrap());
    assert_eq!(sa.legs_people, sb.legs_people);
    let (da, db) = (sa.last.unwrap(), sb.last.unwrap());
    assert_eq!(format!("{:?}", (&da.legs, &da.outcome)), format!("{:?}", (db.legs, db.outcome)));
    assert_eq!(da.outcome.invariant_violations, 0, "bikes are conserved");
    // Every traveller who took the chain got to the hub, by bike or on foot, and on to `B`.
    let chain = a.by_mode[Mode::BikeShareTransit.index()].completion;
    assert_eq!(chain.completed, chain.total_trips, "{chain:?}");
    println!(
        "chain trips {}, transit {}, failed pickups {}",
        chain.total_trips,
        a.by_mode[Mode::Transit.index()].completion.total_trips,
        da.outcome.legs.iter().filter(|o| o.picked.is_none()).count()
    );
}

/// The run time with and without the chain offered (S302 step 1.5's measurement), on the 10 × 10
/// grid of 200 m blocks of the shared-bike measurement, with a line along its middle row
/// (columns 0 to 9, a stop at every other node, 120 s between stops, every 600 s): the same
/// 4 000 trips between random nodes, travellers owning a bike, choosing among walking, their
/// bike and transit; then shared bikes added (25 docks, at every other node of every other row,
/// so the line's five stops have one each); then the chain offered too. Ten loadings of the
/// method of successive averages, the logit
/// (`cargo test --release --test services_transit -- --ignored --nocapture`).
#[test]
#[ignore = "a run-time measurement, run by hand"]
fn run_time_on_the_10_by_10_grid_with_and_without_the_chain() {
    let line: Vec<(u32, u32)> = (0..10).step_by(2).map(|c| (4, c)).collect();
    let t = toy(10, 200.0, &line);
    let rows: Vec<DockRow> = (0..10)
        .step_by(2)
        .flat_map(|r| (0..10).step_by(2).map(move |c| (r, c)))
        .map(|(r, c)| dock(&t.road, &format!("d{r}{c}"), (r, c), 10, 5))
        .collect();
    let svc = service(&t, &rows);
    let mut x: u64 = 12_345;
    let mut next = |n: u32| {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        u32::try_from((x >> 33) % u64::from(n)).expect("small")
    };
    let trips: Vec<RawTrip> = (0..4_000)
        .map(|k| {
            let (a, b, c, d) = (next(10), next(10), next(10), next(10));
            trip(&format!("t{k:04}"), at(&t.road, a, b), at(&t.road, c, d), 28_800 + next(3_600))
        })
        .collect();
    let classes = ClassDefaults::new()
        .with_default("everyone", Ownership { car: false, bike: true, transit_pass: false });
    let options =
        BTreeMap::from([("iterations".to_string(), 10.0), ("gap_tolerance".to_string(), 0.0)]);
    let make = |variant: u8| {
        let (tr, trs) =
            build_travellers(trips.clone(), Vec::new(), &classes, 1, &mut Diagnostics::new())
                .expect("builds");
        let modes: &[Mode] = match variant {
            0 => &[Mode::Walk, Mode::Bike, Mode::Transit],
            1 => &[Mode::Walk, Mode::Bike, Mode::Transit, Mode::BikeShare],
            _ => &[Mode::Walk, Mode::Bike, Mode::Transit, Mode::BikeShare, Mode::BikeShareTransit],
        };
        let mut run = Run::new(Arc::clone(&t.road), Arc::new(tr), Arc::new(trs), Second(86_400))
            .with_layers(Arc::clone(&t.layers))
            .with_transit(Arc::clone(&t.transit))
            .with_choice_model(Arc::from(
                openmobisim_core_choice::model("logit", &BTreeMap::new()).expect("logit"),
            ))
            .with_equilibration(Arc::from(
                openmobisim_core_sim::equilibration::strategy("msa", &options).expect("msa"),
            ))
            .with_mode_choice(modes);
        if variant > 0 {
            run = run.with_services(Arc::new(vec![svc.clone()]));
        }
        run
    };
    for variant in [0, 1, 2, 0, 1, 2] {
        let mut run = make(variant);
        let started = std::time::Instant::now();
        let result = run.execute(&mut Diagnostics::new());
        let seconds = started.elapsed().as_secs_f64();
        let engine = result.services.as_ref().map_or(0.0, |s| s.engine_s);
        let n = |m: Mode| result.by_mode[m.index()].completion.total_trips;
        println!(
            "variant {variant}: {seconds:.3} s, engine {engine:.4} s, trips: bike_share {}, \
             bike_share_transit {}, transit {}",
            n(Mode::BikeShare),
            n(Mode::BikeShareTransit),
            n(Mode::Transit)
        );
    }
}
