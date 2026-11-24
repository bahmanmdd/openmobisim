//! Stage 3, load: the chosen alternatives' route legs as vehicles on the point queue; stage 4,
//! measure: every alternative with a car route re-priced on the new reading; and the realised
//! door-to-door outcome of every trip.

use openmobisim_loading::input::{LoadError, Vehicle};
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_loading::point_queue::{self, Loading};
use openmobisim_types::ids::EntityId;
use openmobisim_types::units::Duration;
use rayon::prelude::*;

use crate::alternative::{Alternative, Leg, path_sizes, price};
use crate::context::Context;

/// The vehicle of trip `i` on `alternative`: its route leg, entered at the instant the walk
/// before it ends; none for an empty route or a journey.
///
/// # Panics
///
/// For an alternative that is not three legs, a walk, a route or a journey, and a walk
/// ([`Alternative::three_legs`]): a fault of the program, since a second leg on the network
/// would not be loaded.
fn vehicle(ctx: &Context<'_>, i: usize, alternative: &Alternative) -> Option<Vehicle> {
    let (before, between, _) = alternative.three_legs();
    let Leg::Route { mode, links } = between else { return None };
    if links.is_empty() {
        return None;
    }
    Some(Vehicle {
        trip: ctx.trip(i),
        mode: *mode,
        route: links.clone(),
        departure: Duration(ctx.departure[i] + before.get()),
    })
}

/// The load stage: every chosen route leg loaded on the point queue over the window.
///
/// # Errors
///
/// As [`point_queue::load`]: never for vehicles the search built, whose routes are on links
/// their mode may use and whose departures are inside the window.
pub(crate) fn load(
    ctx: &Context<'_>,
    sets: &[Vec<Alternative>],
    chosen: &[Option<u32>],
) -> Result<Loading, LoadError> {
    let vehicles: Vec<Vehicle> = (0..ctx.trip_count())
        .filter_map(|i| chosen[i].and_then(|j| vehicle(ctx, i, &sets[i][j as usize])))
        .collect();
    point_queue::load(ctx.network, &vehicles, ctx.window)
}

/// The measure stage: every alternative whose time the loading can change (one with a car
/// route) priced again on `times`. The others keep the time the search priced them at: a bike,
/// a walker and a journey read no car's crossing, so pricing them again gives the same number.
/// The path sizes are the search's: they depend on who is in a set, not on the reading. Both
/// are checked in debug builds ([`left_alone`]).
pub(crate) fn measure(ctx: &Context<'_>, times: &LinkTimes, sets: &mut [Vec<Alternative>]) {
    sets.par_iter_mut().enumerate().for_each(|(i, set)| {
        debug_assert!(
            left_alone(ctx, times, i, set),
            "trip {i}: the reading changed a time or a path size the measure stage leaves alone"
        );
        for alternative in set.iter_mut().filter(|a| a.reads_the_loading()) {
            alternative.time =
                price(times, ctx.start(), ctx.departure[i], &alternative.legs, ctx.trip(i));
        }
    });
}

/// Whether what the measure stage leaves alone in trip `i`'s `set` is what it would compute on
/// `times`, to the bit: the time of every alternative without a car route, and every path
/// size. Read in debug builds only, so that a change that makes either depend on the loading
/// (a bus on roads, a journey reached by car) fails the tests instead of going unseen.
fn left_alone(ctx: &Context<'_>, times: &LinkTimes, i: usize, set: &[Alternative]) -> bool {
    let mut again = set.to_vec();
    path_sizes(ctx.network, &mut again);
    set.iter().zip(&again).all(|(a, b)| {
        let time = || price(times, ctx.start(), ctx.departure[i], &a.legs, ctx.trip(i));
        a.ln_path_size.to_bits() == b.ln_path_size.to_bits()
            && (a.reads_the_loading() || time().to_bits() == a.time.to_bits())
    })
}

/// What every trip did in the loading: its arrival at the door, seconds after the window's
/// start, and whether it got there before the window's end.
pub(crate) struct Realised {
    /// Per trip, the door arrival; `NaN` for a trip without an alternative or unfinished.
    pub arrival: Vec<f64>,
    /// Per trip, whether it arrived before the window's end.
    pub finished: Vec<bool>,
}

/// The outcome of every trip: a loaded trip's arrival is the loading's at its last node plus
/// the walk after it; an unloaded alternative (an empty route, a journey) arrives at its
/// departure plus its measured time. A trip is finished when its door arrival is before the
/// window's end; an unfinished trip's arrival is unknown, `NaN`.
///
/// # Panics
///
/// For a loaded trip whose alternative is not three legs, a walk, a route or a journey, and a
/// walk ([`Alternative::three_legs`]): a fault of the program, since the walk after the route is
/// what takes it from the loading's last node to the door.
pub(crate) fn realise(
    ctx: &Context<'_>,
    loading: &Loading,
    sets: &[Vec<Alternative>],
    chosen: &[Option<u32>],
) -> Realised {
    let n = ctx.trip_count();
    let mut arrival = vec![f64::NAN; n];
    let mut finished = vec![false; n];
    for i in 0..n {
        let Some(j) = chosen[i] else { continue };
        let alternative = &sets[i][j as usize];
        arrival[i] = ctx.departure[i] + alternative.time;
    }
    let (trips, arrivals) = (loading.finished.trips(), loading.finished.arrivals());
    for (&trip, &at) in trips.iter().zip(arrivals) {
        let i = trip.index();
        let Some(j) = chosen[i] else { continue };
        let (_, _, egress) = sets[i][j as usize].three_legs();
        arrival[i] = at.get() + egress.get();
    }
    let mut unfinished = vec![false; n];
    for &trip in loading.unfinished.trips() {
        unfinished[trip.index()] = true;
    }
    let length = ctx.length();
    for i in 0..n {
        finished[i] = !unfinished[i] && arrival[i] < length;
        if !finished[i] {
            arrival[i] = f64::NAN;
        }
    }
    Realised { arrival, finished }
}

#[cfg(test)]
mod tests {
    //! An alternative of a shape the loading does not know is refused, not loaded in part: one
    //! car trip at second 0 on a street a→b of 1 000 m, in a window of an hour.

    use openmobisim_demand::demand::Demand;
    use openmobisim_graph::geometry::{Position, Xy};
    use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
    use openmobisim_loading::input::Vehicle;
    use openmobisim_loading::point_queue;
    use openmobisim_types::ids::{EntityId, LinkId, TripId};
    use openmobisim_types::mode::{Mode, ModeSet};
    use openmobisim_types::time::{Second, StepGrid};
    use openmobisim_types::units::{Duration, Metres};

    use super::{realise, vehicle};
    use crate::alternative::{Alternative, Leg};
    use crate::context::Context;
    use crate::parameters::Parameters;

    /// The street a→b, its link, and a demand of no trip (the stages read the context's trips).
    fn street() -> (Network, LinkId, Demand) {
        let mut b = NetworkBuilder::new();
        b.add_node("a", Position::Metres(Xy::new(0.0, 0.0)));
        b.add_node("b", Position::Metres(Xy::new(1_000.0, 0.0)));
        let row = LinkRow {
            modes: ModeSet::of(&[Mode::Car]),
            lanes: Some(1),
            car_speed_km_h: Some(50.0),
            length_m: Some(1_000.0),
            capacity_veh_h: Some(1_800.0),
            signal_at_downstream_end: false,
        };
        b.add_link("ab", "a", "b", row);
        let values = BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };
        let network = b.build(values).expect("a network");
        let link = network.link_ids().typed_id_of("ab").expect("a link");
        let demand = Demand::build(&[], &["default".to_string()], &network, None, Metres(1_500.0))
            .expect("a demand");
        (network, link, demand)
    }

    /// One car trip at second 0 over the hour.
    fn context<'a>(network: &'a Network, demand: &'a Demand, p: &'a Parameters) -> Context<'a> {
        Context {
            network,
            demand,
            timetable: None,
            parameters: p,
            window: StepGrid::new(Second(0), 60, 3_600).expect("a window"),
            departure: vec![0.0],
            class: vec![0],
            modes: vec![ModeSet::of(&[Mode::Car])],
            car_pair: vec![Some(0)],
            car_pair_trips: vec![vec![0]],
        }
    }

    #[test]
    #[should_panic(
        expected = "an alternative is three legs, a walk, a route or a journey, and a walk"
    )]
    fn a_car_alternative_with_two_routes_is_refused_by_the_loading() {
        let (network, link, demand) = street();
        let parameters = Parameters::defaults(1);
        let ctx = context(&network, &demand, &parameters);
        let route = Leg::Route { mode: Mode::Car, links: vec![link] };
        let legs = vec![Leg::Walk(Duration(0.0)), route.clone(), route, Leg::Walk(Duration(0.0))];
        let alternative = Alternative { mode: Mode::Car, legs, time: 144.0, ln_path_size: 0.0 };
        let _ = vehicle(&ctx, 0, &alternative);
    }

    #[test]
    #[should_panic(
        expected = "an alternative is three legs, a walk, a route or a journey, and a walk"
    )]
    fn a_car_alternative_ending_on_a_route_is_refused_by_the_outcome_of_its_trip() {
        let (network, link, demand) = street();
        let parameters = Parameters::defaults(1);
        let ctx = context(&network, &demand, &parameters);
        let car = Vehicle {
            trip: TripId::from_index(0),
            mode: Mode::Car,
            route: vec![link],
            departure: Duration(0.0),
        };
        let loading = point_queue::load(&network, &[car], ctx.window).expect("a loading");
        let legs = vec![
            Leg::Walk(Duration(0.0)),
            Leg::Route { mode: Mode::Car, links: vec![link] },
            Leg::Walk(Duration(0.0)),
            Leg::Route { mode: Mode::Walk, links: Vec::new() },
        ];
        let alternative = Alternative { mode: Mode::Car, legs, time: 144.0, ln_path_size: 0.0 };
        let _ = realise(&ctx, &loading, &[vec![alternative]], &[Some(0)]);
    }
}
