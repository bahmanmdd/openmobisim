//! The run: the five stages iterated to the stopping rule, and its three tables.

use core::fmt;
use std::time::Instant;

use openmobisim_choice::cost::ChoiceError;
use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::Network;
use openmobisim_loading::input::LoadError;
use openmobisim_loading::link_bins::LinkBins;
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_loading::point_queue;
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, TripId, UserClassId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::{Second, StepGrid, StepGridError};

use crate::alternative::Alternative;
use crate::assess::{Iteration, assess, stops};
use crate::choose::{Model, choose};
use crate::context::Context;
use crate::load::{load, measure, realise};
use crate::parameters::{COLUMNS, Parameters};
use crate::search::search;

/// Why a run was refused. Each names what it refuses.
#[derive(Clone, Debug, PartialEq)]
pub enum RunError {
    /// `link_bin` is zero.
    LinkBinZero,
    /// `iteration_cap` is zero.
    IterationCapZero,
    /// `max_rides` is zero.
    MaxRidesZero,
    /// `nearby_stops` is zero.
    NearbyStopsZero,
    /// The coefficients are not one row per user class.
    CoefficientRows {
        /// The demand's classes.
        classes: u32,
        /// The rows given.
        rows: usize,
    },
    /// A class's cost of a transit part (in vehicle, a walk, a wait, a transfer) is negative or
    /// not finite: the cheapest journey would not exist.
    TransitWeight {
        /// The class, by its place in the coefficients.
        class: usize,
        /// The column.
        column: &'static str,
    },
    /// The window is not a whole number of bins, is empty, or runs past the clock.
    Window(StepGridError),
    /// A trip's departure is outside the window.
    DepartureOutsideWindow(String),
    /// A trip would reach the network, by this mode, at or after the window's end.
    StartsAfterWindow(String, Mode),
    /// The choice model refused its coefficients or a cost.
    Choice(ChoiceError),
    /// The loading refused a vehicle.
    Load(LoadError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LinkBinZero => write!(f, "link_bin is zero"),
            Self::IterationCapZero => write!(f, "iteration_cap is zero"),
            Self::MaxRidesZero => write!(f, "max_rides is zero"),
            Self::NearbyStopsZero => write!(f, "nearby_stops is zero"),
            Self::CoefficientRows { classes, rows } => {
                write!(f, "{rows} coefficient rows for {classes} user classes")
            }
            Self::TransitWeight { class, column } => {
                write!(f, "class {class}: the coefficient of {column} is negative or not finite")
            }
            Self::Window(e) => write!(f, "the window: {e}"),
            Self::DepartureOutsideWindow(t) => write!(f, "trip {t} departs outside the window"),
            Self::StartsAfterWindow(t, m) => {
                write!(f, "trip {t} would reach the network by {m} at or after the window's end")
            }
            Self::Choice(e) => write!(f, "the choice: {e}"),
            Self::Load(e) => write!(f, "the loading: {e}"),
        }
    }
}

impl core::error::Error for RunError {}

impl From<ChoiceError> for RunError {
    fn from(e: ChoiceError) -> Self {
        Self::Choice(e)
    }
}

impl From<LoadError> for RunError {
    fn from(e: LoadError) -> Self {
        Self::Load(e)
    }
}

/// The trips table, one row per trip of the demand, in trip id order.
#[derive(Clone, Debug, PartialEq)]
pub struct Trips {
    /// The trip's class.
    pub class: Vec<UserClassId>,
    /// The trip's departure, a second of the clock.
    pub departure: Vec<Second>,
    /// The mode of the alternative it took; none for a trip without an alternative.
    pub mode: Vec<Option<Mode>>,
    /// Its arrival at the door, seconds after the window's start; `NaN` without an alternative
    /// or unfinished.
    pub arrival: Vec<f64>,
    /// Door to door, seconds, as realised; `NaN` without an alternative or unfinished.
    pub travel_time: Vec<f64>,
    /// Whether it arrived before the window's end.
    pub finished: Vec<bool>,
    /// The cost of the alternative it took, at the last measure; `NaN` without an alternative.
    pub cost: Vec<f64>,
    /// How many alternatives its set holds.
    pub alternatives: Vec<u32>,
}

/// What a run returns: the three tables, whether the rule stopped it, and the sets.
#[derive(Clone, Debug)]
pub struct Run {
    /// The trips table.
    pub trips: Trips,
    /// The links-by-bin table of the last loading.
    pub links: LinkBins,
    /// The iterations table.
    pub iterations: Vec<Iteration>,
    /// Whether the stopping rule stopped the run (else the cap did).
    pub stopped_by_rule: bool,
    sets: Vec<Vec<Alternative>>,
    chosen: Vec<Option<u32>>,
}

impl Run {
    /// A trip's set, in the order the alternatives were found.
    #[must_use]
    pub fn set(&self, trip: TripId) -> &[Alternative] {
        &self.sets[trip.index()]
    }

    /// The alternative a trip took, if any.
    #[must_use]
    pub fn taken(&self, trip: TripId) -> Option<&Alternative> {
        self.chosen[trip.index()].map(|j| &self.sets[trip.index()][j as usize])
    }
}

/// Check the parameters, build the window and the context, and refuse the trips the loading
/// could not take.
fn context<'a>(
    network: &'a Network,
    demand: &'a Demand,
    timetable: Option<&'a Timetable>,
    window: (Second, Second),
    parameters: &'a Parameters,
) -> Result<Context<'a>, RunError> {
    if parameters.link_bin == 0 {
        return Err(RunError::LinkBinZero);
    }
    if parameters.iteration_cap == 0 {
        return Err(RunError::IterationCapZero);
    }
    if parameters.max_rides == 0 {
        return Err(RunError::MaxRidesZero);
    }
    if parameters.nearby_stops == 0 {
        return Err(RunError::NearbyStopsZero);
    }
    if parameters.coefficients.len() != demand.class_count() as usize {
        let (classes, rows) = (demand.class_count(), parameters.coefficients.len());
        return Err(RunError::CoefficientRows { classes, rows });
    }
    if timetable.is_some() {
        for (class, b) in parameters.coefficients.iter().enumerate() {
            if let Some(c) = (3..=9).find(|&c| !(b[c].is_finite() && b[c] >= 0.0)) {
                return Err(RunError::TransitWeight { class, column: COLUMNS[c] });
            }
        }
    }
    let (start, end) = window;
    let length = end.saturating_sub(start);
    let grid = StepGrid::new(start, parameters.link_bin, length).map_err(RunError::Window)?;
    let n = demand.trip_count() as usize;
    let mut departure = Vec::with_capacity(n);
    let mut class = Vec::with_capacity(n);
    for i in 0..n {
        let trip = TripId::from_index(i);
        let name = || demand.trip_ids().external(trip.raw()).to_string();
        if !grid.contains(demand.departure(trip)) {
            return Err(RunError::DepartureOutsideWindow(name()));
        }
        let since = f64::from(demand.departure(trip).saturating_sub(start));
        for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
            let Some(o) = demand.origin_end(trip, mode) else { continue };
            if since + (o.gap / network.walk_speed()).get() >= f64::from(length) {
                return Err(RunError::StartsAfterWindow(name(), mode));
            }
        }
        departure.push(since);
        class.push(demand.class(trip).index());
    }
    Ok(Context { network, demand, timetable, parameters, window: grid, departure, class })
}

/// The trips table of a run's last iteration.
fn trips(
    ctx: &Context<'_>,
    sets: &[Vec<Alternative>],
    chosen: &[Option<u32>],
    realised: &crate::load::Realised,
) -> Trips {
    let n = ctx.trip_count();
    let mut t = Trips {
        class: Vec::with_capacity(n),
        departure: Vec::with_capacity(n),
        mode: Vec::with_capacity(n),
        arrival: realised.arrival.clone(),
        travel_time: Vec::with_capacity(n),
        finished: realised.finished.clone(),
        cost: Vec::with_capacity(n),
        alternatives: Vec::with_capacity(n),
    };
    for i in 0..n {
        let taken = chosen[i].map(|j| &sets[i][j as usize]);
        t.class.push(ctx.class_id(i));
        t.departure.push(ctx.demand.departure(ctx.trip(i)));
        t.mode.push(taken.map(|a| a.mode));
        t.travel_time.push(realised.arrival[i] - ctx.departure[i]);
        t.cost.push(taken.map_or(f64::NAN, |a| ctx.parameters.cost(ctx.class[i], &a.attributes())));
        t.alternatives.push(u32::try_from(sets[i].len()).unwrap_or(u32::MAX));
    }
    t
}

/// Run the loop: iteration 0 on free-flow costs and the timetable, then the five stages per
/// iteration until the stopping rule (read once `iteration_floor` iterations have run) or the cap.
///
/// # Errors
///
/// [`RunError`]: a parameter out of range, a window that is not whole bins, a trip departing
/// outside the window or reaching the network at or after its end; the choice model's or the
/// loading's refusals.
///
/// # Panics
///
/// As the search's and the loading's readings do, for a network and a demand that do not
/// belong together.
pub fn run(
    network: &Network,
    demand: &Demand,
    timetable: Option<&Timetable>,
    window: (Second, Second),
    parameters: &Parameters,
) -> Result<Run, RunError> {
    let ctx = context(network, demand, timetable, window, parameters)?;
    let model = Model::new(parameters)?;
    let n = ctx.trip_count();
    let mut sets: Vec<Vec<Alternative>> = vec![Vec::new(); n];
    let mut chosen: Vec<Option<u32>> = vec![None; n];
    let mut loading = point_queue::load(network, &[], ctx.window)?;
    let mut times = LinkTimes::new(network, &loading.links);
    let mut iterations = Vec::new();
    let mut stopped_by_rule = false;
    let mut realised = None;
    for k in 0..parameters.iteration_cap {
        let t0 = Instant::now();
        let found = search(&ctx, &times, &mut sets, k);
        let t1 = Instant::now();
        let (switches, offered) = choose(&ctx, &model, &sets, &mut chosen, k)?;
        let t2 = Instant::now();
        loading = load(&ctx, &sets, &chosen)?;
        let t3 = Instant::now();
        times = LinkTimes::new(network, &loading.links);
        measure(&ctx, &times, &mut sets);
        let outcome = realise(&ctx, &loading, &sets, &chosen);
        let t4 = Instant::now();
        let mut row = assess(&ctx, &sets, &chosen, &outcome, k, found, (offered, switches));
        realised = Some(outcome);
        let t5 = Instant::now();
        row.stage_seconds = [t1 - t0, t2 - t1, t3 - t2, t4 - t3, t5 - t4].map(|d| d.as_secs_f64());
        iterations.push(row);
        if k + 1 >= parameters.iteration_floor && stops(&iterations) {
            stopped_by_rule = true;
            break;
        }
    }
    let realised = realised.expect("the cap is at least one iteration");
    let trips = trips(&ctx, &sets, &chosen, &realised);
    Ok(Run { trips, links: loading.links, iterations, stopped_by_rule, sets, chosen })
}
