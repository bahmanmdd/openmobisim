"""The starter cases' morning, second recipe: open population and building data, survey totals, gateways.

    python morning2.py paris [--model radiation|gravity] [--weight W] [--out DIR]

**Why**: with mode choice, a morning's totals cannot assume that every trip is a car. This recipe takes its totals from
the cities' travel surveys and its geography from open grids, and adds what enters from outside: cars at the roads that
cross the study area's edge, transit riders at its rail stations, through traffic, and goods vehicles.

**Synthetic, not a forecast**: for starter cases and benchmarks. Every number below is from a source (with its URL) or
stated as an assumption.

1. **Zones**: the first recipe's 1 km grid cells (a cell kept with at least 5 drivable nodes; its point the drivable
   node nearest the mean of its nodes).
2. **Residents and activity per zone**: GHS-POP 2020 (residents) and GHS-BUILT-V NRES 2020 (non-residential built
   volume), both from the JRC's Global Human Settlement Layer (CC BY 4.0), cut out by `ghsl.py`; a grid cell's value
   goes to the zone its centre falls in, or the nearest zone within 1.5 km (water, parks); outside the study area it is
   left out.
3. **Internal trips** in 07:00-09:00 = the survey's internal trips a day × the area's residents over the city's ×
   **16 %** in the window (*an assumption from two sources*: the morning 7-9 and evening 17-19 together hold "near a
   third" of the day's trips in French household surveys, Lavielle, https://www.rhuthmos.eu/IMG/article_PDF/article_a519.pdf;
   Metz's 7:30-8:30 alone holds 10 %, EDGT, https://www.aguram.org/wp-content/uploads/2019/07/Amplitude_horaire_PDU_document_CommissionMobilite.pdf).
   **Productions** are residents; **attractions** 60 % non-residential volume (work and study) and 40 % residents
   (shops, schools, escorting) — *an assumption* (the morning peak's trips are mostly to work and study). Trips within
   a zone (mostly walks under a kilometre) are kept, between the zone's points.
   **2b. Zone points**: each 1 km zone is split into `SPLIT` × `SPLIT` blocks (500 m); a block with
   residents or non-residential volume is a point (the volume-weighted mean of its GHSL cells) with its residents as
   the origin weight and the attraction rule's share (60 % volume, 40 % residents) as the destination weight, written
   to `zone_points.csv`, so a trip starts where people live and ends where they work, never at a zone's centre beside
   a motorway, and no single point stands for a zone's hundreds of trips.
4. **Distribution**: the **radiation model** (Simini, González, Maritan, Barabási 2012, *Nature* 484, 96-100,
   https://doi.org/10.1038/nature10856), parameter-free, normalised for a finite area (Masucci, Serras, Johansson,
   Batty 2013, *Phys. Rev. E* 88, 022812, https://doi.org/10.1103/PhysRevE.88.022812), then balanced to the attractions
   (Furness); or (`--model gravity`) a doubly-constrained gravity model with an exponential decay fitted to the survey's
   mean distance (Ortúzar & Willumsen, *Modelling Transport*, 4th ed., ch. 5). Both are reported; the case uses the one
   in `CITY["model"]`.
5. **From outside**: the survey's exchange trips a day × 16 %, split by its modal shares. **Cars** enter (and leave) at
   **road gateways** — every motorway, trunk, primary or secondary road the area's edge cuts, weighted by its capacity —
   at a car occupancy of 1.2 (*an assumption*: EMD Lyon 2015, exchanges centre-ring: 47 % drivers, 11 % passengers,
   https://www.urbalyon.org/sites/default/files/productions/publications/imports/2019-11/m-emd-agglo.pdf), 70 %
   inbound in the morning (*an assumption*: the cores hold more jobs than workers); **through traffic**, gateway to
   gateway at least 3 km apart, is 30 % of the exchange cars (*an assumption*); their destinations by the attractions,
   with the gravity decay. **Transit riders** from outside start at the area's rail stations, by departures.
   **Goods vehicles** (and taxis where known) are the survey's share of road traffic, between zones by non-residential
   volume. All from outside, and goods, have a fixed mode and a car-only or transit-only class.
   **5b. Counts at motorway and trunk gateways**: such a gateway's flow in 07:00-09:00 is a count, from
   Rijkswaterstaat's INWEVA in the Netherlands (CC0, the morning peak's own field) or, in France, from the national
   network's annual average daily traffic (TMJA 2019, Licence Ouverte; `sources/tmja/` of the data root), both
   directions together, halved and taken at the Dutch counts' morning share of a workday (13.7 %, the median of
   INWEVA's `al_os_wr` over `al_e_wr` at the Dutch gateways: *an assumption* for France); a French gateway on a
   road the national file does not hold (the declassified M6, M7 and Laurent Bonnevay in Lyon, the Périphérique in
   Paris) takes the median share of capacity of the city's counted gateways (*an assumption*); its through
   traffic is the count less its exchange cars. The counts go with the demand as `gateway_counts.csv`. *The French
   cities use the Dutch ratio* (`TRANSFER_RATIO`); the TMJA reading is the option `counts` "tmja".
6. **Weight**: each simulated traveller stands for `weight` people (the case's `demand.weight`), so a run's totals are
   the whole morning's and its time a sample's.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import sys
import zipfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

import openmobisim as ms  # noqa: E402
from openmobisim import _cases  # noqa: E402

from cases import DATA  # noqa: E402

BUNDLE = DATA / "bundle" / "v1"
AM_SHARE = 0.16
#: Checked S282 against ODiN 2023 (CBS StatLine 84702NED, the Netherlands): 12.5 % of all days' trips depart in
#: 07:00-09:00 (Zuid-Holland 12.4 %, Noord-Holland 12.3 %), at 2.72 trips a day over all days and 2.86 on a workday;
#: with weekend mornings holding 0 to 5 % of their days' trips, a workday's 07:00-09:00 holds 15-17 %: the 16 % above.
#: No finer morning profile is published in the open tables of ODiN, the EGT (Paris) or the EMD (Lyon): the bell over
#: the window (`am_peak`, Beta(3, 3)) stays, an assumption.
#: The departure window and its share of the day's trips: 06:30-09:30 with the "am_peak" bell (Beta(3, 3)) puts 92.9 %
#: of the window's trips in 07:00-09:00 and 58.0 % in 07:30-08:30, so 17.2 % of the day in the window gives 16 % in
#: 07:00-09:00 and 10.0 % in 07:30-08:30, both sources' numbers (step 3).
WINDOW = (6 * 3600 + 1800, 9 * 3600 + 1800)
WINDOW_SHARE = AM_SHARE / 0.929
PEAK_HOUR_OF_WINDOW = 0.580
#: Cars from outside and goods vehicles, times this (S265: car is the main cost of a run, so its share is reduced a
#: bit everywhere, systematically, to stay faster): the surveys' car shares and Paris's goods share, 20 % less.
CAR_FACTOR = 0.8
#: A gateway takes at most this share of its capacity in the busiest hour (step 5).
GATEWAY_USE = 0.85
OCCUPANCY = 1.2
INBOUND = 0.7
THROUGH = 0.3  # the default share; a city may give its own (`through`)
#: The share of the window's departures in 07:00-09:00 (the bell above): a count of 07:00-09:00 over it is the
#: window's.
SHARE_07_09 = 0.929
#: Motorway and trunk gateways take their flows from counts (S268, step 5b).
COUNTED = {"motorway", "trunk"}
#: Where a city has no open counts here (`counts` "transfer": Lyon and Paris): a motorway or trunk carriageway at a
#: gateway carries this share of its own capacity in 07:00-09:00 on average. *A tuned assumption* (S282): Dutch urban
#: motorways for a French city's (Amsterdam's 11 counted gateways carry 0.29-1.14; `morning.json`,
#: `counted_gateways`), set so that Lyon's and Paris's 07:00-09:00 travel-time index stays slightly below TomTom's
#: (S266).
TRANSFER_RATIO = 0.35
INWEVA = ("Rijkswaterstaat, INWEVA 2025, morning peak (ochtendspits_intensiteiten_2025): mean workday vehicles per "
          "carriageway in the morning peak, all vehicles (al_os_wr), CC0 1.0, "
          "https://geo.rijkswaterstaat.nl/services/ogc/gdr/inweva/ows")
#: France (S282, `counts` "tmja"): the national network's annual average daily traffic, both directions together, per
#: section with its ends in Lambert 93 (`sources/tmja/` of the data root). The year 2019: the last complete one before
#: the pandemic and the last with the whole network (2022-2024 cover a smaller one). Avatar (Cerema) holds the hourly
#: counts, its measures behind a login, and the cities' own counters (Paris's permanent sensors, Lyon's Criter)
#: are large hourly series: not fetched (the data is too big to download).
TMJA = ("Ministère de la Transition écologique, Trafic moyen journalier annuel sur le réseau routier national, 2019 "
        "(TMJA, both directions, vehicles a day), Licence Ouverte 2.0, "
        "https://www.data.gouv.fr/datasets/trafic-moyen-journalier-annuel-sur-le-reseau-routier-national")
TMJA_FILE = DATA / "sources" / "tmja" / "tmja-2019.csv"
#: A daily count's share in 07:00-09:00 on a workday, per carriageway: the median of INWEVA's `al_os_wr` over
#: `al_e_wr` on the main carriageways around Delft (60 segments, 0.134) and Amsterdam (249, 0.139), S282. *An
#: assumption* for France (a Dutch urban motorway's morning for a French one's; the TMJA is an annual mean of every
#: day, a workday's traffic is somewhat above it, which the share does not correct).
PEAK_SHARE_OF_DAY = 0.137
#: A TMJA section is matched to a gateway when the gateway's point lies within this distance of the section's axis
#: (the axis runs between the two carriageways; the point is on one of them).
TMJA_MATCH_M = 100.0
ATTRACT_NRES = 0.6
CELL_M = 1000.0
#: Zone points (S282): a zone's side is split into this many blocks, each with buildings a point. 2: 500 m blocks, at
#: most four points a zone. Finer blocks place the trips more finely at a higher run time, the route update and the
#: itinerary choice growing with the distinct trip ends, for the same mode shares (S220).
SPLIT = 2
MIN_NODES = 5
LAT_M = 110_574.0
INTRA_KM = 0.52  # the mean distance between two random points in a 1 km square

#: Per city: the survey's numbers (with their sources) and the recipe's choices.
CITY = {
    "paris": dict(
        # EGT H2020 (Île-de-France Mobilités, OMNIL), as the City of Paris reports it: trips a weekday Paris-Paris
        # 8.145 M, Paris-suburbs 4.186 M; the latter 20 % by car, 71 % by transit.
        # https://cdn.paris.fr/paris/2023/09/27/paris_ra2022-deplacements-en-ile-de-france-2-pages-sYJh.pdf
        internal_daily=8_145_000, exchange_daily=4_186_000, exchange_car=0.20, exchange_transit=0.71,
        internal_car=0.04,  # Paris-Paris by car, 4 %, the same source
        city_residents=None,  # the study area is the city
        # Utility vehicles 18 % of Paris's traffic, taxis 5 %: Plan local de mobilité, diagnostic (2024), part 1,
        # https://cdn.paris.fr/paris/2024/03/29/partie-1-diagnostic-vf-web-qwWQ.pdf
        goods_share=0.18, taxi_share=0.05,
        # S268: the study area holds the Périphérique; its motorway gateways' flows by the Dutch ratio (0.35,
        # TRANSFER_RATIO; S282), the rest the default share.
        counts="transfer",
        # The window's share of the day: 12 % (S265: capped at 12 %; *an assumption*: no
        # Paris figure was found; Metz's 07:30-08:30 alone holds 10 %). Paris's trips inside the city include many
        # midday walks, so its morning peaks less than the French surveys' 16 % in 07:00-09:00.
        window_share=0.12,
        # Parisians: 12 km a day (crow-fly) over 4.34 trips: 2.8 km a trip (DRIEA on EGT 2018; OMNIL, Paris EGT H2020).
        mean_km=2.8,
        model="gravity",
    ),
    "lyon": dict(
        # EMD 2015 of the Lyon area (Sytral, UrbaLyon): Scot residents' trips within Lyon-Villeurbanne 1.6 M a day,
        # 17 % by car; 54 % of the Scot's 4.293 M daily trips involve Lyon-Villeurbanne, so 0.718 M are exchanges,
        # 58 % by car (47 % drivers); transit's share of them taken as 30 % (*an assumption*: the TC urban gain).
        # https://www.urbalyon.org/sites/default/files/productions/publications/imports/2019-11/m-emd-agglo.pdf
        internal_daily=1_600_000, exchange_daily=718_000, exchange_car=0.58, exchange_transit=0.30,
        internal_car=0.17,
        city_residents=662_000,  # Lyon 513 000 + Villeurbanne 149 000, the survey's centre (INSEE, 2015)
        goods_share=0.18, taxi_share=0.0,  # Paris's share of utility vehicles (*an assumption* for Lyon)
        # The Métropole's residents: 17 km a day over 3.38 trips: 5.0 km; the centre's trips are shorter (*an
        # assumption*: 3.0 km).
        mean_km=3.0,
        model="gravity",
        counts="transfer",  # S282: a share of capacity, 0.35 (TRANSFER_RATIO)
    ),
    "delft": dict(
        # The Dutch travel survey (ODiN 2023, CBS): 2.718 trips a person a day (6 and over) over all days,
        # https://www.cbs.nl/nl-nl/longread/rapportages/2024/onderweg-in-nederland--odin---2023-plausibiliteitsrapportage/4-gemiddeld-aantal-verplaatsingen-per-persoon-per-dag
        # S282: the cases are a workday morning, so the workday rate: 2.862, the mean of Monday-Friday
        # (2.73, 2.83, 2.92, 2.90, 2.93) in CBS StatLine 84702NED (2023, the Netherlands, all motives, 6 and over),
        # https://opendata.cbs.nl/ODataApi/odata/84702NED
        # Delft's own split is not published there: 55 % of its residents' trips stay in the city and as many trips
        # come in as go out (*assumptions*); the exchanges' modes as Amsterdam's visitors' (45 % car, 30 % transit).
        trips_per_resident=2.862, internal_fraction=0.55,
        exchange_car=0.45, exchange_transit=0.30, internal_car=0.19,
        city_residents=None,
        goods_share=0.18, taxi_share=0.0,  # Paris's share of utility vehicles (*an assumption*)
        mean_km=2.0,  # a small city's crow-fly trip (*an assumption*)
        model="gravity",
        counts="inweva",
    ),
    "amsterdam": dict(
        # Staat van de Stad 2019 (data of 2017): Amsterdammers (12+) 1.8 M trips a workday, 19 % by car; visitors
        # (commuters among them) 720 000 trips to, from and within the city, about 45 % by car (Monitor Autoluw
        # 2019's zero measurement); transit's share of the visitors' taken as 30 % (*an assumption*).
        # https://openresearch.amsterdam/image/2020/7/15/6_mobiliteit_2019_staat_van_de_stad_x_4.pdf
        # https://openresearch.amsterdam/image/2021/3/23/monitor_autoluw_indicatoren_2019.pdf
        internal_daily=1_800_000, exchange_daily=720_000, exchange_car=0.45, exchange_transit=0.30,
        internal_car=0.19,
        city_residents=854_000,  # Amsterdam, 1 January 2017 (CBS)
        goods_share=0.18, taxi_share=0.0,  # Paris's share of utility vehicles (*an assumption*)
        # Amsterdammers: 11.1 M km a workday over 1.8 M trips: 6.2 km by road and track; crow-fly about 4.5 km
        # (*an assumption*: a detour factor of 1.35).
        mean_km=4.5,
        model="gravity",
        counts="inweva",
    ),
}
MAIN = {"motorway", "trunk", "primary", "secondary"}
CLASSES = list(ms.LINK_CLASSES)


def haversine_km(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    """Great-circle distances in km between each (lon, lat) point of ``a`` and each of ``b``."""
    lon1, lat1 = np.radians(a[:, 0])[:, None], np.radians(a[:, 1])[:, None]
    lon2, lat2 = np.radians(b[:, 0])[None, :], np.radians(b[:, 1])[None, :]
    h = np.sin((lat2 - lat1) / 2) ** 2 + np.cos(lat1) * np.cos(lat2) * np.sin((lon2 - lon1) / 2) ** 2
    return 2 * 6371.0088 * np.arcsin(np.sqrt(h))


def inside(points: np.ndarray, region) -> np.ndarray:
    """Whether each (lon, lat) is in the case's region: a box (4 numbers) or a polygon."""
    if len(region) == 4 and not isinstance(region[0], (list, tuple)):
        x0, y0, x1, y1 = region
        return (points[:, 0] >= x0) & (points[:, 0] <= x1) & (points[:, 1] >= y0) & (points[:, 1] <= y1)
    poly = np.asarray(region, dtype=float)
    x, y = points[:, 0], points[:, 1]
    out = np.zeros(len(points), dtype=bool)
    j = len(poly) - 1
    for i in range(len(poly)):  # ray casting
        xi, yi, xj, yj = poly[i, 0], poly[i, 1], poly[j, 0], poly[j, 1]
        cross = ((yi > y) != (yj > y)) & (x < (xj - xi) * (y - yi) / (yj - yi + 1e-300) + xi)
        out ^= cross
        j = i
    return out


def zones(net, region):
    """Step 1: the 1 km grid zones of the first recipe, and the grid's frame."""
    coords, offsets = np.asarray(net.link_geometry()[0]), np.asarray(net.link_geometry()[1], dtype=np.int64)
    frm, to = np.asarray(net.link_from()), np.asarray(net.link_to())
    drivable = np.asarray(net.link_drivable())
    pos = np.full((net.node_count, 2), np.nan)
    pos[frm] = coords[offsets[:-1]]
    pos[to] = coords[offsets[1:] - 1]
    lon0, lat0 = np.nanmin(pos[:, 0]), np.nanmin(pos[:, 1])
    lon_m = 111_320.0 * math.cos(math.radians(float(np.nanmean(pos[:, 1]))))
    nodes = np.unique(np.concatenate([frm[drivable], to[drivable]]))
    nodes = nodes[inside(pos[nodes], region)]
    ix = np.floor((pos[nodes, 0] - lon0) * lon_m / CELL_M).astype(np.int64)
    iy = np.floor((pos[nodes, 1] - lat0) * LAT_M / CELL_M).astype(np.int64)
    keys = iy * 10_000 + ix
    kept, counts = np.unique(keys, return_counts=True)
    kept = kept[counts >= MIN_NODES]
    points = np.zeros((len(kept), 2))
    for i, k in enumerate(kept):
        members = nodes[keys == k]
        mean = pos[members].mean(axis=0)
        d = (pos[members, 0] - mean[0]) ** 2 + (pos[members, 1] - mean[1]) ** 2
        points[i] = pos[members[int(np.argmin(d))]]
    ids = [f"r{int(k) // 10_000:02d}c{int(k) % 10_000:02d}" for k in kept]
    frame = (lon0, lat0, lon_m)
    return ids, points, kept, frame, pos


def weights(case: str, region, kept, frame, points):
    """Step 2: residents and non-residential volume per zone, from the GHSL cut-out."""
    g = np.load(DATA / "cases" / case / "raw" / "ghsl_2020.npz")
    pop, nres, lon0g, lat0g, step = g["pop"], g["nres"], float(g["lon0"]), float(g["lat0"]), float(g["step"])
    rows, cols = np.nonzero((pop > 0) | (nres > 0))
    lon = lon0g + (cols + 0.5) * step
    lat = lat0g - (rows + 0.5) * step
    centres = np.column_stack([lon, lat])
    keep = inside(centres, region)
    centres, p, v = centres[keep], pop[rows[keep], cols[keep]], nres[rows[keep], cols[keep]]
    lon0, lat0, lon_m = frame
    ix = np.floor((centres[:, 0] - lon0) * lon_m / CELL_M).astype(np.int64)
    iy = np.floor((centres[:, 1] - lat0) * LAT_M / CELL_M).astype(np.int64)
    index = {int(k): i for i, k in enumerate(kept)}
    zone = np.array([index.get(int(k), -1) for k in iy * 10_000 + ix])
    lost = zone < 0
    if lost.any():  # a cell in no kept zone: the nearest zone's, within 1.5 km
        d = haversine_km(centres[lost], points)
        nearest = d.argmin(axis=1)
        near = d[np.arange(len(nearest)), nearest] <= 1.5
        zone[np.flatnonzero(lost)[near]] = nearest[near]
    ok = zone >= 0
    residents = np.bincount(zone[ok], weights=p[ok], minlength=len(kept))
    volume = np.bincount(zone[ok], weights=v[ok], minlength=len(kept))
    cells = (centres[ok], zone[ok], p[ok], v[ok])
    return residents, volume, {"residents_in_area": float(p.sum()), "residents_in_zones": float(residents.sum()),
                               "nres_m3_in_zones": float(volume.sum())}, cells


def zone_points(cells, ids, residents, volume, frame) -> list[list]:
    """Step 2b: each zone's points, one per block of ``CELL_M / SPLIT`` metres that holds residents or
    non-residential volume: the block's GHSL cells' centres averaged by (residents + volume, each as a share of the
    area's), the residents as the origin weight, and the attraction rule's share within the zone (``ATTRACT_NRES`` of
    the volume, the rest of the residents) as the destination weight. Rows of ``zone_points.csv``.
    """
    centres, zone, p, v = cells
    lon0, lat0, lon_m = frame
    block = CELL_M / SPLIT
    bx = np.floor((centres[:, 0] - lon0) * lon_m / block).astype(np.int64)
    by = np.floor((centres[:, 1] - lat0) * LAT_M / block).astype(np.int64)
    keys = zone * 1_000_000_000 + by * 100_000 + bx
    w = p / max(p.sum(), 1e-9) + v / max(v.sum(), 1e-9)
    rows = []
    for key in np.unique(keys):
        m = keys == key
        z = int(key // 1_000_000_000)
        res, vol = float(p[m].sum()), float(v[m].sum())
        if res <= 0 and vol <= 0:
            continue
        ww = w[m] if w[m].sum() > 0 else np.ones(int(m.sum()))
        point = (centres[m] * ww[:, None]).sum(axis=0) / ww.sum()
        dest = ATTRACT_NRES * vol / max(volume[z], 1e-9) + (1 - ATTRACT_NRES) * res / max(residents[z], 1e-9)
        rows.append([ids[z], f"{point[0]:.7f}", f"{point[1]:.7f}", f"{res:.1f}", f"{dest:.6f}"])
    return rows


def radiation(points, opportunity, total_by_origin):
    """Step 4: the radiation model between zones, normalised for a finite area (Masucci et al. 2013).

    ``T_ij ∝ m_i n_j / ((m_i + s_ij)(m_i + n_j + s_ij))``, with m, n and s in opportunities (the attraction mass):
    ``s_ij`` the opportunities of the zones closer to i than j is, i and j left out. Each origin's row is scaled to
    ``total_by_origin`` (its trips between zones).
    """
    d = haversine_km(points, points)
    np.fill_diagonal(d, INTRA_KM)
    n = len(points)
    total = opportunity.sum()
    t = np.zeros((n, n))
    for i in range(n):
        order = np.argsort(d[i], kind="stable")
        before = np.cumsum(opportunity[order]) - opportunity[order]  # the opportunities closer than each zone
        s = np.empty(n)
        s[order] = before
        s = np.maximum(s - opportunity[i], 0.0)  # i itself left out
        mi = max(opportunity[i], 1e-9)
        p = mi * opportunity / ((mi + s) * (mi + opportunity + s))
        p[i] = 0.0
        t[i] = p / max(1.0 - mi / total, 1e-9)
    t = t / np.maximum(t.sum(axis=1, keepdims=True), 1e-300) * total_by_origin[:, None]
    return t, d


def furness(seed, productions, attractions, iterations=200):
    """Scale ``seed`` to the row sums ``productions`` and the column sums ``attractions`` (iterative proportional fitting)."""
    t = seed.copy()
    for _ in range(iterations):
        t *= (productions / np.maximum(t.sum(1), 1e-300))[:, None]
        t *= (attractions / np.maximum(t.sum(0), 1e-300))[None, :]
    return t


def gravity(points, productions, attractions, mean_km):
    """Step 4, the alternative: doubly constrained, exponential decay fitted to a mean distance (trips within zones in)."""
    d = haversine_km(points, points)
    np.fill_diagonal(d, INTRA_KM)
    lo, hi = 0.01, 5.0
    for _ in range(50):
        beta = math.sqrt(lo * hi)
        t = furness(np.exp(-beta * d), productions, attractions)
        mean = float((t * d).sum() / t.sum())
        lo, hi = (lo, beta) if mean < mean_km else (beta, hi)
    return t, d, beta


def polygon(region) -> np.ndarray:
    """The region as an array of (lon, lat) vertices; a box becomes its four corners."""
    if len(region) == 4 and not isinstance(region[0], (list, tuple)):
        x0, y0, x1, y1 = region
        return np.array([[x0, y0], [x1, y0], [x1, y1], [x0, y1]], dtype=float)
    return np.asarray(region, dtype=float)


def net_green(net) -> float:
    """The network's green fraction at signals (its `signal_green_fraction` times `green_fraction_factor`)."""
    o = ms.network_options(net)
    return float(o["signal_green_fraction"]) * float(o.get("green_fraction_factor", 1.0))


def gateways(net, region, min_angle_deg: float = 30.0):
    """Step 5: where traffic from outside enters and leaves the area.

    A gateway is a motorway, trunk, primary or secondary link that crosses the area's edge at an angle of at least
    ``min_angle_deg`` (a road running along the edge, as Paris's Périphérique does in and out of its polygon, crosses
    at shallow angles and is no gateway): inbound at the link's outside end when it runs inward, outbound when it runs
    outward. Weighted by the link's capacity; merged within 300 m.
    """
    coords, offsets = np.asarray(net.link_geometry()[0]), np.asarray(net.link_geometry()[1], dtype=np.int64)
    start, end = coords[offsets[:-1]], coords[offsets[1:] - 1]
    cls = np.array([CLASSES[c].replace("_link", "") for c in net.link_class()])
    main = np.isin(cls, list(MAIN)) & np.asarray(net.link_drivable())
    a, b = inside(start, region), inside(end, region)
    cap = np.asarray(net.link_capacity_pcu_h())
    poly = polygon(region)
    lat = float(np.mean(poly[:, 1]))
    sx, sy = 111.320 * math.cos(math.radians(lat)), 110.574
    edges = [(poly[i] * [sx, sy], poly[(i + 1) % len(poly)] * [sx, sy]) for i in range(len(poly))]

    def angle(link: int) -> float:
        p, q = start[link] * [sx, sy], end[link] * [sx, sy]
        u = q - p
        best = None
        for e0, e1 in edges:  # the edge the link crosses: the one its segment intersects
            v = e1 - e0
            den = u[0] * v[1] - u[1] * v[0]
            if abs(den) < 1e-12:
                continue
            t = ((e0[0] - p[0]) * v[1] - (e0[1] - p[1]) * v[0]) / den
            w = ((e0[0] - p[0]) * u[1] - (e0[1] - p[1]) * u[0]) / den
            if 0 <= t <= 1 and 0 <= w <= 1:
                cos = abs(u @ v) / (np.linalg.norm(u) * np.linalg.norm(v) + 1e-12)
                best = math.degrees(math.acos(min(1.0, cos)))
        return 0.0 if best is None else best

    # The capacity an entry really has: its link's, times the green fraction where a signal lies within 300 m inside
    # (along the widest road ahead): a porte of Paris meets its signal a few metres in.
    frm_all, to_all = np.asarray(net.link_from()), np.asarray(net.link_to())
    stopped = np.asarray(net.link_signalised())  # S282: the links a signal stops (a node flag marks every signal)
    length = np.asarray(net.link_length_m())
    green = net_green(net)
    ahead: dict[int, list[int]] = {}
    for link in np.flatnonzero(np.asarray(net.link_drivable())):
        ahead.setdefault(int(frm_all[link]), []).append(int(link))

    def effective(link: int) -> float:
        travelled, here = 0.0, link
        for _ in range(20):
            if stopped[here]:
                return float(cap[link] * green)
            travelled += length[here]
            nxt = [k for k in ahead.get(int(to_all[here]), []) if to_all[k] != frm_all[here]]
            if travelled >= 300.0 or not nxt:
                break
            here = max(nxt, key=lambda k: cap[k])
        return float(cap[link])

    out = {"in": [], "out": []}
    for link in np.flatnonzero(main & ~a & b):
        if angle(link) >= min_angle_deg:
            out["in"].append((start[link], cap[link], cls[link], effective(link), int(link)))
    for link in np.flatnonzero(main & a & ~b):
        if angle(link) >= min_angle_deg:
            out["out"].append((end[link], cap[link], cls[link], float(cap[link]), int(link)))
    # A main road that ends within 500 m of the edge, where the extract itself was cut (Amsterdam's A1 stops just
    # inside its box): its dead end is a gateway too.
    frm, to = np.asarray(net.link_from()), np.asarray(net.link_to())
    neighbours: dict[int, set] = {}
    for x, y in zip(frm[main], to[main], strict=False):
        neighbours.setdefault(int(x), set()).add(int(y))
    every = np.asarray(net.link_drivable())
    degree: dict[int, set] = {}
    for x, y in zip(frm[every], to[every], strict=False):
        degree.setdefault(int(x), set()).add(int(y))
        degree.setdefault(int(y), set()).add(int(x))
    near_edge_start = edge_km(start, region) <= 0.5
    near_edge_end = edge_km(end, region) <= 0.5
    ends = {"in": [], "out": []}
    for link in np.flatnonzero(main):
        if len(degree.get(int(frm[link]), ())) == 1 and near_edge_start[link]:
            ends["in"].append((start[link], cap[link], cls[link], effective(link), int(link)))
        if len(degree.get(int(to[link]), ())) == 1 and near_edge_end[link]:
            ends["out"].append((end[link], cap[link], cls[link], float(cap[link]), int(link)))
    # A motorway or trunk carriageway that begins (ends) within 1.5 km of the edge with no carriageway of its class
    # before (after) it: where the extract stops a motorway at its last interchange (Amsterdam's A1).
    raw = np.array([CLASSES[c] for c in net.link_class()])
    carriage = every & np.isin(raw, ["motorway", "trunk"])
    has_in = np.zeros(net.node_count, dtype=bool)
    has_out = np.zeros(net.node_count, dtype=bool)
    has_in[to[carriage]] = True
    has_out[frm[carriage]] = True
    starts_far = edge_km(start, region) <= 1.5
    ends_far = edge_km(end, region) <= 1.5
    for link in np.flatnonzero(carriage):
        if not has_in[frm[link]] and starts_far[link]:
            ends["in"].append((start[link], cap[link], cls[link], effective(link), int(link)))
        if not has_out[to[link]] and ends_far[link]:
            ends["out"].append((end[link], cap[link], cls[link], float(cap[link]), int(link)))

    def merge(items):  # one gateway per 300 m, its capacities summed
        groups: list[list] = []
        for p, c, k, e, link in sorted(items, key=lambda x: -x[1]):
            for g in groups:
                if haversine_km(np.array([p]), np.array([g[0]]))[0, 0] < 0.3:
                    g[1] += c
                    g[3] += e
                    break
            else:
                groups.append([p, c, k, e, link])  # the group's widest link stands for it
        return groups

    merged = {}
    for way in ("in", "out"):
        crossings = merge(out[way])
        extra = [e for e in merge(ends[way])
                 if not any(haversine_km(np.array([e[0]]), np.array([g[0]]))[0, 0] < 0.3 for g in crossings)]
        merged[way] = crossings + extra
    return merged


def edge_km(points: np.ndarray, region) -> np.ndarray:
    """Each point's distance to the region's edge, in km (a box's sides, or a polygon's)."""
    if len(region) == 4 and not isinstance(region[0], (list, tuple)):
        x0, y0, x1, y1 = region
        poly = np.array([[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
    else:
        poly = np.asarray(region, dtype=float)
    lat = float(np.mean(poly[:, 1]))
    sx, sy = 111.320 * math.cos(math.radians(lat)), 110.574
    p = points * [sx, sy]
    a, b = poly * [sx, sy], np.roll(poly, -1, axis=0) * [sx, sy]
    best = np.full(len(points), np.inf)
    for (ax, ay), (bx, by) in zip(a, b, strict=False):
        dx, dy = bx - ax, by - ay
        u = np.clip(((p[:, 0] - ax) * dx + (p[:, 1] - ay) * dy) / max(dx * dx + dy * dy, 1e-12), 0, 1)
        best = np.minimum(best, np.hypot(p[:, 0] - ax - u * dx, p[:, 1] - ay - u * dy))
    return best


def rail_stations(case: str, region, metro_edge_km: float = 1.0):
    """Step 5: where transit riders from outside start, weighted by departures: the area's rail stations (GTFS route
    types 2, 100-199), and its metro and tram stops (1, 400-499; 0, 900-999) within ``metro_edge_km`` of its edge
    (where lines from the suburbs come in; a feed without the national trains, Lyon's, has only these).
    """
    with zipfile.ZipFile(BUNDLE / case / "gtfs.zip") as z:
        def table(name):
            with z.open(name) as f:
                return list(csv.DictReader(line.decode("utf-8-sig") for line in f))
        kind = {r["route_id"]: int(r["route_type"]) for r in table("routes.txt")}
        trip_kind = {r["trip_id"]: kind.get(r["route_id"], 3) for r in table("trips.txt")}
        stops = {r["stop_id"]: (float(r["stop_lon"]), float(r["stop_lat"]), r.get("parent_station") or r["stop_id"])
                 for r in table("stops.txt") if r.get("stop_lat")}
        deps: dict[str, list] = {}
        with z.open("stop_times.txt") as f:
            for r in csv.DictReader(line.decode("utf-8-sig") for line in f):
                k = trip_kind.get(r["trip_id"], 3)
                rail = k == 2 or 100 <= k < 200
                metro = k in (0, 1) or 400 <= k < 500 or 900 <= k < 1000
                s = stops.get(r["stop_id"])
                if s and (rail or metro):
                    e = deps.setdefault(s[2], [s[0], s[1], 0, False])
                    e[2] += 1
                    e[3] = e[3] or rail
    if not deps:
        return []
    pts = np.array([[v[0], v[1]] for v in deps.values()])
    ok = inside(pts, region)
    near = edge_km(pts, region) <= metro_edge_km
    return [(pts[i], float(v[2]), sid) for i, (sid, v) in enumerate(deps.items()) if ok[i] and (v[3] or near[i])]


def write(folder: Path, zone_rows, od_rows, classes_rows, meta, point_rows) -> None:
    """Write the morning's tables (zones, zone points, OD matrix, classes) and ``morning.json`` to ``folder``."""
    folder.mkdir(parents=True, exist_ok=True)
    with (folder / "zones.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["zone_id", "x_coord", "y_coord", "residents", "nres_m3", "spread_m", "kind"])
        w.writerows(zone_rows)
    with (folder / "zone_points.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["zone_id", "x_coord", "y_coord", "weight_origin", "weight_destination"])
        w.writerows(point_rows)
    with (folder / "od.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["origin_zone", "destination_zone", "trips", "user_class", "mode"])
        w.writerows(od_rows)
    with (folder / "classes.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerows(classes_rows)
    (folder / "morning.json").write_text(json.dumps(meta, indent=1))


def _xy(points: np.ndarray, lat0: float) -> np.ndarray:
    return np.asarray(points) * [111_320.0 * math.cos(math.radians(lat0)), LAT_M]


def main_link(net, g) -> tuple[bool, float]:
    """Whether a gateway's widest link is a carriageway (not a slip road) of a counted class, and its own capacity."""
    raw = CLASSES[int(net.link_class()[g[4]])]
    return raw in COUNTED, float(np.asarray(net.link_capacity_pcu_h())[g[4]])


def inweva_counts(case: str, net, gates) -> dict:
    """Step 5b: each motorway or trunk gateway's count in 07:00-09:00, from Rijkswaterstaat's INWEVA.

    A gateway whose widest link is a motorway or trunk carriageway (not a slip road) is matched to the nearest
    main-carriageway segment (``bnsubsrt`` HR) running the same way within 40 m; a segment counts for one gateway only,
    its nearest. INWEVA lists each carriageway apart, so the nearest parallel one is the gateway's own. The morning
    peak's field is the period's total, not an hour's: its day periods add up to the whole day (``al_d`` 07-19 +
    ``al_a`` + ``al_n`` = ``al_e``), and an hourly reading would leave the rest of the day emptier than the night.
    Taken as 07:00-09:00 (*an assumption*: the layer's metadata names no hours).
    """
    path = DATA / "cases" / case / "raw" / "inweva-ochtendspits-2025.geojson"
    if not path.exists():
        return {}
    feats = [f for f in json.loads(path.read_text())["features"]
             if f["properties"].get("bnsubsrt_e") == "HR" and f["properties"].get("al_os_wr")]
    coords, offsets = np.asarray(net.link_geometry()[0]), np.asarray(net.link_geometry()[1], dtype=np.int64)
    lat0 = float(coords[:, 1].mean())
    lines = [_xy(np.array(f["geometry"]["coordinates"]), lat0) for f in feats]
    pairs = []  # (distance, gateway, feature)
    for gid, g in gates:
        carriageway, _ = main_link(net, g)
        if not carriageway:
            continue
        pts = _xy(coords[offsets[g[4]]:offsets[g[4] + 1]], lat0)
        u = pts[-1] - pts[0]
        for n, line in enumerate(lines):
            a, ab = line[:-1], line[1:] - line[:-1]
            d_min, seg = np.inf, None
            for q in pts:
                w = q - a
                tt = np.clip((w * ab).sum(1) / np.maximum((ab * ab).sum(1), 1e-9), 0, 1)
                d = np.hypot(*(w - ab * tt[:, None]).T)
                k = int(d.argmin())
                if d[k] < d_min:
                    d_min, seg = float(d[k]), ab[k]
            cos = float(u @ seg) / (np.linalg.norm(u) * np.linalg.norm(seg) + 1e-9)
            if d_min <= 40.0 and cos > 0.8:
                pairs.append((d_min, gid, n))
    out, used = {}, set()
    for d, gid, n in sorted(pairs):
        if gid in out or n in used:
            continue
        used.add(n)
        pr = feats[n]["properties"]
        out[gid] = (float(pr["al_os_wr"]), f"INWEVA {pr['vbn_oms_tx']} ({pr['vbn_id']}), {d:.0f} m")
    return out


def lambert93_to_wgs84(x: np.ndarray, y: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Lambert 93 (EPSG:2154, GRS80, two standard parallels) to longitude and latitude in degrees (the closed
    inverse of the conformal conic projection, EPSG guidance note 7-2, method 9802; checked against the IGN's
    reference point: 700 000, 6 600 000 -> 3° E, 46.5° N).
    """
    a, f = 6378137.0, 1 / 298.257222101
    e = math.sqrt(2 * f - f * f)
    lat0, lon0, lat1, lat2 = map(math.radians, (46.5, 3.0, 44.0, 49.0))
    x0, y0 = 700000.0, 6600000.0

    def m(phi):
        return math.cos(phi) / math.sqrt(1 - (e * math.sin(phi)) ** 2)

    def t(phi):
        return math.tan(math.pi / 4 - phi / 2) / ((1 - e * math.sin(phi)) / (1 + e * math.sin(phi))) ** (e / 2)

    n = (math.log(m(lat1)) - math.log(m(lat2))) / (math.log(t(lat1)) - math.log(t(lat2)))
    big_f = m(lat1) / (n * t(lat1) ** n)
    r0 = a * big_f * t(lat0) ** n
    dx, dy = np.asarray(x, dtype=float) - x0, r0 - (np.asarray(y, dtype=float) - y0)
    r = np.sign(n) * np.hypot(dx, dy)
    tt = (r / (a * big_f)) ** (1 / n)
    lon = np.arctan2(dx, dy) / n + lon0
    phi = np.pi / 2 - 2 * np.arctan(tt)
    for _ in range(6):
        phi = np.pi / 2 - 2 * np.arctan(tt * ((1 - e * np.sin(phi)) / (1 + e * np.sin(phi))) ** (e / 2))
    return np.degrees(lon), np.degrees(phi)


def tmja_counts(case: str, net, gates) -> dict:
    """Step 5b: each French motorway or trunk gateway's count in 07:00-09:00, from the national network's
    annual average daily traffic (TMJA 2019; ``TMJA``, ``sources/tmja/`` of the data root).

    A gateway whose widest link is a motorway or trunk carriageway (not a slip road) is matched to the nearest TMJA
    section whose axis passes within ``TMJA_MATCH_M`` of its point. The section's daily traffic counts both
    directions: the gateway's carriageway takes half of it, and ``PEAK_SHARE_OF_DAY`` of that in 07:00-09:00. A
    gateway on a road the national file does not hold (a declassified urban motorway) is left to the caller, which
    gives it the median share of capacity of the city's matched gateways.
    """
    if not TMJA_FILE.exists():
        raise SystemExit(f"{TMJA_FILE} is missing: see the README of these scripts")
    with TMJA_FILE.open(encoding="utf-8-sig", newline="") as f:
        rows = [r for r in csv.DictReader(f, delimiter=";") if r["TMJA"] and r["xD"] and r["xF"]]

    def num(s: str) -> float:
        return float(s.replace(",", "."))

    xd, yd = lambert93_to_wgs84([num(r["xD"]) for r in rows], [num(r["yD"]) for r in rows])
    xf, yf = lambert93_to_wgs84([num(r["xF"]) for r in rows], [num(r["yF"]) for r in rows])
    lat = float(np.mean([g[0][1] for _, g in gates]))
    sx, sy = 111_320.0 * math.cos(math.radians(lat)), LAT_M
    a = np.column_stack([xd * sx, yd * sy])
    b = np.column_stack([xf * sx, yf * sy])
    ab = b - a
    ab2 = np.maximum((ab**2).sum(axis=1), 1e-9)
    out = {}
    for gid, g in gates:
        carriageway, _ = main_link(net, g)
        if not carriageway:
            continue
        p = np.array([g[0][0] * sx, g[0][1] * sy])
        u = np.clip(((p - a) * ab).sum(axis=1) / ab2, 0.0, 1.0)
        d = np.hypot(*(a + u[:, None] * ab - p).T)
        k = int(np.argmin(d))
        if d[k] <= TMJA_MATCH_M:
            r = rows[k]
            daily = num(r["TMJA"])
            count = daily / 2.0 * PEAK_SHARE_OF_DAY  # one direction's 07:00-09:00 (the share is the two hours')
            out[gid] = (count, f"TMJA 2019 {r['route']} PR {r['prD']} ({daily:.0f} a day both ways), {d[k]:.0f} m")
    return out


def main() -> None:
    """Make one city's morning demand and write it (``morning2.py <case>``)."""
    p = argparse.ArgumentParser()
    p.add_argument("case")
    p.add_argument("--model", choices=["radiation", "gravity"])
    p.add_argument("--out", help="the folder to write (default: cases/<case>/demand/morning2)")
    p.add_argument("--transfer-ratio", type=float, default=TRANSFER_RATIO,
                   help="a city with counts \"transfer\": the gateways' share of capacity (for tests)")
    args = p.parse_args()
    case, cfg = args.case, CITY[args.case]
    model = args.model or cfg["model"]
    info = json.loads((BUNDLE / case / "case.json").read_text())
    region = _cases._region(info["network"]["region"] if "network" in info else info["region"])
    net = ms.network_read_osm(str(BUNDLE / case / "network.osm.pbf"), region=region)
    ids, points, kept, frame, _ = zones(net, region)
    residents, volume, wmeta, cells = weights(case, region, kept, frame, points)
    point_rows = zone_points(cells, ids, residents, volume, frame)
    scale = 1.0 if cfg["city_residents"] is None else wmeta["residents_in_zones"] / cfg["city_residents"]
    window_share = cfg.get("window_share", WINDOW_SHARE)
    if "trips_per_resident" in cfg:  # a city without its own survey totals: the national rate (Delft)
        daily = wmeta["residents_in_zones"] * cfg["trips_per_resident"]
        cfg = {**cfg, "internal_daily": daily * cfg["internal_fraction"],
               "exchange_daily": 2 * daily * (1 - cfg["internal_fraction"])}
    internal = cfg["internal_daily"] * scale * window_share
    productions = residents / residents.sum() * internal
    attract = ATTRACT_NRES * volume / volume.sum() + (1 - ATTRACT_NRES) * residents / residents.sum()
    attractions = attract * internal

    grav, d, beta = gravity(points, productions, attractions, cfg["mean_km"])
    within = np.diag(grav).copy()  # trips within a zone, as the fitted gravity model has them
    rad, _ = radiation(points, attract * residents.sum(), productions - within)
    rad = furness(rad + np.diag(within), productions, attractions)
    def stats(t):
        within = float(np.trace(t))
        return {"mean_km_all": float((t * d).sum() / t.sum()), "within_zone_share": within / float(t.sum()),
                "share_under_3km": float(t[d < 3].sum() / t.sum())}
    rad_stats, grav_stats = stats(rad), stats(grav)
    chosen = (rad if model == "radiation" else grav).copy()  # trips within a zone kept (S282)
    # From outside.
    exchange = cfg["exchange_daily"] * window_share
    cars = exchange * cfg["exchange_car"] * CAR_FACTOR / OCCUPANCY
    through = cfg.get("through", THROUGH)
    riders = exchange * cfg["exchange_transit"]
    gw = gateways(net, region)
    stations = rail_stations(case, region)
    # Gravity decay for the gateways' destinations (fitted above, the same for both models).
    def to_zones(origins_pts, weights_, total, toward=True):
        dd = haversine_km(np.array(origins_pts), points)
        f = np.exp(-beta * dd) * attract[None, :]
        f /= f.sum(axis=1, keepdims=True)
        w = np.asarray(weights_, dtype=float)
        return f * (w / w.sum() * total)[:, None]
    zone_rows = [[z, f"{p_[0]:.7f}", f"{p_[1]:.7f}", f"{r:.1f}", f"{v:.0f}", "", "zone"]
                 for z, p_, r, v in zip(ids, points, residents, volume, strict=False)]
    od = []
    for i, j in zip(*np.nonzero(chosen >= 1e-3), strict=False):
        od.append([ids[i], ids[j], f"{chosen[i, j]:.4f}", "", ""])
    gin = [(f"gi{k:03d}", g) for k, g in enumerate(gw["in"])]
    gout = [(f"go{k:03d}", g) for k, g in enumerate(gw["out"])]
    for gid, g in gin + gout:
        zone_rows.append([gid, f"{g[0][0]:.7f}", f"{g[0][1]:.7f}", "", "", "0", f"gateway {g[2]}"])
    # Gateway metering: each gateway's busiest hour at most GATEWAY_USE of its capacity; the rest to gateways with
    # room, by capacity; what fits nowhere left out (it queues upstream, outside the area).
    def meter(capacity, total):
        cap = np.asarray(capacity, dtype=float) * GATEWAY_USE / PEAK_HOUR_OF_WINDOW
        got = np.zeros(len(cap))
        left = total
        for _ in range(50):
            room = cap - got
            open_ = room > 1e-9
            if left <= 1e-9 or not open_.any():
                break
            share = np.where(open_, cap, 0.0) / cap[open_].sum() * left
            add = np.minimum(share, room)
            got += add
            left -= add.sum()
        return got, left
    # Exchange cars by gateway capacity (metered); through traffic per gateway (S268): a motorway or trunk gateway's
    # count less its exchange cars (the count holds both), from INWEVA where it has one, else the Dutch ratio where the
    # city has no counts (`counts` "transfer"); any other gateway the share `through` of its exchange.
    in_ex, in_left = meter([g[3] for _, g in gin], cars * INBOUND)
    out_ex, out_left = meter([g[3] for _, g in gout], cars * (1 - INBOUND))
    counted = inweva_counts(case, net, gin + gout) if cfg.get("counts") == "inweva" else {}
    ratio_city = None
    if cfg.get("counts") == "tmja":  # S282: the matched gateways' median share of capacity for the unmatched ones
        counted = tmja_counts(case, net, gin + gout)
        ratios = [counted[gid][0] / 2.0 / main_link(net, g)[1] for gid, g in gin + gout if gid in counted]
        if not ratios:
            raise SystemExit("no gateway matched a TMJA section: check TMJA_FILE and TMJA_MATCH_M")
        ratio_city = float(np.median(ratios))
    if cfg.get("counts") in ("transfer", "tmja"):
        ratio = args.transfer_ratio if cfg.get("counts") == "transfer" else ratio_city
        label = "transfer" if cfg.get("counts") == "transfer" else "the city's counted gateways' median"
        for gid, g in gin + gout:
            carriageway, capacity = main_link(net, g)
            if carriageway and gid not in counted:
                counted[gid] = (ratio * capacity * 2.0, f"{label}: {ratio:.3f} of capacity, 07-09")

    def through_of(gates, ex, share):
        """Each gateway's through traffic, from its count (and 0 from the share) and from the share (0 if counted)."""
        by_count, by_share = np.zeros(len(gates)), np.zeros(len(gates))
        for k, (gid, g) in enumerate(gates):
            if gid in counted:
                # Metered as every gateway is: its busiest hour at most GATEWAY_USE of its carriageway's capacity;
                # the rest of the count queues outside the area.
                most = GATEWAY_USE * main_link(net, g)[1] / PEAK_HOUR_OF_WINDOW
                by_count[k] = max(0.0, min(counted[gid][0] / SHARE_07_09, most) - ex[k])
            else:
                by_share[k] = ex[k] * share
        return by_count, by_share

    rows_c, rows_s = through_of(gin, in_ex, through / INBOUND)
    cols_c, cols_s = through_of(gout, out_ex, through / (1 - INBOUND))
    tin = to_zones([g[0] for _, g in gin], in_ex, float(in_ex.sum()))
    for a, (gid, _) in enumerate(gin):
        for j in np.flatnonzero(tin[a] >= 1e-3):
            od.append([gid, ids[j], f"{tin[a, j]:.4f}", "from_outside_car", "car"])
    tout = to_zones([g[0] for _, g in gout], out_ex, float(out_ex.sum()))
    for a, (gid, _) in enumerate(gout):
        for j in np.flatnonzero(tout[a] >= 1e-3):
            od.append([ids[j], gid, f"{tout[a, j]:.4f}", "from_outside_car", "car"])
    # Through: gateway in → gateway out, at least 3 km apart, the farther the likelier (by the distance squared:
    # through traffic crosses the area; S268). Counted through traffic goes between counted gateways only (it stays on
    # the motorways); the shared kind between any. Each kind's two sides scaled to their mean (a count in and a count
    # out of one area need not balance), then balanced (Furness).
    pin, pout = np.array([g[0] for _, g in gin]), np.array([g[0] for _, g in gout])
    dd = haversine_km(pin, pout)
    seed = (dd >= 3.0) * dd**2 + 1e-12

    def balance(rows, cols):
        total = (rows.sum() + cols.sum()) / 2
        if total <= 0 or rows.sum() <= 0 or cols.sum() <= 0:
            return np.zeros((len(rows), len(cols))), 0.0
        live = np.outer(rows > 0, cols > 0)
        return furness(seed * live + 1e-12 * live, rows / rows.sum() * total, cols / cols.sum() * total), total

    f_c, total_c = balance(rows_c, cols_c)
    f_s, total_s = balance(rows_s, cols_s)
    f, total_t = f_c + f_s, total_c + total_s
    for a, b in zip(*np.nonzero(f >= 1e-3), strict=False):
        od.append([gin[a][0], gout[b][0], f"{f[a, b]:.4f}", "from_outside_car", "car"])
    # Transit riders from outside: from the rail stations, by departures, to the attractions.
    sids = [(f"st{k:03d}", s) for k, s in enumerate(stations)]
    for sid, s in sids:
        zone_rows.append([sid, f"{s[0][0]:.7f}", f"{s[0][1]:.7f}", "", "", "0", "station"])
    if sids:
        ts = to_zones([s[0] for _, s in sids], [s[1] for _, s in sids], riders)
        for a, (sid, _) in enumerate(sids):
            for j in np.flatnonzero(ts[a] >= 1e-3):
                od.append([sid, ids[j], f"{ts[a, j]:.4f}", "from_outside_transit", "transit"])
    # Goods vehicles (and taxis): a share of all road traffic, zone to zone by non-residential volume.
    through_shared = total_s
    private = internal * cfg["internal_car"] / OCCUPANCY + cars + through_shared
    share = (cfg["goods_share"] + cfg["taxi_share"]) * CAR_FACTOR
    goods = private * share / (1 - share)
    gd = haversine_km(points, points)
    g = np.outer(volume, volume) * np.exp(-beta * gd)
    np.fill_diagonal(g, 0.0)
    g = g / g.sum() * goods
    for i, j in zip(*np.nonzero(g >= 1e-3), strict=False):
        od.append([ids[i], ids[j], f"{g[i, j]:.4f}", "goods", "car"])
    fixed_names = {"from_outside_car", "from_outside_transit", "goods"}  # added below; not twice on a rebuild
    own = BUNDLE / case / "demand" / "classes.csv"
    source = own if own.exists() else BUNDLE / "amsterdam" / "demand" / "classes.csv"  # Delft: the example classes
    base_classes = [r for r in csv.reader(source.open()) if r[0] not in fixed_names]
    header = base_classes[0]
    if "has_transit_pass" in header:  # S282: the example classes hold no pass
        k = header.index("has_transit_pass")
        base_classes = [header] + [[*r[:k], "0", *r[k + 1:]] for r in base_classes[1:]]
    extra = []
    for name, modes, car in (("from_outside_car", "car", "1"), ("from_outside_transit", "transit", "0"),
                             ("goods", "car", "1")):
        row = {h: "" for h in header}
        row.update({"class": name, "share": "0", "modes": modes, "owns_car": car, "owns_bike": "0",
                    "has_transit_pass": "0"})  # S282: a pass is explicit and waives only the fare
        extra.append([row[h] for h in header])
    meta = {
        "what": "the starter case's morning, made by openmobisim's recipe: synthetic, not a forecast",
        "made_by": "cases/morning2.py",
        "model": model,
        "residents": wmeta, "scale_to_survey": scale,
        "window": {"start_s": WINDOW[0], "end_s": WINDOW[1], "share_of_day": window_share, "profile": "am_peak",
                   "zone_points": {"split": SPLIT, "block_m": CELL_M / SPLIT, "points": len(point_rows)}},  # S282
        "gateway_metering": {"inbound_left_out": in_left, "outbound_left_out": out_left,  # exchange cars only (S268)
                             "inbound_capacity_veh_h": float(sum(g[1] for _, g in gin)),
                             "inbound_effective_veh_h": float(sum(g[3] for _, g in gin)),
                             "outbound_capacity_veh_h": float(sum(g[1] for _, g in gout))},
        "window_trips": {"internal": internal, "internal_within_zones": float(np.trace(chosen)),
                         "exchange_cars": cars, "through_cars": float(total_t), "exchange_transit_riders": riders,
                         "goods_and_taxis": goods},
        "counted_gateways": [{"gateway": gid, "class": g[2], "capacity_veh_h": main_link(net, g)[1],
                              "count_07_09": counted[gid][0], "source": counted[gid][1],
                              "ratio_of_capacity": counted[gid][0] / 2.0 / max(main_link(net, g)[1], 1e-9)}
                             for gid, g in gin + gout if gid in counted],
        "radiation": rad_stats, "gravity": {**grav_stats, "beta_per_km": beta},
        "gateways": {"in": len(gin), "out": len(gout), "stations": len(sids)},
        "zones": len(ids), "od_rows": len(od), "assumptions": {"am_share": AM_SHARE, "occupancy": OCCUPANCY,
        "inbound": INBOUND, "through": through, "attract_nres": ATTRACT_NRES, "car_factor": CAR_FACTOR,
        "counts": cfg.get("counts"), "transfer_ratio": args.transfer_ratio if cfg.get("counts") == "transfer" else None,
        "peak_share_of_day": PEAK_SHARE_OF_DAY if cfg.get("counts") == "tmja" else None,
        "city_counted_ratio": ratio_city},
    }
    folder = Path(args.out) if args.out else DATA / "cases" / case / "demand" / "morning2"
    write(folder, zone_rows, od, base_classes + extra, meta, point_rows)
    # The gateways' counts as a table of their own (S268): for validation, and for other tools.
    with (folder / "gateway_counts.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["zone_id", "x_coord", "y_coord", "direction", "road_class", "capacity_veh_h", "vehicles_07_09",
                    "source"])
        for way, gates in (("in", gin), ("out", gout)):
            for gid, g in gates:
                if gid in counted:
                    w.writerow([gid, f"{g[0][0]:.7f}", f"{g[0][1]:.7f}", way, g[2], f"{main_link(net, g)[1]:.0f}",
                                f"{counted[gid][0]:.0f}", counted[gid][1]])
    print(json.dumps(meta, indent=1))


if __name__ == "__main__":
    main()
