"""A run's fingerprint follows every input its results depend on.

What is defended: two runs whose results differ because an input differs get different
fingerprints — here trips stated as bike-and-ride on two bike networks (streets given separated
tracks in the second), whose bike legs and so whose journeys change; and the same inputs give
the same fingerprint. The fingerprint is what ``check`` and every comparison of runs trust to
say "other inputs".
"""

from __future__ import annotations

import openmobisim as ms


def bike_and_ride(network: object) -> ms.Run:
    trips = [
        t[:9] + ("bike_transit",)
        for t in ms.examples.trips_random(network, 400, seed=2, min_m=300, max_m=3000)
    ]
    return ms.Scenario.from_parts(
        network,
        trips,
        classes={"commuter": (True, True, True)},
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
        parking_options={"pr_min_km": 0},
        equilibration="free_flow",
    ).run("fingerprint", quiet=True)


def test_bike_and_ride_on_another_bike_network_has_another_fingerprint():
    net = ms.examples.toy_network()
    bike = net.layer("bike")
    mixed = [i for i, kind in zip(bike.link_ids(), bike.link_infrastructure(), strict=True)
             if kind == 0]  # fmt: skip
    tracks = ms.network_edit(net, bike_facility={i: "separated" for i in mixed})
    base, again, other = bike_and_ride(net), bike_and_ride(net), bike_and_ride(tracks)
    assert other.mean_travel_time_s != base.mean_travel_time_s, "the bike legs changed"
    assert other.fingerprint != base.fingerprint, "other inputs, another fingerprint"
    assert again.fingerprint == base.fingerprint, "the same inputs, the same fingerprint"
