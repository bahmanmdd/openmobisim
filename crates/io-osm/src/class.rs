//! The street classes: which `highway` values are streets, which modes each lets through when
//! no access tag says otherwise, and the car's free-flow speed and lanes when no tag gives them.
//!
//! # The modes by class
//!
//! The Netherlands row of the OpenStreetMap wiki's access defaults ("OSM tags for routing /
//! Access restrictions", <https://wiki.openstreetmap.org/wiki/OSM_tags_for_routing/Access_restrictions>),
//! read for car (`motorcar`), bike (`bicycle`) and walk (`foot`): `yes` and `designated` let the
//! mode through; `no` and `dismount` do not (a bike pushed is a walker). One departure, as in
//! version 1: a `track` is not a car street (the wiki says `yes`), since the class table has no
//! car speed for a farm track. `motorway_link` and the other `_link` values take their road's
//! row. `road` (a road of unknown class) takes `unclassified`'s.
//!
//! # The car's defaults by class
//!
//! The free-flow speed (km/h) where no `maxspeed` is tagged and the lanes in each direction
//! where no `lanes` is tagged: version 1's table (`free_flow_km_h`, `lanes_per_direction`),
//! itself "common OSM speed conventions". It is a default, the row `osm_class_defaults` of
//! `PARAMETERS.md`; the table is in the crate's README, and a test reads every row.

/// One street class: its `highway` values and what it lets through by default.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Class {
    /// The `highway` values of the class.
    pub highway: &'static [&'static str],
    /// The car's default: `Some((free-flow speed in km/h, lanes per direction))` when cars may
    /// use the class, `None` when they may not.
    pub car: Option<(f64, u8)>,
    /// Bikes may ride it by default.
    pub bike: bool,
    /// Walkers may walk it by default.
    pub walk: bool,
    /// Its traffic is one-way along the way's node order unless `oneway` says otherwise
    /// (motorways, by the Vienna Convention, as the wiki's `oneway` page says).
    pub one_way: bool,
}

const fn car(highway: &'static [&'static str], km_h: f64, lanes: u8, bike: bool) -> Class {
    Class { highway, car: Some((km_h, lanes)), bike, walk: bike, one_way: false }
}

const fn no_car(highway: &'static [&'static str], bike: bool, walk: bool) -> Class {
    Class { highway, car: None, bike, walk, one_way: false }
}

/// Every street class. A `highway` value not listed here is not a street (`construction`,
/// `proposed`, `platform`, `raceway`, `bus_guideway`, …), and its way is not read.
pub const CLASSES: [Class; 23] = [
    Class { one_way: true, ..car(&["motorway"], 110.0, 2, false) },
    Class { one_way: true, ..car(&["motorway_link"], 60.0, 1, false) },
    car(&["trunk"], 90.0, 2, true),
    car(&["trunk_link"], 50.0, 1, true),
    car(&["primary"], 60.0, 2, true),
    car(&["primary_link"], 40.0, 1, true),
    car(&["secondary"], 50.0, 1, true),
    car(&["secondary_link"], 40.0, 1, true),
    car(&["tertiary"], 50.0, 1, true),
    car(&["tertiary_link"], 30.0, 1, true),
    car(&["unclassified", "road"], 40.0, 1, true),
    car(&["residential"], 30.0, 1, true),
    car(&["living_street"], 20.0, 1, true),
    car(&["service"], 20.0, 1, true),
    no_car(&["pedestrian"], false, true),
    no_car(&["track"], true, true),
    no_car(&["busway"], false, false),
    no_car(&["footway"], false, true),
    no_car(&["bridleway"], false, false),
    no_car(&["steps"], false, true),
    no_car(&["corridor"], false, true),
    no_car(&["path"], true, true),
    no_car(&["cycleway"], true, true),
];

/// The class of a `highway` value, if it is a street.
#[must_use]
pub fn class_of(highway: &str) -> Option<&'static Class> {
    CLASSES.iter().find(|c| c.highway.contains(&highway))
}
