"""Sphinx configuration of the openmobisim manual.

The site's workflow (in openmobisim/openmobisim.github.io) builds this folder from the code repository. The reference
pages generated from the code are committed with it, so a build needs neither the compiled core nor the starter cases:
``sphinx-build -b html manual _site`` from the repository's root, with ``manual/requirements.txt`` installed.
"""

from __future__ import annotations

import os
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
# The package's source, relative to the repository's root (Sphinx reads this file from inside manual/).
SOURCE = (HERE.parent / os.environ.get("OPENMOBISIM_SOURCE", "python/openmobisim")).resolve()

project = "openmobisim"
author = "Bahman Madadi"
copyright = "2026, Bahman Madadi"  # noqa: A001 (Sphinx's name)
release = os.environ.get("OPENMOBISIM_VERSION", "0.1.0")
version = release

extensions = [
    "myst_nb",
    "autoapi.extension",
    "sphinxcontrib.bibtex",
    "sphinx_copybutton",
    "sphinx.ext.mathjax",
    "sphinx.ext.napoleon",
    "sphinxcontrib.mermaid",
]

# A writer's quick build without the API pages (they are written into this folder while building, so two builds at
# once must not both make them): OPENMOBISIM_MANUAL_NO_API=1.
if os.environ.get("OPENMOBISIM_MANUAL_NO_API"):
    extensions.remove("autoapi.extension")

# Markdown (MyST) for every written page; notebooks rendered from their saved outputs, never executed here.
myst_enable_extensions = ["dollarmath", "amsmath", "colon_fence", "deflist", "fieldlist", "attrs_inline"]
myst_heading_anchors = 3
nb_execution_mode = "off"
source_suffix = {".rst": "restructuredtext", ".md": "myst-nb", ".ipynb": "myst-nb"}  # .rst first: autoapi writes its pages so
exclude_patterns = ["_build", "_static", "_*.md", "**/_*.md", "**/_examples/*", "_examples", "Thumbs.db",
                    ".DS_Store"]

# The API pages, read from the source and the compiled core's type stubs (no build of the core needed).
autoapi_dirs = [str(SOURCE)]
autoapi_root = "reference/api"
autoapi_file_patterns = ["*.pyi", "*.py"]
autoapi_ignore = ["*/__main__.py", "*/tests/*"]
autoapi_options = ["members", "undoc-members", "show-module-summary", "imported-members"]
autoapi_member_order = "groupwise"
autoapi_add_toctree_entry = False
autoapi_keep_files = False
autodoc_typehints = "description"
suppress_warnings = ["autoapi.python_import_resolution"]  # private constants of the figures' style

# Diagrams: Mermaid, drawn in the reader's browser.
mermaid_version = "11.4.1"

# Citations: one bibliography for the whole manual.
bibtex_bibfiles = ["references.bib"]
bibtex_default_style = "plain"
bibtex_reference_style = "author_year"

html_theme = "pydata_sphinx_theme"
html_title = f"openmobisim {release}"
# Published at the release on its own site, built from the code repository by the workflow of the site's repository,
# openmobisim/openmobisim.github.io.
html_baseurl = "https://openmobisim.github.io/"
html_static_path = ["_static"]
html_css_files = ["custom.css"]  # the headings' sizes, the page's width
html_js_files = ["custom.js"]  # a figure opens at full size when clicked
html_logo = "_static/openmobisim_mark_paper.svg"
html_theme_options = {
    "logo": {"text": "openmobisim", "image_dark": "_static/openmobisim_mark_night.svg"},
    "github_url": "https://github.com/bahmanmdd/openmobisim",
    "icon_links": [{"name": "Main website", "url": "https://bahmanmadadi.com", "icon": "fa-solid fa-house",
                    "type": "fontawesome"}],
    "navbar_center": [],  # no tabs at the top: the whole manual is in the left sidebar
    "navbar_align": "left",
    "show_toc_level": 2,
    "navigation_with_keys": False,
}
# The whole manual in the left sidebar, the four parts each folding open to their pages; the page's own contents on
# the right.
templates_path = ["_templates"]
html_sidebars = {"**": ["sidebar-global"]}

# Internal labels in the code's comments mean nothing to a reader of the manual: drop them from the
# docstrings shown in the API pages. What cannot be dropped cleanly is reported by the coverage check.
_LABEL = re.compile(r"\s*\((?:S\d{2,3}|I-[a-z]{2}|(?:design|Foundations|Design) §)[^()]*\)")
_BARE = re.compile(r"\bS\d{2,3}\b(?:[,;:]\s*)?")


_LEADING = re.compile(r"^(\s*)\((?:S\d{2,3}|I-[a-z]{2}|(?:design|Foundations|Design) §)[^()]*\)\.?\s*")


def _public(app, what, name, obj, options, lines):  # noqa: ARG001 (Sphinx's signature)
    for i, line in enumerate(lines):
        line = _LEADING.sub(r"\1", line)  # a label opening a line: keep the line's indentation
        line = _LABEL.sub("", line)
        lines[i] = _BARE.sub("", line) if re.search(r"\bS\d{2,3}\b", line) else line


# One page per namespace a user types: ``openmobisim`` (every public name, re-exported from its module), and the
# submodules used by name (``viz``, ``choice``, ``examples``). The modules whose names are all re-exported at the top
# would only repeat them.
_REPEATED = {
    f"openmobisim.{m}"
    for m in ("audit", "demand", "gmns", "network", "options", "parameters", "parking", "run", "scenario", "table", "transit")
}


def _skip(app, what, name, obj, skip, options):  # noqa: ARG001 (Sphinx's signature)
    if what == "module" and name in _REPEATED:
        return True
    return skip


def setup(app):
    app.connect("autodoc-process-docstring", _public)
    if "autoapi.extension" in extensions:
        app.connect("autoapi-skip-member", _skip)
