"""openmobisim — an agent-based multimodal mobility simulator.

The public Python surface. Most of it is a thin re-export of the compiled
core (``openmobisim._core``), which is private: its shape is free to change behind
this module, whereas this module is a semver contract. ``Scenario``, ``Run``
and ``Table`` (``openmobisim.scenario``) and the fixture helpers in
``openmobisim.examples`` are where the core becomes ergonomic — Foundations §10's
"behaviour in Python, physics in Rust".

Phase 1's scenario surface is intentionally narrow: a network, a trips
table, one run, four output artifacts. No hubs, no equilibration, no
disruptions, no route choice — those arrive with the mechanisms that give
them meaning.
"""

from openmobisim import _core, examples
from openmobisim._core import build_info, choice_draw, fixed_order_sum_f64, step_of
from openmobisim.scenario import Run, Scenario, Table

__version__: str = _core.__version__

__all__ = [
    "Run",
    "Scenario",
    "Table",
    "__version__",
    "build_info",
    "choice_draw",
    "examples",
    "fixed_order_sum_f64",
    "step_of",
]
