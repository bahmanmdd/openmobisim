//! An alternative of a traveller: its mode and its legs; the pricing of legs on a reading; the
//! path sizes of a set's routes.

use std::collections::HashMap;

use openmobisim_graph::network::Network;
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_transit::raptor::Journey;
use openmobisim_types::ids::LinkId;
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

use crate::whole;

/// A leg: one mode between two places.
#[derive(Clone, Debug, PartialEq)]
pub enum Leg {
    /// A walk off the network, priced by its seconds: the gap between a door and its node, or a
    /// transit access or egress walk as the demand priced it. Loaded nowhere.
    Walk(Duration),
    /// A route on the network by car, by bike or on foot, the links in order (empty when the
    /// origin's node is the destination's). Loaded as one vehicle.
    Route {
        /// Car, bike or walk.
        mode: Mode,
        /// The links in the order travelled.
        links: Vec<LinkId>,
    },
    /// A journey on the timetable, as scheduled: it ends when its last ride arrives.
    Journey(Journey),
}

/// One alternative in a traveller's set: three legs, walk, route or journey, walk.
#[derive(Clone, Debug, PartialEq)]
pub struct Alternative {
    /// The alternative's mode: its nest in the choice.
    pub mode: Mode,
    /// The legs in order.
    pub legs: Vec<Leg>,
    /// Door to door, seconds, as priced at the last measure.
    pub time: f64,
    /// The natural logarithm of the route's path size among the set's routes of its mode; 0
    /// for a journey or a lone route.
    pub ln_path_size: f64,
}

impl Alternative {
    /// The links of the route leg, if the alternative has one.
    #[must_use]
    pub fn route(&self) -> Option<&[LinkId]> {
        self.legs.iter().find_map(|leg| match leg {
            Leg::Route { links, .. } => Some(links.as_slice()),
            _ => None,
        })
    }

    /// The choice row's attributes: time in seconds, money in euros (0: no source of money
    /// exists yet), `ln PS`.
    #[must_use]
    pub fn attributes(&self) -> [f64; 3] {
        [self.time, 0.0, self.ln_path_size]
    }

    /// Whether `links` is this alternative's route, by `mode`.
    #[must_use]
    pub fn is_route(&self, mode: Mode, links: &[LinkId]) -> bool {
        self.mode == mode && self.route() == Some(links)
    }
}

/// The door-to-door seconds of `legs` for a traveller leaving `departure` seconds after the
/// window's `start`: a walk adds its seconds; a route leg is entered on the whole second it is
/// reached (as the loading departs it) and takes the reading's time; a journey leg ends when
/// its last ride arrives.
#[must_use]
pub fn price(times: &LinkTimes, start: Second, departure: f64, legs: &[Leg]) -> f64 {
    let mut t = departure;
    for leg in legs {
        match leg {
            Leg::Walk(d) => t += d.get(),
            Leg::Route { links, .. } if links.is_empty() => {}
            Leg::Route { mode, links } => {
                let at = whole(t);
                let second = start.saturating_add(at);
                t = f64::from(at) + times.route_time(links, *mode, second).get();
            }
            Leg::Journey(j) => {
                let alight = j.rides.last().map_or(start, |r| r.alight.1);
                t = f64::from(alight.saturating_sub(start));
            }
        }
    }
    t - departure
}

/// Rewrite every `ln_path_size` of `set`: per mode, over the set's routes of that mode, the
/// path size of Ben-Akiva and Bierlaire (1999), `PS_i = Σ_{a ∈ i} (l_a / L_i) / n_a`, `n_a` the
/// routes of the mode using link `a`; 1 for a lone route, an empty route or a journey.
pub fn path_sizes(network: &Network, set: &mut [Alternative]) {
    for alternative in set.iter_mut() {
        alternative.ln_path_size = 0.0;
    }
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        let routes: Vec<usize> =
            (0..set.len()).filter(|&i| set[i].mode == mode && set[i].route().is_some()).collect();
        if routes.len() < 2 {
            continue;
        }
        let mut users: HashMap<LinkId, u32> = HashMap::new();
        for &i in &routes {
            for &a in set[i].route().unwrap_or(&[]) {
                *users.entry(a).or_insert(0) += 1;
            }
        }
        for &i in &routes {
            let links = set[i].route().unwrap_or(&[]);
            let total: f64 = links.iter().map(|&a| network.length(a).get()).sum();
            let ps = if links.is_empty() {
                1.0
            } else {
                links.iter().map(|&a| network.length(a).get() / total / f64::from(users[&a])).sum()
            };
            set[i].ln_path_size = f64::ln(ps);
        }
    }
}
