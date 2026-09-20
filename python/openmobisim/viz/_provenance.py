"""The words a figure's footer says about the run it draws (S168).

No dependencies, so the interactive map and the static figures share it.
"""

from __future__ import annotations

from typing import Any

__all__ = ["run_identity"]

#: How many hex digits of the fingerprint a footer shows; the whole 16 are in
#: ``Run.fingerprint``, the manifest and every results file.
_SHOWN = 8


def run_identity(run: Any) -> str:
    """``choice logit · msa 6 it · seed 0 · fingerprint 29ff0a1c``: the run, in a footer."""
    parts = [f"choice {run.choice_model}"]
    if run.equilibration != "none":
        iterations = len(run.convergence()["iteration"])
        parts.append(f"{run.equilibration} {iterations} it")
    parts += [f"seed {run.master_seed}", f"fingerprint {run.fingerprint[:_SHOWN]}"]
    return " · ".join(parts)
