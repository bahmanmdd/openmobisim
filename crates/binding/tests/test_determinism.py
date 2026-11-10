"""Determinism, the one self-comparison: the same small logit run (grid.py's logit_run: 400 trips
open to car, bike and walk on an 8 x 8 grid with queues, the nested logit, 8 iterations at most)
twice in this process, and once each at RAYON_NUM_THREADS=1 and 8 in a fresh interpreter:
trips.parquet and links.parquet byte-identical across the four. (iterations.parquet holds the
seconds each stage took, so it is not compared.) For bikes: the bike trips and the bike counts
per link and bin are in the two files compared.
"""

import os
import subprocess
import sys
from pathlib import Path

import pyarrow.parquet as pq

from grid import logit_run

TESTS = Path(__file__).resolve().parent


def test_the_same_files_twice_and_at_1_and_8_threads(tmp_path):
    folders = [tmp_path / "first", tmp_path / "second"]
    for folder in folders:
        logit_run(folder)
    for threads in ("1", "8"):
        folder = tmp_path / f"threads-{threads}"
        env = {**os.environ, "RAYON_NUM_THREADS": threads}
        subprocess.run([sys.executable, str(TESTS / "grid.py"), str(folder)], env=env, check=True)
        folders.append(folder)
    for name in ("trips.parquet", "links.parquet"):
        files = [(folder / name).read_bytes() for folder in folders]
        assert all(f == files[0] for f in files), name
    modes = pq.read_table(folders[0] / "trips.parquet").column("mode").to_pylist()
    print(f"modes taken: {sorted(set(modes) - {None})}")
