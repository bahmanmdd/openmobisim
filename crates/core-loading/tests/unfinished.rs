//! Where the vehicles still travelling are when the window ends (`LtmOutput::unfinished`,
//! the per-trip table's `link_at_end`).
//!
//! The network: `A → B → C`, two residential links of 300 m: 36 s each at 30 km/h. The window
//! ends at 50 s.
//!
//! | Vehicle | Route, departure | Hand value at 50 s |
//! |---|---|---|
//! | 0 | `AB → BC` at 0 s | left `AB` at 36 s, on `BC` until 72 s: **on `BC`** |
//! | 1 | `AB` at 10 s | arrived at 46 s: **not listed** |
//! | 2 | `AB → BC` at 45 s | **on `AB`** until 81 s |
//! | 3 | `AB` at 60 s | departs after the window: **not listed** |

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, Recording, Rules, Vehicle, run_ltm_chained};
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

#[test]
fn the_vehicles_still_travelling_are_listed_on_their_links() {
    let (net, ab, bc) = line();
    let vehicles = [
        Vehicle::new(VehicleId::new(0), vec![ab, bc], Pcu(1.0), Second(0)),
        Vehicle::new(VehicleId::new(1), vec![ab], Pcu(1.0), Second(10)),
        Vehicle::new(VehicleId::new(2), vec![ab, bc], Pcu(1.0), Second(45)),
        Vehicle::new(VehicleId::new(3), vec![ab], Pcu(1.0), Second(60)),
    ];
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let out = run_ltm_chained(
        &net,
        &turns,
        &vehicles,
        &[],
        Duration(50.0),
        Duration(300.0),
        FidelityLevel::Full,
        Recording::Trajectories,
        Rules::default(),
        None,
    );
    let arrived: Vec<VehicleId> = out.trajectories.iter().map(|t| t.vehicle).collect();
    assert_eq!(arrived, vec![VehicleId::new(1)]);
    let mut unfinished = out.unfinished.clone();
    unfinished.sort_unstable_by_key(|&(v, _)| v.raw());
    assert_eq!(unfinished, vec![(VehicleId::new(0), bc), (VehicleId::new(2), ab)]);
}
