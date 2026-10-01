//! Mode choice on the toy network (M5): every number derived by hand.
//!
//! **The pieces** are the toy's (`toy_parking.rs`, `toy_layers.rs`, `toy_transit.rs`). A trip
//! without a stated mode chooses among the modes the run offers, each where the traveller can
//! use it. The one choice set of `W → N1` at 0, with park-and-ride and bike-and-ride offered at
//! any length (`pr_min_km` 0), has five alternatives:
//!
//! | Alternative | Door to door | Walk, wait | Path size |
//! |---|---|---|---|
//! | bike `a1`, contraflow `s1:c`, 600 m mixed | 600 / (15/3.6) = **144 s** | 0, 0 | 1 |
//! | walk `w1`, 300√2 m | 424.26 / (4/3) = **318.198 s** | 318.198 s, 0 | 1 |
//! | park-and-ride at `H` (PR3) | **1200 s** | 0, 600 s | 1000/1200: the tram ride is shared by three |
//! | park-and-ride at `P2` (PR3) | **1200 s** | 225 s, 411 s | 1000/1200 |
//! | bike-and-ride at `H`: the bike `W → S → H` 72 + 248.48 = 320.48 s, parked 350, the 900 tram | **1200 s** | 0, 550 s | 1000/1200 |
//!
//! There is no car alternative (`s1` runs one way, into `S`: a car cannot reach `N1`), and no
//! transit one: `N1` is a stop, and the only journey RAPTOR has rides out to `H` and back,
//! which is dropped. With the default coefficients (time −0.2, walking −0.13 and waiting −0.09
//! on top, per minute; ln path size 1) the utilities are `V = −0.2 t − 0.13 w − 0.09 q + ln PS`.
//!
//! | Case | What | Hand value |
//! |---|---|---|
//! | MC1 | `W → N1`, all modes | deterministic: the bike, **144**; logit and nested logit (μ 0.5): the closed forms of the table; a mode constant of 4 for park-and-ride moves the choice into its nest |
//! | MC2 | a bike trip `W → M`, then `M → X0` choosing | the car stayed at `W`: no car alternative, the bike (**104 s**, the track) is taken; after a car trip `W → M`, the car (`a3`, **48 s**), and no bike |
//! | MC3 | 200 travellers `W → M` at 0, car or bike, `msa` | the queue at `S` makes the car slower than the bike's 120 s: the bike's share grows over the iterations |
//! | MC4 | `modes = [car]` | the run without mode choice, bit for bit (and `[bike]` makes every such trip a bike trip) |
//! | MC5 | park-and-ride out `W → N1`, then `N1 → D2` choosing | the car is parked: the trip back is offered only the ways back to it (`D2` is where the car goes, from either car park, as PR4) |
//! | MC6 | no timetable | walk, bike and car are still chosen among: `W → M` by car, **80 s** |

#![allow(clippy::float_cmp, reason = "hand-derived values")]

use std::sync::Arc;

use openmobisim_core_choice::{Logit, NestedLogit, Options};
use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::toy_network_parkings;
use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_graph::{LonLat, toy_network, toy_network_layers};
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::equilibration::strategy;
use openmobisim_core_sim::{
    EventType, FlowMotor, LayerSetup, NO_MODE, ParkingDefaults, ParkingSetup, Run, RunResult,
    StaticLayers, TransitSetup,
};
use openmobisim_core_transit::TransitDefaults;
use openmobisim_core_transit::examples::toy_tram;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

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

/// How a test's run is set up.
struct Setup {
    transit: bool,
    parking_at_any_length: bool,
    model: Option<Arc<dyn openmobisim_core_choice::ChoiceModel>>,
    modes: Vec<Mode>,
    msa: Option<u32>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            transit: true,
            parking_at_any_length: true,
            model: None,
            modes: Mode::ALL.to_vec(),
            msa: None,
        }
    }
}

impl Toy {
    fn at(&self, name: &str) -> LonLat {
        let node = self.road.node_external_ids().typed_id_of(name).expect("a toy node");
        self.road.node_lonlat(node)
    }

    /// A trip; `mode` `None` chooses.
    fn trip(&self, who: &str, seq: u32, od: (&str, &str), t: u32, mode: Option<Mode>) -> RawTrip {
        RawTrip {
            traveller_id: who.to_string(),
            trip_seq: seq,
            origin: self.at(od.0),
            destination: self.at(od.1),
            departure_time: Second(t),
            user_class: "everyone".to_string(),
            weight: None,
            mode,
        }
    }

    fn parking(&self, any_length: bool) -> Arc<ParkingSetup> {
        let mut d = ParkingDefaults::SHIPPED;
        if any_length {
            d.pr_min_km = 0.0;
        }
        Arc::new(
            ParkingSetup::new(
                &toy_network_parkings(),
                self.road.clone(),
                self.layers.bike.as_ref(),
                &self.transit,
                d,
            )
            .expect("the toy's parkings"),
        )
    }

    fn build(&self, rows: Vec<RawTrip>, setup: &Setup) -> Run {
        let defaults = ClassDefaults::new()
            .with_default("everyone", Ownership { car: true, bike: true, transit_pass: false });
        let (travellers, trips) =
            build_travellers(rows, Vec::new(), &defaults, 1, &mut Diagnostics::new()).unwrap();
        let mut run =
            Run::new(self.road.clone(), Arc::new(travellers), Arc::new(trips), Second(20_000))
                .with_layers(self.layers.clone())
                .with_mode_choice(&setup.modes);
        if setup.transit {
            run = run
                .with_transit(self.transit.clone())
                .with_parking(self.parking(setup.parking_at_any_length));
        }
        if let Some(model) = &setup.model {
            run = run.with_choice_model(model.clone());
        }
        if let Some(iterations) = setup.msa {
            let options: Options = [("iterations".to_string(), f64::from(iterations))].into();
            run = run
                .with_equilibration(Arc::from(strategy("msa", &options).expect("built in")))
                .with_flow_motor(FlowMotor::Ltm {
                    turns: Arc::new(TurnTable::build(&self.road, SignalDefaults::SHIPPED)),
                    step: Duration(300.0),
                    level: FidelityLevel::Full,
                });
        }
        run
    }

    fn run(&self, rows: Vec<RawTrip>, setup: &Setup) -> RunResult {
        self.build(rows, setup).execute(&mut Diagnostics::new())
    }

    fn external_parking(&self, index: u32) -> String {
        self.parking(true).external_id(index).to_string()
    }
}

fn options(pairs: &[(&str, f64)]) -> Options {
    pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
}

/// Each trip's outcome and second, in trip order.
fn outcomes(result: &RunResult) -> Vec<(EventType, u32)> {
    let mut e: Vec<_> =
        result.events.iter().map(|e| (e.entity_id, e.event_type, e.second.get())).collect();
    e.sort_by_key(|x| x.0);
    e.into_iter().map(|(_, t, s)| (t, s)).collect()
}

/// `W → N1`'s alternatives as `(mode, parking, utility, nest)` under the default coefficients
/// plus `constant` on park-and-ride.
fn w_n1(constant: f64) -> Vec<(Mode, &'static str, f64)> {
    let walk_s = 300.0 * 2f64.sqrt() / (4.8 / 3.6);
    let ln_ps = (1000.0_f64 / 1200.0).ln();
    let v =
        |t: f64, w: f64, q: f64, ps: f64| -0.2 * t / 60.0 - 0.13 * w / 60.0 - 0.09 * q / 60.0 + ps;
    vec![
        (Mode::Bike, "", v(144.0, 0.0, 0.0, 0.0)),
        (Mode::Walk, "", v(walk_s, walk_s, 0.0, 0.0)),
        (Mode::CarTransit, "H-car", v(1200.0, 0.0, 600.0, ln_ps) + constant),
        (Mode::CarTransit, "P2", v(1200.0, 225.0, 411.0, ln_ps) + constant),
        (Mode::BikeTransit, "H-bike", v(1200.0, 0.0, 550.0, ln_ps)),
    ]
}

/// The probability of alternative `i` of `alts` under a nested logit with one nest per mode
/// and scale `mu` (1: the flat logit).
fn nested_probability(alts: &[(Mode, &str, f64)], i: usize, mu: f64) -> f64 {
    let inclusive =
        |m: Mode| mu * alts.iter().filter(|a| a.0 == m).map(|a| (a.2 / mu).exp()).sum::<f64>().ln();
    let mut modes: Vec<Mode> = alts.iter().map(|a| a.0).collect();
    modes.dedup();
    let denominator: f64 = modes.iter().map(|&m| inclusive(m).exp()).sum();
    let (mode, _, v) = alts[i];
    let within: f64 = alts.iter().filter(|a| a.0 == mode).map(|a| (a.2 / mu).exp()).sum();
    inclusive(mode).exp() / denominator * (v / mu).exp() / within
}

/// The chosen alternative of trip `position` of `result`, as `(mode, parking id)`.
fn chosen(t: &Toy, result: &RunResult, position: usize) -> (Mode, String) {
    let it = result.itineraries.as_ref().expect("itineraries");
    assert_ne!(it.mode[position], NO_MODE, "a choice");
    let mode = Mode::ALL[it.mode[position] as usize];
    let parking = it.parking[position];
    let id = if parking == u32::MAX { String::new() } else { t.external_parking(parking) };
    (mode, id)
}

#[test]
fn mc1_the_deterministic_model_takes_the_bike() {
    let t = toy();
    let r = t.run(vec![t.trip("a", 0, ("W", "N1"), 0, None)], &Setup::default());
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 144)]);
    let it = r.itineraries.as_ref().unwrap();
    assert_eq!(it.alternatives, [5], "bike, walk, two park-and-ride, one bike-and-ride");
    assert_eq!(it.choosing, [true]);
    assert_eq!(chosen(&t, &r, 0), (Mode::Bike, String::new()));
    assert_eq!(it.expected_s, [144.0]);
    // The trip counts as a bike trip.
    assert_eq!(r.by_mode[Mode::Bike.index()].completion.completed, 1);
    assert_eq!(r.by_mode[Mode::Car.index()].completion.total_trips, 0);
    // Its bike rode the bike layer.
    assert!(r.bike_link_bins.as_ref().is_none_or(|b| !b.is_empty()));
}

#[test]
fn mc1_the_logit_and_the_nested_logit_match_their_closed_forms() {
    let t = toy();
    for (mu, constant) in [(1.0, 0.0), (0.5, 0.0), (0.5, 4.0), (0.3, 4.0)] {
        let mut o = vec![("beta_mode_car_transit", constant)];
        let model: Arc<dyn openmobisim_core_choice::ChoiceModel> = if mu == 1.0 {
            Arc::new(Logit::from_options(&options(&o)).unwrap())
        } else {
            o.push(("mu", mu));
            Arc::new(NestedLogit::from_options(&options(&o)).unwrap())
        };
        let setup = Setup { model: Some(model), ..Setup::default() };
        let alts = w_n1(constant);
        // Several travellers, so several draws: each one's probability is its alternative's.
        let rows = (0..8).map(|i| t.trip(&format!("a{i}"), 0, ("W", "N1"), 0, None)).collect();
        let r = t.run(rows, &setup);
        let it = r.itineraries.as_ref().unwrap();
        for p in 0..8 {
            let (mode, parking) = chosen(&t, &r, p);
            let i = alts.iter().position(|a| a.0 == mode && a.1 == parking).expect("in the set");
            let expected = nested_probability(&alts, i, mu);
            assert!(
                (it.probability[p] - expected).abs() < 1e-9,
                "mu {mu}, constant {constant}: {mode:?} {parking}: {} vs {expected}",
                it.probability[p]
            );
        }
    }
    // The constant moves the choice into park-and-ride's nest: from 1% to 35% at mu 0.5.
    let nest = |c: f64| -> f64 { (2..4).map(|i| nested_probability(&w_n1(c), i, 0.5)).sum() };
    assert!((nest(0.0) - 0.0099).abs() < 1e-3 && (nest(4.0) - 0.3536).abs() < 1e-3);
    let alts = w_n1(4.0);
    // At mu 1 the nest is two logit alternatives; at 0.5 the stations compete more within it.
    let (h1, h5) = (nested_probability(&alts, 2, 1.0), nested_probability(&alts, 2, 0.5));
    let (p1, p5) = (nested_probability(&alts, 3, 1.0), nested_probability(&alts, 3, 0.5));
    assert!(h5 / p5 > h1 / p1, "sharper within the nest");
}

#[test]
fn mc2_a_vehicle_left_elsewhere_is_not_offered() {
    let t = toy();
    let setup = Setup { parking_at_any_length: false, ..Setup::default() };
    // The bike to M: the car stays at W; the next trip rides the bike on (the track, 104 s).
    let r = t.run(
        vec![
            t.trip("a", 0, ("W", "M"), 0, Some(Mode::Bike)),
            t.trip("a", 1, ("M", "X0"), 1000, None),
        ],
        &setup,
    );
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 120), (EventType::TripCompleted, 1104)]);
    let it = r.itineraries.as_ref().unwrap();
    assert_eq!(it.trip, [1], "only the choosing trip is an itinerary trip");
    assert_eq!(it.alternatives, [3], "bike, walk, transit: no car");
    assert_eq!(chosen(&t, &r, 0).0, Mode::Bike);
    // By car to M: the car is there, and a3 takes 48 s.
    let r = t.run(
        vec![
            t.trip("a", 0, ("W", "M"), 0, Some(Mode::Car)),
            t.trip("a", 1, ("M", "X0"), 1000, None),
        ],
        &setup,
    );
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 80), (EventType::TripCompleted, 1048)]);
    let it = r.itineraries.as_ref().unwrap();
    assert_eq!(it.alternatives, [3], "car, walk, transit: the bike is still at W");
    assert_eq!(chosen(&t, &r, 0).0, Mode::Car);
    // Both choosing: the first takes the car (80.554 s against the bike's 120 s), so the
    // second has it.
    let r = t.run(
        vec![t.trip("a", 0, ("W", "M"), 0, None), t.trip("a", 1, ("M", "X0"), 1000, None)],
        &setup,
    );
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 80), (EventType::TripCompleted, 1048)]);
    assert_eq!((chosen(&t, &r, 0).0, chosen(&t, &r, 1).0), (Mode::Car, Mode::Car));
}

#[test]
fn mc3_congestion_moves_travellers_to_the_bike() {
    let t = toy();
    let rows = || (0..200).map(|i| t.trip(&format!("a{i}"), 0, ("W", "M"), 0, None)).collect();
    let model = || -> Arc<dyn openmobisim_core_choice::ChoiceModel> {
        Arc::new(NestedLogit::from_options(&Options::new()).unwrap())
    };
    let setup = |msa| Setup {
        transit: false,
        model: Some(model()),
        modes: vec![Mode::Car, Mode::Bike],
        msa,
        ..Setup::default()
    };
    let bike_share =
        |r: &RunResult| f64::from(r.by_mode[Mode::Bike.index()].completion.total_trips) / 200.0;
    // One loading: the free-flow choice, P(bike) = 1 / (1 + e^(-0.2 (80.554 - 120) / 60)).
    let once = t.run(rows(), &setup(Some(1)));
    let p_bike = 1.0 / (1.0 + (-0.2 * (80.554 - 120.0) / 60.0_f64).exp());
    assert!((bike_share(&once) - p_bike).abs() < 0.1, "{} vs {p_bike}", bike_share(&once));
    // Five: the queue at the signal at S (a car every 5.357 s) makes driving slower than
    // riding, and msa moves travellers to the bike.
    let five = t.run(rows(), &setup(Some(5)));
    assert!(
        bike_share(&five) > bike_share(&once) + 0.15,
        "{} then {}",
        bike_share(&once),
        bike_share(&five)
    );
    let changed: Vec<f64> = five.iterations.iter().map(|r| r.mode_changed_share).collect();
    assert!(changed[0].is_nan(), "nothing chose before the first loading");
    assert!(changed[1] > 0.0 && changed[1] <= 0.5, "{changed:?}");
    assert_eq!(
        bike_share(&five) * 200.0,
        f64::from(five.by_mode[Mode::Bike.index()].completion.completed)
    );
}

#[test]
fn mc4_one_mode_is_no_choice_and_car_alone_is_the_run_without_mode_choice() {
    let t = toy();
    let rows = || {
        vec![
            t.trip("a", 0, ("W", "M"), 0, None),
            t.trip("a", 1, ("M", "X0"), 600, None),
            t.trip("b", 0, ("N2", "X0"), 30, None),
            t.trip("c", 0, ("W", "N1"), 60, Some(Mode::Bike)),
        ]
    };
    let logit = || -> Arc<dyn openmobisim_core_choice::ChoiceModel> {
        Arc::new(Logit::from_options(&Options::new()).unwrap())
    };
    let base = Setup { model: Some(logit()), modes: Vec::new(), msa: Some(3), ..Setup::default() };
    let car =
        Setup { modes: vec![Mode::Car], model: Some(logit()), msa: Some(3), ..Setup::default() };
    let mut a = t.build(rows(), &base);
    let mut b = t.build(rows(), &car);
    assert_eq!(a.description().fingerprint, b.description().fingerprint);
    let (a, b) = (a.execute(&mut Diagnostics::new()), b.execute(&mut Diagnostics::new()));
    assert_eq!(a.events, b.events);
    assert_eq!(a.iterations, b.iterations);
    assert_eq!(a.route_choices, b.route_choices);
    assert!(b.itineraries.is_none(), "no trip chooses");
    // One mode other than the car: the trips without one take it.
    let bike = t.run(rows(), &Setup { modes: vec![Mode::Bike], ..Setup::default() });
    assert_eq!(bike.by_mode[Mode::Bike.index()].completion.total_trips, 4);
    assert_eq!(bike.by_mode[Mode::Car.index()].completion.total_trips, 0);
}

#[test]
fn mc5_a_parked_car_is_fetched() {
    let t = toy();
    // A constant that makes park-and-ride all but certain on the way out.
    let model: Arc<dyn openmobisim_core_choice::ChoiceModel> =
        Arc::new(Logit::from_options(&options(&[("beta_mode_car_transit", 50.0)])).unwrap());
    let setup = Setup { model: Some(model), ..Setup::default() };
    let r = t.run(
        vec![t.trip("a", 0, ("W", "N1"), 0, None), t.trip("a", 1, ("N1", "D2"), 1800, None)],
        &setup,
    );
    let it = r.itineraries.as_ref().unwrap();
    assert_eq!(chosen(&t, &r, 0).0, Mode::CarTransit);
    assert_eq!(it.direction, [1, 2], "out, then back to the car");
    // Back: only the ways back to the car were offered (one journey from N1 to its stop).
    assert_eq!(chosen(&t, &r, 1).0, Mode::CarTransit);
    assert_eq!(it.parking[0], it.parking[1]);
    assert!(outcomes(&r).iter().all(|o| o.0 == EventType::TripCompleted), "{:?}", outcomes(&r));
}

#[test]
fn mc6_without_a_timetable_walk_bike_and_car_are_chosen_among() {
    let t = toy();
    let r = t.run(
        vec![t.trip("a", 0, ("W", "M"), 0, None)],
        &Setup { transit: false, ..Setup::default() },
    );
    assert_eq!(outcomes(&r), [(EventType::TripCompleted, 80)]);
    let it = r.itineraries.as_ref().unwrap();
    assert_eq!(it.alternatives, [3], "car, bike, walk");
    assert_eq!(chosen(&t, &r, 0).0, Mode::Car);
    assert_eq!(r.by_mode[Mode::Car.index()].completion.completed, 1);
}
