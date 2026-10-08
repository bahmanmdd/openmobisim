"""The words a figure's footer says about the run it draws (S168).

No dependencies, so the interactive map and the static figures share it.
"""

from __future__ import annotations

from typing import Any

__all__ = ["network_source", "run_identity"]

#: How many hex digits of the fingerprint a footer shows; the whole 16 are in
#: ``Run.fingerprint``, the manifest and every results file.
_SHOWN = 8


def run_identity(run: Any) -> str:
    """``choice logit · msa 6 it, gap 4.2% · seed 0 · fingerprint 29ff0a1c``, for a footer."""
    parts = [f"choice {run.choice_model}"]
    if run.equilibration != "free_flow":
        text = f"{run.equilibration} {len(run.convergence()['iteration'])} it"
        gap = run.convergence_gap
        if gap == gap:  # not nan
            text += f", gap {100 * gap:.1f}%"
        parts.append(text)
    parts += [f"seed {run.master_seed}", f"fingerprint {run.fingerprint[:_SHOWN]}"]
    return " · ".join(parts)


def network_source(network: Any) -> str:
    """Where the drawn network came from, for a footer.

    The OpenStreetMap credit its licence asks for, or ``table network``, ``synthetic network``
    and so on for a network that is not OpenStreetMap's (X-59).
    """
    if network.source == "osm":
        return "© OpenStreetMap contributors (ODbL)"
    return f"{network.source} network"
