"""Temporary measurement (removed before the pull request): the package's full function for the
user running this script. Each case writes the hand run's three tables as Parquet files, runs
from them, runs again from the record with run_file, opens the record with Run.open, and writes
the map and the summary of the fresh and the opened run. A case passes when every step returns,
the rerun's trips.parquet and links.parquet are the first run's bytes, and the opened run's pages
are the fresh run's bytes.

    python measure.py CHECKOUT WORK SHARE CASE...

WORK: a folder the user owns on the checkout's drive; SHARE: a network share's folder the user
may write (\\\\localhost\\<share>); CASE among who, suite, b, c1, c2, d1, d2, d3, e1, e2, symlink,
f1, f2, chrome. Every result is printed; the exit code is always 0.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import traceback
import warnings
from pathlib import Path

CHECKOUT, WORK, SHARE = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
TESTS = CHECKOUT / "crates" / "binding" / "tests"
WORK.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(TESTS))

import pyarrow as pa  # noqa: E402
import pyarrow.parquet as pq  # noqa: E402

import openmobisim as oms  # noqa: E402
from test_hand_run import hand_run, hand_tables  # noqa: E402

TABLES = ("trips.parquet", "links.parquet")
LOCAL_TEMP = Path(tempfile.gettempdir())


def shell(command):
    done = subprocess.run(command, capture_output=True, text=True, shell=True)
    return f"(exit {done.returncode}) " + (done.stdout + done.stderr).strip()


def who():
    print("== who", flush=True)
    for command in ("whoami", "whoami /groups", "whoami /priv"):
        print(f"-- {command}\n{shell(command)}", flush=True)
    print(f"python {sys.executable}; openmobisim {oms.__file__}; temp {LOCAL_TEMP}", flush=True)


def suite():
    print("== (a) the binding's tests", flush=True)
    started = time.perf_counter()
    done = subprocess.run(
        [sys.executable, "-m", "pytest", str(TESTS), "-p", "no:cacheprovider", "-rsfE",
         "--tb=short", "--disable-warnings", "--basetemp", str(WORK / "pytest")],
        capture_output=True, text=True, cwd=WORK)
    print(done.stdout[-30_000:] + done.stderr[-5_000:], flush=True)
    print(f"(a) pytest exit {done.returncode} in {time.perf_counter() - started:.1f} s", flush=True)


def junction(link, target):
    """A directory junction (mklink /J, no administrator right needed)."""
    target.mkdir(parents=True, exist_ok=True)
    print(f"  mklink /J {link} {target}: {shell(f'mklink /J {link} {target}')}", flush=True)


def symbolic(link, target):
    target.mkdir(parents=True, exist_ok=True)
    os.symlink(target, link, target_is_directory=True)
    print(f"  symbolic link {link} -> {target}: created", flush=True)


def case(label, inputs, out, given=None, before=None):
    """`given`: the folder the inputs are given through (a link), else `inputs`; `before`: what
    to make first (a link)."""
    print(f"== case {label}: inputs written in {inputs}, given through {given or inputs}, "
          f"out {out}", flush=True)
    started = time.perf_counter()
    stage = "make the folders"
    try:
        if before:
            before()
        stage = "write the three tables (pyarrow)"
        Path(inputs).mkdir(parents=True, exist_ok=True)
        names = []
        for name, table in zip(("nodes", "links", "trips"), hand_tables()):
            pq.write_table(pa.table(table), os.path.join(inputs, f"{name}.parquet"))
            names.append(f"{name}.parquet")
        files = [os.path.join(given or inputs, n) for n in names]
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            stage = "run from the files"
            first = hand_run(*files, os.path.join(out, "first"))
            record = tomllib.loads(Path(first.record).read_text(encoding="utf-8"))
            print(f"  record {first.record}: nodes_file = {record['network']['nodes_file']!r}, "
                  f"links_file = {record['network']['links_file']!r}, "
                  f"trips_file = {record['demand']['trips_file']!r}", flush=True)
            stage = "run_file"
            again = oms.run_file(first.record, out=os.path.join(out, "again"))
            same = [Path(first.record).with_name(t).read_bytes()
                    == Path(again.record).with_name(t).read_bytes() for t in TABLES]
            print(f"  run_file: ran; trips.parquet, links.parquet the first run's bytes: {same}",
                  flush=True)
            stage = "the fresh run's map and summary"
            for folder in ("fresh", "opened", "again-pages"):
                os.makedirs(os.path.join(out, folder), exist_ok=True)
            fresh = [Path(first.map(os.path.join(out, "fresh", "map.html"))).read_bytes(),
                     Path(first.summary(os.path.join(out, "fresh", "summary.html"))).read_bytes()]
            stage = "Run.open"
            opened = oms.Run.open(first.record)
            stage = "the opened run's map and summary"
            pages = [Path(opened.map(os.path.join(out, "opened", "map.html"))).read_bytes(),
                     Path(opened.summary(os.path.join(out, "opened", "summary.html"))).read_bytes()]
            print(f"  Run.open: opened; map and summary the fresh run's bytes: "
                  f"{[a == b for a, b in zip(fresh, pages)]} ({len(pages[0])}, {len(pages[1])} bytes)",
                  flush=True)
            stage = "Run.open of the rerun's record, its map"
            oms.Run.open(again.record).map(os.path.join(out, "again-pages", "map.html"))
        fingerprints = [str(w.message)[:160] for w in caught if "fingerprint" in str(w.message)]
        print(f"  warnings: {len(caught)}, of them about a fingerprint: {fingerprints}", flush=True)
        passed = all(same) and fresh == pages and not fingerprints
        verdict = "PASSED" if passed else "FAILED (a comparison above)"
    except Exception as e:  # every failure is printed, the next case goes on
        verdict = f"FAILED at '{stage}': {type(e).__name__}: {e}"
        traceback.print_exc(limit=3, file=sys.stdout)
    print(f"CASE {label}: {verdict} ({time.perf_counter() - started:.1f} s)", flush=True)


def can_symlink():
    print("== symbolic link by this user", flush=True)
    target, link = WORK / "symlink-target", WORK / "symlink-link"
    target.mkdir(parents=True, exist_ok=True)
    try:
        os.symlink(target, link, target_is_directory=True)
        print("SYMLINK: created", flush=True)
    except OSError as e:
        print(f"SYMLINK: refused: {type(e).__name__}: {e}", flush=True)


def chrome():
    print("== Chrome", flush=True)
    names = ("chromium", "chromium-browser", "google-chrome", "google-chrome-stable", "chrome")
    print(f"  on PATH: {[shutil.which(n) for n in names]}", flush=True)
    for p in (r"C:\Program Files\Google\Chrome\Application\chrome.exe",
              r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"):
        print(f"  {p}: exists {Path(p).is_file()}", flush=True)
    for kw in ({"preexec_fn": lambda: None}, {"pass_fds": (3,)}):
        try:
            subprocess.Popen(["cmd", "/c", "exit"], **kw).wait()
            print(f"  Popen with {list(kw)}: accepted", flush=True)
        except Exception as e:
            print(f"  Popen with {list(kw)}: {type(e).__name__}: {e}", flush=True)


def main():
    share = Path(SHARE)
    cases = {
        "who": who,
        "suite": suite,
        "b": lambda: case("b (one drive)", WORK / "b" / "inputs", WORK / "b" / "out"),
        "c1": lambda: case("c1 (inputs D:, out C:)", WORK / "c1" / "inputs",
                           LOCAL_TEMP / "oms-c1" / "out"),
        "c2": lambda: case("c2 (inputs C:, out D:)", LOCAL_TEMP / "oms-c2" / "inputs",
                           WORK / "c2" / "out"),
        "d1": lambda: case("d1 (inputs on the share, out local)", share / "d1" / "inputs",
                           WORK / "d1" / "out"),
        "d2": lambda: case("d2 (inputs and out on the share)", share / "d2" / "inputs",
                           share / "d2" / "out"),
        "d3": lambda: case("d3 (inputs local, out on the share)", WORK / "d3" / "inputs",
                           share / "d3" / "out"),
        "e1": lambda: case("e1 (out through a junction)", WORK / "e1" / "inputs",
                           WORK / "e1" / "short" / "run",
                           before=lambda: junction(WORK / "e1" / "short",
                                                   WORK / "e1" / "real" / "one" / "two")),
        "e2": lambda: case("e2 (inputs through a junction)", WORK / "e2" / "inputs",
                           WORK / "e2" / "out", given=WORK / "e2" / "in-link",
                           before=lambda: junction(WORK / "e2" / "in-link", WORK / "e2" / "inputs")),
        "symlink": can_symlink,
        "f1": lambda: case("f1 (out through a symbolic link)", WORK / "f1" / "inputs",
                           WORK / "f1" / "short" / "run",
                           before=lambda: symbolic(WORK / "f1" / "short",
                                                   WORK / "f1" / "real" / "one" / "two")),
        "f2": lambda: case("f2 (inputs through a symbolic link)", WORK / "f2" / "inputs",
                           WORK / "f2" / "out", given=WORK / "f2" / "link",
                           before=lambda: symbolic(WORK / "f2" / "link", WORK / "f2" / "inputs")),
        "chrome": chrome,
    }
    for name in sys.argv[4:]:
        try:
            cases[name]()
        except Exception:
            traceback.print_exc(file=sys.stdout)
    print("MEASURE DONE", flush=True)


main()
