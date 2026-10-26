//! Bus lanes (S299): the tags read per direction, the lanes cars keep, a contraflow bus lane
//! as a busway against a one-way street, a lane also for carpools kept for cars, and the import
//! report's counts.

use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_io_osm::import::{ImportOptions, ImportReport, import};
use openmobisim_io_osm::source::{MemorySource, OsmNode, OsmWay};
use openmobisim_io_osm::tags::{self, BusLanes};

fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn bus(forward: u8, backward: u8) -> BusLanes {
    BusLanes { forward, backward }
}

/// The bus lanes of a way with `pairs`, read with its own direction.
fn read(pairs: &[(&str, &str)]) -> BusLanes {
    let t = tags(pairs);
    let class = tags::classify(&t, 2).expect("a road");
    tags::bus_lanes(&t, tags::direction(&t, class))
}

#[test]
fn busway_tags_give_each_direction_its_lane() {
    // A plain tag on a two-way road: a lane each way.
    assert_eq!(read(&[("highway", "primary"), ("busway", "lane")]), bus(1, 1));
    // Sides on a two-way road: right serves the way's direction, left the other (right-hand
    // traffic).
    assert_eq!(read(&[("highway", "primary"), ("busway:right", "lane")]), bus(1, 0));
    assert_eq!(read(&[("highway", "primary"), ("busway:left", "lane")]), bus(0, 1));
    assert_eq!(read(&[("highway", "primary"), ("busway:both", "lane")]), bus(1, 1));
    // A side's tag wins over the plain one.
    assert_eq!(
        read(&[("highway", "primary"), ("busway", "lane"), ("busway:left", "no")]),
        bus(1, 0)
    );
    // `no` and unknown values are no lane.
    assert_eq!(read(&[("highway", "primary"), ("busway", "no")]), bus(0, 0));
    assert_eq!(read(&[("highway", "primary"), ("busway", "track")]), bus(0, 0));
}

#[test]
fn on_a_one_way_street_opposite_lane_runs_against_it() {
    let one_way = [("highway", "primary"), ("oneway", "yes")];
    let with = |more: &[(&'static str, &'static str)]| {
        let mut all = one_way.to_vec();
        all.extend_from_slice(more);
        read(&all)
    };
    // A lane with the traffic, on either side.
    assert_eq!(with(&[("busway", "lane")]), bus(1, 0));
    assert_eq!(with(&[("busway:left", "lane")]), bus(1, 0));
    // A contraflow bus lane.
    assert_eq!(with(&[("busway", "opposite_lane")]), bus(0, 1));
    assert_eq!(with(&[("busway:left", "opposite_lane")]), bus(0, 1));
    // `busway:<side>:oneway=-1` runs the side's lane against the way's node order.
    assert_eq!(with(&[("busway:left", "lane"), ("busway:left:oneway", "-1")]), bus(0, 1));
    // Both: one with the traffic, one against.
    assert_eq!(with(&[("busway:right", "lane"), ("busway:left", "opposite_lane")]), bus(1, 1));
    // On `oneway=-1` the traffic runs against the node order, and so does its lane.
    assert_eq!(read(&[("highway", "primary"), ("oneway", "-1"), ("busway", "lane")]), bus(0, 1));
    assert_eq!(
        read(&[("highway", "primary"), ("oneway", "-1"), ("busway", "opposite_lane")]),
        bus(1, 0)
    );
}

#[test]
fn lane_counts_are_read_per_direction() {
    let road = [("highway", "secondary")];
    let with = |more: &[(&'static str, &'static str)]| {
        let mut all = road.to_vec();
        all.extend_from_slice(more);
        read(&all)
    };
    assert_eq!(with(&[("lanes:bus:backward", "1")]), bus(0, 1));
    assert_eq!(with(&[("lanes:psv:forward", "1"), ("lanes:psv:backward", "2")]), bus(1, 2));
    // A total on a two-way road is split evenly; an odd one's last lane has no direction.
    assert_eq!(with(&[("lanes:bus", "2")]), bus(1, 1));
    assert_eq!(with(&[("lanes:bus", "1")]), bus(0, 0));
    // On a one-way street the total is the street's direction; a backward count is a contraflow.
    let one_way = read(&[("highway", "secondary"), ("oneway", "yes"), ("lanes:bus", "1")]);
    assert_eq!(one_way, bus(1, 0));
    let contraflow =
        read(&[("highway", "secondary"), ("oneway", "yes"), ("lanes:bus:backward", "1")]);
    assert_eq!(contraflow, bus(0, 1));
}

#[test]
fn lane_lists_count_their_designated_lanes() {
    let one_way = [("highway", "primary"), ("oneway", "yes"), ("lanes", "3")];
    let with = |more: &[(&'static str, &'static str)]| {
        let mut all = one_way.to_vec();
        all.extend_from_slice(more);
        read(&all)
    };
    assert_eq!(with(&[("bus:lanes", "no|no|designated")]), bus(1, 0));
    assert_eq!(with(&[("psv:lanes", "designated|yes|designated")]), bus(2, 0));
    // `yes` (buses allowed, as everywhere) is not a bus lane.
    assert_eq!(with(&[("bus:lanes", "yes|yes|yes")]), bus(0, 0));
    // On a two-way road only the `:forward` and `:backward` forms say which way.
    let two_way = read(&[
        ("highway", "primary"),
        ("lanes", "4"),
        ("bus:lanes:forward", "no|designated"),
        ("bus:lanes:backward", "designated|no"),
        ("bus:lanes", "designated|no|no|designated"),
    ]);
    assert_eq!(two_way, bus(1, 1));
    let plain_two_way = read(&[("highway", "primary"), ("bus:lanes", "designated|no")]);
    assert_eq!(plain_two_way, bus(0, 0));
}

#[test]
fn several_families_describing_one_lane_count_it_once() {
    let t = read(&[
        ("highway", "primary"),
        ("oneway", "yes"),
        ("lanes", "3"),
        ("busway:right", "lane"),
        ("lanes:bus", "1"),
        ("bus:lanes", "no|no|designated"),
        ("lanes:psv", "1"),
    ]);
    assert_eq!(t, bus(1, 0));
}

/// The Boulevard Périphérique's tags (Paris; e.g. OSM way 10228044): its left lane is for
/// buses, taxis, carpools and emergency vehicles.
const PERIPHERIQUE: [(&str, &str); 6] = [
    ("highway", "trunk"),
    ("oneway", "yes"),
    ("lanes", "4"),
    ("psv:lanes", "designated|yes|yes|yes"),
    ("hov:lanes", "designated|yes|yes|yes"),
    ("emergency:lanes", "designated|yes|yes|yes"),
];

#[test]
fn a_lane_also_for_carpools_is_not_a_bus_lane() {
    assert_eq!(read(&PERIPHERIQUE), bus(0, 0));
    let one_way = [("highway", "primary"), ("oneway", "yes"), ("lanes", "3")];
    let with = |more: &[(&'static str, &'static str)]| {
        let mut all = one_way.to_vec();
        all.extend_from_slice(more);
        read(&all)
    };
    // A bus lane alone is still taken.
    assert_eq!(with(&[("busway", "lane")]), bus(1, 0));
    assert_eq!(with(&[("psv:lanes", "designated|yes|yes")]), bus(1, 0));
    // A count with a carpool lane in the list: the carpool lane is the bus lane.
    assert_eq!(with(&[("lanes:psv", "1"), ("hov:lanes", "designated|no|no")]), bus(0, 0));
    assert_eq!(with(&[("busway:right", "lane"), ("lanes:hov", "1")]), bus(0, 0));
    // Two lists of one length: the places say which bus lane is also the carpools'.
    let apart = [("bus:lanes", "no|no|designated"), ("hov:lanes", "designated|no|no")];
    assert_eq!(with(&apart), bus(1, 0));
    let one_of_two = [("psv:lanes", "designated|no|designated"), ("hov:lanes", "designated|no|no")];
    assert_eq!(with(&one_of_two), bus(1, 0));
    // `yes` (carpools allowed, as everywhere) keeps nothing; neither does a conditional tag.
    assert_eq!(with(&[("psv:lanes", "designated|no|no"), ("hov:lanes", "yes|yes|yes")]), bus(1, 0));
    assert_eq!(
        with(&[
            ("psv:lanes", "designated|no|no"),
            ("hov:lanes:conditional", "designated|| @ Mo-Fr")
        ]),
        bus(1, 0)
    );
    // A way that is the lane, `hov=designated`.
    assert_eq!(
        with(&[("bus", "designated"), ("busway", "lane"), ("hov", "designated")]),
        bus(0, 0)
    );
    // Per direction on a two-way road.
    let two_way = read(&[
        ("highway", "primary"),
        ("lanes", "4"),
        ("bus:lanes:forward", "no|designated"),
        ("bus:lanes:backward", "no|designated"),
        ("hov:lanes:forward", "no|designated"),
    ]);
    assert_eq!(two_way, bus(0, 1));
    // A contraflow bus lane, which cars do not drive, stays the buses'.
    assert_eq!(with(&[("busway", "opposite_lane"), ("hov", "designated")]), bus(0, 1));
}

#[test]
fn cars_keep_the_lanes_less_the_bus_lanes_never_below_one() {
    assert_eq!(tags::car_lanes(Some(3), 1), Some(2));
    assert_eq!(tags::car_lanes(Some(2), 2), Some(1));
    assert_eq!(tags::car_lanes(Some(1), 1), Some(1));
    assert_eq!(tags::car_lanes(Some(2), 0), Some(2));
    assert_eq!(tags::car_lanes(None, 1), None, "no count: the class default, less the lane");
}

// --- the import -------------------------------------------------------------------------------

const STEP: f64 = 0.001;

fn node(id: i64, i: f64) -> OsmNode {
    OsmNode::new(id, 4.80 + i * STEP, 45.70)
}

fn import_way(pairs: &[(&str, &str)], options: ImportOptions) -> (Vec<Link>, ImportReport) {
    let source = MemorySource::new().nodes([node(1, 0.0), node(2, 1.0)]).way(OsmWay::new(
        100,
        [1, 2],
        pairs.iter().copied(),
    ));
    let mut diagnostics = Diagnostics::new();
    let (net, report, _) = import(&source, options, &mut diagnostics).expect("importable");
    let links = (0..net.link_count())
        .map(|i| {
            let l = LinkId::new(i);
            Link {
                forward: net.link_from(l).raw() == 0,
                class: net.link_class(l),
                lanes: net.link_lanes(l),
                bus_lanes: net.link_bus_lanes(l),
                capacity: net.link_parameters(l).capacity.as_veh_per_hour(),
            }
        })
        .collect();
    (links, report)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Link {
    forward: bool,
    class: RoadClass,
    lanes: u8,
    bus_lanes: u8,
    capacity: f64,
}

fn find(links: &[Link], forward: bool) -> Link {
    *links.iter().find(|l| l.forward == forward).expect("a link that way")
}

#[test]
fn a_bus_lane_is_taken_out_of_the_cars_lanes_and_capacity() {
    let pairs = [("highway", "primary"), ("lanes", "4"), ("busway:right", "lane")];
    let (links, report) = import_way(&pairs, ImportOptions::default());
    let (f, b) = (find(&links, true), find(&links, false));
    assert_eq!((f.lanes, f.bus_lanes), (1, 1), "two lanes this way, one of them the bus's");
    assert_eq!((b.lanes, b.bus_lanes), (2, 0));
    assert!((f.capacity * 2.0 - b.capacity).abs() < 1e-9, "half the lanes, half the capacity");
    assert_eq!(report.bus_lane_links, 1);
    assert!(report.bus_lane_length_m > 70, "about 78 m: {}", report.bus_lane_length_m);
    assert_eq!(report.bus_lane_car_lane_length_removed_m, report.bus_lane_length_m);
    assert_eq!(report.contraflow_bus_links, 0);

    // Off: every lane is the cars'.
    let off = ImportOptions { bus_lanes: false, ..ImportOptions::default() };
    let (links, report) = import_way(&pairs, off);
    assert_eq!((find(&links, true).lanes, find(&links, true).bus_lanes), (2, 0));
    assert_eq!(report.bus_lane_links, 0);
}

#[test]
fn without_a_lanes_tag_the_class_default_loses_the_bus_lane() {
    // `primary`'s default is two lanes per direction; one goes to the bus.
    let (links, report) =
        import_way(&[("highway", "primary"), ("busway", "lane")], ImportOptions::default());
    for l in &links {
        assert_eq!((l.lanes, l.bus_lanes), (1, 1));
    }
    assert_eq!(report.bus_lane_links, 2);
    // `residential`'s is one; the cars keep it.
    let (links, report) =
        import_way(&[("highway", "residential"), ("busway", "lane")], ImportOptions::default());
    for l in &links {
        assert_eq!((l.lanes, l.bus_lanes), (1, 1));
    }
    assert_eq!(report.bus_lane_car_lane_length_removed_m, 0);
}

#[test]
fn a_contraflow_bus_lane_is_a_busway_against_the_one_way_street() {
    let pairs = [("highway", "secondary"), ("oneway", "yes"), ("busway", "opposite_lane")];
    let (links, report) = import_way(&pairs, ImportOptions::default());
    assert_eq!(links.len(), 2);
    let (with, against) = (find(&links, true), find(&links, false));
    assert_eq!((with.class, with.bus_lanes), (RoadClass::Secondary, 0));
    assert_eq!((against.class, against.lanes, against.bus_lanes), (RoadClass::Busway, 1, 1));
    assert_eq!(report.contraflow_bus_links, 1);
    assert_eq!(report.bus_lane_links, 0, "the street's own direction has no bus lane");
    // Off: the one-way street alone.
    let off = ImportOptions { bus_lanes: false, ..ImportOptions::default() };
    assert_eq!(import_way(&pairs, off).0.len(), 1);
}

#[test]
fn a_one_way_street_buses_ride_both_ways_has_a_busway_against_it() {
    for key in ["oneway:bus", "oneway:psv"] {
        let pairs = [("highway", "tertiary"), ("oneway", "yes"), (key, "no")];
        let (links, report) = import_way(&pairs, ImportOptions::default());
        assert_eq!(links.len(), 2, "{key}");
        let against = find(&links, false);
        assert_eq!(
            (against.class, against.bus_lanes),
            (RoadClass::Busway, 0),
            "no lane of its own"
        );
        assert_eq!(report.contraflow_bus_links, 1);
        let off = ImportOptions { bus_lanes: false, ..ImportOptions::default() };
        assert_eq!(import_way(&pairs, off).0.len(), 1);
    }
    // A two-way street needs nothing more.
    let (links, report) =
        import_way(&[("highway", "tertiary"), ("oneway:bus", "no")], ImportOptions::default());
    assert!(links.iter().all(|l| l.class == RoadClass::Tertiary));
    assert_eq!(report.contraflow_bus_links, 0);
}

#[test]
fn the_peripheriques_four_lanes_stay_the_cars() {
    let (links, report) = import_way(&PERIPHERIQUE, ImportOptions::default());
    assert_eq!(links.len(), 1);
    assert_eq!((links[0].lanes, links[0].bus_lanes), (4, 0));
    assert_eq!(report.bus_lane_links, 0);
    assert_eq!(report.bus_lane_car_lane_length_removed_m, 0);
    // A bus lane alone on the same road: three lanes for cars.
    let mut busway = PERIPHERIQUE[..3].to_vec();
    busway.push(("busway", "lane"));
    let (links, report) = import_way(&busway, ImportOptions::default());
    assert_eq!((links[0].lanes, links[0].bus_lanes), (3, 1));
    assert_eq!(report.bus_lane_links, 1);
}
