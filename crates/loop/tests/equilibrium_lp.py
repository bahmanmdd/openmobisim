#!/usr/bin/env python3
"""The capacity-constrained equilibrium of a network at constant demand, the outside answer of
tests 5 and 6: the steady state of the point queue is the link-capacity side-constrained static
equilibrium with constant link times (Larsson and Patriksson 1995; for one origin and one
destination a theorem, Cominetti, Correa and Olver 2022). Link flows solve

    min Σ_a τ_a x_a   over multicommodity flows (one commodity per origin) meeting the OD rates
                      within the capacities, x_a ≤ ν_a;

the queue delays are the capacity rows' dual prices, positive only on links at capacity, and the
OD times are the conservation rows' dual prices. The duals need not be unique: further programs
over the dual's optimal face give the range of the total travel time (a range under 0.1 % makes
the mean trip a reference), of every link's queue and of every OD time. Solved by scipy's HiGHS,
nothing of the simulator in it.

    python3 equilibrium_lp.py nguyen_dupuis 0.9 1
    python3 equilibrium_lp.py sioux_falls 0.045 0.1

writes fixtures/<case>/equilibrium.csv for the case's demand times the first factor (veh/h) and
its capacities times the second. Nguyen–Dupuis: the tables of Sensors 25(7) 2262 (2025, CC BY
4.0) as version 1 of openmobisim made them dynamic (minutes as free-flow times, capacities and
demand ×10 per hour). Sioux Falls: TNTP (bstabler/TransportationNetworks), capacities in veh/h,
free-flow times in minutes, the trip table in vehicles per hour. Python 3.13, numpy 2.2,
scipy 1.15."""

import csv
import sys
from pathlib import Path

import numpy as np
from scipy.optimize import linprog

HERE = Path(__file__).resolve().parent / "fixtures"


def nguyen_dupuis():
    folder = HERE / "nguyen_dupuis"
    arcs, tau, cap = [], {}, {}
    for r in csv.DictReader((folder / "links.csv").open()):
        arcs.append((r["id"], int(r["from"]), int(r["to"])))
        tau[r["id"]] = float(r["free_flow_time"]) * 60.0
        cap[r["id"]] = float(r["capacity"])
    demand = {(int(r["origin"]), int(r["destination"])): float(r["trips"])
              for r in csv.DictReader((folder / "od.csv").open())}
    return folder, arcs, tau, cap, demand


def sioux_falls():
    folder = HERE / "sioux_falls"
    arcs, tau, cap = [], {}, {}
    for line in (folder / "SiouxFalls_net.tntp").read_text().splitlines():
        f = line.replace(";", "").split()
        if len(f) < 10 or not f[0].isdigit():
            continue
        a, b = int(f[0]), int(f[1])
        key = f"{a}-{b}"
        arcs.append((key, a, b))
        cap[key] = float(f[2])
        tau[key] = float(f[4]) * 60.0
    demand = {}
    for r in csv.DictReader((folder / "od.csv").open()):
        o, d, t = int(r["origin_zone"]), int(r["destination_zone"]), float(r["trips"])
        if t > 0 and o != d:
            demand[(o, d)] = t
    return folder, arcs, tau, cap, demand


def solve(arcs, tau, nu, demand):
    nodes = sorted({a[1] for a in arcs} | {a[2] for a in arcs})
    origins = sorted({o for o, _ in demand})
    n_arc, n_org = len(arcs), len(origins)
    cost = np.tile([tau[k] for k, _, _ in arcs], n_org)
    rows, rhs, index = [], [], {}
    for i, o in enumerate(origins):
        for n in nodes:
            if n == o:
                continue
            row = np.zeros(n_arc * n_org)
            for j, (_, a, b) in enumerate(arcs):
                if a == n:
                    row[i * n_arc + j] -= 1.0
                if b == n:
                    row[i * n_arc + j] += 1.0
            index[(o, n)] = len(rows)
            rows.append(row)
            rhs.append(demand.get((o, n), 0.0))
    caps = np.zeros((n_arc, n_arc * n_org))
    for j in range(n_arc):
        caps[j, j::n_arc] = 1.0
    res = linprog(cost, A_ub=caps, b_ub=[nu[k] for k, _, _ in arcs], A_eq=np.array(rows), b_eq=rhs,
                  bounds=[(0.0, None)] * (n_arc * n_org), method="highs")
    assert res.status == 0, res.message
    flow = res.x.reshape(n_org, n_arc).sum(axis=0)
    queue = -np.asarray(res.ineqlin.marginals)
    od_time = {od: float(res.eqlin.marginals[index[od]]) for od in demand}
    # The dual's optimal face: π[o, n] free (π[o, o] = 0), q[a] ≥ 0, π[o, b] − π[o, a] − q[a] ≤ τ_a,
    # Σ d π − Σ ν q ≥ C* − ε; the range of Σ ν q over it is the range of the total time less Σ τ x.
    n_pi = len(index)
    a_ub, b_ub = [], []
    for o in origins:
        for j, (k, a, b) in enumerate(arcs):
            row = np.zeros(n_pi + n_arc)
            if (o, b) in index:
                row[index[(o, b)]] += 1.0
            if (o, a) in index:
                row[index[(o, a)]] -= 1.0
            row[n_pi + j] = -1.0
            a_ub.append(row)
            b_ub.append(tau[k])
    gain = np.zeros(n_pi + n_arc)
    for (o, d), q in demand.items():
        gain[index[(o, d)]] += q
    nu_v = np.array([nu[k] for k, _, _ in arcs])
    gain[n_pi:] = -nu_v
    face = np.vstack([a_ub, -gain])
    bounds = [(None, None)] * n_pi + [(0.0, None)] * n_arc
    def over_face(objective):
        """The least and the greatest of `objective · (π, q)` over the dual's optimal face."""
        out = []
        for sign in (1.0, -1.0):
            r = linprog(sign * objective, A_ub=face, b_ub=list(b_ub) + [-(res.fun - 1e-6 * max(1.0, abs(res.fun)))],
                        bounds=bounds, method="highs")
            assert r.status == 0, r.message
            out.append(sign * r.fun)
        return min(out), max(out)

    total_q = np.zeros(n_pi + n_arc)
    total_q[n_pi:] = nu_v
    spread = over_face(total_q)
    queue_range = {}
    for j, (k, _, _) in enumerate(arcs):
        e = np.zeros(n_pi + n_arc)
        e[n_pi + j] = 1.0
        queue_range[k] = over_face(e)
    od_range = {}
    for od in demand:
        e = np.zeros(n_pi + n_arc)
        e[index[od]] = 1.0
        od_range[od] = over_face(e)
    total = sum(demand.values())
    return {"flow": dict(zip([k for k, _, _ in arcs], flow)), "queue": dict(zip([k for k, _, _ in arcs], queue)),
            "queue_range": queue_range, "od_range": od_range,
            "tau_x": float(res.fun), "od_time": od_time, "demand": total,
            "mean_lo": (res.fun + spread[0]) / total, "mean_hi": (res.fun + spread[1]) / total}


def main():
    case, d_scale, c_scale = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
    folder, arcs, tau, cap, demand = {"nguyen_dupuis": nguyen_dupuis, "sioux_falls": sioux_falls}[case]()
    nu = {k: c * c_scale for k, c in cap.items()}
    dem = {od: q * d_scale for od, q in demand.items()}
    out = solve(arcs, tau, nu, dem)
    with (folder / "equilibrium.csv").open("w") as f:
        f.write(f"scale,{d_scale},{c_scale}\n")
        f.write(f"mean,{out['mean_lo']:.6f},{out['mean_hi']:.6f}\n")
        f.write(f"taux,{out['tau_x']:.6f},{out['demand']:.6f}\n")
        for k, _, _ in arcs:
            lo, hi = out["queue_range"][k]
            f.write(f"link,{k},{out['flow'][k]:.6f},{lo:.6f},{hi:.6f},{nu[k]:.6f}\n")
        for (o, d), t in sorted(out["od_time"].items()):
            lo, hi = out["od_range"][(o, d)]
            f.write(f"od,{o},{d},{dem[(o, d)]:.6f},{lo:.6f},{hi:.6f}\n")
    print(f"{case} at demand ×{d_scale}, capacities ×{c_scale}: {len(arcs)} links, {len(dem)} OD pairs, "
          f"{out['demand']:.1f} veh/h")
    print(f"  mean trip {out['mean_lo'] / 60:.3f} to {out['mean_hi'] / 60:.3f} min "
          f"(range {(out['mean_hi'] - out['mean_lo']) / out['mean_lo'] * 100:.3f} %); "
          f"Σ τ x {out['tau_x'] / 60:.1f} veh·min per hour, {out['tau_x'] / out['demand'] / 60:.3f} min a trip")
    queued = [(k, out["queue_range"][k], out["flow"][k], nu[k]) for k, _, _ in arcs if out["queue_range"][k][1] > 1e-6]
    print("  links that may queue (queue range s, flow, capacity):",
          [(k, (round(lo), round(hi)), round(x, 1), round(c, 1)) for k, (lo, hi), x, c in queued])
    must = [k for k, _, _ in arcs if out["queue_range"][k][0] > 1e-6]
    print("  links that must queue:", must)
    if len(out["od_time"]) <= 8:
        print("  OD time ranges (min):", {od: (round(lo / 60, 2), round(hi / 60, 2)) for od, (lo, hi) in out["od_range"].items()})
    else:
        widths = [(hi - lo) / 60 for lo, hi in out["od_range"].values()]
        print(f"  OD time ranges: {sum(1 for w in widths if w < 0.01)} of {len(widths)} unique, widest {max(widths):.2f} min")


if __name__ == "__main__":
    main()
