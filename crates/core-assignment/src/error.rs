//! What can stop an assignment before or while it runs.

use core::fmt;

use openmobisim_core_choice::ChoiceError;

/// Why an assignment could not run.
#[derive(Clone, Debug, PartialEq)]
pub enum AssignError {
    /// The inputs do not fit together (a coefficient missing for a class, a money vector of the
    /// wrong length, a state made for other demand).
    Input(String),
    /// A user class's coefficients cannot price a route: the car planner needs a positive cost
    /// per minute in a vehicle, and no link may cost less than nothing.
    Coefficients {
        /// The class, by its index among the demand's user classes.
        class: u32,
        /// What is wrong.
        reason: String,
    },
    /// The choice model failed.
    Choice(ChoiceError),
}

impl fmt::Display for AssignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(m) => write!(f, "the assignment's inputs do not fit: {m}"),
            Self::Coefficients { class, reason } => {
                write!(f, "user class {class}'s coefficients cannot price a route: {reason}")
            }
            Self::Choice(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AssignError {}

impl From<ChoiceError> for AssignError {
    fn from(e: ChoiceError) -> Self {
        Self::Choice(e)
    }
}
