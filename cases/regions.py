"""The test networks: which extract, which study area, and why.

Sizes are counted in **drivable links** (links a car may use). A city that maps
its sidewalks and crossings (New York, Singapore, Stockholm) has three to four
times as many links in all, none of which carries a car, so the total says more
about the mapping than about the cost of a run.

The extracts are the city files of BBBike (https://download.bbbike.org/osm/bbbike/,
OpenStreetMap data, ODbL); Andorra and Noord-Holland are Geofabrik's. Each lives in
its case's ``raw/`` folder (``cases/<key>/raw/`` under the data root). The figures
carry the attribution.
"""

#: Manhattan below 60th Street, derived from the data (``derive_manhattan_region.py``):
#: the convex hull of the streets of Manhattan island (bridges and tunnels removed
#: so the island stands alone) south of the 60th Street line, plus 40 m. It runs
#: through the middle of the rivers on the sides and along 60th Street on the north,
#: so the FDR Drive and the West Side Highway are inside, and the bridges and
#: tunnels are cut where they leave the island.
MANHATTAN_BELOW_60TH = [(-73.95855, 40.75911), (-73.96278, 40.76099), (-73.99256, 40.77353), (-73.99395, 40.7731), (-74.00149, 40.76551), (-74.00425, 40.76248), (-74.00715, 40.7547), (-74.00862, 40.75045), (-74.01161, 40.74063), (-74.01876, 40.70655), (-74.01496, 40.70102), (-74.01419, 40.70073), (-74.01417, 40.70073), (-74.01415, 40.70072), (-74.01413, 40.70072), (-74.00935, 40.70164), (-73.97954, 40.71102), (-73.97941, 40.71106), (-73.97928, 40.71111), (-73.97915, 40.71116), (-73.97903, 40.71122), (-73.97891, 40.71128), (-73.9788, 40.71134), (-73.97868, 40.71141), (-73.97858, 40.71148), (-73.97846, 40.71155), (-73.97836, 40.71163), (-73.97827, 40.71171), (-73.97817, 40.71179), (-73.97809, 40.71187), (-73.978, 40.71196), (-73.97792, 40.71205), (-73.97785, 40.71214), (-73.97777, 40.71223), (-73.97771, 40.71233), (-73.97765, 40.71243), (-73.97759, 40.71253), (-73.976, 40.71554), (-73.97539, 40.71675), (-73.97509, 40.71734), (-73.97504, 40.71743), (-73.975, 40.71753)]

#: Stockholm's inner city, roughly the congestion-charge cordon (Norrmalm,
#: Östermalm, Kungsholmen, Södermalm, Gamla stan, Djurgården). Drawn by hand from
#: the map; approximate.
STOCKHOLM_INNER_CITY = [
    (18.0100, 59.3360), (18.0400, 59.3480), (18.0800, 59.3520), (18.1150, 59.3480),
    (18.1300, 59.3350), (18.1150, 59.3200), (18.0950, 59.3080), (18.0600, 59.3080),
    (18.0250, 59.3110), (18.0050, 59.3200),
]  # fmt: skip

#: Amsterdam inside the A10 ring road (S195: the multimodal work's city). Drawn by hand
#: along the ring from the map; approximate. The extract is Geofabrik's Noord-Holland.
AMSTERDAM_A10 = [
    (4.842, 52.336), (4.842, 52.392), (4.868, 52.404), (4.930, 52.403),
    (4.966, 52.378), (4.957, 52.345), (4.915, 52.335),
]  # fmt: skip

#: Paris inside the Boulevard Périphérique (S204: a case study candidate): the
#: twenty arrondissements without the two woods, drawn by hand along the ring road from the
#: map, the Portes in order from Porte Maillot clockwise; approximate.
PARIS_INSIDE_PERIPHERIQUE = [
    (2.2790, 48.8780), (2.2950, 48.8870), (2.3130, 48.8945), (2.3310, 48.8990), (2.3590, 48.8985),
    (2.3860, 48.8975), (2.3985, 48.8850), (2.4105, 48.8690), (2.4135, 48.8465), (2.4020, 48.8300),
    (2.3780, 48.8205), (2.3590, 48.8165), (2.3265, 48.8215), (2.3010, 48.8285), (2.2880, 48.8330),
    (2.2690, 48.8350), (2.2560, 48.8390), (2.2550, 48.8480), (2.2630, 48.8570), (2.2700, 48.8660),
    (2.2750, 48.8730),
]

#: Paris with its Boulevard Périphérique and the Portes' junctions (S268): inside, the ring
#: met the city's streets at 16 on-ramps and 18 off-ramps, since most Portes' junctions lay just
#: outside and were cut, so routes between places near it never took it. Drawn from the ring itself
#: (the trunk road within 1 km outside the polygon above): every 6 degrees around Paris's centre, the
#: ring's farthest point plus 350 m; the woods' edges near the ring come in, the woods do not.
PARIS_WITH_PERIPHERIQUE = [
    (2.4191, 48.8582), (2.4186, 48.8647), (2.4182, 48.8713), (2.4147, 48.8775), (2.4103, 48.8834),
    (2.4030, 48.8880), (2.4014, 48.8949), (2.3970, 48.9011), (2.3888, 48.9050), (2.3758, 48.9044),
    (2.3657, 48.9048), (2.3565, 48.9048), (2.3478, 48.9044), (2.3399, 48.9043), (2.3324, 48.9042),
    (2.3251, 48.9038), (2.3180, 48.9034), (2.3113, 48.9013), (2.3052, 48.8989), (2.2998, 48.8959),
    (2.2949, 48.8929), (2.2893, 48.8909), (2.2846, 48.8881), (2.2800, 48.8852), (2.2766, 48.8816),
    (2.2733, 48.8781), (2.2702, 48.8744), (2.2660, 48.8710), (2.2626, 48.8670), (2.2590, 48.8628),
    (2.2534, 48.8582), (2.2467, 48.8528), (2.2480, 48.8474), (2.2481, 48.8416), (2.2505, 48.8362),
    (2.2559, 48.8318), (2.2697, 48.8316), (2.2770, 48.8295), (2.2833, 48.8274), (2.2886, 48.8249),
    (2.2947, 48.8233), (2.3008, 48.8220), (2.3068, 48.8208), (2.3126, 48.8193), (2.3187, 48.8178),
    (2.3251, 48.8164), (2.3322, 48.8138), (2.3397, 48.8129), (2.3473, 48.8131), (2.3560, 48.8124),
    (2.3647, 48.8128), (2.3720, 48.8155), (2.3801, 48.8178), (2.3870, 48.8214), (2.3969, 48.8237),
    (2.4059, 48.8273), (2.4148, 48.8318), (2.4185, 48.8381), (2.4194, 48.8450), (2.4204, 48.8516),
]  # fmt: skip

#: Lyon and Villeurbanne (S204: a case study candidate): the Presqu'île, Part-Dieu,
#: the Croix-Rousse, Confluence, the 7th and 8th arrondissements and Villeurbanne, as a box.
LYON_CENTRE = (4.772, 45.707, 4.923, 45.808)

#: ``key``, title, city, file (in ``cases/<key>/raw/``), region (a polygon or a ``(west, south, east, north)``
#: box; ``None`` for the whole extract), driving side, the size class and a note.
NETWORKS = [
    dict(key="manhattan", title="Manhattan below 60th Street", where="New York, USA", pbf="manhattan/raw/NewYork.osm.pbf",
         region=MANHATTAN_BELOW_60TH, left_hand=False, size_class="cordon",
         note="New York's congestion-pricing zone. Dense one-way grid, many signals, tunnels and bridges cut at the edge."),
    dict(key="stockholm", title="Stockholm inner city", where="Stockholm, Sweden", pbf="stockholm/raw/Stockholm.osm.pbf",
         region=STOCKHOLM_INNER_CITY, left_hand=False, size_class="cordon",
         note="Roughly the congestion-charge cordon. Islands and bridges, so many gateways."),
    dict(key="singapore", title="Singapore central area", where="Singapore", pbf="singapore/raw/Singapore.osm.pbf",
         region=(103.838, 1.276, 103.866, 1.304), left_hand=True, size_class="small",
         note="The central business district and Marina Bay. Left-hand traffic; expressways and tunnels."),
    dict(key="andorra", title="Andorra", where="Andorra", pbf="andorra/raw/andorra-260915.osm.pbf",
         region=None, left_hand=False, size_class="mid",
         note="The country extract already used for the figures; mountain valleys, a few main roads."),
    dict(key="seoul", title="Seoul: Gangnam and Seocho", where="Seoul, South Korea", pbf="seoul/raw/Seoul.osm.pbf",
         region=(127.00, 37.455, 127.10, 37.535), left_hand=False, size_class="mid",
         note="A district south of the Han river: wide arterials, a regular grid, large blocks."),
    dict(key="sanfrancisco", title="San Francisco", where="San Francisco, USA", pbf="sanfrancisco/raw/SanFrancisco.osm.pbf",
         region=(-122.517, 37.708, -122.355, 37.832), left_hand=False, size_class="large",
         note="The whole city on its peninsula: hills, a grid, freeways and bridges cut at the edge."),
    dict(key="zurich", title="Zurich", where="Zurich, Switzerland", pbf="zurich/raw/Zuerich.osm.pbf",
         region=(8.47, 47.34, 8.60, 47.42), left_hand=False, size_class="large",
         note="The city and its inner suburbs: an irregular medieval core, lakeside and river crossings, tram lines beside the roads."),
    dict(key="amsterdam", title="Amsterdam inside the A10", where="Amsterdam, Netherlands", pbf="amsterdam/raw/noord-holland-260924.osm.pbf",
         region=AMSTERDAM_A10, left_hand=False, size_class="large",
         note="The city inside its ring road: canals and bridges, one-way streets with contraflow cycling, a dense network of cycle tracks."),
    dict(key="paris", title="Paris inside the Périphérique", where="Paris, France", pbf="paris/raw/paris-bbbike-260926.osm.pbf",
         region=PARIS_INSIDE_PERIPHERIQUE, left_hand=False, size_class="large",
         note="The twenty arrondissements inside the ring road: Haussmann boulevards, dense one-way streets, many cycle tracks."),
    dict(key="lyon", title="Lyon and Villeurbanne", where="Lyon, France", pbf="lyon/raw/lyon-bbbike-260926.osm.pbf",
         region=LYON_CENTRE, left_hand=False, size_class="large",
         note="Between and across the Rhône and the Saône: the Presqu'île, Part-Dieu, Villeurbanne."),
]  # fmt: skip
