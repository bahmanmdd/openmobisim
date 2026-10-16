"""The package's messages through ``logging`` (C2, P-7): the ``openmobisim`` logger.

A run says one line at INFO when it ends (its stage-by-stage timings at DEBUG), a page or figure
where it was written, the reference check each case's row. Without any logging set up by the user
these lines go to standard output, so a notebook or a terminal shows them as before; once the user
configures logging (a handler on the root logger), they go there instead, never twice. Silence
them with ``quiet=True`` where a function takes it, or
``logging.getLogger("openmobisim").setLevel(logging.WARNING)``; see the timings with ``DEBUG``.
"""

from __future__ import annotations

import logging
import sys

logger = logging.getLogger("openmobisim")


class _Fallback(logging.Handler):
    """Standard output, only while the user has set up no logging of their own."""

    def emit(self, record: logging.LogRecord) -> None:
        if logging.getLogger().handlers:  # the user's handlers take the record (propagation)
            return
        try:
            print(self.format(record), file=sys.stdout, flush=True)
        except Exception:  # noqa: BLE001 (logging's rule: a handler never raises)
            self.handleError(record)


if not any(isinstance(h, _Fallback) for h in logger.handlers):
    _handler = _Fallback()
    _handler.setFormatter(logging.Formatter("%(message)s"))
    logger.addHandler(_handler)
if logger.level == logging.NOTSET:
    logger.setLevel(logging.INFO)
