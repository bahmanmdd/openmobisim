//! Docked bike sharing in a run: trips choosing their mode are offered shared bikes between the
//! docks within a walk of their origin and destination, planned on the docks' fixed expected
//! extra walks (S303); once the run has stopped, the fleet engine plays the legs of the last
//! loading's shared-bike trips in one service day, and reports what they met beside what they
//! planned.
//!
//! The network: a 5 × 5 two-way grid of 200 m blocks. Docks of four slots at the corners of its
//! first and last rows: a (row 0, column 0), b (0, 1), y (4, 3), z (4, 4). Travellers own no
//! bike and no car; they choose between walking and a shared bike.

#![allow(clippy::float_cmp, reason = "counts of bikes and trips are whole numbers held exactly")]

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
use openmobisim_core_sim::services::{DockDefaults, DockRow, ServiceAssumptions, ServiceSetup};
use openmobisim_core_sim::{LayerSetup, Prices, Run, RunResult, StaticLayers};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::EntityId;
use openmobisim_core_types::time::Second;

fn grid() -> Arc<RoadNetwork> {
    Arc::new(manhattan_grid(5, 200.0, false).0)
}

fn layers(road: &RoadNetwork) -> Arc<StaticLayers> {
    let d = StaticLayerDefaults::SHIPPED;
    let derive = |layer| {
        Arc::new(StaticNetwork::derive(road, layer, d, &mut Diagnostics::new()).expect("derives"))
    };
    Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(derive(StaticLayer::Bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(derive(StaticLayer::Walk), BikeCost::Dedicated, d)),
    })
}

fn at(road: &RoadNetwork, row: u32, col: u32) -> LonLat {
    let id = road.node_external_ids().id_of(&node_name(row, col)).expect("a grid node");
    road.node_lonlat(EntityId::new(id))
}

/// The four docks, `bikes` at the start of the day (a, b, y, z), four slots each.
fn service(
    road: &RoadNetwork,
    layers: &StaticLayers,
    bikes: [u32; 4],
    walk_max_s: f64,
    fail_s: f64,
) -> ServiceSetup {
    let row = |id: &str, p: LonLat, b: u32| DockRow {
        dock_id: id.to_string(),
        name: None,
        hub_id: None,
        position: p,
        capacity: 4,
        bikes: Some(b),
    };
    let rows = vec![
        row("a", at(road, 0, 0), bikes[0]),
        row("b", at(road, 0, 1), bikes[1]),
        row("y", at(road, 4, 3), bikes[2]),
        row("z", at(road, 4, 4), bikes[3]),
    ];
    ServiceSetup::bike_docked(
        "bike_share",
        &rows,
        layers.walk.as_ref().unwrap(),
        layers.bike.as_ref().unwrap(),
        Some(road),
        DockDefaults { walk_max_s, fail_s, ..DockDefaults::SHIPPED },
    )
    .expect("set up")
}

fn trip(who: &str, from: LonLat, to: LonLat, t: u32) -> RawTrip {
    RawTrip {
        traveller_id: who.to_string(),
        trip_seq: 0,
        origin: from,
        destination: to,
        departure_time: Second(t),
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

/// What a run is given besides its trips.
struct Setup {
    bikes: [u32; 4],
    docks: bool,
    model: Arc<dyn ChoiceModel>,
    iterations: u32,
    prices: Prices,
    walk_max_s: f64,
    fail_s: f64,
    assumptions: ServiceAssumptions,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            bikes: [2, 2, 2, 2],
            docks: true,
            model: Arc::new(Deterministic),
            iterations: 1,
            prices: Prices::SHIPPED,
            walk_max_s: DockDefaults::SHIPPED.walk_max_s,
            fail_s: DockDefaults::SHIPPED.fail_s,
            assumptions: ServiceAssumptions::SHIPPED,
        }
    }
}

fn run(rows: Vec<RawTrip>, s: &Setup) -> RunResult {
    let road = grid();
    let layers = layers(&road);
    let service = service(&road, &layers, s.bikes, s.walk_max_s, s.fail_s);
    let (travellers, trips) = demand(rows);
    let mut run = Run::new(road, travellers, trips, Second(86_400))
        .with_layers(layers)
        .with_choice_model(Arc::clone(&s.model))
        .with_prices(s.prices)
        .with_service_assumptions(s.assumptions)
        .with_mode_choice(&[Mode::Walk, Mode::BikeShare]);
    if s.docks {
        run = run.with_services(Arc::new(vec![service]));
    }
    if s.iterations > 1 {
        // Every loading made: no stopping on the gap.
        let options = BTreeMap::from([
            ("iterations".to_string(), f64::from(s.iterations)),
            ("gap_tolerance".to_string(), 0.0),
        ]);
        run = run.with_equilibration(Arc::from(
            openmobisim_core_sim::equilibration::strategy("msa", &options).expect("msa"),
        ));
    }
    run.execute(&mut Diagnostics::new())
}

fn corner_trips(road: &RoadNetwork, n: u32, every_s: u32) -> Vec<RawTrip> {
    (0..n)
        .map(|k| trip(&format!("t{k:02}"), at(road, 0, 0), at(road, 4, 4), 100 + k * every_s))
        .collect()
}

#[test]
fn travellers_without_a_bike_choose_a_shared_one_over_a_long_walk() {
    let road = grid();
    let result = run(corner_trips(&road, 3, 1), &Setup::default());
    // 1.6 km on foot is about 20 minutes; by shared bike about 8: the deterministic model rides.
    let share = result.by_mode[Mode::BikeShare.index()];
    assert_eq!(share.completion.completed, 3, "{:?}", result.by_mode);
    assert_eq!(result.by_mode[Mode::Walk.index()].completion.total_trips, 0);
    let services = result.services.as_ref().expect("the run has a service");
    let day = services.last.as_ref().expect("shared bikes were ridden");
    // Dock a has two bikes: the third traveller finds it empty and takes one at b.
    let picked: Vec<Option<u32>> = day.outcome.legs.iter().map(|o| o.picked).collect();
    assert_eq!(picked, [Some(0), Some(0), Some(1)]);
    let returned: Vec<Option<u32>> = day.outcome.legs.iter().map(|o| o.returned).collect();
    // z holds four: its two bikes and the first two returned fill it. The destination is at z:
    // riding on to y and walking back is slower than walking from z, so the third leaves its
    // bike at z above capacity (S302).
    assert_eq!(returned, [Some(3), Some(3), Some(3)]);
    assert!(day.outcome.legs[2].overflow);
    assert_eq!(day.outcome.invariant_violations, 0, "bikes are conserved");
    // The first: docks at its origin and destination nodes, so no walk; the ride is the bike
    // layer's corner-to-corner time (about 384 s, 1 600 m at the layer's speed), with a minute to
    // take the bike and half a minute to dock it (the closed form of the earlier branch's toy).
    let d = DockDefaults::SHIPPED;
    let first = day.outcome.legs[0].arrived_s - 100.0;
    assert!(
        first > 384.0 + d.pickup_s + d.return_s - 1.0 && first < 400.0 + d.pickup_s + d.return_s,
        "{first}"
    );
    // The third walked from a to b and back to nothing more: its extra is that walk.
    let third = day.outcome.legs[2];
    // One dock tried after the planned one, b at the pickup; none at the return.
    assert_eq!(third.redirections, 1);
    assert!(third.extra_walk_s > 0.0 && third.arrived_s > day.outcome.legs[0].arrived_s);
    // The modes listed: the service's among them.
    assert!(result.modes_listed().contains(&Mode::BikeShare));
}

#[test]
fn without_a_service_bike_sharing_is_not_offered_nor_listed() {
    let road = grid();
    let result = run(corner_trips(&road, 3, 1), &Setup { docks: false, ..Setup::default() });
    assert_eq!(result.by_mode[Mode::Walk.index()].completion.completed, 3);
    assert_eq!(result.by_mode[Mode::BikeShare.index()].completion.total_trips, 0);
    assert!(result.services.is_none());
    assert!(!result.modes_listed().contains(&Mode::BikeShare));
}

#[test]
fn a_shared_bike_is_taken_when_it_beats_walking_and_not_when_no_dock_is_within_the_walk() {
    // From row 0, column 2 to the far corner (row 4, column 4): 1 200 m on foot. With the
    // shipped reach (600 s on foot) dock b is 200 m from the origin and z at the destination:
    // walk, ride b → z, walk nothing, faster than the walk.
    let road = grid();
    let rows = || vec![trip("t", at(&road, 0, 2), at(&road, 4, 4), 100)];
    let result = run(rows(), &Setup::default());
    assert_eq!(result.by_mode[Mode::BikeShare.index()].completion.completed, 1);
    // With a reach of 100 s on foot no dock is within the origin's walk (b is 200 m away): no
    // pair is offered, and the trip walks.
    let result = run(rows(), &Setup { walk_max_s: 100.0, ..Setup::default() });
    assert_eq!(result.by_mode[Mode::Walk.index()].completion.completed, 1, "{:?}", result.by_mode);
    assert!(result.services.as_ref().unwrap().last.is_none(), "nobody took a shared bike");
}

#[test]
fn a_price_per_ride_moves_travellers_from_shared_bikes_to_walking() {
    let road = grid();
    let logit = |prices: Prices| {
        let options = BTreeMap::new();
        let model: Arc<dyn ChoiceModel> =
            Arc::from(openmobisim_core_choice::model("logit", &options).expect("logit"));
        let rows = corner_trips(&road, 40, 15);
        let r = run(rows, &Setup { model, prices, bikes: [4, 4, 0, 0], ..Setup::default() });
        r.by_mode[Mode::BikeShare.index()].completion.total_trips
    };
    let free = logit(Prices::SHIPPED);
    let priced = logit(Prices { bike_share_eur_per_ride: 3.0, ..Prices::SHIPPED });
    assert!(free > 0 && priced < free, "free {free}, at 3 € a ride {priced}");
}

/// The shared-bike trips (people) of forty corner trips under logit at 3 € a ride, with `bikes`
/// at the start and the assumptions `e`.
fn shared_under(bikes: [u32; 4], e: ServiceAssumptions) -> u32 {
    let road = grid();
    let model: Arc<dyn ChoiceModel> =
        Arc::from(openmobisim_core_choice::model("logit", &BTreeMap::new()).expect("logit"));
    // A fare that puts the shared bike near walking (as in the fare test above), so what is
    // expected at the docks shows in the share.
    let prices = Prices { bike_share_eur_per_ride: 3.0, ..Prices::SHIPPED };
    let s = Setup { model, bikes, prices, assumptions: e, ..Setup::default() };
    let r = run(corner_trips(&road, 40, 15), &s);
    r.by_mode[Mode::BikeShare.index()].completion.total_trips
}

#[test]
fn a_dock_s_assumed_extra_walk_is_set_in_seconds_and_moves_the_shared_bike_share() {
    // Dock a starts empty: the default assumes, at a, the walk to b (the start-of-day bikes);
    // no extra walk assumed (0 s at both ends) draws at least as many.
    let bikes = [0, 2, 0, 0];
    let zero = ServiceAssumptions {
        assumed_extra_walk_at_pickup_s: 0.0,
        assumed_extra_walk_at_return_s: 0.0,
        ..ServiceAssumptions::SHIPPED
    };
    let none = shared_under(bikes, zero);
    let default = shared_under(bikes, ServiceAssumptions::SHIPPED);
    println!("shared-bike trips: no extra walk assumed {none}, start-of-day bikes {default}");
    assert!(none >= default);
    // a and b start full of bikes: the default assumes no extra walk at the pickup; 300 s at
    // every pickup dock lowers the share.
    let full = [4, 4, 0, 0];
    let free = shared_under(full, ServiceAssumptions::SHIPPED);
    let walk =
        ServiceAssumptions { assumed_extra_walk_at_pickup_s: 300.0, ..ServiceAssumptions::SHIPPED };
    let slowed = shared_under(full, walk);
    println!("shared-bike trips with full docks: default {free}, 300 s at the pickup {slowed}");
    assert!(free > slowed, "default {free}, 300 s at the pickup {slowed}");
}

#[test]
fn same_seed_same_day() {
    let road = grid();
    let a = run(corner_trips(&road, 12, 20), &Setup { iterations: 4, ..Setup::default() });
    let b = run(corner_trips(&road, 12, 20), &Setup { iterations: 4, ..Setup::default() });
    let (sa, sb) = (a.services.unwrap(), b.services.unwrap());
    let (da, db) = (sa.last.unwrap(), sb.last.unwrap());
    // Compared as printed: a bike leg's outcome holds NaN (no taxi was sent), never equal to itself.
    assert_eq!(
        format!("{:?}", (da.legs, da.outcome, da.outcomes, da.summary)),
        format!("{:?}", (db.legs, db.outcome, db.outcomes, db.summary)),
        "the same days, but their clocks"
    );
    assert_eq!(sa.assumed, sb.assumed);
    assert_eq!(a.events, b.events);
}

/// The assumptions are fixed for the run (S303): whatever the loadings, the docks' assumed
/// costs are the start-of-day bikes', and the trips end as planned on them; the service day,
/// played once after the run, shows what actually happened. Twenty travellers over ten minutes
/// from a's corner to z's, a and b holding four bikes between them: in the service day most
/// find them empty and walk on to other docks or find no bike at all. Printed
/// (`cargo test --test services_in_runs -- --nocapture`): the service day's summary.
#[test]
fn the_assumptions_stay_fixed_and_the_service_day_shows_what_actually_happened() {
    let road = grid();
    let options = BTreeMap::new();
    let model: Arc<dyn ChoiceModel> =
        Arc::from(openmobisim_core_choice::model("logit", &options).expect("logit"));
    let s = Setup { model, iterations: 6, bikes: [2, 2, 0, 0], ..Setup::default() };
    let result = run(corner_trips(&road, 20, 30), &s);
    let services = result.services.as_ref().expect("a service");
    let layers = layers(&road);
    let setup = service(&road, &layers, s.bikes, s.walk_max_s, s.fail_s);
    let start = openmobisim_core_sim::services::costs::DockCosts::initial(&setup, (0.0, 86_400.0));
    assert_eq!(services.assumed.as_ref(), Some(&start), "the start-of-day bikes', unchanged");
    let day = services.last.as_ref().expect("shared bikes were ridden");
    println!("{:?}", day.summary);
    let share = result.by_mode[Mode::BikeShare.index()].completion;
    let sm = day.summary;
    assert_eq!(sm.requests, f64::from(share.total_trips));
    assert_eq!(sm.requests, sm.rides + sm.no_bike + sm.unfinished);
    assert!(sm.no_bike > 0.0, "some find no bike at any dock tried");
    // The extra walks of the rides: these corner trips find their pickup docks empty, so most
    // of it is the pickup's; some rides were sent on to another dock.
    let (p, r) = (sm.extra_walk_at_pickup_mean_s, sm.extra_walk_at_return_mean_s);
    assert!(p > 0.0 && r >= 0.0 && p > r, "pickup {p}, return {r}");
    assert!(sm.redirected_share > 0.0 && sm.redirected_share <= 1.0);
    let rows: Vec<&str> = sm.rows((f64::NAN, f64::NAN)).iter().map(|(n, _)| *n).collect();
    assert_eq!(
        rows,
        [
            "requests",
            "rides",
            "no_bike",
            "unfinished",
            "extra_walk_at_pickup_mean_s",
            "extra_walk_at_return_mean_s",
            "assumed_extra_walk_at_pickup_s",
            "assumed_extra_walk_at_return_s",
            "redirected_share"
        ]
    );
    // Each trip's row is as planned: completed at its planned end, not at what happened.
    for (l, &(_, end)) in day.legs.iter().zip(&day.outcomes) {
        let done =
            result.events.iter().find(|e| e.entity_id == l.trip.raw()).expect("the trip's event");
        assert_eq!(f64::from(done.second.get()), l.planned_end_s(&setup).floor());
        assert!(end.is_nan() || end >= l.planned_arrival_s(&setup) - 1e-9);
    }
}

#[test]
fn a_class_without_shared_bikes_never_takes_one() {
    let road = grid();
    let mut allowed = [true; Mode::COUNT];
    allowed[Mode::BikeShare.index()] = false;
    let (travellers, trips) = demand(corner_trips(&road, 3, 1));
    let layers = layers(&road);
    let service = service(&road, &layers, [2, 2, 2, 2], 600.0, 1200.0);
    let result = Run::new(Arc::clone(&road), travellers, trips, Second(86_400))
        .with_layers(layers)
        .with_choice_model(Arc::new(Deterministic))
        .with_mode_choice(&[Mode::Walk, Mode::BikeShare])
        .with_services(Arc::new(vec![service]))
        .with_class_modes(vec![allowed])
        .execute(&mut Diagnostics::new());
    assert_eq!(result.by_mode[Mode::Walk.index()].completion.completed, 3);
    assert_eq!(result.by_mode[Mode::BikeShare.index()].completion.total_trips, 0);
}

/// The run time with and without the service on a 10 × 10 grid of 200 m blocks (S302 step 1.3's
/// measurement): the same 4 000 trips between random grid nodes, travellers owning a bike,
/// choosing among walking and their bike, and shared bikes in the second run (docks of 10 slots
/// at every other node of every other row: 25 docks); 10 loadings of the method of successive
/// averages, the logit. Prints the seconds of each run and the engine's share
/// (`cargo test --release --test services_in_runs -- --ignored --nocapture`).
#[test]
#[ignore = "a run-time measurement, run by hand"]
fn run_time_on_the_10_by_10_grid_with_and_without_the_service() {
    let road = Arc::new(manhattan_grid(10, 200.0, false).0);
    let layers = layers(&road);
    let rows: Vec<DockRow> = (0..10)
        .step_by(2)
        .flat_map(|r| (0..10).step_by(2).map(move |c| (r, c)))
        .map(|(r, c)| DockRow {
            dock_id: format!("d{r}{c}"),
            name: None,
            hub_id: None,
            position: at(&road, r, c),
            capacity: 10,
            bikes: Some(5),
        })
        .collect();
    let service = ServiceSetup::bike_docked(
        "bike_share",
        &rows,
        layers.walk.as_ref().unwrap(),
        layers.bike.as_ref().unwrap(),
        Some(&road),
        DockDefaults::SHIPPED,
    )
    .expect("set up");
    // Random trips by a fixed linear congruential sequence, so both runs have the same.
    let mut x: u64 = 12_345;
    let mut next = |n: u32| {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        u32::try_from((x >> 33) % u64::from(n)).expect("small")
    };
    let trips: Vec<RawTrip> = (0..4_000)
        .map(|k| {
            let (a, b, c, d) = (next(10), next(10), next(10), next(10));
            trip(&format!("t{k:04}"), at(&road, a, b), at(&road, c, d), 28_800 + next(3_600))
        })
        .collect();
    let classes = ClassDefaults::new()
        .with_default("everyone", Ownership { car: false, bike: true, transit_pass: false });
    let options =
        BTreeMap::from([("iterations".to_string(), 10.0), ("gap_tolerance".to_string(), 0.0)]);
    let make = |with: bool| {
        let (t, tr) =
            build_travellers(trips.clone(), Vec::new(), &classes, 1, &mut Diagnostics::new())
                .expect("builds");
        let modes: &[Mode] = if with {
            &[Mode::Walk, Mode::Bike, Mode::BikeShare]
        } else {
            &[Mode::Walk, Mode::Bike]
        };
        let mut run = Run::new(Arc::clone(&road), Arc::new(t), Arc::new(tr), Second(86_400))
            .with_layers(Arc::clone(&layers))
            .with_choice_model(Arc::from(
                openmobisim_core_choice::model("logit", &BTreeMap::new()).expect("logit"),
            ))
            .with_equilibration(Arc::from(
                openmobisim_core_sim::equilibration::strategy("msa", &options).expect("msa"),
            ))
            .with_mode_choice(modes);
        if with {
            run = run.with_services(Arc::new(vec![service.clone()]));
        }
        run
    };
    for with in [false, true, false, true] {
        let mut run = make(with);
        let started = std::time::Instant::now();
        let result = run.execute(&mut Diagnostics::new());
        let seconds = started.elapsed().as_secs_f64();
        let engine = result.services.as_ref().map_or(0.0, |s| s.engine_s);
        let share = result.by_mode[Mode::BikeShare.index()].completion.total_trips;
        println!(
            "service {with}: {seconds:.3} s, engine {engine:.4} s ({:.1} %), shared-bike trips {share}",
            100.0 * engine / seconds
        );
        let mut by: BTreeMap<&str, f64> = BTreeMap::new();
        for st in run.timings() {
            *by.entry(st.name).or_default() += st.seconds;
        }
        println!("  by stage (s): {by:?}");
    }
}
