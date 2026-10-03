//! Chained vehicles in the loading (S199): a bus's next leg departs when the
//! leg before it arrives, after its dwell and not before its scheduled time,
//! and a dwelling bus holds no room.
//!
//! The network: `A → B → C`, two residential links of 300 m: 36 s at 30 km/h,
//! capacity 1400/h (a front passes a link end 1/0.3889 = 2.571 s per PCU after
//! the one before). The bus is 2 PCU; its first leg rides `AB` from 0 s and
//! arrives at `B` at **36 s**.
//!
//! | Case | Hand value |
//! |---|---|
//! | dwell 20 s, no schedule to keep | leaves `B` at 36 + 20 = **56**, at `C` at **92** |
//! | dwell 20 s, not before 100 | leaves at **100**, at `C` at **136** |
//! | a car on `AB → BC` from 1 s | leaves `AB` at max(1 + 36, 36 + 2.571) = 38.571 and is at `C` at 74.571 = **74**: past the dwelling bus, which enters `BC` at 56 |
//! | the window ends at 30 s | the first leg never arrives, so the second never departs |

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    Chain, FidelityLevel, Recording, Rules, Trajectory, Vehicle, run_ltm_chained,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

fn line() -> (RoadNetwork, LinkId, LinkId) {
    let mut b = RoadNetworkBuilder::new();
    for (i, n) in ["A", "B", "C"].iter().enumerate() {
        #[allow(clippy::cast_precision_loss, reason = "three nodes")]
        b.add_node(*n, LonLat::new(4.9 + 0.005 * i as f64, 52.37));
    }
    for (id, from, to) in [("AB", "A", "B"), ("BC", "B", "C")] {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(300.0);
        b.add_link(id, from, to, spec);
    }
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .unwrap();
    let link = |id: &str| net.link_external_ids().typed_id_of::<LinkId>(id).unwrap();
    let (ab, bc) = (link("AB"), link("BC"));
    (net, ab, bc)
}

fn load(net: &RoadNetwork, vehicles: &[Vehicle], chains: &[Chain], window: f64) -> Vec<Trajectory> {
    let turns = TurnTable::build(net, SignalDefaults::SHIPPED);
    run_ltm_chained(
        net,
        &turns,
        vehicles,
        chains,
        Duration(window),
        Duration(300.0),
        FidelityLevel::Full,
        Recording::Trajectories,
        Rules::default(),
        None,
    )
    .trajectories
}

fn by_id(trajectories: &[Trajectory], id: u32) -> Option<&Trajectory> {
    trajectories.iter().find(|t| t.vehicle == VehicleId::new(id))
}

fn bus(net: &RoadNetwork, not_before: f64, window: f64) -> Vec<Trajectory> {
    let (_, ab, bc) = (net, line().1, line().2);
    let legs = [
        Vehicle::new(VehicleId::new(0), vec![ab], Pcu(2.0), Second(0)),
        Vehicle::new(VehicleId::new(1), vec![bc], Pcu(2.0), Second(0)),
    ];
    load(net, &legs, &[Chain { vehicle: 1, after: 0, wait: 20.0, not_before }], window)
}

#[test]
fn the_next_leg_leaves_after_the_dwell() {
    let (net, _, _) = line();
    let t = bus(&net, 0.0, 3600.0);
    assert_eq!(by_id(&t, 0).unwrap().arrival(), Second(36));
    let second = by_id(&t, 1).unwrap();
    assert_eq!(second.departure, Second(56));
    assert_eq!(second.links[0].enter, Second(56));
    assert_eq!(second.arrival(), Second(92));
}

#[test]
fn an_early_bus_waits_for_its_scheduled_time() {
    let (net, _, _) = line();
    let t = bus(&net, 100.0, 3600.0);
    let second = by_id(&t, 1).unwrap();
    assert_eq!((second.departure, second.arrival()), (Second(100), Second(136)));
}

#[test]
fn a_dwelling_bus_does_not_hold_up_the_car_behind() {
    let (net, ab, bc) = line();
    let vehicles = [
        Vehicle::new(VehicleId::new(0), vec![ab], Pcu(2.0), Second(0)),
        Vehicle::new(VehicleId::new(1), vec![bc], Pcu(2.0), Second(0)),
        Vehicle::new(VehicleId::new(7), vec![ab, bc], Pcu(1.0), Second(1)),
    ];
    let t = load(
        &net,
        &vehicles,
        &[Chain { vehicle: 1, after: 0, wait: 20.0, not_before: 0.0 }],
        3600.0,
    );
    let car = by_id(&t, 7).unwrap();
    assert_eq!(car.links[0].exit, Second(38), "38.571, floored");
    assert_eq!(car.arrival(), Second(74), "74.571, floored");
    let bus = by_id(&t, 1).unwrap();
    assert!(
        car.links[1].enter < bus.links[0].enter,
        "the car is on BC before the bus leaves its stop"
    );
}

#[test]
fn a_leg_whose_leader_never_arrives_never_departs() {
    let (net, _, _) = line();
    let t = bus(&net, 0.0, 30.0);
    assert!(t.is_empty(), "{t:?}");
}
