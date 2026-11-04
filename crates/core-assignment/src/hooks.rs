//! Stage hooks: code that runs at fixed points of the loop, absent unless given.
//!
//! A [`StageHook`] is called by the loop at four points: once before the first iteration, and
//! in every iteration just before the loading, just after it, and just after the measure
//! stage. Every method does nothing by default, and [`NoHooks`] is the hook of a run that asks
//! for none. A feature that needs to act at one of these points (a recorder of every
//! iteration's state, a disruption applied before the loading) is a hook; the loop never
//! reads an option to decide whether to run it.

use openmobisim_core_loading::Vehicle;

use crate::loader::Loaded;
use crate::state::State;

/// Code run at fixed points of the loop. See the [module docs](self).
pub trait StageHook {
    /// Before the first iteration, with the state the loop starts from.
    fn before_loop(&mut self, _state: &State) {}

    /// In `iteration`, after the choose stage, with the state and the vehicles about to be
    /// loaded.
    fn before_load(&mut self, _iteration: u32, _state: &State, _vehicles: &[Vehicle]) {}

    /// In `iteration`, with what the loading returned.
    fn after_load(&mut self, _iteration: u32, _loaded: &Loaded) {}

    /// In `iteration`, with the state after the measure stage: the loaded link times in place
    /// and every chain of every set priced on them.
    fn after_measure(&mut self, _iteration: u32, _state: &State) {}
}

/// The hook of a run that asks for none: every stage hook does nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHooks;

impl StageHook for NoHooks {}
