"""The command line: the starter cases.

    python -m openmobisim cases                  # the starter cases and their scenarios
    python -m openmobisim run amsterdam [base]   # run one case: its numbers, page and map
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
from openmobisim._paths import show_path


def _cases(args: argparse.Namespace) -> int:
    found = 0
    for name in case_names():
        try:
            c = case(name, args.root, check=False)
        except FileNotFoundError:
            print(f"{name:14s} (not on this machine)")
            continue
        found += 1
        scenarios = ", ".join(c.info.get("scenarios", {"base": {}}))
        print(f"{name:14s} {scenarios:16s} {c.info.get('title', '')}")
    if not found:
        print(
            "No case found. A case is downloaded once when first run; a bundle already on this "
            "machine is found with --root FOLDER or OPENMOBISIM_DATA=FOLDER (the folder holding "
            "<case>/case.json)."
        )
    return 0


def _run(args: argparse.Namespace) -> int:
    c = case(args.case, args.root)
    out = Path(args.output or Path("openmobisim-runs") / f"{args.case}-{args.scenario}")
    # Per-link results in 15-minute bins, so the page and the map show the traffic, unless the
    # case's scenario records them already (then its fingerprint is the reference's).
    settings = c.info.get("scenarios", {}).get(args.scenario, {})
    bins = {} if "link_bin_s" in settings or args.no_pages else {"link_bin_s": 900}
    scenario = c.scenario(args.scenario, **bins)
    run = scenario.run(f"{args.case}-{args.scenario}", output_dir=str(out), overwrite=True)
    s = case_summary(run)
    print(
        f"\n{args.case} / {args.scenario}: {s['completed']} of {s['trips']} trips completed, "
        f"mean trip {s['mean_trip_s'] / 60:.2f} min, {s['loadings']} loadings"
    )
    shares = ", ".join(f"{m} {v:.2f}" for m, v in s["mode_share_pct"].items())
    print(f"modes (% of people): {shares}")
    print(f"fingerprint {s['fingerprint']} · openmobisim {s['version']} · {s['platform']}")
    show_path("Files", Path(run.timings_path).parent)
    if args.no_pages:
        return 0
    from openmobisim import viz

    title = c.info.get("title", args.case)
    viz.map_interactive(run, out / "map.html", title=f"{title}: {args.scenario}")
    try:
        viz.summary(run, str(out / "summary.html"), title=f"{title}: {args.scenario}")
    except ImportError:
        print('The summary page needs matplotlib: pip install "openmobisim[viz]"')
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
    listing.add_argument("--root", help="a folder holding the bundle")
    listing.set_defaults(go=_cases)
    r = sub.add_parser("run", help="run one starter case's scenario")
    r.add_argument("case", choices=case_names())
    r.add_argument("scenario", nargs="?", default="base")
    r.add_argument("--root", help="a folder holding the bundle")
    r.add_argument(
        "--output", help="where the run's files go (default: ./openmobisim-runs/<case>-<scenario>)"
    )
    r.add_argument("--no-pages", action="store_true", help="no summary page and no map")
    r.set_defaults(go=_run)
    k = sub.add_parser("check", help="run the default scenarios, compare them with the reference")
    k.add_argument("cases", nargs="*", help="the cases (default: all)")
    k.add_argument("--root", help="a folder holding the bundle")
    k.set_defaults(go=_check)
    args = p.parse_args(argv)
    return args.go(args)


if __name__ == "__main__":
    sys.exit(main())
