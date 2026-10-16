"""Where a figure or page was written, as a link a reader can click."""

from __future__ import annotations

import sys
from pathlib import Path

from openmobisim._log import logger


def _in_notebook() -> bool:
    """Whether this runs in a Jupyter kernel (JupyterLab, VS Code, PyCharm, Colab)."""
    ipython = sys.modules.get("IPython")
    shell = ipython.get_ipython() if ipython is not None else None
    return getattr(shell, "kernel", None) is not None


def show_path(what: str, path: str | Path) -> None:
    """Say where a file was written, as a link a reader can click.

    In a terminal, the ``file://`` address (which terminals and editors open on a click). In a
    notebook, also a link relative to the notebook's folder, which JupyterLab and VS Code open in a
    tab, when the file is under that folder.
    """
    path = Path(path).resolve()
    if _in_notebook():
        from IPython.display import Markdown, display

        try:
            relative = path.relative_to(Path.cwd()).as_posix()
        except ValueError:
            relative = None
        link = f"[{relative}]({relative})" if relative else f"`{path}`"
        display(Markdown(f"**{what}:** {link} — {path.as_uri()}"))
    else:
        logger.info(f"{what}: {path.as_uri()}")
