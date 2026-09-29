//! The timetable of one service day (S199).
//!
//! What a run of scheduled public transport needs: **stops** (a stop is a node
//! of the transit layer, so a hub's transit access point is `(Layer::Transit,
//! stop)`), **routes** (a line and its [`ServiceKind`]), and **runs** — one GTFS
//! trip on the service day (Foundations §1's "transit trip (run)") — each an
//! ordered list of **calls**: a stop, an arrival and a departure time, and
//! whether passengers may board and alight there.
//!
//! **Times** are `u32` seconds after the service day's midnight, which is the
//! run clock's zero (S199). A call's times are held apart from the calls, in a
//! [`CallTimes`]: the timetable's own are the **scheduled** ones; a loading
//! produces **realised** ones for the runs that ride the roads, with the same
//! shape, and passengers route on those (design §18.2).
//!
//! **Pattern groups.** Runs of the same route that call at the same stops with
//! the same boarding rules form a group. RAPTOR's patterns are these groups
//! split so that no run overtakes another ([`crate::raptor`]); the split depends
//! on the times, so it is made where the times are known.
//!
//! **Identity.** Stops, routes and runs get dense ids by sorted external id
//! (Foundations §1), so two builds of the same feed have the same ids; groups
//! are numbered in the order of their smallest run id.
//!
//! **Cost:** per call, a stop id, a flag byte and two times — 13 bytes; per run,
//! a route id and an offset; per stop, a position, a name and a parent. A day
//! of Amsterdam's services (102 thousand calls) is about 1.5 MB; the whole
//! Netherlands (2.3 million) about 30 MB.

use std::collections::HashMap;

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_types::ids::{
    EntityId, ExternalIdTable, ExternalIdTableBuilder, NULL_ID, NodeId, TransitRunId,
};

use crate::date::ServiceDate;
use crate::kind::ServiceKind;

/// A time not known: a realised call the loading never reached.
pub const UNKNOWN_TIME: u32 = u32::MAX;

/// Passengers may board at the call.
pub const BOARD: u8 = 1;
/// Passengers may alight at the call.
pub const ALIGHT: u8 = 2;

/// A stop, before ids are assigned.
#[derive(Clone, Debug)]
pub struct StopSpec {
    /// The GTFS `stop_id`.
    pub external_id: String,
    /// Its name.
    pub name: String,
    /// Where it is.
    pub position: LonLat,
    /// The `parent_station`, if any: the station it belongs to.
    pub parent: Option<String>,
}

/// A route, before ids are assigned.
#[derive(Clone, Debug)]
pub struct RouteSpec {
    /// The GTFS `route_id`.
    pub external_id: String,
    /// The short name riders know it by (`route_short_name`, else the long name).
    pub short_name: String,
    /// The GTFS `route_type`.
    pub route_type: u16,
}

/// One call of a run, before ids are assigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallSpec {
    /// The stop, as its index among the builder's stops.
    pub stop: u32,
    /// Arrival, seconds after midnight.
    pub arrival: u32,
    /// Departure, seconds after midnight.
    pub departure: u32,
    /// [`BOARD`] and [`ALIGHT`], or'ed.
    pub flags: u8,
}

/// An explicit minimum transfer time between two stops (`transfers.txt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Transfer {
    /// From this stop.
    pub from: NodeId,
    /// To this one.
    pub to: NodeId,
    /// In at least this many seconds.
    pub seconds: u32,
}

/// What a build dropped or repaired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimetableReport {
    /// Runs with fewer than two calls, or of an unknown route: dropped.
    pub runs_too_short: u32,
    /// Runs whose external id repeated an earlier one: dropped (the first wins).
    pub runs_repeated: u32,
    /// Calls whose times ran backwards, moved forward to the previous time.
    pub times_repaired: u32,
}

/// Collects stops, routes and runs; [`Self::build`] assigns ids.
#[derive(Clone, Debug)]
pub struct TimetableBuilder {
    date: ServiceDate,
    stops: Vec<StopSpec>,
    routes: Vec<RouteSpec>,
    runs: Vec<(String, u32)>,
    run_calls: Vec<u32>,
    calls: Vec<CallSpec>,
    transfers: Vec<(u32, u32, u32)>,
}

impl TimetableBuilder {
    /// An empty timetable of `date`.
    #[must_use]
    pub fn new(date: ServiceDate) -> Self {
        Self {
            date,
            stops: Vec::new(),
            routes: Vec::new(),
            runs: Vec::new(),
            run_calls: vec![0],
            calls: Vec::new(),
            transfers: Vec::new(),
        }
    }

    /// Add a stop; returns its index among the builder's stops.
    ///
    /// # Panics
    ///
    /// Panics if there are more than `u32::MAX` stops.
    pub fn add_stop(&mut self, stop: StopSpec) -> u32 {
        self.stops.push(stop);
        u32::try_from(self.stops.len() - 1).expect("stops fit u32")
    }

    /// Add a route; returns its index among the builder's routes.
    ///
    /// # Panics
    ///
    /// Panics if there are more than `u32::MAX` routes.
    pub fn add_route(&mut self, route: RouteSpec) -> u32 {
        self.routes.push(route);
        u32::try_from(self.routes.len() - 1).expect("routes fit u32")
    }

    /// Add a run of route `route` (a builder index) with its calls in order.
    ///
    /// # Panics
    ///
    /// Panics if there are more than `u32::MAX` calls.
    pub fn add_run(&mut self, external_id: impl Into<String>, route: u32, calls: &[CallSpec]) {
        self.runs.push((external_id.into(), route));
        self.calls.extend_from_slice(calls);
        self.run_calls.push(u32::try_from(self.calls.len()).expect("calls fit u32"));
    }

    /// Add an explicit minimum transfer time from stop `from` to stop `to`
    /// (builder indices).
    pub fn add_transfer(&mut self, from: u32, to: u32, seconds: u32) {
        self.transfers.push((from, to, seconds));
    }

    /// How many stops have been added.
    #[must_use]
    pub fn stop_count(&self) -> usize {
        self.stops.len()
    }

    /// Assign ids and freeze. Runs with fewer than two calls or an unknown
    /// route are dropped; a call whose time runs backwards is moved forward to
    /// the previous time (both counted in the report).
    ///
    /// # Panics
    ///
    /// Panics if a call names a stop index that was never added.
    #[must_use]
    pub fn build(self) -> (Timetable, TimetableReport) {
        let mut report = TimetableReport::default();

        let mut b = ExternalIdTableBuilder::with_capacity(self.stops.len());
        b.extend(self.stops.iter().map(|s| s.external_id.clone()));
        let stop_ids = b.build();
        let mut b = ExternalIdTableBuilder::with_capacity(self.stops.len());
        b.extend(self.stops.iter().filter_map(|s| s.parent.clone()));
        let parent_ids = b.build();
        let stop_count = stop_ids.len();
        // Builder index -> stop id (the first of a repeated external id wins).
        let stop_of: Vec<u32> = self
            .stops
            .iter()
            .map(|s| stop_ids.id_of(&s.external_id).expect("every stop was offered"))
            .collect();
        let mut stop_position = vec![LonLat::new(0.0, 0.0); stop_count];
        let mut stop_name: Vec<Box<str>> = vec![Box::from(""); stop_count];
        let mut stop_parent = vec![NULL_ID; stop_count];
        let mut seen = vec![false; stop_count];
        for (spec, &id) in self.stops.iter().zip(&stop_of) {
            let i = id as usize;
            if seen[i] {
                continue;
            }
            seen[i] = true;
            stop_position[i] = spec.position;
            stop_name[i] = spec.name.as_str().into();
            stop_parent[i] =
                spec.parent.as_deref().and_then(|p| parent_ids.id_of(p)).unwrap_or(NULL_ID);
        }

        let mut b = ExternalIdTableBuilder::with_capacity(self.routes.len());
        b.extend(self.routes.iter().map(|r| r.external_id.clone()));
        let route_ids = b.build();
        let route_count = route_ids.len();
        let route_of: Vec<u32> = self
            .routes
            .iter()
            .map(|r| route_ids.id_of(&r.external_id).expect("every route was offered"))
            .collect();
        let mut route_short_name: Vec<Box<str>> = vec![Box::from(""); route_count];
        let mut route_type = vec![0u16; route_count];
        let mut seen = vec![false; route_count];
        for (spec, &id) in self.routes.iter().zip(&route_of) {
            let i = id as usize;
            if !seen[i] {
                seen[i] = true;
                route_short_name[i] = spec.short_name.as_str().into();
                route_type[i] = spec.route_type;
            }
        }
        let route_kind = route_type.iter().map(|&t| ServiceKind::from_route_type(t)).collect();

        // Runs that are kept, by external id; the first of a repeated one wins.
        let mut kept: Vec<usize> = (0..self.runs.len())
            .filter(|&i| {
                let calls = (self.run_calls[i + 1] - self.run_calls[i]) as usize;
                let ok = calls >= 2 && (self.runs[i].1 as usize) < self.routes.len();
                if !ok {
                    report.runs_too_short += 1;
                }
                ok
            })
            .collect();
        kept.sort_by(|&a, &b| self.runs[a].0.cmp(&self.runs[b].0).then(a.cmp(&b)));
        kept.dedup_by(|b, a| {
            let repeated = self.runs[*a].0 == self.runs[*b].0;
            if repeated {
                report.runs_repeated += 1;
            }
            repeated
        });
        let mut b = ExternalIdTableBuilder::with_capacity(kept.len());
        b.extend(kept.iter().map(|&i| self.runs[i].0.clone()));
        let run_ids = b.build();

        let mut run_route = Vec::with_capacity(kept.len());
        let mut run_start = Vec::with_capacity(kept.len() + 1);
        run_start.push(0u32);
        let total: usize =
            kept.iter().map(|&i| (self.run_calls[i + 1] - self.run_calls[i]) as usize).sum();
        let mut call_stop = Vec::with_capacity(total);
        let mut call_flags = Vec::with_capacity(total);
        let mut arrival = Vec::with_capacity(total);
        let mut departure = Vec::with_capacity(total);
        for &i in &kept {
            run_route.push(route_of[self.runs[i].1 as usize]);
            let calls = &self.calls[self.run_calls[i] as usize..self.run_calls[i + 1] as usize];
            let mut clock = 0u32;
            for c in calls {
                let stop = stop_of[c.stop as usize];
                let a = if c.arrival < clock {
                    report.times_repaired += 1;
                    clock
                } else {
                    c.arrival
                };
                let d = if c.departure < a {
                    report.times_repaired += 1;
                    a
                } else {
                    c.departure
                };
                clock = d;
                call_stop.push(NodeId::new(stop));
                call_flags.push(c.flags & (BOARD | ALIGHT));
                arrival.push(a);
                departure.push(d);
            }
            run_start.push(u32::try_from(call_stop.len()).expect("calls fit u32"));
        }

        let mut transfers: Vec<Transfer> = self
            .transfers
            .iter()
            .map(|&(f, t, s)| Transfer {
                from: NodeId::new(stop_of[f as usize]),
                to: NodeId::new(stop_of[t as usize]),
                seconds: s,
            })
            .collect();
        transfers.sort();
        transfers.dedup_by(|b, a| a.from == b.from && a.to == b.to);

        let mut table = Timetable {
            date: self.date,
            stop_ids,
            stop_position,
            stop_name,
            stop_parent,
            parent_ids,
            route_ids,
            route_short_name,
            route_type,
            route_kind,
            run_ids,
            run_route,
            run_start,
            call_stop,
            call_flags,
            scheduled: CallTimes { arrival, departure },
            transfers,
            group_of_run: Vec::new(),
            group_start: Vec::new(),
            group_runs: Vec::new(),
        };
        table.group_runs_into_patterns();
        (table, report)
    }
}

/// Arrival and departure times for every call of a [`Timetable`], by call
/// index: the scheduled ones, or realised ones of the same shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallTimes {
    /// Arrival at each call, seconds after midnight, or [`UNKNOWN_TIME`].
    pub arrival: Vec<u32>,
    /// Departure from each call, likewise.
    pub departure: Vec<u32>,
}

/// The timetable of one service day: see the [module docs](self).
#[derive(Clone, Debug)]
pub struct Timetable {
    date: ServiceDate,
    stop_ids: ExternalIdTable,
    stop_position: Vec<LonLat>,
    stop_name: Vec<Box<str>>,
    stop_parent: Vec<u32>,
    parent_ids: ExternalIdTable,
    route_ids: ExternalIdTable,
    route_short_name: Vec<Box<str>>,
    route_type: Vec<u16>,
    route_kind: Vec<ServiceKind>,
    run_ids: ExternalIdTable,
    run_route: Vec<u32>,
    /// `runs + 1` offsets into the call arrays.
    run_start: Vec<u32>,
    call_stop: Vec<NodeId>,
    call_flags: Vec<u8>,
    scheduled: CallTimes,
    transfers: Vec<Transfer>,
    group_of_run: Vec<u32>,
    /// `groups + 1` offsets into `group_runs`.
    group_start: Vec<u32>,
    /// Each group's runs, by scheduled departure from the first call, then id.
    group_runs: Vec<TransitRunId>,
}

impl Timetable {
    /// The service day.
    #[must_use]
    pub fn date(&self) -> ServiceDate {
        self.date
    }

    /// How many stops.
    #[must_use]
    pub fn stop_count(&self) -> u32 {
        self.stop_ids.count()
    }

    /// The stops' external ids (GTFS `stop_id`).
    #[must_use]
    pub fn stop_ids(&self) -> &ExternalIdTable {
        &self.stop_ids
    }

    /// Where a stop is.
    #[must_use]
    pub fn stop_position(&self, stop: NodeId) -> LonLat {
        self.stop_position[stop.index()]
    }

    /// A stop's name.
    #[must_use]
    pub fn stop_name(&self, stop: NodeId) -> &str {
        &self.stop_name[stop.index()]
    }

    /// The station a stop belongs to (`parent_station`), if any.
    #[must_use]
    pub fn stop_parent(&self, stop: NodeId) -> Option<&str> {
        let p = self.stop_parent[stop.index()];
        (p != NULL_ID).then(|| self.parent_ids.external(p))
    }

    /// How many routes.
    #[must_use]
    pub fn route_count(&self) -> u32 {
        self.route_ids.count()
    }

    /// The routes' external ids (GTFS `route_id`).
    #[must_use]
    pub fn route_ids(&self) -> &ExternalIdTable {
        &self.route_ids
    }

    /// A route's short name.
    #[must_use]
    pub fn route_short_name(&self, route: u32) -> &str {
        &self.route_short_name[route as usize]
    }

    /// A route's GTFS `route_type`.
    #[must_use]
    pub fn route_type(&self, route: u32) -> u16 {
        self.route_type[route as usize]
    }

    /// A route's kind of service.
    #[must_use]
    pub fn route_kind(&self, route: u32) -> ServiceKind {
        self.route_kind[route as usize]
    }

    /// How many runs.
    #[must_use]
    pub fn run_count(&self) -> u32 {
        self.run_ids.count()
    }

    /// The runs' external ids (GTFS `trip_id`).
    #[must_use]
    pub fn run_ids(&self) -> &ExternalIdTable {
        &self.run_ids
    }

    /// The route a run belongs to.
    #[must_use]
    pub fn run_route(&self, run: TransitRunId) -> u32 {
        self.run_route[run.index()]
    }

    /// The kind of service a run is.
    #[must_use]
    pub fn run_kind(&self, run: TransitRunId) -> ServiceKind {
        self.route_kind[self.run_route[run.index()] as usize]
    }

    /// A run's calls, as a range of call indices.
    #[must_use]
    pub fn run_calls(&self, run: TransitRunId) -> core::ops::Range<usize> {
        self.run_start[run.index()] as usize..self.run_start[run.index() + 1] as usize
    }

    /// How many calls, over all runs.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.call_stop.len()
    }

    /// The stop of a call.
    #[must_use]
    pub fn call_stop(&self, call: usize) -> NodeId {
        self.call_stop[call]
    }

    /// Whether passengers may board ([`BOARD`]) and alight ([`ALIGHT`]) at a call.
    #[must_use]
    pub fn call_flags(&self, call: usize) -> u8 {
        self.call_flags[call]
    }

    /// The scheduled times.
    #[must_use]
    pub fn scheduled(&self) -> &CallTimes {
        &self.scheduled
    }

    /// The explicit minimum transfer times between stops, sorted.
    #[must_use]
    pub fn transfers(&self) -> &[Transfer] {
        &self.transfers
    }

    /// How many pattern groups (see the [module docs](self)).
    #[must_use]
    pub fn group_count(&self) -> u32 {
        // At most one group per run, and runs are counted in u32.
        #[allow(clippy::cast_possible_truncation, reason = "no more groups than runs, a u32 count")]
        let groups = self.group_start.len().saturating_sub(1) as u32;
        groups
    }

    /// A group's runs, by scheduled departure from their first call.
    #[must_use]
    pub fn group_runs(&self, group: u32) -> &[TransitRunId] {
        let g = group as usize;
        &self.group_runs[self.group_start[g] as usize..self.group_start[g + 1] as usize]
    }

    /// The group a run is in.
    #[must_use]
    pub fn run_group(&self, run: TransitRunId) -> u32 {
        self.group_of_run[run.index()]
    }

    /// Bytes held.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.stop_ids.arena_bytes()
            + self.parent_ids.arena_bytes()
            + self.route_ids.arena_bytes()
            + self.run_ids.arena_bytes()
            + self.stop_position.len() * core::mem::size_of::<LonLat>()
            + self.stop_name.iter().map(|n| n.len() + 16).sum::<usize>()
            + self.stop_parent.len() * 4
            + self.route_short_name.iter().map(|n| n.len() + 16).sum::<usize>()
            + self.route_type.len() * 3
            + (self.run_route.len() + self.run_start.len() + self.group_of_run.len()) * 4
            + self.call_stop.len() * (4 + 1 + 8)
            + self.transfers.len() * core::mem::size_of::<Transfer>()
            + (self.group_start.len() + self.group_runs.len()) * 4
    }

    fn group_runs_into_patterns(&mut self) {
        let runs = self.run_count() as usize;
        let mut key_of: HashMap<(u32, Vec<(u32, u8)>), u32> = HashMap::new();
        let mut members: Vec<Vec<TransitRunId>> = Vec::new();
        let mut group_of_run = vec![0u32; runs];
        // Runs in id order, so a group's number is the order of its smallest run id.
        for (r, group_slot) in group_of_run.iter_mut().enumerate() {
            let run = TransitRunId::from_index(r);
            let key: Vec<(u32, u8)> = self
                .run_calls(run)
                .map(|c| (self.call_stop[c].raw(), self.call_flags[c]))
                .collect();
            let next = u32::try_from(members.len()).expect("groups fit u32");
            let g = *key_of.entry((self.run_route[r], key)).or_insert(next);
            if g == next {
                members.push(Vec::new());
            }
            members[g as usize].push(run);
            *group_slot = g;
        }
        let mut group_start = vec![0u32];
        let mut group_runs = Vec::with_capacity(runs);
        for mut list in members {
            list.sort_by_key(|&r| {
                (self.scheduled.departure[self.run_start[r.index()] as usize], r)
            });
            group_runs.extend(list);
            group_start.push(u32::try_from(group_runs.len()).expect("runs fit u32"));
        }
        self.group_of_run = group_of_run;
        self.group_start = group_start;
        self.group_runs = group_runs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(id: &str) -> StopSpec {
        StopSpec {
            external_id: id.into(),
            name: id.to_uppercase(),
            position: LonLat::new(4.9, 52.37),
            parent: None,
        }
    }

    fn call(stop: u32, t: u32) -> CallSpec {
        CallSpec { stop, arrival: t, departure: t, flags: BOARD | ALIGHT }
    }

    #[test]
    fn ids_follow_sorted_external_ids_and_short_runs_are_dropped() {
        let mut b = TimetableBuilder::new(ServiceDate::parse("20261009").unwrap());
        let (z, a) = (b.add_stop(stop("z")), b.add_stop(stop("a")));
        let line = b.add_route(RouteSpec {
            external_id: "L".into(),
            short_name: "1".into(),
            route_type: 3,
        });
        b.add_run("t2", line, &[call(z, 100), call(a, 200)]);
        b.add_run("t1", line, &[call(z, 50), call(a, 150)]);
        b.add_run("short", line, &[call(z, 10)]);
        b.add_run("t1", line, &[call(a, 1), call(z, 2)]);
        let (t, report) = b.build();
        assert_eq!(report.runs_too_short, 1);
        assert_eq!(report.runs_repeated, 1);
        assert_eq!(t.stop_ids().id_of("a"), Some(0));
        assert_eq!(t.run_count(), 2);
        let t1 = TransitRunId::new(t.run_ids().id_of("t1").unwrap());
        let calls = t.run_calls(t1);
        assert_eq!(t.call_stop(calls.start), NodeId::new(1), "t1 starts at z");
        assert_eq!(t.scheduled().departure[calls.start], 50);
        assert_eq!(t.route_kind(0), ServiceKind::Bus);
        assert_eq!(t.group_count(), 1, "same route, same stops: one group");
        let group: Vec<&str> =
            t.group_runs(0).iter().map(|&r| t.run_ids().external(r.raw())).collect();
        assert_eq!(group, ["t1", "t2"], "by departure");
    }

    #[test]
    fn backward_times_are_moved_forward() {
        let mut b = TimetableBuilder::new(ServiceDate::parse("20261009").unwrap());
        let (x, y, z) = (b.add_stop(stop("x")), b.add_stop(stop("y")), b.add_stop(stop("z")));
        let line = b.add_route(RouteSpec {
            external_id: "L".into(),
            short_name: "1".into(),
            route_type: 0,
        });
        b.add_run(
            "r",
            line,
            &[
                call(x, 100),
                CallSpec { stop: y, arrival: 90, departure: 80, flags: BOARD },
                call(z, 200),
            ],
        );
        let (t, report) = b.build();
        assert_eq!(report.times_repaired, 2);
        let s = t.scheduled();
        assert_eq!((s.arrival[1], s.departure[1]), (100, 100));
        assert_eq!(t.call_flags(1), BOARD);
    }
}
