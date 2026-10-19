"""Each starter city's residents and non-residential building volume, cut out of the GHSL grids.

    python ghsl.py [delft amsterdam paris lyon]    # needs numpy, tifffile, imagecodecs

Sources (open, CC BY 4.0, (c) European Union): the Global Human Settlement Layer, release R2023A, epoch 2020, in WGS84
at 3 arc-seconds (about 60 × 90 m here):
- **GHS-POP** (residents per cell): Schiavina, Freire, Carioli, MacManus (2023), GHS-POP R2023A, European Commission,
  Joint Research Centre, doi:10.2905/2FF68A52-5B5B-4A22-8F40-C41DA8332CFE,
  https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/GHSL/GHS_POP_GLOBE_R2023A/
- **GHS-BUILT-V NRES** (non-residential built volume per cell, m³): Pesaresi, Politis (2023), GHS-BUILT-V R2023A,
  European Commission, Joint Research Centre, doi:10.2905/AB2F107A-03CD-47A3-85E5-139D8EC63283,
  https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/GHSL/GHS_BUILT_V_GLOBE_R2023A/

The tiles (10° × 10°) are in `sources/ghsl/` of the data root (download them with the URLs above: tile R4_C19 holds
Delft and Amsterdam, R5_C19 Paris and Lyon). Reads each city's region from `bundle/v1/<case>/case.json` (made by
`bundle.py date`). Writes `cases/<case>/raw/ghsl_2020.npz` of the data root: `pop` and `nres` (float32, the city's box
plus a margin), `lon0`, `lat0` (the north-west corner) and `step` (degrees per cell).
"""

from __future__ import annotations

import json
import sys
import zipfile
from pathlib import Path

import numpy as np
import tifffile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from cases import DATA  # noqa: E402

SOURCES = DATA / "sources" / "ghsl"
MARGIN_DEG = 0.05  # about 4-5 km around the study area, for the cells that straddle its edge
TILES = {"delft": "R4_C19", "amsterdam": "R4_C19", "paris": "R5_C19", "lyon": "R5_C19"}
PRODUCTS = {
    "pop": "GHS_POP_E2020_GLOBE_R2023A_4326_3ss_V1_0_{tile}",
    "nres": "GHS_BUILT_V_NRES_E2020_GLOBE_R2023A_4326_3ss_V1_0_{tile}",
}


def box(case: str) -> tuple[float, float, float, float]:
    """A case's study-area box (west, south, east, north) from its ``case.json``, with the margin."""
    info = json.loads((DATA / "bundle" / "v1" / case / "case.json").read_text())
    region = info["network"]["region"] if "network" in info else info["region"]
    if len(region) == 4 and not isinstance(region[0], list):
        lon_min, lat_min, lon_max, lat_max = region
    else:
        pts = np.array(region)
        lon_min, lat_min = pts.min(axis=0)
        lon_max, lat_max = pts.max(axis=0)
    return lon_min - MARGIN_DEG, lat_min - MARGIN_DEG, lon_max + MARGIN_DEG, lat_max + MARGIN_DEG


def crop(product: str, tile: str, b: tuple[float, float, float, float]) -> tuple[np.ndarray, float, float, float]:
    """A GHSL tile's values inside the box ``b``: the array (no-data as 0), its north-west corner and its step."""
    name = PRODUCTS[product].format(tile=tile)
    with zipfile.ZipFile(SOURCES / f"{name}.zip") as z, z.open(f"{name}.tif") as f, tifffile.TiffFile(f) as t:
        page = t.pages[0]
        tags = {k.name: k.value for k in page.tags}
        step = float(tags["ModelPixelScaleTag"][0])
        west, north = float(tags["ModelTiepointTag"][3]), float(tags["ModelTiepointTag"][4])
        image = page.asarray()
    c0, c1 = int((b[0] - west) / step), int(np.ceil((b[2] - west) / step))
    r0, r1 = int((north - b[3]) / step), int(np.ceil((north - b[1]) / step))
    part = image[r0:r1, c0:c1].astype(np.float32)
    part[part < 0] = 0  # no-data (-200 in GHS-POP) is nobody
    return part, west + c0 * step, north - r0 * step, step


def main() -> None:
    """Cut out the residents and the non-residential volume of the named cases (of all if none is named)."""
    for case in sys.argv[1:] or list(TILES):
        b = box(case)
        pop, lon0, lat0, step = crop("pop", TILES[case], b)
        nres, *_ = crop("nres", TILES[case], b)
        out = DATA / "cases" / case / "raw" / "ghsl_2020.npz"
        np.savez_compressed(out, pop=pop, nres=nres, lon0=lon0, lat0=lat0, step=step)
        print(f"{case}: {pop.shape}, residents {pop.sum():,.0f}, non-residential volume {nres.sum() / 1e6:,.1f} million m3"
              f" -> {out.relative_to(DATA)}")


if __name__ == "__main__":
    main()
