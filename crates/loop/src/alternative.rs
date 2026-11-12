//! An alternative of a traveller: its mode and its legs; the pricing of legs on a reading; the
//! path sizes of a set's routes.

use std::collections::HashMap;

use openmobisim_graph::network::Network;
use openmobisim_loading::link_times::{LinkTimes, Reader};
use openmobisim_transit::raptor::Journey;
use openmobisim_types::ids::{LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

use crate::parameters::{LN_PS, N};

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
    /// The natural logarithm of a car route's path size among the set's car routes; 0 for a
    /// lone car route and for every other mode.
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

    /// The choice row of the alternative's mode, the columns of
    /// [`crate::parameters::COLUMNS`]; times in minutes; a column of another mode is 0.
    #[must_use]
    pub fn attributes(&self) -> [f64; N] {
        let mut x = [0.0; N];
        let gaps: f64 =
            self.legs.iter().map(|leg| if let Leg::Walk(d) = leg { d.get() } else { 0.0 }).sum();
        match self.mode {
            Mode::Car => {
                x[0] = (self.time - gaps) / 60.0;
                x[2] = gaps / 60.0;
                x[LN_PS] = self.ln_path_size;
            }
            Mode::Bike => {
                x[1] = (self.time - gaps) / 60.0;
                x[2] = gaps / 60.0;
            }
            Mode::Walk => x[2] = self.time / 60.0,
            Mode::Transit => self.transit_parts(&mut x),
        }
        x[12 + usize::from(self.mode.as_u8())] = 1.0;
        x
    }

    /// The parts of a journey, minutes: in vehicle, the walk to the first stop, the walk from
    /// the last, the first wait, the waits at transfers, the walks between stops, the transfers.
    /// The first wait runs from the end of the access walk to the first boarding, so it holds
    /// the wait for the whole second the stop is reached on, and the six times add up to the
    /// door-to-door time.
    fn transit_parts(&self, x: &mut [f64; N]) {
        let (Some(Leg::Walk(access)), Some(Leg::Journey(j)), Some(Leg::Walk(egress))) =
            (self.legs.first(), self.legs.get(1), self.legs.get(2))
        else {
            return;
        };
        let board = f64::from(j.rides[0].board.1.get());
        let alight = j.rides.last().map_or(board, |r| f64::from(r.alight.1.get()));
        let first_wait = self.time - access.get() - egress.get() - (alight - board);
        x[3] = j.in_vehicle().get() / 60.0;
        x[4] = access.get() / 60.0;
        x[5] = egress.get() / 60.0;
        x[6] = first_wait / 60.0;
        x[7] = (j.wait().get() - (board - f64::from(j.access.1.get()))) / 60.0;
        x[8] = j.transfer_walk().get() / 60.0;
        x[9] = f64::from(u32::try_from(j.ride_count()).unwrap_or(u32::MAX)) - 1.0;
    }

    /// Whether `links` is this alternative's route, by `mode`.
    #[must_use]
    pub fn is_route(&self, mode: Mode, links: &[LinkId]) -> bool {
        self.mode == mode && self.route() == Some(links)
    }
}

/// The door-to-door seconds of `legs` for a traveller leaving `departure` seconds after the
/// window's `start`, read as the traveller of `trip`: a walk adds its seconds; a route leg is
/// entered at the instant it is reached (as the loading departs it) and takes the reading's
/// time for that traveller; a journey leg ends when its last ride arrives.
#[must_use]
pub fn price(times: &LinkTimes, start: Second, departure: f64, legs: &[Leg], trip: TripId) -> f64 {
    let mut t = departure;
    for leg in legs {
        match leg {
            Leg::Walk(d) => t += d.get(),
            Leg::Route { links, .. } if links.is_empty() => {}
            Leg::Route { mode, links } => {
                t += times.route_time(links, *mode, Duration(t), Reader::of(trip)).get();
            }
            Leg::Journey(j) => {
                let alight = j.rides.last().map_or(start, |r| r.alight.1);
                t = f64::from(alight.saturating_sub(start));
            }
        }
    }
    t - departure
}

/// Rewrite every `ln_path_size` of `set`: over the set's car routes, the path size of
/// Ben-Akiva and Bierlaire (1999), `PS_i = Σ_{a ∈ i} (l_a / L_i) / n_a`, `n_a` the car routes
/// using link `a`; 1 for a lone car route, an empty route, a bike or walk route and a journey.
pub fn path_sizes(network: &Network, set: &mut [Alternative]) {
    for alternative in set.iter_mut() {
        alternative.ln_path_size = 0.0;
    }
    for mode in [Mode::Car] {
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
