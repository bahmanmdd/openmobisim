//! The toy network's timetable (S199): the fixture the hand-derived transit
//! cases run on.
//!
//! **Tram `T1`** — a two-stop shuttle, segregated (it runs by the schedule),
//! between stop `N1` at the road node `N1` and stop `H` at `D2`, the hub site:
//! out from `N1` at `600·k` seconds, in `300` s to `H`; back from `H` at
//! `600·k + 300`, in `300` s to `N1`; for `k = 0..18`, three hours from the
//! run's zero. It carries the return leg the one-way roads cannot.
//!
//! **Bus `B1`** — on the one-way arterial `W → S → M → X0 → X1 → X2 → X3 →
//! D1`, calling at `W`, `M` and `D1` (stops at those road nodes): from `W` at
//! `600·k`, at `M` at `600·k + 120`, at `D1` at `600·k + 240`; `k = 0..18`; one
//! direction, as the road is one-way. It rides the roads among the cars.

use openmobisim_core_graph::examples::toy_network;
use openmobisim_core_types::ids::NodeId;

use crate::date::ServiceDate;
use crate::timetable::{ALIGHT, BOARD, CallSpec, RouteSpec, StopSpec, Timetable, TimetableBuilder};

/// The tram's headway, in seconds.
pub const TOY_TRAM_HEADWAY_S: u32 = 600;
/// The tram's ride between its two stops, in seconds.
pub const TOY_TRAM_RIDE_S: u32 = 300;
/// How many runs each way.
pub const TOY_TRAM_RUNS: u32 = 18;
/// The bus's headway, in seconds.
pub const TOY_BUS_HEADWAY_S: u32 = 600;
/// The bus's scheduled times at `W`, `M` and `D1` after it leaves `W`.
pub const TOY_BUS_SCHEDULE_S: [u32; 3] = [0, 120, 240];
/// How many bus runs.
pub const TOY_BUS_RUNS: u32 = 18;

/// The toy tram `T1` (see the [module docs](self)).
#[must_use]
pub fn toy_tram() -> Timetable {
    toy_with(true, false)
}

/// The toy bus `B1` (see the [module docs](self)).
#[must_use]
pub fn toy_bus() -> Timetable {
    toy_with(false, true)
}

/// The toy network's whole timetable: the tram `T1` and the bus `B1`.
#[must_use]
pub fn toy_transit() -> Timetable {
    toy_with(true, true)
}

fn toy_with(tram: bool, bus: bool) -> Timetable {
    let (network, _) = toy_network();
    let at = |name: &str| {
        let node: NodeId = network.node_external_ids().typed_id_of(name).expect("a toy node");
        network.node_lonlat(node)
    };
    let mut b = TimetableBuilder::new(ServiceDate::parse("20261009").expect("a date"));
    let stop = |b: &mut TimetableBuilder, name: &str, node: &str, what: &str| {
        b.add_stop(StopSpec {
            external_id: name.to_string(),
            name: format!("{name} ({what})"),
            position: at(node),
            parent: None,
        })
    };
    let call =
        |stop: u32, t: u32| CallSpec { stop, arrival: t, departure: t, flags: BOARD | ALIGHT };
    if tram {
        let n1 = stop(&mut b, "N1", "N1", "tram");
        let h = stop(&mut b, "H", "D2", "tram");
        let line = b.add_route(RouteSpec {
            external_id: "T1".into(),
            short_name: "T1".into(),
            route_type: 0,
        });
        for k in 0..TOY_TRAM_RUNS {
            let out = k * TOY_TRAM_HEADWAY_S;
            b.add_run(
                format!("T1:out:{k:02}"),
                line,
                &[call(n1, out), call(h, out + TOY_TRAM_RIDE_S)],
            );
            let back = out + TOY_TRAM_RIDE_S;
            b.add_run(
                format!("T1:back:{k:02}"),
                line,
                &[call(h, back), call(n1, back + TOY_TRAM_RIDE_S)],
            );
        }
    }
    if bus {
        let stops: Vec<u32> = ["W", "M", "D1"].iter().map(|&n| stop(&mut b, n, n, "bus")).collect();
        let line = b.add_route(RouteSpec {
            external_id: "B1".into(),
            short_name: "B1".into(),
            route_type: 3,
        });
        for k in 0..TOY_BUS_RUNS {
            let start = k * TOY_BUS_HEADWAY_S;
            let calls: Vec<CallSpec> =
                stops.iter().zip(TOY_BUS_SCHEDULE_S).map(|(&s, t)| call(s, start + t)).collect();
            b.add_run(format!("B1:{k:02}"), line, &calls);
        }
    }
    b.build().0
}
