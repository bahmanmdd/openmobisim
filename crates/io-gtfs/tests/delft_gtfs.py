"""The outside answer for the Delft feed: the busiest weekday and what runs on it, by gtfs-kit.

Writes `fixtures/delft/expected.csv` (key,value) for `delft.rs`:
  date        the busiest weekday: the Monday to Friday of the feed's whole range of dates with the most trips in
              service (gtfs-kit's `compute_trip_activity`, the reference's calendar and exception rule), earliest on a tie
  trips       the trips in service that day
  runs        those with at least two calls whose first and last calls have a time, plus the night runs
  calls       their calls (rows of `stop_times.txt` at a stop of `stops.txt` with `location_type` 0 or blank), plus the
              night runs' calls
  night_runs  the trips in service the day before with at least two calls departing at or after 24:00:00
  stops       the rows of `stops.txt` with `location_type` 0 or blank and a position

Run from this folder with gtfs-kit 14.1.1 (pandas 3.1.1) in a virtual environment:
  python delft_gtfs.py fixtures/delft/gtfs.zip fixtures/delft/expected.csv
"""

import datetime as dt
import sys
import time

import gtfs_kit as gk
import pandas as pd


def seconds(text):
    """`H:MM:SS` as seconds after midnight, hours past 24 kept; NaN for a blank."""
    if not isinstance(text, str) or not text.strip():
        return float("nan")
    h, m, s = text.strip().split(":")
    return int(h) * 3600 + int(m) * 60 + int(s)


def main(path, out):
    started = time.perf_counter()
    feed = gk.read_feed(path, dist_units="km")
    assert feed.frequencies is None or feed.frequencies.empty, "this script does not expand frequencies"
    weekdays = [d for d in feed.get_dates() if dt.datetime.strptime(d, "%Y%m%d").weekday() < 5]
    activity = feed.compute_trip_activity(weekdays)
    counts = activity[weekdays].sum()
    best = max(weekdays, key=lambda d: (counts[d], -int(d)))
    assert counts[best] > 0, "no weekday has a trip"
    before = (dt.datetime.strptime(best, "%Y%m%d") - dt.timedelta(days=1)).strftime("%Y%m%d")
    both = feed.compute_trip_activity([best, before])
    today = set(both.loc[both[best] == 1, "trip_id"])
    # gtfs-kit leaves out a date outside the feed's range: nothing runs on it.
    yesterday = set(both.loc[both[before] == 1, "trip_id"]) if before in both else set()

    stops = feed.stops
    kind = stops["location_type"] if "location_type" in stops else pd.Series(0, index=stops.index)
    stops = stops[(kind.isna() | (kind == 0)) & stops["stop_lat"].notna() & stops["stop_lon"].notna()]
    st = feed.stop_times[feed.stop_times["stop_id"].isin(set(stops["stop_id"]))].copy()
    st["arrival"] = st["arrival_time"].map(seconds)
    st["departure"] = st["departure_time"].map(seconds)
    st["departure"] = st["departure"].fillna(st["arrival"])
    st["arrival"] = st["arrival"].fillna(st["departure"])
    st = st.sort_values(["trip_id", "stop_sequence"])

    runs = calls = 0
    for _, calls_of in st[st["trip_id"].isin(today)].groupby("trip_id"):
        if len(calls_of) >= 2 and calls_of["arrival"].iloc[[0, -1]].notna().all():
            runs += 1
            calls += len(calls_of)
    night_runs = 0
    for _, calls_of in st[st["trip_id"].isin(yesterday)].groupby("trip_id"):
        late = calls_of[calls_of["departure"] >= 86_400]
        if len(late) >= 2:
            night_runs += 1
            calls += len(late)
    rows = {
        "date": best,
        "trips": len(today),
        "runs": runs + night_runs,
        "calls": calls,
        "night_runs": night_runs,
        "stops": len(stops),
    }
    with open(out, "w") as f:
        f.write("key,value\n")
        for k, v in rows.items():
            f.write(f"{k},{v}\n")
    past_midnight = int(((st["arrival"] >= 86_400) | (st["departure"] >= 86_400)).sum())
    print(rows, f"stop times at or past 24:00:00: {past_midnight};", f"{time.perf_counter() - started:.2f} s")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
