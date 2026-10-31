//! A run's operator (`Scenario.run(policy=...)`, S304): `operator_tasks.parquet`, the decisions
//! it took in the run's service day (the last loading's service trips, played once after the
//! run has stopped), written only by a run with an operator. Seconds count from midnight.
//!
//! **`operator_tasks.parquet`** — one row per decision, in the order taken, accepted or refused
//! (a move the built-in solver made for the step's targets included):
//! `run_id`; `step` (the policy step's index, 0 at the window's start), `t_s` (its second),
//! `service`, `kind` (`move`: bikes taken by van from one dock to another; `target`: the bikes a
//! dock should hold, which the built-in solver turns into moves; `relocate`: an idle taxi sent
//! empty to a road node; `charge`: an idle taxi sent to a charger); the parameters by id
//! (`from_dock`, `to_dock` and `n`, the bikes asked for, of a move; `dock` and `level`, the bikes
//! it should hold, of a target; `vehicle` of a relocation or a charge; `node` of a relocation;
//! `charger` and `to_soc`, the share of the battery to charge to, of a charge; null where a kind
//! has none); `accepted`, `reason` (why it was refused; null if accepted), `executed_s` (the
//! second it started), `finished_s` (the van's or vehicle's arrival, the charge's end; null if
//! refused or not finished by the window's end), `moved` (a move's bikes in the van, 0 for the
//! other kinds), `metres` (the drive's; null if refused or not a drive) and `from_target` (a
//! move the solver made for the step's targets). The same columns, after `run_id`, as
//! `Run.operator_decisions()` (where a null number is `NaN`).
//!
//! The file's key-value metadata carries `openmobisim.run_fingerprint` and
//! `openmobisim.master_seed`. **Cost.** About 150 bytes per row while writing.

use std::path::Path;
use std::sync::Arc;

use arrow_array::BooleanArray;
use arrow_schema::DataType;
use openmobisim_core_sim::RunDescription;
use openmobisim_core_sim::services::decisions::OperatorTaskTable;

use crate::WriteError;
use crate::bike_share::Columns;

/// A number column's values with `NaN` (a value the row does not have) as null.
fn present(v: &[f64]) -> Vec<Option<f64>> {
    v.iter().map(|&x| (!x.is_nan()).then_some(x)).collect()
}

/// Write `operator_tasks.parquet` (see the [module docs](self)).
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_operator_tasks(
    path: impl AsRef<Path>,
    run_id: &str,
    t: &OperatorTaskTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let flag = |v: &[bool]| Arc::new(BooleanArray::from(v.to_vec()));
    let mut c = Columns::default();
    c.text("run_id", vec![run_id; t.len()])
        .count("step", &t.step)
        .number("t_s", &t.t_s)
        .text("service", &t.service)
        .text("kind", &t.kind)
        .maybe_text("from_dock", &t.from_dock)
        .maybe_text("to_dock", &t.to_dock)
        .maybe_number("n", &present(&t.n))
        .maybe_text("dock", &t.dock)
        .maybe_number("level", &present(&t.level))
        .maybe_text("vehicle", &t.vehicle)
        .maybe_text("node", &t.node)
        .maybe_text("charger", &t.charger)
        .maybe_number("to_soc", &present(&t.to_soc))
        .push("accepted", DataType::Boolean, false, flag(&t.accepted))
        .maybe_text("reason", &t.reason)
        .maybe_number("executed_s", &present(&t.executed_s))
        .maybe_number("finished_s", &present(&t.finished_s))
        .count("moved", &t.moved)
        .maybe_number("metres", &present(&t.metres))
        .push("from_target", DataType::Boolean, false, flag(&t.from_target));
    c.write(path.as_ref(), description, &[])
}
