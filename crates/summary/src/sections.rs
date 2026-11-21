//! The page's sections, each computed from the run and written as HTML: the headline tiles, the
//! modes, when people left, the roads, transit, bikes and walking, the convergence, the computation
//! time with the network's size.

use std::collections::{BTreeMap, BTreeSet};

use openmobisim_io_parquet::RouteLeg;
use openmobisim_map::rules::{DELAY_BAND_LABELS, DELAY_BANDS, delay};
use openmobisim_map::{Ridden, clock, html_text};
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;

use crate::Summarised;
use crate::html::{
    Cell, bars, count, line, named, num, percent, stack, stacked, swatch, table, tile,
};

/// The rows of the modes: car, bike, walk, transit, then the trips without an alternative.
pub(crate) const MODE_NAMES: [&str; 5] = ["car", "bike", "walk", "transit", "no alternative"];
/// Each row's key in the page's names and its colour class.
const MODE_KEYS: [&str; 5] = ["car", "bike", "walk", "transit", "none"];

/// A trip's row: its mode's place in [`MODE_NAMES`], 4 without an alternative.
fn row_of(s: &Summarised<'_>, t: usize) -> usize {
    s.kept.trips()[t].mode.map_or(4, |m| usize::from(m.as_u8()))
}

/// Per row of [`MODE_NAMES`]: trips, finished trips, and the finished trips' seconds door to door.
fn by_mode(s: &Summarised<'_>) -> [(usize, usize, f64); 5] {
    let mut out = [(0, 0, 0.0); 5];
    for (t, trip) in s.kept.trips().iter().enumerate() {
        let r = &mut out[row_of(s, t)];
        r.0 += 1;
        if trip.finished {
            r.1 += 1;
            r.2 += trip.travel_time;
        }
    }
    out
}

/// The cars on the roads, from the links table: per link and bin, its delay over free flow (the
/// bin's mean crossing time over the free-flow time, less one: the map's), banded.
pub(crate) struct Roads {
    /// Vehicle-km and vehicle-hours of the cars that entered a link in the window.
    pub vkm: f64,
    pub vh: f64,
    /// The hours beyond free flow.
    pub delay_h: f64,
    /// Vehicle-km by delay band.
    pub bands: [f64; 4],
    /// Cars entering a link, per bin of the window.
    pub per_bin: Vec<f64>,
    /// Per link with cars: the cars and the sum of their crossing times, s.
    pub links: BTreeMap<LinkId, (u32, f64)>,
}

/// The cars on the roads.
pub(crate) fn roads(s: &Summarised<'_>) -> Roads {
    let r = s.kept.rows();
    let n_bins = s.kept.window().n_steps() as usize;
    let mut out = Roads {
        vkm: 0.0,
        vh: 0.0,
        delay_h: 0.0,
        bands: [0.0; 4],
        per_bin: vec![0.0; n_bins],
        links: BTreeMap::new(),
    };
    for i in (0..r.link.len()).filter(|&i| r.cars[i] > 0) {
        let (l, cars) = (r.link[i], f64::from(r.cars[i]));
        let free = s.network.free_flow_time(l, Mode::Car).get();
        let late = delay(r.car_time[i] / cars, free);
        let km = cars * s.network.length(l).get() / 1_000.0;
        out.bands[DELAY_BANDS.iter().filter(|&&e| late >= e).count()] += km;
        out.vkm += km;
        out.vh += r.car_time[i] / 3_600.0;
        out.delay_h += cars * free * late / 3_600.0;
        out.per_bin[r.bin[i] as usize] += cars;
        let e = out.links.entry(l).or_insert((0, 0.0));
        (e.0, e.1) = (e.0 + r.cars[i], e.1 + r.car_time[i]);
    }
    out
}

/// The window, `HH:MM–HH:MM`.
pub(crate) fn window(s: &Summarised<'_>) -> String {
    let w = s.kept.window();
    format!("{}–{}", clock(w.origin().get()), clock(w.end().get()))
}

/// The run's wall-clock seconds: the five stages summed over the iterations.
fn run_seconds(s: &Summarised<'_>) -> f64 {
    s.iterations.iter().flat_map(|i| i.stage_seconds).sum()
}

/// The trips tile's line: the window, then the trips given and dropped by reason when the
/// record says any was dropped (`openmobisim_map::record::dropped`).
fn given_and_dropped(s: &Summarised<'_>) -> String {
    let (given, reasons) = openmobisim_map::record::dropped(s.record);
    let big = |n: u64| count(usize::try_from(n).unwrap_or(usize::MAX));
    let mut line = format!("in {}", window(s));
    if let (Some(given), false) = (given, reasons.is_empty()) {
        let why: Vec<String> =
            reasons.iter().map(|(why, n)| format!("{} {why}", big(*n))).collect();
        let dropped: u64 = reasons.iter().map(|r| r.1).sum();
        let lead = if s.kept.trip_count() == 0 { "no trip is left: " } else { "" };
        line.push_str(&format!(
            " · {lead}of {} given, {} dropped: {}",
            named("given", &big(given)),
            named("dropped", &big(dropped)),
            why.join(", ")
        ));
    }
    line
}

/// The headline tiles.
pub(crate) fn tiles(s: &Summarised<'_>, roads: &Roads) -> String {
    let m = by_mode(s);
    let total = s.kept.trip_count();
    let finished: usize = m.iter().map(|r| r.1).sum();
    let seconds: f64 = m.iter().map(|r| r.2).sum();
    let without = m[4].0;
    let travelling = total - finished - without;
    #[allow(clippy::cast_precision_loss, reason = "a count of trips, far below 2^53")]
    let mean = if finished > 0 { num(seconds / finished as f64 / 60.0, 1) } else { "—".into() };
    let mut out = vec![
        tile("trips", "trips", &count(total), "", &given_and_dropped(s)),
        tile(
            "completed",
            "completed",
            &percent(finished, total, 2),
            "%",
            &format!("{} trips", named("finished", &count(finished))),
        ),
        tile(
            "not completed",
            "not-completed",
            &percent(travelling + without, total, 2),
            "%",
            &format!(
                "{} still travelling at the window's end · {} without an alternative",
                named("still-travelling", &count(travelling)),
                named("without-alternative", &count(without))
            ),
        ),
        tile("mean trip", "mean-trip", &mean, "min", "of the trips that finished"),
        tile(
            "total travel time",
            "travel-time",
            &num(seconds / 3_600.0, 1),
            "h",
            "traveller-hours of the trips that finished",
        ),
    ];
    if roads.vkm > 0.0 {
        let speed = num(roads.vkm / roads.vh, 1);
        out.push(tile(
            "on the roads",
            "vehicle-km",
            &num(roads.vkm, 1),
            "vehicle\u{2011}km",
            &format!(
                "{} km/h mean speed: vehicle-km over the cars' hours on links",
                named("mean-speed", &speed)
            ),
        ));
    }
    out.push(tile(
        "run time",
        "run-time",
        &num(run_seconds(s), 1),
        "s",
        &format!(
            "the five stages over {} iterations, this machine; reading the inputs and writing the \
             files left out",
            s.iterations.len()
        ),
    ));
    format!("<div class=\"tiles\">{}</div>", out.concat())
}

/// The modes: the share bar and the table, then the trips without an alternative by class.
pub(crate) fn modes(s: &Summarised<'_>) -> String {
    let m = by_mode(s);
    let total = s.kept.trip_count();
    #[allow(clippy::cast_precision_loss, reason = "counts of trips, far below 2^53")]
    let share = |n: usize| if total > 0 { n as f64 / total as f64 } else { 0.0 };
    let parts: Vec<(String, f64, String)> = (0..5)
        .map(|i| {
            (
                format!("k-{}", MODE_KEYS[i]),
                share(m[i].0),
                format!("{}: {} %", MODE_NAMES[i], percent(m[i].0, total, 1)),
            )
        })
        .collect();
    let mut rows = Vec::new();
    for i in (0..5).filter(|&i| m[i].0 > 0) {
        let (trips, done, seconds) = m[i];
        let k = |c: &str| format!("mode.{}.{c}", MODE_KEYS[i]);
        #[allow(clippy::cast_precision_loss, reason = "a count of trips, far below 2^53")]
        let mean = if done > 0 { num(seconds / done as f64 / 60.0, 1) } else { "—".into() };
        let moving = i < 4;
        rows.push(vec![
            Cell::text(swatch(&format!("k-{}", MODE_KEYS[i]), MODE_NAMES[i])),
            Cell::named(k("trips"), count(trips)),
            Cell::named(k("share"), percent(trips, total, 1)),
            Cell::named(k("completed"), if moving { percent(done, trips, 1) } else { "—".into() }),
            Cell::named(k("mean"), mean),
            Cell::named(k("hours"), if moving { num(seconds / 3_600.0, 1) } else { "—".into() }),
        ]);
    }
    let head =
        ["mode", "trips", "share (%)", "completed (%)", "mean trip (min)", "travel time (h)"];
    stack(&parts) + &table(&head, rows, 1) + &without_alternative(s, m[4].0)
}

/// The trips without an alternative, overall and by user class; nothing when there are none.
fn without_alternative(s: &Summarised<'_>, without: usize) -> String {
    if without == 0 {
        return String::new();
    }
    let total = s.kept.trip_count();
    let classes = s.kept.classes();
    let mut per = vec![(0_usize, 0_usize); classes.len()];
    for trip in s.kept.trips() {
        let c = &mut per[trip.class as usize];
        c.0 += 1;
        c.1 += usize::from(trip.mode.is_none());
    }
    let rows = per
        .iter()
        .enumerate()
        .map(|(i, &(trips, none))| {
            vec![
                Cell::text(html_text(&classes[i])),
                Cell::named(format!("class.{i}.trips"), count(trips)),
                Cell::named(format!("class.{i}.without"), count(none)),
                Cell::named(format!("class.{i}.share"), percent(none, trips, 2)),
            ]
        })
        .collect();
    format!(
        "<p class=\"note\">Trips without an alternative: <b>{}</b> ({} % of the trips): none of the \
         modes open to them reaches the destination (for transit: no stop within reach, or no \
         journey between the stops in reach); counted, not simulated.</p>",
        count(without),
        percent(without, total, 2)
    ) + &table(&["user class", "trips", "without an alternative", "share (%)"], rows, 1)
}

/// When people left: departures by quarter hour of the clock and mode, stacked bars and the table.
pub(crate) fn departures(s: &Summarised<'_>) -> String {
    let trips = s.kept.trips();
    let quarter = |t: usize| trips[t].departure.get() / 900;
    let (Some(lo), Some(hi)) =
        ((0..trips.len()).map(quarter).min(), (0..trips.len()).map(quarter).max())
    else {
        return String::new();
    };
    let n = (hi - lo + 1) as usize;
    let mut counts = [(); 5].map(|()| vec![0.0; n]);
    for t in 0..trips.len() {
        counts[row_of(s, t)][(quarter(t) - lo) as usize] += 1.0;
    }
    let bins: Vec<String> = (lo..=hi).map(|q| clock(q * 900)).collect();
    // A quarter hour the window cuts (it starts or ends inside it) is marked, and said under.
    let (w0, w1) = (s.kept.window().origin().get(), s.kept.window().end().get());
    let cut_first = w0 % 900 != 0 && lo == w0 / 900;
    let cut_last = w1 % 900 != 0 && hi == (w1 - 1) / 900;
    let shown: Vec<String> = (0..n)
        .map(|b| {
            let cut = (b == 0 && cut_first) || (b + 1 == n && cut_last);
            if cut { format!("{}*", bins[b]) } else { bins[b].clone() }
        })
        .collect();
    let present: Vec<usize> = (0..5).filter(|&i| counts[i].iter().sum::<f64>() > 0.0).collect();
    let classes: Vec<String> = present.iter().map(|&i| format!("k-{}", MODE_KEYS[i])).collect();
    let series: Vec<(&str, &str, Vec<f64>)> = present
        .iter()
        .zip(&classes)
        .map(|(&i, c)| (MODE_NAMES[i], c.as_str(), counts[i].clone()))
        .collect();
    let chart = stacked(&shown, &series, "departures by quarter hour (trips)");
    let row = |label: &str, text: &str, at: &dyn Fn(usize) -> f64| {
        let mut cells = vec![Cell::text(text)];
        for &i in &present {
            cells.push(Cell::named(format!("dep.{label}.{}", MODE_KEYS[i]), num(at(i), 0)));
        }
        let all: f64 = present.iter().map(|&i| at(i)).sum();
        cells.push(Cell::named(format!("dep.{label}.all"), num(all, 0)));
        cells
    };
    let mut rows: Vec<Vec<Cell>> =
        (0..n).map(|b| row(&bins[b], &shown[b], &|i| counts[i][b])).collect();
    rows.push(row("all", "all", &|i| counts[i].iter().sum()));
    let mut head = vec!["from"];
    head.extend(present.iter().map(|&i| MODE_NAMES[i]));
    head.push("all");
    let mut cut = Vec::new();
    if cut_first {
        cut.push(format!("the window starts at {}", clock(w0)));
    }
    if cut_last {
        cut.push(format!("it ends at {}", clock(w1)));
    }
    let note = if cut.is_empty() {
        String::new()
    } else {
        format!("<p class=\"note\">* part of a quarter hour: {}.</p>", cut.join("; "))
    };
    chart + "<div class=\"below\">" + &table(&head, rows, 1) + &note + "</div>"
}

/// On the roads: vehicle-km by delay band, cars entering links by bin, the five busiest links.
pub(crate) fn on_the_roads(s: &Summarised<'_>, r: &Roads) -> String {
    let parts: Vec<(String, f64, String)> = (0..4)
        .map(|i| {
            (
                format!("k-b{i}"),
                r.bands[i] / r.vkm,
                format!("{}: {} vehicle-km", DELAY_BAND_LABELS[i], num(r.bands[i], 1)),
            )
        })
        .collect();
    let rows = (0..4)
        .map(|i| {
            vec![
                Cell::text(swatch(&format!("k-b{i}"), DELAY_BAND_LABELS[i])),
                Cell::named(format!("band.{i}.km"), num(r.bands[i], 1)),
                Cell::named(format!("band.{i}.share"), num(100.0 * r.bands[i] / r.vkm, 1)),
            ]
        })
        .collect();
    let w = s.kept.window();
    let xs: Vec<String> =
        (0..w.n_steps()).map(|b| clock(w.origin().get() + b * w.step_seconds())).collect();
    let chart = line(
        &xs,
        &r.per_bin,
        &format!(
            "entries of cars into links, per bin of {} s (a car counts on every link it enters)",
            w.step_seconds()
        ),
        "cars",
        0,
    );
    let note = format!(
        "<p class=\"note\">Over {}: <b>{}</b> vehicle-km in <b>{}</b> vehicle-hours on links, a mean \
         speed of <b>{}</b> km/h; <b>{}</b> % of that time is delay beyond free flow. A link's delay in \
         a bin is its cars' mean crossing time over the free-flow time, less one, as on the map.</p>",
        window(s),
        num(r.vkm, 1),
        num(r.vh, 1),
        num(r.vkm / r.vh, 1),
        num(100.0 * r.delay_h / r.vh, 1)
    );
    format!(
        "<div class=\"grid2\"><div>{}{}{note}</div><div>{chart}</div></div><h3>Busiest links, by \
         volume per direction</h3>{}",
        stack(&parts),
        table(&["delay over free flow", "vehicle-km", "share (%)"], rows, 1),
        busiest(s, r)
    )
}

/// The five busiest links by cars over the window (ties to the lower link id).
fn busiest(s: &Summarised<'_>, r: &Roads) -> String {
    let hours = f64::from(s.kept.window().end().get() - s.kept.window().origin().get()) / 3_600.0;
    let mut links: Vec<(&LinkId, &(u32, f64))> = r.links.iter().collect();
    links.sort_by_key(|(l, (cars, _))| (std::cmp::Reverse(*cars), **l));
    let rows = links
        .iter()
        .take(5)
        .enumerate()
        .map(|(k, (l, (cars, time)))| {
            let free = s.network.free_flow_time(**l, Mode::Car).get();
            let late = 100.0 * delay(time / f64::from(*cars), free);
            vec![
                Cell::named(
                    format!("busy.{k}.link"),
                    html_text(s.network.link_ids().external(l.raw())),
                ),
                Cell::named(format!("busy.{k}.length"), num(s.network.length(**l).get(), 0)),
                Cell::named(format!("busy.{k}.volume"), num(f64::from(*cars) / hours, 0)),
                Cell::named(format!("busy.{k}.delay"), num(late, 1)),
            ]
        })
        .collect();
    table(&["link", "length (m)", "volume (veh/h)", "delay over free flow (%)"], rows, 1)
}

/// Transit, bikes and walking in numbers; nothing without any of them. The boardings are the map's
/// count (`ridden`); a trip's rides, for the share that change vehicle, are its journey's.
pub(crate) fn services(s: &Summarised<'_>, ridden: &Ridden) -> String {
    let mut rows = Vec::new();
    if s.timetable.is_some() {
        let rides: Vec<usize> = s
            .kept
            .trips()
            .iter()
            .filter(|t| t.mode == Some(Mode::Transit))
            .filter_map(|t| t.legs.as_ref())
            .map(|legs| legs.iter().filter(|l| matches!(l, RouteLeg::Ride(_))).count())
            .collect();
        let boardings = ridden.boardings as usize;
        let changing = rides.iter().filter(|&&n| n > 1).count();
        #[allow(clippy::cast_precision_loss, reason = "counts of rides, far below 2^53")]
        let per_trip = if rides.is_empty() {
            "—".into()
        } else {
            num(boardings as f64 / rides.len() as f64, 2)
        };
        rows.push(
            [
                "boardings",
                &named("boardings", &count(boardings)),
                "rides boarded over the window, every journey taken",
            ]
            .map(String::from),
        );
        rows.push(
            [
                "rides per transit trip",
                &named("rides-per-trip", &per_trip),
                &format!(
                    "{} % of the {} transit trips change vehicle at least once",
                    named("changing", &percent(changing, rides.len(), 1)),
                    count(rides.len())
                ),
            ]
            .map(String::from),
        );
    }
    let r = s.kept.rows();
    for (word, key, counts, what) in [
        (
            "cycled",
            "km-cycled",
            &r.bikes,
            "bikes on links times the links' lengths, over the window",
        ),
        (
            "walked on the network",
            "km-walked",
            &r.walkers,
            "walkers on links times the links' lengths; the walks between a door and the network or a stop are not on links",
        ),
    ] {
        let km: f64 = (0..r.link.len())
            .map(|i| f64::from(counts[i]) * s.network.length(r.link[i]).get())
            .sum::<f64>()
            / 1_000.0;
        if km > 0.0 {
            rows.push([format!("kilometres {word}"), named(key, &num(km, 1)), what.to_owned()]);
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    let rows = rows
        .into_iter()
        .map(|[a, b, c]| vec![Cell::text(a), Cell(b, None), Cell::text(c)])
        .collect();
    table(&["", "value", ""], rows, 1).replace("<td class=\"n\">", "<td class=\"n v2\">")
}

/// The convergence: the mean trip by iteration and the table of the iterations; what ended the run.
pub(crate) fn convergence(s: &Summarised<'_>) -> String {
    let its = s.iterations;
    let xs: Vec<String> = its.iter().map(|i| i.iteration.to_string()).collect();
    let means: Vec<f64> = its.iter().map(|i| i.mean_trip_time / 60.0).collect();
    let chart = line(&xs, &means, "mean trip (min) by iteration", "min", 2);
    let rows = its
        .iter()
        .map(|i| {
            let k = |c: &str| format!("it.{}.{c}", i.iteration);
            vec![
                Cell::text(i.iteration.to_string()),
                Cell::named(k("mean"), num(i.mean_trip_time / 60.0, 2)),
                Cell::named(k("switched"), num(100.0 * i.switch_share, 2)),
                Cell::named(k("offered"), num(100.0 * i.offered_share, 2)),
                Cell::named(k("found"), num(100.0 * i.found_share, 2)),
                Cell::named(k("would"), num(100.0 * i.would_change_share, 2)),
                Cell::named(
                    k("unfinished"),
                    count(usize::try_from(i.unfinished).unwrap_or(usize::MAX)),
                ),
                Cell::named(k("gap"), num(100.0 * i.gap, 2)),
            ]
        })
        .collect();
    let head = [
        "iteration",
        "mean trip (min)",
        "switched (%)",
        "offered (%)",
        "found cheaper (%)",
        "would change (%)",
        "unfinished (trips)",
        "duality gap (%)",
    ];
    let by = match s.stopped_by_rule {
        Some(true) => format!("{} ended the run", named("stopped-by", "The stopping rule")),
        Some(false) => format!("{} ended the run", named("stopped-by", "The iteration cap")),
        None => named("stopped-by", "The record does not say what ended the run"),
    };
    format!(
        "<div class=\"grid2\"><div>{chart}</div><div>{}</div></div><p class=\"note\">{} \
         after {} iterations. Iteration 0 chooses on free-flow times. Each model reports the one \
         measure that can reach zero, never judged; the other shows as —. Under the least-cost \
         choice it is the duality gap, the chosen alternatives' cost over the least in each set, \
         less one, summed over the trips with an alternative: the routes in use cost the same at \
         the equilibrium, so the share of travellers not on their lowest-cost alternative does \
         not settle, and the gap, which weighs each traveller by what it would save, is the \
         measure. Under the logit it is the share who would change, the trips with an \
         alternative that the logit, choosing again at the costs the iteration measured, would \
         put on another alternative than the one they hold: the logit puts a share of travellers \
         on an alternative that is not their lowest-cost one, so the gap does not reach zero.</p>",
        table(&head, rows, 1),
        by,
        its.len()
    )
}

/// The computation time: the five stages summed over the iterations; then the network's size.
pub(crate) fn computation(s: &Summarised<'_>) -> String {
    let names = ["search", "choose", "load", "measure", "assess"];
    let stages: Vec<(&str, f64)> = names
        .iter()
        .enumerate()
        .map(|(k, n)| (*n, s.iterations.iter().map(|i| i.stage_seconds[k]).sum()))
        .collect();
    format!(
        "{}<p class=\"note\">Run total <b>{}</b> s: the five stages summed over {} iterations, wall \
         clock on this machine; reading the inputs and writing the files are not counted.</p><h3>The \
         network</h3>{}",
        bars(&stages, "s"),
        num(run_seconds(s), 2),
        s.iterations.len(),
        network(s)
    )
}

/// The network's links, nodes and kilometres by mode; the timetable's stops and runs.
fn network(s: &Summarised<'_>) -> String {
    let net = s.network;
    let rows = [(Mode::Car, "car"), (Mode::Bike, "bike"), (Mode::Walk, "walk")]
        .iter()
        .map(|&(mode, name)| {
            let links: Vec<LinkId> = (0..net.link_count())
                .map(LinkId::new)
                .filter(|&l| net.modes(l).contains(mode))
                .collect();
            let nodes: BTreeSet<u32> = links
                .iter()
                .flat_map(|&l| [net.upstream(l).raw(), net.downstream(l).raw()])
                .collect();
            let km: f64 = links.iter().map(|&l| net.length(l).get()).sum::<f64>() / 1_000.0;
            vec![
                Cell::text(name),
                Cell::named(format!("net.{name}.links"), count(links.len())),
                Cell::named(format!("net.{name}.nodes"), count(nodes.len())),
                Cell::named(format!("net.{name}.km"), num(km, 1)),
            ]
        })
        .collect();
    let transit = s.timetable.map_or(String::new(), |tt| {
        format!(
            "<p class=\"note\">Transit: {} stops, {} runs.</p>",
            named("net.stops", &count(tt.stop_count() as usize)),
            named("net.runs", &count(tt.run_count() as usize))
        )
    });
    table(&["mode", "links", "nodes", "length (km)"], rows, 1) + &transit
}
