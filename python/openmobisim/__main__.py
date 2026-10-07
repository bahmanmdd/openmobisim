"""The command line (S243): the starter cases, run one by one or checked against their references.

    python -m openmobisim cases                  # the starter cases and their scenarios
    python -m openmobisim run amsterdam [base]   # run one case's scenario, print what it gave
    python -m openmobisim check [siouxfalls ...] # run the default scenarios, compare them

``--root`` names a folder holding the bundle (else ``OPENMOBISIM_DATA``, else the cache).
``check`` exits with 1 if any scenario differs from its reference. Once installed, the same
commands are ``openmobisim cases`` and so on.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from openmobisim import __version__
from openmobisim._cases import case, case_check, case_names, case_summary


def _cases(_: argparse.Namespace) -> int:
    for name in case_names():
        try:
            c = case(name, check=False)
        except FileNotFoundError:
            print(f"{name:14s} (not on this machine)")
            continue
        scenarios = ", ".join(c.info.get("scenarios", {"base": {}}))
        print(f"{name:14s} {scenarios:16s} {c.info.get('title', '')}")
    return 0


def _run(args: argparse.Namespace) -> int:
    c = case(args.case, args.root)
    run = c.scenario(args.scenario).run(f"{args.case}-{args.scenario}", output_dir=args.output)
    s = case_summary(run)
    print(
        f"\n{args.case} / {args.scenario}: {s['completed']} of {s['trips']} trips completed, "
        f"mean trip {s['mean_trip_s'] / 60:.2f} min, {s['loadings']} loadings"
    )
    shares = ", ".join(f"{m} {v:.2f}" for m, v in s["mode_share_pct"].items())
    print(f"modes (% of people): {shares}")
    print(f"fingerprint {s['fingerprint']} · openmobisim {s['version']} · {s['platform']}")
    print(f"files: {Path(run.timings_path).parent}")
    return 0


def _check(args: argparse.Namespace) -> int:
    rows = case_check(args.cases or None, args.root)
    bad = sum(r["status"] == "different" for r in rows)
    missing = sum(r["status"] == "no reference" for r in rows)
    print(
        f"\n{len(rows)} scenarios: {len(rows) - bad - missing} match, {bad} differ, "
        f"{missing} without a reference"
    )
    return 1 if bad else 0


def main(argv: list[str] | None = None) -> int:
    """Run the command line; returns the exit code."""
    p = argparse.ArgumentParser(
        prog="openmobisim", description=f"openmobisim {__version__}: the starter cases"
    )
    sub = p.add_subparsers(dest="command", required=True)
    listing = sub.add_parser("cases", help="list the starter cases and their scenarios")
    listing.set_defaults(go=_cases)
    r = sub.add_parser("run", help="run one starter case's scenario")
    r.add_argument("case", choices=case_names())
    r.add_argument("scenario", nargs="?", default="base")
    r.add_argument("--root", help="a folder holding the bundle")
    r.add_argument("--output", help="where the run's files go (default: a temporary folder)")
    r.set_defaults(go=_run)
    k = sub.add_parser("check", help="run the default scenarios, compare them with the reference")
    k.add_argument("cases", nargs="*", help="the cases (default: all)")
    k.add_argument("--root", help="a folder holding the bundle")
    k.set_defaults(go=_check)
    args = p.parse_args(argv)
    return args.go(args)


if __name__ == "__main__":
    sys.exit(main())
