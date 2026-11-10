//! Stage 2, choose: the choice model over each traveller's set; the method of successive
//! averages decides who re-chooses.

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

/// The choice of trip `i` over `set`: the model's row.
fn choice(
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

/// The choose stage: at iteration 0 every traveller chooses; after it a traveller re-chooses
/// with probability `1 / (iteration + 1)`, drawn from [`Stream::MsaReselection`] at (trip,
/// iteration), and the others keep their alternative. Returns how many changed alternative.
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
) -> Result<u64, ChoiceError> {
    let msa = StreamRng::new(RngKey::new(ctx.parameters.seed), Stream::MsaReselection);
    let p = 1.0 / f64::from(iteration + 1);
    let outcome: Vec<Result<(Option<u32>, bool), ChoiceError>> = (0..sets.len())
        .into_par_iter()
        .map(|i| {
            let (set, before) = (&sets[i], chosen[i]);
            if set.is_empty() {
                return Ok((None, false));
            }
            let again = iteration == 0
                || before.is_none()
                || msa.block(DrawAddress::from_pair(ctx.trip(i).raw(), iteration)).bernoulli(0, p);
            if !again {
                return Ok((before, false));
            }
            let now = choice(ctx, model, i, set)?;
            Ok((Some(now), before.is_some_and(|b| b != now)))
        })
        .collect();
    let mut switches = 0;
    for (slot, result) in chosen.iter_mut().zip(outcome) {
        let (now, switched) = result?;
        *slot = now;
        switches += u64::from(switched);
    }
    Ok(switches)
}
