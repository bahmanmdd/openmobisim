"""Plug-ins contributed to openmobisim: extensions written in Python, outside the core.

A contributed choice model lives in a module here and registers itself by name on import
(``openmobisim.choice_model_register``), so that
``Scenario.from_parts(..., choice_model="its_name")`` runs it like a built-in one. None ships
with 0.1.

``openmobisim.contrib.policies`` holds reference operator policies, one of each of Powell's
four classes for docked bikes and for taxis, with ``evaluate`` and ``tune``; the ones that
solve programs need SciPy (``pip install "openmobisim[policies]"``).
"""
