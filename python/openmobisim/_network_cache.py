"""Built networks and parkings cached on disk, so that a case's files are read only the first time.

``Case.network()`` reads a starter case's network from a file under
``~/.cache/openmobisim/networks/<case>/`` (``XDG_CACHE_HOME`` respected) when this build wrote it
from the same files and reading options, and otherwise imports the network and writes the file:
the same network, bit for bit (the same fingerprint, the same results), in a fraction of the
import's time. ``Case.parkings()`` does the same with the parkings it reads from the case's
extract, under ``~/.cache/openmobisim/parkings/<case>/``. ``OPENMOBISIM_NO_NETWORK_CACHE=1``
reads both from the case's files every time and writes nothing.

A file's name holds a checksum of what it is made from (the case's files and reading options),
the defaults table's version, the file format's and a stamp of the code (the compiled core's
commit, size and modification time, and this package's Python files, since a link table is read
in Python): another build writes a file of its own, so two installations used side by side do
not replace each other's. The compiled core checks the file's header again when it reads it. The
three files of a case used last are kept, the others removed.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
from collections.abc import Callable
from functools import cache
from pathlib import Path
from typing import Any, NamedTuple, TypeVar

from openmobisim import _core
from openmobisim._log import logger

#: The environment variable that turns the cache off, the networks' and the parkings': any value
#: but ``""`` and ``"0"``.
NO_CACHE_VARIABLE = "OPENMOBISIM_NO_NETWORK_CACHE"
#: How many files of one case are kept: the ones used last.
KEEP = 3

T = TypeVar("T")


class _Kind(NamedTuple):
    """One kind of file: where it goes, the compiled core's functions for it, what the log says."""

    #: The cache's folder for it, under ``~/.cache/openmobisim/``.
    folder: str
    #: Its files' suffix.
    suffix: str
    #: The prefix of ``_core``'s ``<prefix>_cache_header``, ``_read_cache`` and ``_write_cache``
    #: (looked up at each call).
    prefix: str
    #: The log's lines: read from the cache, how long making it took, made and cached, a file not
    #: used, a file not written.
    hit: str
    made_took: str
    made: str
    unused: str
    unwritten: str


_NETWORK = _Kind(
    folder="networks",
    suffix=".network",
    prefix="network",
    hit="{name}: network read from the cache in {read_s:.2f} s{was}: {path}",
    made_took=" (its import took {made:.2f} s)",
    made="{name}: network imported{took}, cached for the next time: {path}",
    unused="{name}: the cached network is not used ({e})",
    unwritten="{name}: the network was not cached ({e})",
)
_PARKINGS = _Kind(
    folder="parkings",
    suffix=".parkings",
    prefix="parking",
    hit="{name}: parkings read from the cache in {read_s:.2f} s{was}: {path}",
    made_took=" (reading them from the extract took {made:.2f} s)",
    made="{name}: parkings read from the extract{took}, cached for the next time: {path}",
    unused="{name}: the cached parkings are not used ({e})",
    unwritten="{name}: the parkings were not cached ({e})",
)


def _off() -> bool:
    return os.environ.get(NO_CACHE_VARIABLE, "") not in ("", "0")


def _folder(kind: str = "networks") -> Path:
    base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "openmobisim" / kind


@cache
def _code_stamp() -> str:
    """The code that makes a network or parkings: the compiled core and this package's files."""
    files = [Path(_core.__file__), *sorted(Path(__file__).parent.glob("*.py"))]
    rows = [(p.name, p.stat().st_size, p.stat().st_mtime_ns) for p in files]
    described = repr((_core.build_info()["git_sha"], rows))
    return hashlib.sha256(described.encode()).hexdigest()[:12]


def _cached(kind: _Kind, name: str, made_from: dict[str, Any], make: Callable[[], T]) -> T:
    """What ``make()`` builds from ``made_from``, read from the cache of ``kind`` if it holds it."""
    if _off():
        return make()
    key = hashlib.sha256(json.dumps(made_from, sort_keys=True).encode()).hexdigest()
    code = _code_stamp()
    source = f"{key}/{code}"
    info = _core.build_info()
    versions = f"{info['defaults_version']}-{info['network_format_version']}"
    folder = _folder(kind.folder) / name
    path = folder / f"{key[:16]}-{versions}-{code[:8]}{kind.suffix}"
    if path.exists():
        try:
            header = getattr(_core, f"{kind.prefix}_cache_header")(str(path))
            if header["current"] and header["source"] == source:
                value = getattr(_core, f"{kind.prefix}_read_cache")(str(path), source=source)
                os.utime(path)  # used now: kept before the others
                made = header["made_s"]
                was = "" if math.isnan(made) else kind.made_took.format(made=made)
                logger.info(kind.hit.format(name=name, read_s=value.read_s, was=was, path=path))
                return value
        except (OSError, ValueError) as e:  # damaged, or unreadable: make it again
            logger.debug(kind.unused.format(name=name, e=e))
    value = make()
    try:
        folder.mkdir(parents=True, exist_ok=True)
        getattr(_core, f"{kind.prefix}_write_cache")(value, str(path), source=source)
    except (OSError, ValueError) as e:
        logger.debug(kind.unwritten.format(name=name, e=e))
        return value
    try:
        files = sorted(
            folder.glob(f"*{kind.suffix}"), key=lambda p: p.stat().st_mtime, reverse=True
        )
        for old in files[KEEP:]:
            old.unlink(missing_ok=True)
    except OSError:  # another process removed one first
        pass
    took = f" in {value.read_s:.2f} s" if value.read_s is not None else ""
    logger.info(kind.made.format(name=name, took=took, path=path))
    return value


def network_cached(
    name: str, made_from: dict[str, Any], make: Callable[[], _core.Network]
) -> _core.Network:
    """The network ``make()`` builds from ``made_from``, read from the cache if it holds it.

    ``made_from`` says everything the network is made from (the files' checksums, the reading
    options), as JSON; ``name`` is the cache's folder for it (a case's name).
    """
    return _cached(_NETWORK, name, made_from, make)


def parkings_cached(
    name: str, made_from: dict[str, Any], make: Callable[[], _core.Parkings]
) -> _core.Parkings:
    """The parkings ``make()`` reads from ``made_from``, read from the cache if it holds them.

    ``made_from`` says everything the parkings are read from (the extract's checksum, the reading
    options), as JSON; ``name`` is the cache's folder for them (a case's name).
    """
    return _cached(_PARKINGS, name, made_from, make)
