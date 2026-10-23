"""The planning defaults.

The point queue for every loading, stopping when converged, rerouting only where queues take
space, and the list of links where a queue outgrew the road. What is defended: a default run is a
point-queue run that stops before its most iterations once converged and never re-routes;
``Run.report_spillback()`` lists links whose peak occupancy exceeded storage under the point
queue, and nothing at free flow; the interactive map carries them as one optional layer.
"""

from pathlib import Path

import numpy as np
import openmobisim as ms

CAR = {"commuter": (True, False, False)}


def jam(name, spread_s=600, **kwargs):
    net = ms.examples.manhattan_grid(n=6, block_metres=150.0, signals=False)
    rows = ms.examples.trips_random(net, 4000, seed=3, min_m=300.0, max_m=900.0, spread_s=spread_s)
    settings = {"classes": CAR, "window": (0, 10_800)}
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, rows, **settings).run(name, quiet=True)


def test_a_default_run_is_a_point_queue_run_that_stops_when_converged():
    run = jam("pd-default")
    assert run.flow_level == 2
    m = run.manifest()
    assert "gap_tolerance=0.05" in m["equilibration_descriptor"]
    assert "route_growth_tolerance=1" in m["equilibration_descriptor"]
    explicit = jam("pd-explicit", loading_options={"reroute": 0})
    assert explicit.fingerprint == run.fingerprint, "rerouting is off: nothing waits for room"
    c = run.convergence()
    assert run.converged and len(c["iteration"]) < 10, "it stopped before its most iterations"
    assert (c["reroute_searches"] == 0).all() and run.completion["truncated"] == 0


# The spillback tests leave in five minutes what the others spread over ten: at the classic point
# queue (S293) a queue grows only on a link fed above its capacity, which over ten minutes this
# grid's demand is not; before, the headway at nodes queued vehicles on the links upstream.
def test_spillback_links_are_listed_and_none_at_free_flow():
    point = jam("pd-pq", spread_s=300, equilibration="free_flow")
    s = point.report_spillback()
    assert s is not None and len(s["link"]) > 0, "a jammed grid's queues outgrow some links"
    excess = s["peak_pcu"] - s["storage_pcu"]
    assert (excess > 5.0).all(), "at least five cars past the road, by default"
    assert (np.diff(excess) <= 1e-9).all(), "worst first"
    every = point.report_spillback(min_excess_pcu=0.0)
    assert len(every["link"]) >= len(s["link"])
    free = jam("pd-free", spread_s=300, equilibration="free_flow", flow_level=0)
    assert free.report_spillback() is None


def test_the_interactive_map_offers_them_as_one_layer(tmp_path):
    from openmobisim import viz

    run = jam("pd-map", spread_s=300, equilibration="free_flow", link_bin_s=900)
    text = Path(viz.map_interactive(run, str(tmp_path / "m.html"))).read_text(encoding="utf-8")
    n = len(run.report_spillback()["link"])
    assert f'"n_spillback":{n}' in text and 'id="spill"' in text and '"n":"sb"' in text
