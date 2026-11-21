//! Stage 2, choose: the choice model over each traveller's set; who is offered a new choice is
//! decided by the step of the method of successive averages; under the least-cost choice that
//! step is raised to the share of its cost a traveller would save on its best alternative of the
//! same mode when that share is above a band, under the logit it is not.

use openmobisim_choice::cost::{ChoiceError, Coefficients, Row, Traveller};
use openmobisim_choice::model::{Choice, Deterministic, Logit};
use openmobisim_types::ids::{EntityId, UserClassId};
use openmobisim_types::rng::{DrawAddress, RngKey, Stream, StreamRng};
use rayon::prelude::*;

use crate::alternative::Alternative;
use crate::context::Context;
use crate::parameters::{COLUMNS, ChoiceModel, Parameters};

/// The run's choice model, built from the parameters.
pub(crate) enum Model {
    /// The least cost.
    Deterministic(Deterministic),
    /// The logit family.
    Logit(Logit),
}

impl Model {
    /// Build the parameters' model.
    ///
    /// # Errors
    ///
    /// As `Coefficients::new` and `Logit::new`.
    pub fn new(parameters: &Parameters) -> Result<Self, ChoiceError> {
        let by_class: Vec<(UserClassId, Vec<f64>)> = parameters
            .coefficients
            .iter()
            .enumerate()
            .map(|(c, b)| (UserClassId::from_index(c), b.to_vec()))
            .collect();
        let classes = u32::try_from(by_class.len()).unwrap_or(u32::MAX);
        let coefficients = Coefficients::new(&COLUMNS, classes, &by_class)?;
        Ok(match parameters.choice {
            ChoiceModel::Deterministic => Self::Deterministic(Deterministic::new(coefficients)),
            ChoiceModel::Logit => Self::Logit(Logit::new(
                coefficients,
                parameters.nest_scale,
                RngKey::new(parameters.seed),
            )?),
        })
    }

    fn choose(&self, traveller: &Traveller, rows: &[Row<'_>]) -> Result<usize, ChoiceError> {
        match self {
            Self::Deterministic(m) => m.choose(traveller, rows),
            Self::Logit(m) => m.choose(traveller, rows),
        }
    }
}

/// The choice of trip `i` over `set`: the model's row. The same at any moment for the same costs
/// and set (under the logit the traveller's draws are fixed for the run), so the assess stage
/// reads it too.
pub(crate) fn choice(
    ctx: &Context<'_>,
    model: &Model,
    i: usize,
    set: &[Alternative],
) -> Result<u32, ChoiceError> {
    let attributes: Vec<[f64; crate::parameters::N]> =
        set.iter().map(Alternative::attributes).collect();
    let rows: Vec<Row<'_>> = set
        .iter()
        .zip(&attributes)
        .map(|(a, x)| Row { attributes: x, mode: a.mode, nest: u32::from(a.mode.as_u8()) })
        .collect();
    let traveller = Traveller { trip: ctx.trip(i), class: ctx.class_id(i) };
    let j = model.choose(&traveller, &rows)?;
    Ok(u32::try_from(j).unwrap_or(u32::MAX))
}

/// The band of indifference of the least-cost choice: a traveller whose best alternative of the
/// same mode would save a share of the cost of the one it holds at or below this is offered a
/// new choice only with the plain step of the method of successive averages (Mahmassani and
/// Chang 1987). A fixed constant of the rule, not a parameter of a run.
const BAND: f64 = 0.2;

/// The chance that trip `i`, holding the alternative at `held` of `set`, is offered a new choice
/// at `iteration`.
///
/// Under the logit: `1 / (iteration + 1)`, the step of the plain method of successive averages,
/// for every traveller. The logit's shares, computed from the costs of the alternatives, put
/// some travellers on an alternative that is not the lowest-cost one of its mode. When the costs
/// change, travellers have to move in both directions: toward the lower-cost alternative and
/// toward the higher-cost one. A chance that rose with the cost a traveller would save by moving
/// to the lowest-cost alternative of its mode would rise only for the holders of a higher-cost
/// alternative. Moves toward the lower cost would then be asked for often and moves toward the
/// higher cost rarely, and the higher-cost alternatives would hold too few travellers by an
/// amount that falls only as `1 / k`. So under the logit every traveller is asked at the same
/// step.
///
/// Under the least-cost choice: `max(1 / (iteration + 1), s)`. The second term is after Smith's
/// proportional switching (1984), in its spirit rather than his process: `s = r` when
/// `r > BAND`, else 0, with `r = (c - b) / c`, `c` the cost of the alternative it holds and `b`
/// the least cost among the alternatives of its set of the same mode as the one it holds, as the
/// choice that follows reads them (systematic costs at the times of the last measure). `s` is 0
/// when `c` is not positive, and never above 1 (`r` passes 1 only when `b` is negative). No
/// other traveller's costs enter. Only the mode held enters `b`, but an offered traveller
/// chooses over its whole set, its mode included, so one above the band reconsiders its mode at
/// the rate `r` too. Under the least-cost choice every traveller belongs on the lowest-cost
/// alternative of its set, so the raised chance speeds only moves that every traveller has to
/// make. `r` is the same in any unit of cost but shrinks when a constant is added to every cost.
fn chance(ctx: &Context<'_>, i: usize, set: &[Alternative], held: u32, iteration: u32) -> f64 {
    let plain = 1.0 / f64::from(iteration + 1);
    if ctx.parameters.choice == ChoiceModel::Logit {
        return plain;
    }
    let cost = |a: &Alternative| ctx.parameters.cost(ctx.class[i], &a.attributes());
    let taken = &set[held as usize];
    let c = cost(taken);
    let b = set.iter().filter(|a| a.mode == taken.mode).map(cost).fold(f64::INFINITY, f64::min);
    let r = if c > 0.0 { (c - b) / c } else { 0.0 };
    let s = if r > BAND { r.min(1.0) } else { 0.0 };
    plain.max(s)
}

/// A trip's outcome of the choose stage: the alternative it holds after it, whether it was
/// offered a new choice, whether it changed alternative.
type Outcome = (Option<u32>, bool, bool);

/// The choose stage: a traveller holding nothing (everyone at iteration 0) chooses. After it a
/// traveller is offered a new choice with the probability of [`chance`], drawn from
/// [`Stream::MsaReselection`] at (trip, iteration); one that is offered takes the model's
/// choice over its whole set, its mode included, whatever it held; the others keep their
/// alternative. Returns how many changed alternative and how many were offered a new choice
/// (a traveller holding nothing counts as offered: everyone at iteration 0).
///
/// # Errors
///
/// As the model's choice (a cost that is not finite).
pub(crate) fn choose(
    ctx: &Context<'_>,
    model: &Model,
    sets: &[Vec<Alternative>],
    chosen: &mut [Option<u32>],
    iteration: u32,
) -> Result<(u64, u64), ChoiceError> {
    let msa = StreamRng::new(RngKey::new(ctx.parameters.seed), Stream::MsaReselection);
    let outcome: Vec<Result<Outcome, ChoiceError>> = (0..sets.len())
        .into_par_iter()
        .map(|i| {
            let (set, before) = (&sets[i], chosen[i]);
            if set.is_empty() {
                return Ok((None, false, false));
            }
            let again = match before {
                None => true,
                Some(held) => {
                    let p = chance(ctx, i, set, held, iteration);
                    let address = DrawAddress::from_pair(ctx.trip(i).raw(), iteration);
                    msa.block(address).bernoulli(0, p)
                }
            };
            if !again {
                return Ok((before, false, false));
            }
            let now = choice(ctx, model, i, set)?;
            Ok((Some(now), true, before.is_some_and(|b| b != now)))
        })
        .collect();
    let (mut switches, mut offered) = (0, 0);
    for (slot, result) in chosen.iter_mut().zip(outcome) {
        let (now, again, switched) = result?;
        *slot = now;
        offered += u64::from(again);
        switches += u64::from(switched);
    }
    Ok((switches, offered))
}
