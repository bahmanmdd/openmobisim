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
    """``seed 0 · fingerprint 29ff0a1c``: what identifies the run in a footer."""
    return f"seed {run.master_seed} · fingerprint {run.fingerprint[:_SHOWN]}"
