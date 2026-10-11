"""The first steps (S264): the tutorial notebooks, the welcome, and links to what a run wrote.

Every way in leads to the first notebook, ``00_getting_started.ipynb``, which lists the others:

* ``openmobisim`` in a terminal, with no command, prints the welcome: what to run first.
* ``openmobisim notebooks`` (or ``openmobisim.notebooks()`` in Python) copies the tutorials into a
  folder and opens the first in JupyterLab (in Python: links it).
* ``import openmobisim`` in an interactive session (a console, Jupyter) says, in one line, how to
  get them, until they have been fetched once; ``OPENMOBISIM_QUIET=1`` silences it. A script never
  prints it.

The tutorials travel inside the package without their outputs (small, and the same version as the
code); the repository holds them with their outputs, to read before installing. A build without
them (one made from the repository before a release) points to the repository instead.
``OPENMOBISIM_NOTEBOOKS`` names another folder to copy them from.
"""

from __future__ import annotations

import json
import os
import shutil
import sys
from pathlib import Path

#: The code repository: the notebooks with their outputs, and the place to report a problem.
REPOSITORY = "https://github.com/bahmanmdd/openmobisim"
NOTEBOOKS_URL = f"{REPOSITORY}/tree/main/notebooks"
ISSUES_URL = f"{REPOSITORY}/issues"
#: The first notebook: an overview of the others, and a first run.
FIRST = "00_getting_started.ipynb"
#: Where ``notebooks()`` puts them unless told otherwise, in the current folder.
FOLDER = "openmobisim-notebooks"

_TUTORIALS = Path(__file__).with_name("tutorials")


def _source() -> Path | None:
    """The folder the tutorials are copied from, if this build has them."""
    given = os.environ.get("OPENMOBISIM_NOTEBOOKS")
    for folder in ([Path(given)] if given else []) + [_TUTORIALS]:
        if (folder / FIRST).is_file():
            return folder
    return None


def _marker() -> Path:
    base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "openmobisim" / "tutorials.json"


def _in_notebook() -> bool:
    """Whether this runs in a Jupyter kernel (JupyterLab, VS Code, PyCharm, Colab)."""
    ipython = sys.modules.get("IPython")
    shell = ipython.get_ipython() if ipython is not None else None
    return getattr(shell, "kernel", None) is not None


def _interactive() -> bool:
    """A console or a notebook, not a script."""
    if hasattr(sys, "ps1") or sys.flags.interactive:
        return True
    ipython = sys.modules.get("IPython")
    return ipython is not None and ipython.get_ipython() is not None


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
        print(f"{what}: {path.as_uri()}")


def notebooks(folder: str | Path = FOLDER, *, overwrite: bool = False) -> Path:
    """Copy the tutorial notebooks into ``folder`` and say where the first one is.

    Ten notebooks, ``00_getting_started.ipynb`` to ``09_applications.ipynb``, without their outputs
    (run them to see them; the repository has them run). A notebook already in ``folder`` is kept,
    with your changes, unless ``overwrite``. Open the first in JupyterLab (``jupyter lab`` in that
    folder, or ``openmobisim notebooks`` in a terminal does it) or in your editor.

    Args:
        folder: Where to put them, ``openmobisim-notebooks`` in the current folder by default.
        overwrite: Replace notebooks already there.

    Returns:
        The folder.

    Raises:
        FileNotFoundError: If this build carries no notebooks (one built from the repository
            between releases); they are in the repository, ``NOTEBOOKS_URL``.
    """
    source = _source()
    if source is None:
        raise FileNotFoundError(
            f"this build of openmobisim carries no tutorial notebooks; read or download them at "
            f"{NOTEBOOKS_URL}"
        )
    target = Path(folder).resolve()
    target.mkdir(parents=True, exist_ok=True)
    copied = kept = 0
    for notebook in sorted(source.glob("[0-9][0-9]_*.ipynb")):
        destination = target / notebook.name
        if destination.exists() and not overwrite:
            kept += 1
            continue
        shutil.copyfile(notebook, destination)
        copied += 1
    try:  # remembered, so the pointer on import stops; nothing depends on it
        _marker().parent.mkdir(parents=True, exist_ok=True)
        _marker().write_text(json.dumps({"folder": str(target)}))
    except OSError:
        pass
    note = f" ({kept} already there, kept)" if kept else ""
    print(f"{copied} tutorial notebooks copied to {target}{note}.")
    show_path("Start with", target / FIRST)
    return target


def welcome() -> str:
    """The text ``openmobisim`` prints with no command: what it is and what to run first."""
    from openmobisim import __version__

    has = _source() is not None
    first = (
        "  openmobisim notebooks       the ten tutorials in ./openmobisim-notebooks, opened in\n"
        "                              JupyterLab at 00_getting_started: it lists the others\n"
        if has
        else f"  the tutorial notebooks      {NOTEBOOKS_URL}\n"
        "                              (start with 00_getting_started: it lists the others)\n"
    )
    return (
        f"openmobisim {__version__} — a multimodal mobility simulator: cars, bicycles, walking,\n"
        "public transport, park-and-ride and bike-and-ride on one network.\n"
        "An alpha, for testing by students and colleagues: the tests are not complete. Use it,\n"
        f"and report what goes wrong: {ISSUES_URL}\n"
        "\n"
        "Start here\n"
        f"{first}"
        "\n"
        "Commands\n"
        "  openmobisim notebooks [folder]  copy the tutorials (--no-open: no JupyterLab)\n"
        "  openmobisim cases               the starter cases: Amsterdam, Paris, Lyon,\n"
        "                                  Sioux Falls, Nguyen–Dupuis\n"
        "  openmobisim run amsterdam       run a case; write its summary page and map\n"
        "  openmobisim check               run every case, compare with the references\n"
        "\n"
        f"More: {REPOSITORY}\n"
    )


def pointer_on_import() -> None:
    """One line on import in a console or notebook, until the tutorials have been fetched.

    Never in a script, never with ``OPENMOBISIM_QUIET`` set, never from a build without the
    tutorials. On stdout, so a notebook shows it as ordinary output.
    """
    if os.environ.get("OPENMOBISIM_QUIET") or not _interactive() or _source() is None:
        return
    if _marker().exists():
        return
    from openmobisim import __version__

    print(
        f"openmobisim {__version__}: new here? openmobisim.notebooks() copies the tutorials into a "
        f"folder and links the first (OPENMOBISIM_QUIET=1 hides this line)."
    )
