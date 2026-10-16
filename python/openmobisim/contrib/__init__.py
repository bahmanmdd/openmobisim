"""Plug-ins contributed to openmobisim: extensions written in Python, outside the core.

A contributed choice model lives in a module here and registers itself by name on import
(``openmobisim.choice_model_register``), so that
``Scenario.from_parts(..., choice_model="its_name")`` runs it like a built-in one. None ships
with 0.1.
"""
