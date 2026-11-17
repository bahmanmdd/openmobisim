//! A place's name, release 0.1's rule: its nearest stop's name when that stop is within 400 m
//! ("near <name>" beyond 60 m); else where it lies from the network's centre, "1.2 km SE of the
//! centre" ("the centre" within 300 m). The centre is the mean of the links' ends; a stop's name
//! is its GTFS `stop_name`, else its id.

use openmobisim_graph::geometry::Xy;
use openmobisim_graph::network::Network;
use openmobisim_io_gtfs::Lines;
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, LinkId, StopId};

/// The eight directions, from east counter-clockwise.
const COMPASS: [&str; 8] = ["E", "NE", "N", "NW", "W", "SW", "S", "SE"];

/// The centre of the network: the mean of its links' two ends.
#[allow(clippy::cast_precision_loss, reason = "a count of links, far below 2^53")]
fn centre(network: &Network) -> Xy {
    let (mut x, mut y) = (0.0, 0.0);
    for l in (0..network.link_count()).map(LinkId::new) {
        for node in [network.upstream(l), network.downstream(l)] {
            let p = network.node_position(node);
            (x, y) = (x + p.x, y + p.y);
        }
    }
    let n = 2.0 * f64::from(network.link_count()).max(0.5);
    Xy::new(x / n, y / n)
}

/// The name of each of `points` (metres in the network's plane).
#[must_use]
pub(crate) fn names(
    network: &Network,
    timetable: Option<&Timetable>,
    lines: Option<&Lines>,
    points: &[Xy],
) -> Vec<String> {
    let stops: Vec<(Xy, String)> = timetable.map_or(Vec::new(), |tt| {
        StopId::iter_space(tt.stop_count())
            .map(|s| {
                let (x, y) = tt.stop_position(s);
                let id = tt.stop_ids().external(s.raw());
                let name = lines.and_then(|l| l.stop_name(id)).unwrap_or(id);
                (Xy::new(x.get(), y.get()), name.to_owned())
            })
            .collect()
    });
    let c = centre(network);
    points
        .iter()
        .map(|&p| {
            let nearest = stops
                .iter()
                .map(|(at, name)| (at.distance_to(p), name))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((d, name)) = nearest.filter(|(d, _)| *d <= 400.0) {
                return if d > 60.0 { format!("near {name}") } else { name.clone() };
            }
            let (dx, dy) = (p.x - c.x, p.y - c.y);
            let km = dx.hypot(dy) / 1_000.0;
            if km < 0.3 {
                return "the centre".to_owned();
            }
            #[allow(clippy::cast_possible_truncation, reason = "a direction from -4 to 4")]
            let k = (dy.atan2(dx) / core::f64::consts::FRAC_PI_4).round_ties_even() as i64;
            format!(
                "{km:.1} km {} of the centre",
                COMPASS[usize::try_from(k.rem_euclid(8)).unwrap_or(0)]
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! By hand: a network of one link from (−1 000, 0) to (1 000, 0) m, so its centre is (0, 0);
    //! one stop, "Station", at (5 000, 0).

    use openmobisim_graph::geometry::{Position, Xy};
    use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
    use openmobisim_io_gtfs::Lines;
    use openmobisim_transit::timetable::{Timetable, TimetableBuilder};
    use openmobisim_types::mode::{Mode, ModeSet};
    use openmobisim_types::units::Metres;

    use super::names;

    fn network() -> Network {
        let mut b = NetworkBuilder::new();
        b.add_node("w", Position::Metres(Xy::new(-1_000.0, 0.0)));
        b.add_node("e", Position::Metres(Xy::new(1_000.0, 0.0)));
        let walk = LinkRow {
            modes: ModeSet::of(&[Mode::Walk]),
            lanes: None,
            car_speed_km_h: None,
            length_m: None,
            capacity_veh_h: None,
            signal_at_downstream_end: false,
        };
        b.add_link("we", "w", "e", walk);
        let values = BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };
        b.build(values).expect("a network")
    }

    fn station() -> (Timetable, Lines) {
        let mut b = TimetableBuilder::new();
        b.add_stop("S", (Metres(5_000.0), Metres(0.0)));
        let mut lines = Lines::default();
        lines.stop_names.insert("S".to_owned(), "Station".to_owned());
        (b.build().expect("a timetable"), lines)
    }

    #[test]
    fn the_eight_directions_with_y_to_the_north() {
        let d = 1_000.0 * core::f64::consts::FRAC_1_SQRT_2;
        let points = [
            (1_000.0, 0.0),
            (d, d),
            (0.0, 1_000.0),
            (-d, d),
            (-1_000.0, 0.0),
            (-d, -d),
            (0.0, -1_000.0),
            (d, -d),
        ]
        .map(|(x, y)| Xy::new(x, y));
        let got = names(&network(), None, None, &points);
        let compass = ["E", "NE", "N", "NW", "W", "SW", "S", "SE"];
        let hand: Vec<String> =
            compass.iter().map(|c| format!("1.0 km {c} of the centre")).collect();
        assert_eq!(got, hand);
    }

    #[test]
    fn the_centre_within_300_m() {
        let points = [Xy::new(0.0, 0.0), Xy::new(299.0, 0.0), Xy::new(0.0, -300.0)];
        assert_eq!(
            names(&network(), None, None, &points),
            ["the centre", "the centre", "0.3 km S of the centre"]
        );
    }

    #[test]
    fn a_stop_names_a_place_within_60_m_and_is_near_it_within_400_m() {
        let (tt, lines) = station();
        let points = [5_060.0, 5_061.0, 5_400.0, 5_401.0].map(|x| Xy::new(x, 0.0));
        assert_eq!(
            names(&network(), Some(&tt), Some(&lines), &points),
            ["Station", "near Station", "near Station", "5.4 km E of the centre"]
        );
        let unnamed = names(&network(), Some(&tt), None, &[Xy::new(5_000.0, 0.0)]);
        assert_eq!(unnamed, ["S"], "a stop without a name by its id");
    }
}
