"""Download the raw files the collection lists, where they are missing.

    python fetch.py              # every case
    python fetch.py paris lyon   # some

Each raw file comes from the URL recorded in ``cases.py`` and goes to ``cases/<key>/raw/``. A provider
serves its latest version, so a file fetched again is usually newer than the one the results in this
collection were made from: ``MANIFEST.csv`` keeps the checksum of the one used, and this script says
when they differ.
"""

from __future__ import annotations

import csv
import hashlib
import shutil
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from cases import CASES, DATA, raw_path  # noqa: E402


def recorded() -> dict[str, str]:
    """The checksums recorded in the data root's ``MANIFEST.csv``, by path; none if it is missing."""
    path = DATA / "MANIFEST.csv"
    if not path.exists():
        return {}
    with path.open() as f:
        return {r["path"]: r["sha256"] for r in csv.DictReader(f)}


def main(keys: list[str]) -> None:
    """Download the missing raw files of the named cases (of all cases if none is named)."""
    known = recorded()
    for key in keys or list(CASES):
        for raw in CASES[key].raw:
            dst = raw_path(key, raw.name)
            if dst.exists():
                continue
            print(f"[{key}] {raw.name} ← {raw.url}", flush=True)
            dst.parent.mkdir(parents=True, exist_ok=True)
            tmp = dst.with_suffix(dst.suffix + ".part")
            with urllib.request.urlopen(raw.url) as r, tmp.open("wb") as f:  # noqa: S310 (listed sources)
                shutil.copyfileobj(r, f, 1 << 20)
            tmp.rename(dst)
            h = hashlib.sha256(dst.read_bytes()).hexdigest()
            old = known.get(str(dst.relative_to(DATA)))
            if old and old != h:
                print(f"[{key}] {raw.name}: a newer version than the one recorded (results were made "
                      f"with {old[:12]}…, this is {h[:12]}…)")


if __name__ == "__main__":
    main(sys.argv[1:])
