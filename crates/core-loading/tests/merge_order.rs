//! Who goes first when two vehicles reach a merge at the same instant: the order must depend
//! neither on the hour of the day nor on the links' ids.
//!
//! The case is a diamond: an origin `O`, two sides `O → P → M` and `O → Q → M` whose two links
//! have the same lengths in the opposite order, so a car on either side reaches `M` at the same
//! instant, and a bottleneck `M → D` of 600 veh/h (a discharge headway of 6 s). Two cars leave
//! `O` at the same second, one on each side. Both reach the downstream end of the bottleneck at
//! the same instant; the first leaves at once, the other one headway later. The values are
//! derived from the case's constants: a side takes `L₁ / v + L₂ / v`, the bottleneck `L_j / v`,
//! the second car 3 600 / 600 = 6 s more, and a trip's arrival is floored to the whole second.
//! The engine's rule at a tie (the vehicle that waited longest at the merge first, then the
//! lower vehicle id) puts car 0 first.

#![allow(clippy::float_cmp, reason = "exact comparisons of whole seconds")]

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    FidelityLevel, Recording, Rules, Vehicle, run_ltm, run_ltm_chained,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

/// The free-flow speed of every link, km/h.
const KM_H: f64 = 50.0;
/// The bottleneck's capacity, veh/h; the sides carry 1 800 veh/h.
const BOTTLENECK_VEH_H: f64 = 600.0;
/// The bottleneck's length, m.
const BOTTLENECK_M: f64 = 100.0;

/// The diamond, its five links named `names` in the order `O → P`, `P → M`, `O → Q`, `Q → M`,
/// `M → D`; the first side `L₁` then `L₂` metres long, the second `L₂` then `L₁`.
fn diamond(names: [&str; 5], l1: f64, l2: f64) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (n, x, y) in [
        ("O", 0.0, 0.0),
        ("P", 100.0, 100.0),
        ("Q", 100.0, -100.0),
        ("M", 200.0, 0.0),
        ("D", 300.0, 0.0),
    ] {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    let spec = |metres: f64, veh_h: f64| {
        let mut s = LinkSpec::new(RoadClass::Secondary);
        s.length_m = Some(metres);
        s.maxspeed_km_h = Some(KM_H);
        s.capacity_veh_h = Some(veh_h);
        s.lanes = Some(1);
        s
    };
    b.add_link(names[0], "O", "P", spec(l1, 1800.0));
    b.add_link(names[1], "P", "M", spec(l2, 1800.0));
    b.add_link(names[2], "O", "Q", spec(l2, 1800.0));
    b.add_link(names[3], "Q", "M", spec(l1, 1800.0));
    b.add_link(names[4], "M", "D", spec(BOTTLENECK_M, BOTTLENECK_VEH_H));
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn link(network: &RoadNetwork, external: &str) -> LinkId {
    network.link_external_ids().typed_id_of::<LinkId>(external).expect("known link")
}

/// The two cars, both leaving at `departure`: car 0 on the first side, car 1 on the second.
fn cars(network: &RoadNetwork, names: [&str; 5], departure: u32) -> Vec<Vehicle> {
    let route =
        |a: &str, b: &str| vec![link(network, a), link(network, b), link(network, names[4])];
    vec![
        Vehicle::new(VehicleId::new(0), route(names[0], names[1]), Pcu(1.0), Second(departure)),
        Vehicle::new(VehicleId::new(1), route(names[2], names[3]), Pcu(1.0), Second(departure)),
    ]
}

/// Each car's trip time, whole seconds, loaded at `level` with the loading's clock starting at
/// `start` (seconds after midnight).
fn trip_times(
    network: &RoadNetwork,
    names: [&str; 5],
    departure: u32,
    start: u32,
    level: FidelityLevel,
) -> [u32; 2] {
    let turns = TurnTable::build(network, network.signals());
    let vehicles = cars(network, names, departure);
    let window = Duration(f64::from(departure) + 600.0);
    let trajectories = if start == 0 {
        run_ltm(network, &turns, &vehicles, window, Duration(300.0), level)
    } else {
        let rules = Rules { start_s: f64::from(start), ..Rules::default() };
        run_ltm_chained(
            network,
            &turns,
            &vehicles,
            &[],
            window,
            Duration(300.0),
            level,
            Recording::Trajectories,
            rules,
            None,
        )
        .trajectories
    };
    let time = |id: usize| {
        let t = trajectories.iter().find(|t| t.vehicle.index() == id).expect("arrived");
        t.arrival().get() - departure
    };
    [time(0), time(1)]
}

/// The derived trip times: the first car `⌊L₁/v + L₂/v + L_j/v⌋`, the second one bottleneck
/// headway later. Checks the built free-flow times against the lengths first.
fn derived(network: &RoadNetwork, names: [&str; 5], l1: f64, l2: f64) -> [u32; 2] {
    let v = KM_H / 3.6;
    for (name, metres) in [(names[0], l1), (names[1], l2), (names[4], BOTTLENECK_M)] {
        let built = network.free_flow_time(link(network, name)).get();
        assert!((built - metres / v).abs() < 1e-9, "{name}: {built} s against {} s", metres / v);
    }
    let first = l1 / v + l2 / v + BOTTLENECK_M / v;
    let headway = 3600.0 / BOTTLENECK_VEH_H;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "whole seconds")]
    let floor = |x: f64| x.floor() as u32;
    [floor(first), floor(first + headway)]
}

const LEVELS: [FidelityLevel; 3] =
    [FidelityLevel::PointQueue, FidelityLevel::SpatialQueue, FidelityLevel::Full];

/// The hour of the day: sides of 359.37 m and 621.53 m, so that `(d + T₁) + T₂` and
/// `(d + T₂) + T₁` differ by one rounding step of a floating-point clock below 32 768 s and are
/// equal above it. The cars leave at 09:00 and, in a second run, at 10:00 (the clock counted
/// from midnight), and at 09:00 and 10:00 with the clock started at 08:00 and 09:00: the same
/// trip times every time, car 0 first.
#[test]
fn a_merge_serves_the_same_car_first_at_any_hour() {
    let names = ["a", "b", "c", "e", "j"];
    let (l1, l2) = (359.37, 621.53);
    let network = diamond(names, l1, l2);
    let want = derived(&network, names, l1, l2);
    for level in LEVELS {
        for (departure, start) in [(32_400, 0), (36_000, 0), (32_400, 28_800), (36_000, 32_400)] {
            assert_eq!(
                trip_times(&network, names, departure, start, level),
                want,
                "{level:?}, cars leaving at {departure} s, the clock from {start} s"
            );
        }
    }
}

/// The links' ids: equal sides of 100 m, so both cars reach the merge at the same instant,
/// with the links of the two sides named so that their ids, and so their order, are swapped.
/// Car 0 goes first either way.
#[test]
fn a_merge_serves_the_same_car_first_whatever_the_link_ids() {
    let (l1, l2) = (100.0, 100.0);
    for names in [["a", "b", "c", "e", "j"], ["c", "e", "a", "b", "j"]] {
        let network = diamond(names, l1, l2);
        let want = derived(&network, names, l1, l2);
        for level in LEVELS {
            assert_eq!(
                trip_times(&network, names, 32_400, 0, level),
                want,
                "{level:?}, the sides' links named {names:?}"
            );
        }
    }
}
