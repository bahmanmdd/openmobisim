"""Built networks cached on disk, so that a network is imported only the first time it is opened.

``Case.network()`` reads a starter case's network from a file under
``~/.cache/openmobisim/networks/<case>/`` (``XDG_CACHE_HOME`` respected) when this build wrote it
from the same files and reading options, and otherwise imports the network and writes the file:
the same network, bit for bit (the same fingerprint, the same results), in a fraction of the
import's time. ``OPENMOBISIM_NO_NETWORK_CACHE=1`` imports every time and writes nothing.

A file's name holds a checksum of what the network is made from (the case's network files and
reading options), the defaults table's version, the file format's and a stamp of the code (the
compiled core's commit, size and modification time, and this package's Python files, since a link
table is read in Python): another build writes a file of its own, so two installations used side
by side do not replace each other's. The compiled core checks the file's header again when it reads
it. The three files of a case used last are kept, the others removed.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
from collections.abc import Callable
from functools import cache
from pathlib import Path
from typing import Any

from openmobisim import _core
from openmobisim._log import logger

#: The environment variable that turns the cache off: any value but ``""`` and ``"0"``.
NO_CACHE_VARIABLE = "OPENMOBISIM_NO_NETWORK_CACHE"
#: How many files of one case are kept: the ones used last.
KEEP = 3


def _off() -> bool:
    return os.environ.get(NO_CACHE_VARIABLE, "") not in ("", "0")


def _folder() -> Path:
    base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "openmobisim" / "networks"


@cache
def _code_stamp() -> str:
    """The code that makes a network: the compiled core and this package's Python files."""
    files = [Path(_core.__file__), *sorted(Path(__file__).parent.glob("*.py"))]
    rows = [(p.name, p.stat().st_size, p.stat().st_mtime_ns) for p in files]
    described = repr((_core.build_info()["git_sha"], rows))
    return hashlib.sha256(described.encode()).hexdigest()[:12]


def network_cached(
    name: str, made_from: dict[str, Any], make: Callable[[], _core.Network]
) -> _core.Network:
    """The network ``make()`` builds from ``made_from``, read from the cache if it holds it.

    ``made_from`` says everything the network is made from (the files' checksums, the reading
    options), as JSON; ``name`` is the cache's folder for it (a case's name).
    """
    if _off():
        return make()
    key = hashlib.sha256(json.dumps(made_from, sort_keys=True).encode()).hexdigest()
    code = _code_stamp()
    source = f"{key}/{code}"
    info = _core.build_info()
    versions = f"{info['defaults_version']}-{info['network_format_version']}"
    folder = _folder() / name
    path = folder / f"{key[:16]}-{versions}-{code[:8]}.network"
    if path.exists():
        try:
            header = _core.network_cache_header(str(path))
            if header["current"] and header["source"] == source:
                network = _core.network_read_cache(str(path), source=source)
                os.utime(path)  # used now: kept before the others
                made = header["made_s"]
                was = "" if math.isnan(made) else f" (its import took {made:.2f} s)"
                logger.info(
                    f"{name}: network read from the cache in {network.read_s:.2f} s{was}: {path}"
                )
                return network
        except (OSError, ValueError) as e:  # damaged, or unreadable: import it again
            logger.debug(f"{name}: the cached network is not used ({e})")
    network = make()
    try:
        folder.mkdir(parents=True, exist_ok=True)
        _core.network_write_cache(network, str(path), source=source)
    except (OSError, ValueError) as e:
        logger.debug(f"{name}: the network was not cached ({e})")
        return network
    try:
        files = sorted(folder.glob("*.network"), key=lambda p: p.stat().st_mtime, reverse=True)
        for old in files[KEEP:]:
            old.unlink(missing_ok=True)
    except OSError:  # another process removed one first
        pass
    took = f" in {network.read_s:.2f} s" if network.read_s is not None else ""
    logger.info(f"{name}: network imported{took}, cached for the next time: {path}")
    return network
