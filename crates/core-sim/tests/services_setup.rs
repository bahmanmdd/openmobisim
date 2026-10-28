//! Services set up on the toy network: docks on the walk and bike layers and in the run's hub
//! set; a taxi fleet and its chargers on the roads; the options and the rows refused.
//!
//! The toy (`openmobisim_core_graph::examples::toy_network`): the tram stop `H` at `D2`; `R2`
//! 300 m north of `D2` on the road `e2`; `N1` about 830 m from `R2`.

use std::sync::Arc;

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::hubs::ResourceKind;
use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::{toy_network, toy_network_layers};
use openmobisim_core_sim::services::hubs::{DEFAULT_HUB_JOIN_M, RunHubs};
use openmobisim_core_sim::services::{
    ChargerRow, DockDefaults, DockRow, Options, ServiceError, ServiceParts, ServiceSetup,
    TaxiDefaults, VehicleRow,
};
use openmobisim_core_sim::{LayerSetup, StaticLayers, TransitSetup};
use openmobisim_core_transit::TransitDefaults;
use openmobisim_core_transit::examples::toy_tram;
use openmobisim_core_types::ids::NodeId;

struct Toy {
    road: Arc<RoadNetwork>,
    layers: Arc<StaticLayers>,
    transit: Arc<TransitSetup>,
}

fn toy() -> Toy {
    let (road, _) = toy_network();
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
    Toy { road: Arc::new(road), layers, transit }
}

impl Toy {
    fn at(&self, name: &str) -> LonLat {
        let node: NodeId = self.road.node_external_ids().typed_id_of(name).expect("a toy node");
        self.road.node_lonlat(node)
    }

    fn docked(&self, rows: &[DockRow]) -> Result<ServiceSetup, ServiceError> {
        ServiceSetup::bike_docked(
            "bike_share",
            rows,
            self.layers.walk.as_ref().unwrap(),
            self.layers.bike.as_ref().unwrap(),
            None,
            DockDefaults::SHIPPED,
        )
    }
}

fn dock(id: &str, at: LonLat, capacity: u32, bikes: Option<u32>) -> DockRow {
    DockRow { dock_id: id.into(), name: None, hub_id: None, position: at, capacity, bikes }
}

#[test]
fn docks_snap_to_the_layers_join_the_stop_s_hub_and_know_their_neighbours() {
    let t = toy();
    let far = LonLat::new(t.at("D2").lon + 0.2, t.at("D2").lat);
    let rows = [
        dock("d-R2", t.at("R2"), 10, Some(4)),
        dock("d-H", t.at("D2"), 8, None),
        dock("d-zero", t.at("N1"), 0, None),
        dock("d-far", far, 10, None),
    ];
    let s = t.docked(&rows).expect("set up");
    let report = s.report();
    assert_eq!((report.rows, report.kept, report.empty, report.off_layer), (4, 2, 1, 1));
    let ServiceParts::BikeDocked(docks, _) = s.parts() else { panic!("a docked service") };
    assert_eq!(docks.external, ["d-H", "d-R2"], "in id order, whatever the table's");
    assert_eq!(docks.start, [4, 4], "half of 8 slots by the start share; the row's 4");
    let near = |d: usize| {
        &docks.neighbours
            [docks.neighbours_start[d] as usize..docks.neighbours_start[d + 1] as usize]
    };
    assert_eq!(near(0).iter().map(|n| n.0).collect::<Vec<_>>(), [1], "300 m apart: neighbours");
    assert!(near(0)[0].1 > 0.0, "a walk of some seconds");
    // Into the run's hub set: the dock at D2 joins the stop H's hub; the one at R2, 300 m from
    // every stop, is a hub of its own.
    let rows = s.resource_rows();
    let hubs = RunHubs::build(Some(&t.transit), &[], None, &rows, DEFAULT_HUB_JOIN_M);
    let mut s = s;
    s.link(&hubs, 0);
    let set = hubs.set();
    let at_h = s.resource(0).expect("linked");
    assert_eq!(hubs.hub_external_id(set.resource_hub(at_h)), "H");
    assert_eq!(set.resource_kind(at_h), ResourceKind::Dock);
    assert_eq!(set.resource_service(at_h), Some("bike_share"));
    assert_eq!((set.resource_capacity(at_h), set.resource_initial(at_h)), (8, 4));
    assert_eq!(hubs.hub_external_id(set.resource_hub(s.resource(1).unwrap())), "d-R2");
}

#[test]
fn a_repeated_dock_or_none_left_is_refused() {
    let t = toy();
    let twice = [dock("a", t.at("D2"), 5, None), dock("a", t.at("R2"), 5, None)];
    assert!(matches!(t.docked(&twice), Err(ServiceError::BadRow(_))));
    let none = [dock("z", t.at("D2"), 0, None)];
    assert!(matches!(t.docked(&none), Err(ServiceError::Empty(_))));
}

#[test]
fn a_taxi_fleet_and_its_chargers_snap_to_the_roads() {
    let t = toy();
    let vehicles = [
        VehicleRow { vehicle_id: "v1".into(), position: t.at("R1"), soc: None },
        VehicleRow { vehicle_id: "v2".into(), position: t.at("N1"), soc: Some(0.3) },
    ];
    let chargers = [
        ChargerRow {
            charger_id: "c1".into(),
            hub_id: Some("H".into()),
            position: t.at("D2"),
            plugs: 2,
            kw: None,
        },
        ChargerRow {
            charger_id: "c0".into(),
            hub_id: None,
            position: t.at("R2"),
            plugs: 0,
            kw: None,
        },
    ];
    let s = ServiceSetup::taxi("taxi", &t.road, &vehicles, &chargers, TaxiDefaults::SHIPPED)
        .expect("set up");
    let ServiceParts::Taxi(fleet, d) = s.parts() else { panic!("a taxi fleet") };
    assert_eq!(fleet.vehicle_soc, [0.8, 0.3], "the default start charge, and the row's");
    assert_eq!(
        (fleet.charger_ids.len(), s.report().empty),
        (1, 1),
        "a charger without plugs is left out"
    );
    assert!((fleet.charger_kw[0] - d.charger_kw).abs() < 1e-12);
    let hubs = RunHubs::build(Some(&t.transit), &[], None, &s.resource_rows(), DEFAULT_HUB_JOIN_M);
    let mut s = s;
    s.link(&hubs, 0);
    let c = s.resource(0).unwrap();
    assert_eq!(hubs.hub_external_id(hubs.set().resource_hub(c)), "H", "the hub its row names");
    assert_eq!(hubs.set().resource_kind(c), ResourceKind::Charger);
    let bad = [VehicleRow { vehicle_id: "v".into(), position: t.at("R1"), soc: Some(1.5) }];
    assert!(matches!(
        ServiceSetup::taxi("taxi", &t.road, &bad, &[], TaxiDefaults::SHIPPED),
        Err(ServiceError::BadRow(_))
    ));
}

#[test]
fn options_are_taken_by_name_and_refused_with_the_list() {
    let docks = DockDefaults::from_options(&Options::from([("redirect_max".into(), 1.0)])).unwrap();
    assert_eq!(docks.redirect_count(), 1);
    let err = DockDefaults::from_options(&Options::from([("redirect".into(), 1.0)])).unwrap_err();
    assert!(err.to_string().contains("redirect_max_m"), "{err}");
    assert!(DockDefaults::from_options(&Options::from([("start_share".into(), 1.5)])).is_err());
    let taxis =
        TaxiDefaults::from_options(&Options::from([("charger_reach_s".into(), 1800.0)])).unwrap();
    assert!((taxis.charger_reach_s - 1800.0).abs() < 1e-12);
    let err =
        TaxiDefaults::from_options(&Options::from([("charge_below".into(), 0.9)])).unwrap_err();
    assert!(err.to_string().contains("charge_below < charge_to"), "{err}");
}
