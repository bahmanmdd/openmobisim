//! The toy network's timetable (S199): the fixture the hand-derived transit
//! cases run on.
//!
//! **Tram `T1`** — a two-stop shuttle, segregated (it runs by the schedule),
//! between stop `N1` at the road node `N1` and stop `H` at `D2`, the hub site:
//! out from `N1` at `600·k` seconds, in `300` s to `H`; back from `H` at
//! `600·k + 300`, in `300` s to `N1`; for `k = 0..18`, three hours from the
//! run's zero. It carries the return leg the one-way roads cannot.

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

/// The toy tram `T1` (see the [module docs](self)).
///
/// # Panics
///
/// Never in practice: the toy network always has the nodes named.
#[must_use]
pub fn toy_tram() -> Timetable {
    let (network, _) = toy_network();
    let at = |name: &str| {
        let node: NodeId = network.node_external_ids().typed_id_of(name).expect("a toy node");
        network.node_lonlat(node)
    };
    let mut b = TimetableBuilder::new(ServiceDate::parse("20261009").expect("a date"));
    let stop = |name: &str, node: &str| StopSpec {
        external_id: name.to_string(),
        name: format!("{name} (tram)"),
        position: at(node),
        parent: None,
    };
    let n1 = b.add_stop(stop("N1", "N1"));
    let h = b.add_stop(stop("H", "D2"));
    let tram =
        b.add_route(RouteSpec { external_id: "T1".into(), short_name: "T1".into(), route_type: 0 });
    let call =
        |stop: u32, t: u32| CallSpec { stop, arrival: t, departure: t, flags: BOARD | ALIGHT };
    for k in 0..TOY_TRAM_RUNS {
        let out = k * TOY_TRAM_HEADWAY_S;
        b.add_run(format!("T1:out:{k:02}"), tram, &[call(n1, out), call(h, out + TOY_TRAM_RIDE_S)]);
        let back = out + TOY_TRAM_RIDE_S;
        b.add_run(
            format!("T1:back:{k:02}"),
            tram,
            &[call(h, back), call(n1, back + TOY_TRAM_RIDE_S)],
        );
    }
    b.build().0
}
