//! Stage 3, load: the chosen alternatives' route legs as vehicles on the point queue; stage 4,
//! measure: every alternative of every set re-priced on the new reading; and the realised
//! door-to-door outcome of every trip.

use openmobisim_loading::input::{LoadError, Vehicle};
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_loading::point_queue::{self, Loading};
use openmobisim_types::ids::EntityId;
use rayon::prelude::*;

use crate::alternative::{Alternative, Leg, path_sizes, price};
use crate::context::Context;
use crate::whole;

/// The vehicle of trip `i` on `alternative`: its route leg, entered on the whole second the walk
/// before it ends; none for an empty route or a journey.
fn vehicle(ctx: &Context<'_>, i: usize, alternative: &Alternative) -> Option<Vehicle> {
    let mut t = ctx.departure[i];
    for leg in &alternative.legs {
        match leg {
            Leg::Walk(d) => t += d.get(),
            Leg::Route { mode, links } => {
                if links.is_empty() {
                    return None;
                }
                let departure = ctx.start().saturating_add(whole(t));
                return Some(Vehicle {
                    trip: ctx.trip(i),
                    mode: *mode,
                    route: links.clone(),
                    departure,
                });
            }
            Leg::Journey(_) => return None,
        }
    }
    None
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

/// The measure stage: every alternative of every set priced again on `times`, then the path
/// sizes over each set.
pub(crate) fn measure(ctx: &Context<'_>, times: &LinkTimes, sets: &mut [Vec<Alternative>]) {
    sets.par_iter_mut().enumerate().for_each(|(i, set)| {
        for alternative in set.iter_mut() {
            alternative.time = price(times, ctx.start(), ctx.departure[i], &alternative.legs);
        }
        path_sizes(ctx.network, set);
    });
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
        let egress = match sets[i][j as usize].legs.last() {
            Some(Leg::Walk(d)) => d.get(),
            _ => 0.0,
        };
        arrival[i] = at.get() + egress;
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
