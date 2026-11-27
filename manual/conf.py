"""Sphinx configuration of the openmobisim manual.

The site's workflow (in openmobisim/openmobisim.github.io) builds this folder from the code repository:
``sphinx-build -b html manual _site`` from the repository's root, with ``manual/requirements.txt`` installed. The
tutorials are notebooks shown from their saved outputs, so a build needs neither the compiled core nor the cases.
"""

import os

project = "openmobisim"
author = "Bahman Madadi"
copyright = "2026, Bahman Madadi"  # noqa: A001 (Sphinx's name)
release = os.environ.get("OPENMOBISIM_VERSION", "0.2.0")
version = release

extensions = ["myst_nb", "sphinx_copybutton", "sphinx.ext.mathjax"]
myst_enable_extensions = ["dollarmath", "amsmath", "colon_fence", "deflist"]
myst_heading_anchors = 3
nb_execution_mode = "off"  # the notebooks are run by their writer, never here
exclude_patterns = ["_build", "_site", "_static", "Thumbs.db", ".DS_Store"]

html_theme = "pydata_sphinx_theme"
html_title = f"openmobisim {release}"
html_baseurl = "https://openmobisim.github.io/"
html_static_path = ["_static"]
html_show_sourcelink = False  # the pages' Markdown sources are in the repository, not on the site
html_copy_source = False
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
# The whole manual in the left sidebar, the four parts each folding open to their pages.
templates_path = ["_templates"]
html_sidebars = {"**": ["sidebar-global"]}
