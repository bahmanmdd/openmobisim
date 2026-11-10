#!/usr/bin/env python3
"""The outside answer of the bottleneck-with-mode-choice test: nothing of the simulator in it.

One origin, one destination. A car takes TAU_CAR seconds at free flow and then queues at a
bottleneck of capacity c; a bike takes TAU_BIKE seconds, always. Travellers depart at a constant
rate q > c and choose by the logit (one alternative per mode, so the nest scale plays no part)
on the loop's default utilities. With no walk between door and node and one car route (path
size 1, ln PS = 0), the car's cost is 0.1 per minute of driving plus its constant 0, and the
bike's 0.1 per minute of riding plus its constant 0: 0.1 per minute is BETA = 0.1 / 60 = 1/600
per second on both, and the two constants are equal, so the difference of the two costs is
BETA times the difference of the two times, as it was when the cost was time alone at 1/600 per
second. The walking, waiting, transfer, money and other constants' coefficients play no part.
For the traveller departing at t, with w(t) the queue delay a car departing at t meets:

    car share     p(w) = 1 / (1 + exp(BETA * (TAU_CAR + w - TAU_BIKE)))
    queue         dw/dt = (q / c) * p(w) - 1,   w(0) = 0     (first in, first out: the car
                  departing at t waits for the cars that departed before it, nothing else)

Steady state (dw/dt = 0): p* = c / q and w* = TAU_BIKE - TAU_CAR + ln(q / c - 1) / BETA.

The differential equation is solved here by scipy (LSODA, tolerances 1e-10) and checked against
its implicit closed form: with u = exp(BETA (TAU_CAR + w - TAU_BIKE)) and a = q / c - 1,

    BETA * t = (1 / a) ln(u / u0) - ((a + 1) / a) ln((a - u) / (a - u0)).

Output, fixtures/bottleneck/reference.csv: one header row of the constants, then one row per GROUP
seconds of departures: the group's start, the car share over the group (the mean of p), the mean
door-to-door time of its cars (TAU_CAR + w weighted by p), and w at the group's middle.

    python3 crates/loop/tests/bottleneck_reference.py        (Python 3.13, numpy 2.2, scipy 1.15)
"""

from pathlib import Path

import numpy as np
from scipy.integrate import solve_ivp

TAU_CAR = 144.0     # s: 2 000 m at 50 km/h
TAU_BIKE = 360.0    # s: 1 500 m at 15 km/h
BETA = 0.1 / 60.0   # per s: the default 0.1 per minute driving and riding (equal to 1/600)
RATIO = 3.0         # q / c: demand over capacity
HOURS = 3.0         # of demand
GROUP = 300         # s


def share(w):
    return 1.0 / (1.0 + np.exp(BETA * (TAU_CAR + w - TAU_BIKE)))


def implicit_time(w):
    """The time at which the queue delay is w, by the closed form."""
    a = RATIO - 1.0
    u0 = np.exp(BETA * (TAU_CAR - TAU_BIKE))
    u = np.exp(BETA * (TAU_CAR + w - TAU_BIKE))
    return ((1.0 / a) * np.log(u / u0) - ((a + 1.0) / a) * np.log((a - u) / (a - u0))) / BETA


def main():
    end = HOURS * 3_600.0
    t = np.arange(0.0, end + 0.5, 1.0)
    sol = solve_ivp(lambda _, w: RATIO * share(w) - 1.0, (0.0, end), [0.0], t_eval=t,
                    method="LSODA", rtol=1e-10, atol=1e-10)
    assert sol.success, sol.message
    w = sol.y[0]
    # The closed form, wherever the queue is still measurably below its limit.
    w_star = TAU_BIKE - TAU_CAR + np.log(RATIO - 1.0) / BETA
    rising = (w > 1.0) & (w < w_star - 1.0)
    worst = np.max(np.abs(implicit_time(w[rising]) - t[rising]))
    assert worst < 1e-3, worst
    p = share(w)
    rows = []
    for start in range(0, int(end), GROUP):
        s = slice(start, start + GROUP + 1)
        mean_p = np.trapezoid(p[s], t[s]) / GROUP
        car_time = TAU_CAR + np.trapezoid(p[s] * w[s], t[s]) / np.trapezoid(p[s], t[s])
        rows.append((start, mean_p, car_time, w[start + GROUP // 2]))
    out = Path(__file__).resolve().parent / "fixtures" / "bottleneck" / "reference.csv"
    with out.open("w") as f:
        f.write(f"constants,{TAU_CAR},{TAU_BIKE},{BETA!r},{RATIO},{HOURS},{GROUP}\n")
        for start, mean_p, car_time, mid in rows:
            f.write(f"group,{start},{mean_p:.9f},{car_time:.6f},{mid:.6f}\n")
    print(f"steady state: car share {1.0 / RATIO:.6f}, queue delay {w_star:.3f} s, "
          f"car trip {TAU_CAR + w_star:.3f} s")
    print(f"scipy against the implicit closed form: at most {worst:.2e} s apart in time")
    print(f"queue delay at 1 h {w[3600]:.3f} s, at 2 h {w[7200]:.3f} s, at 3 h {w[-1]:.3f} s")
    third = [r for r in rows if r[0] >= 7_200]
    print(f"third hour: car share {np.mean([r[1] for r in third]):.6f}, "
          f"car trip {np.mean([r[2] for r in third]):.3f} s")


if __name__ == "__main__":
    main()
