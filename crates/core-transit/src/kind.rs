//! What kind of service a line is, from GTFS `route_type`, and whether it rides
//! the roads (design §18.2).

/// The kind of service a route runs.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum ServiceKind {
    /// Trains: intercity, regional, suburban.
    Rail = 0,
    /// Metro, underground, light rail on its own track, monorail.
    Metro = 1,
    /// Tram.
    Tram = 2,
    /// Bus, coach and trolleybus: on the roads.
    Bus = 3,
    /// Ferry and other water transport.
    Ferry = 4,
    /// Cable cars, funiculars and anything else scheduled.
    Other = 5,
}

impl ServiceKind {
    /// Every kind, in id order.
    pub const ALL: [ServiceKind; 6] = [
        ServiceKind::Rail,
        ServiceKind::Metro,
        ServiceKind::Tram,
        ServiceKind::Bus,
        ServiceKind::Ferry,
        ServiceKind::Other,
    ];

    /// The kind of a GTFS `route_type`: the basic types 0–12 and the extended
    /// (Google "Hierarchical Vehicle Type") types 100–1799.
    #[must_use]
    pub const fn from_route_type(route_type: u16) -> Self {
        match route_type {
            0 | 5 | 900..=999 => ServiceKind::Tram,
            1 | 12 | 400..=699 => ServiceKind::Metro,
            2 | 100..=199 | 300..=399 => ServiceKind::Rail,
            3 | 11 | 200..=299 | 700..=899 => ServiceKind::Bus,
            4 | 1000..=1099 | 1200..=1299 => ServiceKind::Ferry,
            _ => ServiceKind::Other,
        }
    }

    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ServiceKind::Rail => "rail",
            ServiceKind::Metro => "metro",
            ServiceKind::Tram => "tram",
            ServiceKind::Bus => "bus",
            ServiceKind::Ferry => "ferry",
            ServiceKind::Other => "other",
        }
    }

    /// Whether its vehicles ride the road network, among the cars, in this
    /// version: buses (design §18.2). Trams are run by the schedule for now,
    /// since GTFS does not say which run on the street and which on their own
    /// track; rail, metro and ferries follow the schedule by construction.
    #[must_use]
    pub const fn rides_road(self) -> bool {
        matches!(self, ServiceKind::Bus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_and_extended_types() {
        let cases = [
            (0, ServiceKind::Tram),
            (1, ServiceKind::Metro),
            (2, ServiceKind::Rail),
            (3, ServiceKind::Bus),
            (4, ServiceKind::Ferry),
            (5, ServiceKind::Tram),
            (6, ServiceKind::Other),
            (7, ServiceKind::Other),
            (11, ServiceKind::Bus),
            (12, ServiceKind::Metro),
            (102, ServiceKind::Rail),
            (200, ServiceKind::Bus),
            (401, ServiceKind::Metro),
            (700, ServiceKind::Bus),
            (715, ServiceKind::Bus),
            (800, ServiceKind::Bus),
            (900, ServiceKind::Tram),
            (1000, ServiceKind::Ferry),
            (1200, ServiceKind::Ferry),
            (1300, ServiceKind::Other),
            (1700, ServiceKind::Other),
        ];
        for (route_type, kind) in cases {
            assert_eq!(ServiceKind::from_route_type(route_type), kind, "route_type {route_type}");
        }
        assert!(ServiceKind::Bus.rides_road());
        assert!(!ServiceKind::Tram.rides_road());
    }
}
