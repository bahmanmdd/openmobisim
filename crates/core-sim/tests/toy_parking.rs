//! Park-and-ride and bike-and-ride on the toy network (M4, S201): every number
//! derived by hand (the toy network's multimodal extension).
//!
//! **The pieces.** Hub `H` at `D2` holds **6 car and 3 bike spaces**, where tram
//! `T1`'s stop `H` is (a walk of 0 m); car park **`P2` at `R2`**, 20 spaces, is
//! 300 m from that stop on foot (`e2` walked, **225 s**). The tram leaves `H` for
//! `N1` at `600k + 300` (300 s ride) and `N1` for `H` at `600k` (300 s); a
//! passenger boards only if at the stop 60 s before. Cars at free flow: `W → D2`
//! **180.394 s** (a1's Webster delay 20.554 s included), `W → R2` **144.394 s**
//! (without `e2`, 36 s), `R2 → D2` **36 s**. The bike `S → D2` **248.48 s**.
//! Arrivals are floored to whole seconds when recorded. Parking takes **120 s**
//! for a car and **30 s** for a bike when there is room, plus **600 s** and
//! **180 s** times the share of the time bin (900 s) the parking was full;
//! fetching takes the floor alone.
//!
//! | Case | Trip | Hand value |
//! |---|---|---|
//! | PR1 | car `W → H`, park, tram to `N1`, leaving 0 | car at 180, parked 300 (0 + 180.394 + 120, floored), ready 360 > 300: the 900 tram, `N1` at **1200** |
//! | PR2 | seven cars `W → H` at 0 | the seventh finds 6 spaces taken: overflow 1; full from 180 to the end: availability 0.2 in bin 0, so everyone pays 120 + 600 · 0.8 = **600 s** (expected 120: mismatch **480 s**), parked at 780, the 900 tram, **1200** |
//! | PR3 | `W → N1`, parkings `H` and `P2` | `H`: parked 300, waits 600 for the 900 tram; `P2`: parked 264, walks 225, at the stop 489, waits 411; both **1200**. Deterministic: `H` (the tie goes to the lower parking). Logit: ΔU = −0.09·(600 − 411)/60 + 0.13·225/60 = **0.204**, P(H) = 1/(1 + e^−0.204) |
//! | PR4 | out via `P2` (`P2` only), back `N1 → D2` at 1800 | out: **1200** as PR3's `P2`; back: at `N1` 1800, the 2400 tram, `H` 2700, walk 225 to `P2` 2925, fetch 120, drive 36: **3081** |
//! | PR5 | out via `H`, then a car trip `N1 → W` | the car is at `H`: **no vehicle available** |
//! | BR1 | bike `S → H`, park, tram to `N1` | bike at 248, parked 278, ready 338 > 300: the 900 tram, **1200** |
//! | BR2 | four bikes `S → H` at 0 | the fourth finds 3 spaces taken; full from 248: availability 1 − 652/900, parking 30 + 180 · 652/900 = **160.4 s**, parked 408, the 900 tram, **1200** |

#![allow(clippy::float_cmp, reason = "travel times here are whole seconds")]

use std::sync::Arc;

use openmobisim_core_choice::{Logit, Options};
use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::examples::toy_network_parkings;
use openmobisim_core_graph::hubs::ParkingRow;
use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::{LonLat, toy_network, toy_network_layers};
use openmobisim_core_sim::{
    EventType, LayerSetup, NO_PARKING, ParkingDefaults, ParkingSetup, Run, RunResult, StaticLayers,
    TransitSetup,
};
use openmobisim_core_transit::TransitDefaults;
use openmobisim_core_transit::examples::toy_tram;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, NodeId};
use openmobisim_core_types::time::Second;

struct Toy {
    road: Arc<RoadNetwork>,
    layers: Arc<StaticLayers>,
    transit: Arc<TransitSetup>,
}

fn toy() -> Toy {
    let (road, _) = toy_network();
    let road = Arc::new(road);
    let (bike, walk) = toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let layers = Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(Arc::new(bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d)),
    });
    let transit = Arc::new(TransitSetup::new(
        Arc::new(toy_tram()),
        layers.walk.as_ref().unwrap(),
        layers.bike.as_ref(),
        TransitDefaults::SHIPPED,
    ));
    Toy { road, layers, transit }
}

impl Toy {
    fn at(&self, name: &str) -> LonLat {
        let node: NodeId = self.road.node_external_ids().typed_id_of(name).expect("a toy node");
        self.road.node_lonlat(node)
    }

    fn trip(&self, who: &str, seq: u32, from: &str, to: &str, t: u32, mode: Mode) -> RawTrip {
        RawTrip {
            traveller_id: who.to_string(),
            trip_seq: seq,
            origin: self.at(from),
            destination: self.at(to),
            departure_time: Second(t),
            user_class: "everyone".to_string(),
            weight: None,
            mode: Some(mode),
        }
    }

    fn parking(&self, rows: &[ParkingRow]) -> Arc<ParkingSetup> {
        Arc::new(
            ParkingSetup::new(
                rows,
                self.road.clone(),
                self.layers.bike.as_ref(),
                &self.transit,
                ParkingDefaults::SHIPPED,
            )
            .expect("the toy's parkings"),
        )
    }

    fn run_with(&self, rows: Vec<RawTrip>, parkings: &[ParkingRow], logit: bool) -> RunResult {
        let defaults = ClassDefaults::new()
            .with_default("everyone", Ownership { car: true, bike: true, transit_pass: false });
        let (travellers, trips) =
            build_travellers(rows, Vec::new(), &defaults, 1, &mut Diagnostics::new()).unwrap();
        let mut run =
            Run::new(self.road.clone(), Arc::new(travellers), Arc::new(trips), Second(20_000))
                .with_layers(self.layers.clone())
                .with_transit(self.transit.clone())
                .with_link_bins(300);
        if !parkings.is_empty() {
            run = run.with_parking(self.parking(parkings));
        }
        if logit {
            run = run.with_choice_model(Arc::new(Logit::from_options(&Options::new()).unwrap()));
        }
        run.execute(&mut Diagnostics::new())
    }

    fn run(&self, rows: Vec<RawTrip>) -> RunResult {
        self.run_with(rows, &toy_network_parkings(), false)
    }
}

/// Each trip's outcome and second, in trip order.
fn outcomes(result: &RunResult) -> Vec<(EventType, u32)> {
    let mut e: Vec<_> =
        result.events.iter().map(|e| (e.entity_id, e.event_type, e.second.get())).collect();
    e.sort_by_key(|x| x.0);
    e.into_iter().map(|(_, t, s)| (t, s)).collect()
}

fn only(rows: &[&str]) -> Vec<ParkingRow> {
    toy_network_parkings().into_iter().filter(|r| rows.contains(&r.parking_id.as_str())).collect()
}

#[test]
fn the_parkings_are_hubs_with_their_stops_on_foot() {
    let t = toy();
    let p = t.parking(&toy_network_parkings());
    assert_eq!(p.report().rows, 3);
    assert_eq!((p.report().kept_car, p.report().kept_bike), (2, 1));
    assert_eq!(p.hubs().len(), 2, "H (car and bike) and P2");
    let by_id = |id: &str| (0..3u32).find(|&i| p.external_id(i) == id).expect("a parking");
    let (h, p2) = (by_id("H-car"), by_id("P2"));
    assert_eq!(p.hub_external_id(h), "H");
    assert_eq!(p.capacity(h), 6);
    assert_eq!(p.capacity(by_id("H-bike")), 3);
    let stop_h = t.transit.timetable().stop_ids().id_of("H").expect("stop H");
    assert_eq!(p.stops(h), &[(NodeId::new(stop_h), 0)], "the stop is where the parking is");
    assert_eq!(p.stops(p2), &[(NodeId::new(stop_h), 225)], "300 m on foot");
}

#[test]
fn pr1_park_and_ride_drives_parks_and_rides_on() {
    let t = toy();
    let r = t.run(vec![t.trip("a", 0, "W", "N1", 0, Mode::CarTransit)]);
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 1200)]);
    let it = r.itineraries.as_ref().expect("itineraries");
    assert_eq!(it.direction, [1]);
    assert_eq!(it.rides, [1]);
    assert_eq!(it.alternatives, [2], "H and P2");
    let parking = r.parking.as_ref().expect("parking");
    assert_eq!((parking.arrivals, parking.overflow_arrivals), (1.0, 0.0));
    assert_eq!(parking.mismatch_s, 0.0);
    // The car drove on the roads: a1 carries it.
    let bins = r.link_bins.as_ref().expect("bins");
    assert!(!bins.is_empty());
    // One boarding at H.
    let transit = r.transit.as_ref().expect("transit");
    assert_eq!(transit.boardings.iter().sum::<f64>(), 1.0);
}

#[test]
fn pr2_the_seventh_car_parks_anyway_and_everyone_pays_the_bins_availability() {
    let t = toy();
    let rows =
        (0..7).map(|i| t.trip(&format!("c{i}"), 0, "W", "N1", 0, Mode::CarTransit)).collect();
    let r = t.run_with(rows, &only(&["H-car"]), false);
    assert_eq!(outcomes(&r), vec![(EventType::TripCompleted, 1200); 7]);
    let p = r.parking.as_ref().expect("parking");
    assert_eq!((p.arrivals, p.overflow_arrivals), (7.0, 1.0));
    assert!((p.mismatch_s - 480.0).abs() < 1e-9, "{}", p.mismatch_s);
    let b = &p.bins;
    assert_eq!((b.bin_s, b.occupancy_max[0], b.overflow_max[0]), (900, 7.0, 1.0));
    assert!((b.full_s[0] - 720.0).abs() < 1e-9);
    assert_eq!(b.full_s[1], 900.0, "full for the rest of the day");
    assert_eq!(b.arrivals[0], 7.0);
}

#[test]
fn pr3_the_station_is_chosen_deterministically_and_by_the_logits_closed_form() {
    let t = toy();
    let rows = || vec![t.trip("a", 0, "W", "N1", 0, Mode::CarTransit)];
    let r = t.run(rows());
    let it = r.itineraries.as_ref().expect("itineraries");
    let p = t.parking(&toy_network_parkings());
    assert_eq!(p.external_id(it.parking[0]), "H-car", "a tie on time goes to the lower parking");
    assert_eq!(it.expected_s, [1200.0]);
    // Logit: H waits 600 s, P2 walks 225 s and waits 411 s.
    let r = t.run_with(rows(), &toy_network_parkings(), true);
    let it = r.itineraries.as_ref().expect("itineraries");
    let du: f64 = -0.09 * (600.0 - 411.0) / 60.0 + 0.13 * 225.0 / 60.0;
    let p_h = 1.0 / (1.0 + (-du).exp());
    let taken = p.external_id(it.parking[0]);
    let expected = if taken == "H-car" { p_h } else { 1.0 - p_h };
    assert!(
        (it.probability[0] - expected).abs() < 1e-9,
        "{taken}: {} vs {expected}",
        it.probability[0]
    );
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 1200)]);
}

#[test]
fn pr4_the_trip_back_rides_to_the_parking_fetches_the_car_and_drives_home() {
    let t = toy();
    let rows = vec![
        t.trip("a", 0, "W", "N1", 0, Mode::CarTransit),
        t.trip("a", 1, "N1", "D2", 1800, Mode::CarTransit),
    ];
    let r = t.run_with(rows, &only(&["P2"]), false);
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 1200), (EventType::TripCompleted, 3081)]);
    let it = r.itineraries.as_ref().expect("itineraries");
    assert_eq!(it.direction, [1, 2], "out, then back");
    assert_eq!(it.parking[0], it.parking[1]);
    assert_eq!(it.return_mismatch_s, 0.0, "the tram keeps to the schedule");
    // Parked from 264 to 3045: one space taken in between.
    let p = r.parking.as_ref().expect("parking");
    let b = &p.bins;
    assert_eq!(b.departures[3045 / 900], 1.0);
    assert_eq!(b.occupancy_max[1], 1.0);
    assert_eq!(b.occupancy_max[4], 0.0, "empty again after 3600");
}

#[test]
fn pr5_a_car_left_at_the_parking_is_not_at_its_owners_next_origin() {
    let t = toy();
    let rows = vec![
        t.trip("a", 0, "W", "N1", 0, Mode::CarTransit),
        t.trip("a", 1, "N1", "W", 1800, Mode::Car),
    ];
    let r = t.run(rows);
    assert_eq!(
        outcomes(&r),
        [(EventType::TripCompleted, 1200), (EventType::NoVehicleAvailable, 1800)]
    );
}

#[test]
fn br1_bike_and_ride_rides_parks_and_takes_the_tram() {
    let t = toy();
    let r = t.run(vec![t.trip("a", 0, "S", "N1", 0, Mode::BikeTransit)]);
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 1200)]);
    let it = r.itineraries.as_ref().expect("itineraries");
    let p = t.parking(&toy_network_parkings());
    assert_eq!(p.external_id(it.parking[0]), "H-bike");
    // The ride is on the bike layer.
    let bike = r.bike_link_bins.as_ref().expect("bike bins");
    assert!(!bike.is_empty());
}

#[test]
fn br2_the_fourth_bike_finds_the_rack_full() {
    let t = toy();
    let rows =
        (0..4).map(|i| t.trip(&format!("b{i}"), 0, "S", "N1", 0, Mode::BikeTransit)).collect();
    let r = t.run(rows);
    assert_eq!(outcomes(&r), vec![(EventType::TripCompleted, 1200); 4]);
    let p = r.parking.as_ref().expect("parking");
    assert_eq!((p.arrivals, p.overflow_arrivals), (4.0, 1.0));
    let paid = 30.0 + 180.0 * 652.0 / 900.0;
    assert!((p.mismatch_s - (paid - 30.0)).abs() < 1e-9, "{}", p.mismatch_s);
}

#[test]
fn without_parkings_a_park_and_ride_trip_is_not_available_and_plain_transit_is_unchanged() {
    let t = toy();
    let rows = vec![
        t.trip("a", 0, "W", "N1", 0, Mode::CarTransit),
        t.trip("b", 0, "N1", "D2", 0, Mode::Transit),
    ];
    let r = t.run_with(rows, &[], false);
    assert_eq!(
        outcomes(&r),
        [(EventType::ModeNotAvailable, 0), (EventType::TripCompleted, 900)],
        "TR1's value"
    );
    let it = r.itineraries.as_ref().expect("the transit trip");
    assert_eq!(it.parking, [NO_PARKING]);
}
