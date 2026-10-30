//! A taxi fleet in a run (S302): trips choosing their mode are offered a taxi, door to door;
//! after each loading the fleet engine plays the requests of those who took one (the built-in
//! matching, the charging rule), on the loading's link times, and the fleet's realised waits and
//! failures are averaged into what the next choice expects.
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
use openmobisim_core_sim::services::outcome::LegStatus;
use openmobisim_core_sim::services::taxi_tables::{
    TaxiEventTable, TaxiTripTable, TaxiVehicleTable,
};
use openmobisim_core_sim::services::{
    ChargerRow, DockDefaults, DockRow, ServiceSetup, TaxiDefaults, VehicleRow,
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
    assert_eq!(status, [LegStatus::Served; 3]);
    // The first two: sent at once, 0 km from the pickup, boarded a minute later.
    let d = TaxiDefaults::SHIPPED;
    for o in &day.outcome[..2] {
        assert_eq!(o.waited_s, 0.0);
        assert!(o.arrived_s > o.requested_s + d.board_s);
    }
    // The third waited for the first vehicle back: its whole round trip and more.
    let third = day.outcome[2];
    assert!(third.waited_s > 2.0 * (day.outcome[0].arrived_s - 100.0 - d.board_s));
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
fn a_request_no_vehicle_reaches_walks_on_from_the_origin() {
    let road = grid();
    // One vehicle at the far corner, a longest wait of 30 s: it cannot reach the pickup in
    // time, so the request fails and the traveller walks the whole way, setting off at 130 s.
    let s = Setup {
        places: vec![(4, 4)],
        defaults: TaxiDefaults { max_wait_s: 30.0, ..TaxiDefaults::SHIPPED },
        ..Setup::default()
    };
    let rows = vec![trip("t", at(&road, 0, 0), at(&road, 4, 4), 100)];
    // With the constant of the taxi made large (logit), it is taken, fails and walks on.
    let options = BTreeMap::from([("beta_mode_taxi".to_string(), 50.0)]);
    let model: Arc<dyn ChoiceModel> =
        Arc::from(openmobisim_core_choice::model("logit", &options).expect("logit"));
    let (travellers, trips) = demand(rows.clone());
    let setup = fleet(&road, &s.places, s.soc, s.defaults);
    let result = run(rows, &Setup { model, ..s });
    let day = result.taxis.as_ref().unwrap().last.as_ref().expect("a taxi asked for");
    assert_eq!(day.outcome[0].status, LegStatus::Failed);
    let walk = day.legs[0].walk_s;
    assert!(walk > 1000.0, "1.6 km on foot: {walk}");
    assert_eq!(day.arrived_s[0], 100.0 + 30.0 + walk);
    assert_eq!(result.by_mode[Mode::Taxi.index()].completion.completed, 1);
    let t = TaxiTripTable::new(day, &setup, &trips, &travellers);
    assert_eq!(t.result, ["no_taxi_walked_to_destination"]);
    let e = TaxiEventTable::new(day, &setup, &trips, &travellers);
    assert_eq!(
        e.event,
        [
            "asked_for_taxi",
            "waiting_for_taxi",
            "no_taxi_within_wait_limit",
            "gave_up_walking_to_destination",
            "reached_destination"
        ]
    );
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
    assert_eq!(ta.legs_people, tb.legs_people);
    let bits = |v: &[f64]| v.iter().map(|m| m.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&ta.mismatch_s), bits(&tb.mismatch_s));
    let (da, db) = (ta.last.unwrap(), tb.last.unwrap());
    // Compared as printed: outcomes hold NaN, never equal to itself.
    assert_eq!(
        format!("{:?}", (&da.legs, &da.outcome, &da.day)),
        format!("{:?}", (&db.legs, &db.outcome, &db.day)),
    );
    assert_eq!(ta.expected, tb.expected);
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
    let carried = trip_rows
        .result
        .iter()
        .filter(|r| matches!(**r, "as_planned" | "waited_longer_than_expected"))
        .count() as f64;
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
}

/// The hypothesis of the expected costs: the mismatch per trip between the door-to-door seconds
/// a choice expected and those its traveller met shrinks over the loadings. Printed per loading
/// (`cargo test --test services_taxi_runs -- --nocapture`): the travellers, the mean absolute
/// mismatch, the signed mean (met less expected), the mean wait and the share with no taxi.
#[test]
fn the_mismatch_per_loading_is_printed_and_shrinks() {
    let road = grid();
    // Thirty travellers over fifteen minutes, three vehicles at their origin and a longest wait
    // of five minutes: the first loading, on the start-of-day estimate (no wait), sends nearly
    // everyone by taxi, and many get none.
    let s = Setup {
        model: logit(),
        iterations: 15,
        places: vec![(0, 0); 3],
        defaults: TaxiDefaults { max_wait_s: 300.0, ..TaxiDefaults::SHIPPED },
        ..Setup::default()
    };
    let result = run(corner_trips(&road, 30, 30), &s);
    let t = result.taxis.as_ref().expect("a taxi fleet");
    println!("loading | taxi | mean |met - expected| s | mean met - expected s | wait s | no taxi");
    for k in 0..t.mismatch_s.len() {
        println!(
            "{k:7} | {:4} | {:24.1} | {:21.1} | {:6.1} | {:7.3}",
            t.legs_people[k], t.mismatch_s[k], t.bias_s[k], t.wait_mean_s[k], t.no_taxi_share[k]
        );
    }
    assert_eq!(t.mismatch_s.len(), result.iterations.len());
    let first = t.mismatch_s[0];
    let last = *t.mismatch_s.last().unwrap();
    assert!(first.is_finite() && last < first, "first {first}, last {last}");
}

#[test]
fn shared_bikes_and_taxis_play_in_one_day_and_the_bikes_as_alone() {
    let road = grid();
    // Walk, shared bike and taxi offered; the one vehicle stands at the far corner and the
    // longest wait is 30 s, so every request is expected to fail and the taxi is slower than the
    // shared bike: the same legs ride the bikes as without a taxi fleet, and their day must be
    // the same in one engine day with the fleet.
    let taxi = TaxiDefaults { max_wait_s: 30.0, ..TaxiDefaults::SHIPPED };
    let both = run(
        corner_trips(&road, 6, 5),
        &Setup { docks: true, places: vec![(4, 4)], defaults: taxi, ..Setup::default() },
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
