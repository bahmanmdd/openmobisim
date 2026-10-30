//! A taxi fleet in a run (S302, S303): trips choosing their mode are offered a taxi, door to
//! door, planned on a fixed assumed wait and chance of no taxi; once the run has stopped, the
//! fleet engine plays the requests of its last loading's taxi trips (the built-in matching, the
//! charging rule), on that loading's link times, and reports what actually happened beside what
//! was planned.
//!
//! The network: a 5 × 5 two-way grid of 200 m blocks. Travellers own no car and no bike; they
//! choose between walking and a taxi. The fleet's vehicles stand at grid corners.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    reason = "counts of trips and vehicles are small whole numbers held exactly"
)]

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
use openmobisim_core_sim::services::engine::{FleetDay, FleetEngine};
use openmobisim_core_sim::services::in_run::ServiceOutcome;
use openmobisim_core_sim::services::outcome::LegStatus;
use openmobisim_core_sim::services::taxi_tables::{
    TaxiEventTable, TaxiTripTable, TaxiVehicleTable,
};
use openmobisim_core_sim::services::taxis::TaxiEventKind;
use openmobisim_core_sim::services::world::FrozenWorld;
use openmobisim_core_sim::services::{
    ChargerRow, DockDefaults, DockRow, ServiceAssumptions, ServiceSetup, TaxiDefaults, VehicleRow,
};
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

/// One layer of `road`, owned, as a frozen day's city holds it.
fn layer(road: &RoadNetwork, which: StaticLayer) -> Arc<LayerSetup> {
    let d = StaticLayerDefaults::SHIPPED;
    let net = StaticNetwork::derive(road, which, d, &mut Diagnostics::new()).expect("derives");
    Arc::new(LayerSetup::new(Arc::new(net), BikeCost::Dedicated, d))
}

fn at(road: &RoadNetwork, row: u32, col: u32) -> LonLat {
    let id = road.node_external_ids().id_of(&node_name(row, col)).expect("a grid node");
    road.node_lonlat(EntityId::new(id))
}

/// A fleet of vehicles at `places` (row, column), with `soc` each, and one charger of one plug
/// at the grid's centre.
fn fleet(road: &RoadNetwork, places: &[(u32, u32)], soc: f64, d: TaxiDefaults) -> ServiceSetup {
    let vehicles: Vec<VehicleRow> = places
        .iter()
        .enumerate()
        .map(|(i, &(r, c))| VehicleRow {
            vehicle_id: format!("v{i}"),
            position: at(road, r, c),
            soc: Some(soc),
        })
        .collect();
    let chargers = vec![ChargerRow {
        charger_id: "c".to_string(),
        hub_id: None,
        position: at(road, 2, 2),
        plugs: 1,
        kw: None,
    }];
    ServiceSetup::taxi("taxi", road, &vehicles, &chargers, d).expect("set up")
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
    places: Vec<(u32, u32)>,
    soc: f64,
    defaults: TaxiDefaults,
    taxis: bool,
    docks: bool,
    model: Arc<dyn ChoiceModel>,
    iterations: u32,
    prices: Prices,
    class_taxi: bool,
    assumptions: ServiceAssumptions,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            places: vec![(0, 0), (0, 0)],
            soc: 0.8,
            defaults: TaxiDefaults::SHIPPED,
            taxis: true,
            docks: false,
            model: Arc::new(Deterministic),
            iterations: 1,
            prices: Prices::SHIPPED,
            class_taxi: true,
            assumptions: ServiceAssumptions::SHIPPED,
        }
    }
}

/// Four docks of four slots, two bikes each, at the first row's first two corners and the last
/// row's last two.
fn docks(road: &RoadNetwork, layers: &StaticLayers) -> ServiceSetup {
    let row = |id: &str, p: LonLat| DockRow {
        dock_id: id.to_string(),
        name: None,
        hub_id: None,
        position: p,
        capacity: 4,
        bikes: Some(2),
    };
    let rows = vec![
        row("a", at(road, 0, 0)),
        row("b", at(road, 0, 1)),
        row("y", at(road, 4, 3)),
        row("z", at(road, 4, 4)),
    ];
    ServiceSetup::bike_docked(
        "bike_share",
        &rows,
        layers.walk.as_ref().unwrap(),
        layers.bike.as_ref().unwrap(),
        Some(road),
        DockDefaults::SHIPPED,
    )
    .expect("set up")
}

fn run(rows: Vec<RawTrip>, s: &Setup) -> RunResult {
    let road = grid();
    let layers = layers(&road);
    let (travellers, trips) = demand(rows);
    let mut modes = vec![Mode::Walk, Mode::Taxi];
    let mut services = Vec::new();
    if s.docks {
        services.push(docks(&road, &layers));
        modes.push(Mode::BikeShare);
    }
    if s.taxis {
        services.push(fleet(&road, &s.places, s.soc, s.defaults));
    }
    let mut run = Run::new(road, travellers, trips, Second(86_400))
        .with_layers(layers)
        .with_choice_model(Arc::clone(&s.model))
        .with_prices(s.prices)
        .with_mode_choice(&modes)
        .with_service_assumptions(s.assumptions)
        .with_services(Arc::new(services));
    if !s.class_taxi {
        let mut allowed = [true; Mode::COUNT];
        allowed[Mode::Taxi.index()] = false;
        run = run.with_class_modes(vec![allowed]);
    }
    if s.iterations > 1 {
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

/// `n` trips from the grid's first corner to its last, one every `every_s` seconds from 100 s.
fn corner_trips(road: &RoadNetwork, n: u32, every_s: u32) -> Vec<RawTrip> {
    (0..n)
        .map(|k| trip(&format!("t{k:02}"), at(road, 0, 0), at(road, 4, 4), 100 + k * every_s))
        .collect()
}

fn logit() -> Arc<dyn ChoiceModel> {
    Arc::from(openmobisim_core_choice::model("logit", &BTreeMap::new()).expect("logit"))
}

#[test]
fn a_taxi_is_chosen_over_a_long_walk_and_its_requests_played_on_the_engine() {
    let road = grid();
    // Three travellers a second apart; two vehicles at their origin: two leave at once, the
    // third waits for one to come back from the far corner (about 1.6 km).
    let result = run(corner_trips(&road, 3, 1), &Setup::default());
    let taxi = result.by_mode[Mode::Taxi.index()];
    assert_eq!(taxi.completion.completed, 3, "{:?}", result.by_mode);
    assert_eq!(result.by_mode[Mode::Walk.index()].completion.total_trips, 0);
    let taxis = result.taxis.as_ref().expect("the run has a taxi fleet");
    let day = taxis.last.as_ref().expect("taxis were taken");
    let status: Vec<LegStatus> = day.outcome.iter().map(|o| o.status).collect();
    assert_eq!(status, [LegStatus::Completed; 3]);
    // The first two: matched at once, 0 km from the pickup, there after the dispatch delay,
    // boarded a minute later.
    let d = TaxiDefaults::SHIPPED;
    for o in &day.outcome[..2] {
        assert_eq!(o.waited_s, d.dispatch_delay_s);
        assert!(o.arrived_s > o.requested_s + d.boarding_s);
    }
    // The third waited for the first vehicle back: its whole round trip and more.
    let third = day.outcome[2];
    assert!(third.waited_s > 2.0 * (day.outcome[0].arrived_s - 100.0 - d.boarding_s));
    assert_eq!(third.vehicle, Some(0), "the first freed, the first offered");
    assert_eq!(day.day.requests.len(), 3, "one seat each");
    assert!(result.modes_listed().contains(&Mode::Taxi));
    assert!(!result.modes_listed().contains(&Mode::BikeShare));
    assert!(result.services.is_none(), "no docked service, no bike-share results");
}

#[test]
fn without_a_taxi_fleet_the_taxi_is_neither_offered_nor_listed() {
    let road = grid();
    let result = run(corner_trips(&road, 3, 1), &Setup { taxis: false, ..Setup::default() });
    assert_eq!(result.by_mode[Mode::Walk.index()].completion.completed, 3);
    assert_eq!(result.by_mode[Mode::Taxi.index()].completion.total_trips, 0);
    assert!(result.taxis.is_none());
    assert!(!result.modes_listed().contains(&Mode::Taxi));
}

#[test]
fn a_request_no_taxi_can_pick_up_in_time_is_cancelled_and_ends_there_with_no_walk_on() {
    let road = grid();
    // One vehicle at the far corner, a cancel limit of 30 s: it cannot reach the pickup in
    // time, so the request is cancelled, and the trip ends there (S303): no walk on, no event
    // after.
    let s = Setup {
        places: vec![(4, 4)],
        defaults: TaxiDefaults {
            request_cancel_s: 30.0,
            dispatch_delay_s: 0.0,
            ..TaxiDefaults::SHIPPED
        },
        ..Setup::default()
    };
    let rows = vec![trip("t", at(&road, 0, 0), at(&road, 4, 4), 100)];
    // With the constant of the taxi made large (logit), it is taken, and cancelled.
    let options = BTreeMap::from([("beta_mode_taxi".to_string(), 50.0)]);
    let model: Arc<dyn ChoiceModel> =
        Arc::from(openmobisim_core_choice::model("logit", &options).expect("logit"));
    let (travellers, trips) = demand(rows.clone());
    let setup = fleet(&road, &s.places, s.soc, s.defaults);
    let result = run(rows, &Setup { model, ..s });
    let day = result.taxis.as_ref().unwrap().last.as_ref().expect("a taxi requested");
    assert_eq!(day.outcome[0].status, LegStatus::Cancelled);
    assert!(day.arrived_s[0].is_nan(), "no arrival: {}", day.arrived_s[0]);
    assert_eq!(day.outcomes[0].0, ServiceOutcome::Cancelled);
    assert!(day.outcomes[0].1.is_nan());
    let sm = day.summary;
    assert_eq!((sm.requests, sm.cancelled, sm.rides, sm.unfinished), (1.0, 1.0, 0.0, 0.0));
    assert!(sm.waiting_mean_s.is_nan() && sm.waiting_p90_s.is_nan(), "no ride to average");
    // The trip row shows the planned trip: completed at its departure plus its planned time.
    assert_eq!(result.by_mode[Mode::Taxi.index()].completion.completed, 1);
    let planned = (100.0_f64 + day.legs[0].planned_s).floor();
    let done = result.events.iter().find(|e| e.entity_id == 0).expect("the trip's event");
    assert_eq!(f64::from(done.second.get()), planned);
    let t = TaxiTripTable::new(day, &setup, &trips, &travellers);
    assert_eq!(t.outcome, ["cancelled"]);
    assert_eq!(
        (t.pickup_s.clone(), t.dropoff_s.clone(), t.waiting_s.clone()),
        (vec![None], vec![None], vec![None])
    );
    let e = TaxiEventTable::new(day, &setup, &trips, &travellers);
    assert_eq!(e.event, ["asked_for_taxi", "waiting_for_taxi", "request_cancelled"]);
    // In the engine's own log, nothing after the cancellation.
    let log = day.day.log.as_ref().expect("a log");
    assert_eq!(log.last().map(|e| e.kind), Some(TaxiEventKind::RequestCancelled));
}

#[test]
fn a_class_without_taxis_never_takes_one() {
    let road = grid();
    let result = run(corner_trips(&road, 3, 1), &Setup { class_taxi: false, ..Setup::default() });
    assert_eq!(result.by_mode[Mode::Walk.index()].completion.completed, 3);
    assert_eq!(result.by_mode[Mode::Taxi.index()].completion.total_trips, 0);
}

#[test]
fn a_fare_moves_travellers_from_taxis_to_walking() {
    let road = grid();
    let share = |prices: Prices| {
        let r = run(
            corner_trips(&road, 40, 15),
            &Setup { model: logit(), prices, places: vec![(0, 0); 4], ..Setup::default() },
        );
        r.by_mode[Mode::Taxi.index()].completion.total_trips
    };
    let free = share(Prices::SHIPPED);
    let per_km = share(Prices { taxi_eur_per_km: 2.0, ..Prices::SHIPPED });
    let per_ride = share(Prices { taxi_eur_per_ride: 4.0, ..Prices::SHIPPED });
    assert!(free > 0 && per_km < free && per_ride < free, "free {free}, {per_km}, {per_ride}");
}

#[test]
fn same_seed_same_day() {
    let road = grid();
    let s = || Setup { model: logit(), iterations: 4, ..Setup::default() };
    let a = run(corner_trips(&road, 12, 20), &s());
    let b = run(corner_trips(&road, 12, 20), &s());
    let (ta, tb) = (a.taxis.unwrap(), b.taxis.unwrap());
    let (da, db) = (ta.last.unwrap(), tb.last.unwrap());
    // Compared as printed: outcomes hold NaN, never equal to itself.
    assert_eq!(
        format!("{:?}", (&da.legs, &da.outcome, &da.day, &da.summary)),
        format!("{:?}", (&db.legs, &db.outcome, &db.day, &db.summary)),
    );
    assert_eq!(ta.assumed, tb.assumed);
    assert_eq!(a.events, b.events);
}

#[test]
fn the_events_add_up_to_the_trips_and_the_vehicles() {
    let road = grid();
    // Twelve travellers, two vehicles low on charge: some wait, the vehicles go to charge.
    let s = Setup { model: logit(), soc: 0.205, ..Setup::default() };
    let rows = corner_trips(&road, 12, 20);
    let (travellers, trips) = demand(rows.clone());
    let result = run(rows, &s);
    let setup = fleet(&road, &s.places, s.soc, s.defaults);
    let day = result.taxis.as_ref().unwrap().last.as_ref().expect("taxis taken");
    let events = TaxiEventTable::new(day, &setup, &trips, &travellers);
    let trip_rows = TaxiTripTable::new(day, &setup, &trips, &travellers);
    let vehicles = TaxiVehicleTable::new(Some(day), &setup, (0.0, 86_400.0));
    let count = |e: &str| events.event.iter().filter(|&&x| x == e).count() as f64;
    let carried = trip_rows.outcome.iter().filter(|r| **r == "completed").count() as f64;
    assert_eq!(count("asked_for_taxi"), trip_rows.len() as f64);
    assert_eq!(count("dropped_off"), carried);
    assert_eq!(count("dropped_off"), vehicles.passengers_dropped_off.iter().sum::<f64>());
    let km = |e: &str| {
        events
            .event
            .iter()
            .zip(&events.drive_km)
            .filter(|(x, _)| **x == e)
            .map(|(_, k)| k.unwrap_or(0.0))
            .sum::<f64>()
    };
    let occupied: f64 = vehicles.km_occupied.iter().sum();
    let empty: f64 = vehicles.km_empty.iter().sum();
    assert!((km("picked_up") - occupied).abs() < 1e-9, "{} {occupied}", km("picked_up"));
    assert!((km("taxi_assigned") + km("vehicle_went_to_charge") - empty).abs() < 1e-9);
    assert!(count("vehicle_went_to_charge") > 0.0, "the vehicles charge");
    // One plug: a vehicle that finds it taken queues, then starts when the plug frees.
    assert!(count("queued_for_plug") > 0.0, "one plug for two vehicles");
    assert_eq!(count("vehicle_went_to_charge"), count("started_charging"));
    assert!(count("charged") <= count("started_charging"));
    // Each vehicle's seconds in every state add up to the window, bin by bin.
    let len = 86_400.0 / f64::from(openmobisim_core_sim::services::outcome::FLEET_BIN_S);
    let total: f64 = vehicles.seconds.iter().flatten().sum();
    assert!((total - 2.0 * 86_400.0).abs() < 1e-6, "{total}, {len} bins");
    assert_eq!(day.day.requests.len(), 12);
    // The summary in a dispatcher's words adds up and agrees with the trip rows.
    let sm = day.summary;
    assert_eq!(sm.requests, 12.0);
    assert_eq!(sm.rides + sm.cancelled + sm.unfinished, sm.requests);
    assert_eq!(sm.rides, carried);
    let waits: Vec<f64> = trip_rows
        .outcome
        .iter()
        .zip(&trip_rows.waiting_s)
        .filter(|(o, _)| **o == "completed")
        .map(|(_, w)| w.expect("a ride waited"))
        .collect();
    let mean = waits.iter().sum::<f64>() / waits.len() as f64;
    assert!((sm.waiting_mean_s - mean).abs() < 1e-9, "{} vs {mean}", sm.waiting_mean_s);
    assert!(sm.waiting_median_s <= sm.waiting_p90_s);
    assert!(waits.contains(&sm.waiting_median_s) && waits.contains(&sm.waiting_p90_s));
    assert!((sm.empty_km_per_ride * sm.rides - empty).abs() < 1e-9);
    assert!(sm.utilisation > 0.0 && sm.utilisation < 1.0, "{}", sm.utilisation);
    assert!((sm.rides_per_taxi - sm.rides / 2.0).abs() < 1e-12);
}

/// The taxi trips of the corner trips (40, logit, a fare of 1 EUR so that some but not all take
/// a taxi) when travellers assume `assumed` about the fleet whose defaults are `defaults`.
fn taxi_share(road: &Arc<RoadNetwork>, assumed: ServiceAssumptions, defaults: TaxiDefaults) -> u32 {
    let s = Setup {
        model: logit(),
        places: vec![(0, 0); 4],
        prices: Prices { taxi_eur_per_ride: 1.0, ..Prices::SHIPPED },
        assumptions: assumed,
        defaults,
        ..Setup::default()
    };
    let r = run(corner_trips(road, 40, 15), &s);
    let taxi = r.by_mode[Mode::Taxi.index()].completion.total_trips;
    let summary = r.taxis.as_ref().and_then(|t| t.last.as_ref()).map(|d| d.summary);
    println!("{assumed:?} | taxi trips {taxi:2} of 40 | {summary:?}");
    taxi
}

/// A shorter assumed wait draws more taxi trips, and a chance of no taxi fewer (S303). Printed
/// (`cargo test --test services_taxi_runs -- --nocapture`): the taxi trips and the service
/// day's summary.
#[test]
fn the_assumed_wait_and_chance_of_no_taxi_move_the_taxi_share() {
    let road = grid();
    let with = |wait: f64, p: f64| {
        let a = ServiceAssumptions {
            assumed_wait_s: wait,
            assumed_no_taxi_share: p,
            ..ServiceAssumptions::SHIPPED
        };
        taxi_share(&road, a, TaxiDefaults::SHIPPED)
    };
    let (short, shipped, unsure) = (with(300.0, 0.0), with(600.0, 0.0), with(600.0, 0.25));
    assert!(short > shipped, "300 s {short}, 600 s {shipped}");
    assert!(shipped > unsure, "p 0 {shipped}, p 0.25 {unsure}");
    assert!(shipped > 0 && shipped < 40, "some but not all: {shipped}");
}

/// The assumed wait is what travellers believe, whatever the fleet's cancel limit (S303): above
/// the limit it still lowers the taxi share (nothing caps it), and the engine never reads it:
/// the same requests give the same service day whatever the travellers assumed.
#[test]
fn an_assumed_wait_above_the_cancel_limit_is_not_capped_and_the_engine_ignores_it() {
    let road = grid();
    let fleet_limit = TaxiDefaults { request_cancel_s: 600.0, ..TaxiDefaults::SHIPPED };
    let at = |wait: f64| ServiceAssumptions { assumed_wait_s: wait, ..ServiceAssumptions::SHIPPED };
    let (limit, above) =
        (taxi_share(&road, at(600.0), fleet_limit), taxi_share(&road, at(900.0), fleet_limit));
    assert!(above < limit, "900 s assumed {above}, 600 s {limit}: no cap");
    // Every trip takes a taxi (a large constant): the same legs whatever the assumed wait, and
    // the same day in the engine; only the planned arrivals differ.
    let options = BTreeMap::from([("beta_mode_taxi".to_string(), 50.0)]);
    let model: Arc<dyn ChoiceModel> =
        Arc::from(openmobisim_core_choice::model("logit", &options).expect("logit"));
    let day = |wait: f64| {
        let s = Setup {
            model: Arc::clone(&model),
            places: vec![(0, 0); 4],
            assumptions: at(wait),
            defaults: fleet_limit,
            ..Setup::default()
        };
        run(corner_trips(&road, 12, 15), &s).taxis.unwrap().last.expect("taxis requested")
    };
    let (a, b) = (day(600.0), day(900.0));
    assert_eq!(a.legs.len(), 12);
    assert_eq!(
        format!("{:?}", (a.legs.iter().map(|l| l.leg).collect::<Vec<_>>(), &a.outcome, &a.day)),
        format!("{:?}", (b.legs.iter().map(|l| l.leg).collect::<Vec<_>>(), &b.outcome, &b.day)),
    );
    // 300 s later, to rounding (exact on one platform, a last digit off on another).
    assert!(
        a.legs.iter().zip(&b.legs).all(|(x, y)| (y.planned_s - x.planned_s - 300.0).abs() < 1e-6)
    );
}

/// The run's service day is what an engine day of its legs on its road times plays (S303): the
/// day a frozen day of the run replays (`fleet_day(frozen=...)` builds this world from the same
/// legs and road times), bikes and taxis, leg by leg.
#[test]
fn the_service_day_is_the_replay_of_its_legs_on_its_road_times() {
    let road = grid();
    let layers = layers(&road);
    // A taxi constant of 2 (logit): some take a taxi, some a shared bike.
    let options = BTreeMap::from([("beta_mode_taxi".to_string(), 2.0)]);
    let model: Arc<dyn ChoiceModel> =
        Arc::from(openmobisim_core_choice::model("logit", &options).expect("logit"));
    let s = Setup {
        model,
        docks: true,
        iterations: 3,
        places: vec![(0, 0), (2, 2)],
        ..Setup::default()
    };
    let result = run(corner_trips(&road, 16, 40), &s);
    let share = result.services.as_ref().unwrap().last.as_ref().expect("bikes ridden");
    let taxis = result.taxis.as_ref().unwrap().last.as_ref().expect("taxis taken");
    let times = result.service_road_times.clone().expect("a run with services keeps them");
    let mut legs: Vec<_> = share.legs.iter().map(|l| l.leg).collect();
    legs.extend(taxis.legs.iter().map(|l| l.leg));
    let setups = Arc::new(vec![docks(&road, &layers), fleet(&road, &s.places, s.soc, s.defaults)]);
    let turns = Arc::new(openmobisim_core_graph::turns::TurnTable::build(
        &road,
        openmobisim_core_graph::defaults::SignalDefaults::SHIPPED,
    ));
    let world = FrozenWorld::new(
        Arc::clone(&road),
        turns,
        layer(&road, StaticLayer::Walk),
        layer(&road, StaticLayer::Bike),
        times,
        Arc::clone(&setups),
        &legs,
    );
    let day = FleetDay { window: (0.0, 86_400.0), legs, seed: 0 };
    let replay = FleetEngine::in_world(setups, Arc::new(day), Arc::new(world), None)
        .run_day()
        .expect("no policy");
    let n = share.legs.len();
    println!("shared-bike legs {n}, taxi legs {}", taxis.legs.len());
    assert_eq!(format!("{:?}", &replay.legs[..n]), format!("{:?}", share.outcome.legs));
    assert_eq!(format!("{:?}", &replay.legs[n..]), format!("{:?}", taxis.outcome));
    assert_eq!(format!("{:?}", replay.taxis[1]), format!("{:?}", Some(&taxis.day)));
    assert_eq!(format!("{:?}", replay.docks[0]), format!("{:?}", share.outcome.docks[0]));
}

#[test]
fn shared_bikes_and_taxis_play_in_one_day_and_the_bikes_as_alone() {
    let road = grid();
    // Walk, shared bike and taxi offered; the one vehicle stands at the far corner, the
    // cancel limit is 30 s and every request is assumed cancelled (the assumed chance of no
    // taxi 1), so the taxi costs the cancel limit plus the best other alternative in the
    // choice, slower than the shared bike: the same legs ride the bikes as without a taxi
    // fleet, and their day must be the same in one engine day with the fleet.
    let taxi =
        TaxiDefaults { request_cancel_s: 30.0, dispatch_delay_s: 0.0, ..TaxiDefaults::SHIPPED };
    let assumptions =
        ServiceAssumptions { assumed_no_taxi_share: 1.0, ..ServiceAssumptions::SHIPPED };
    let both = run(
        corner_trips(&road, 6, 5),
        &Setup {
            docks: true,
            places: vec![(4, 4)],
            defaults: taxi,
            assumptions,
            ..Setup::default()
        },
    );
    let bikes_alone =
        run(corner_trips(&road, 6, 5), &Setup { docks: true, taxis: false, ..Setup::default() });
    assert!(both.modes_listed().contains(&Mode::Taxi));
    assert!(both.modes_listed().contains(&Mode::BikeShare));
    assert_eq!(both.by_mode[Mode::BikeShare.index()].completion.completed, 6, "{:?}", both.by_mode);
    let (a, b) = (both.services.unwrap(), bikes_alone.services.unwrap());
    let (da, db) = (a.last.expect("bikes ridden"), b.last.expect("bikes ridden"));
    assert_eq!(format!("{:?}", da.outcome.legs), format!("{:?}", db.outcome.legs));
    // The docked service first in both runs; the day with taxis holds the fleet's entry after.
    assert_eq!(format!("{:?}", da.outcome.docks[0]), format!("{:?}", db.outcome.docks[0]));
    assert_eq!(format!("{:?}", da.outcome.dock_log[0]), format!("{:?}", db.outcome.dock_log[0]));
    assert_eq!(da.outcome.docks.len(), 2);
}
