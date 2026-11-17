//! The network's arrays (nodes, links, each link's shape, facts and drawing class) and the
//! traffic on it: cars per link and bin, bikes and walkers per link over the window.

use openmobisim_graph::network::Network;
use openmobisim_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_types::mode::Mode;

use crate::MapError;
use crate::blob::Blob;
use crate::kept::Kept;
use crate::rules;

/// Where positions are counted from, metres: the least x and the least y of the nodes (0 for a
/// network without nodes). The page holds positions in decimetres from it.
pub(crate) fn origin(network: &Network) -> (f64, f64) {
    let mut o = (f64::INFINITY, f64::INFINITY);
    for i in 0..network.node_count() {
        let p = network.node_position(NodeId::new(i));
        o = (o.0.min(p.x), o.1.min(p.y));
    }
    if o.0.is_finite() { o } else { (0.0, 0.0) }
}

/// `v` metres as whole decimetres from `from`, rounded to the nearest.
///
/// # Errors
///
/// [`MapError::TooFar`] beyond 2³¹ − 1 decimetres (214 748 km).
pub(crate) fn decimetres(v: f64, from: f64) -> Result<i32, MapError> {
    let d = ((v - from) * 10.0).round();
    if d.is_nan() || d.abs() > f64::from(i32::MAX) {
        return Err(MapError::TooFar { metres: v - from });
    }
    #[allow(clippy::cast_possible_truncation, reason = "checked above: within i32")]
    Ok(d as i32)
}

/// A finite value as a 32-bit float, the page's precision (a few parts in ten million).
#[allow(clippy::cast_possible_truncation, reason = "a value drawn or shown, not computed with")]
pub(crate) fn f32_of(x: f64) -> f32 {
    x as f32
}

/// Texts in the block: their UTF-8 bytes, each followed by a 0 byte (`name`); the page splits them.
fn texts<'a>(blob: &mut Blob, name: &'static str, texts: impl Iterator<Item = &'a str>) {
    let mut bytes = Vec::new();
    for t in texts {
        bytes.extend_from_slice(t.as_bytes());
        bytes.push(0);
    }
    blob.u8(name, &bytes);
}

/// Add the nodes (`nx`, `ny`: decimetres from `origin`; their external ids, `nid`) and
/// each link's external id (`lid`), upstream and downstream
/// node (`lu`, `ld`), shape (`vs`, per link the first of its points and one past the last; `dx`,
/// `dy` the points, each in decimetres from the one before it in the array, the first from
/// `origin`), modes (`lm`: 1 car, 2 bike, 4 walk, summed), drawing class (`cls`), length
/// (`len`, m), car free-flow time (`ff`, s), capacity (`cap`, veh/h), lanes, and car speed (`spd`,
/// km/h); the car fields 0 on a link without cars, the capacity 0 where it has no limit. When every
/// link has a road class, the drawing class is its rank ([`rules::road_rank`]) and `rc` its name
/// among the distinct names returned; else the class by capacity ([`rules::class`]) and `None`.
///
/// # Errors
///
/// [`MapError::TooFar`] for a node or shape point too far from the origin.
pub(crate) fn add_network(
    blob: &mut Blob,
    network: &Network,
    origin: (f64, f64),
) -> Result<Option<Vec<String>>, MapError> {
    let (mut nx, mut ny) = (Vec::new(), Vec::new());
    for i in 0..network.node_count() {
        let p = network.node_position(NodeId::new(i));
        nx.push(decimetres(p.x, origin.0)?);
        ny.push(decimetres(p.y, origin.1)?);
    }
    blob.i32("nx", &nx);
    blob.i32("ny", &ny);
    add_shapes(blob, network, origin)?;
    texts(blob, "nid", (0..network.node_count()).map(|i| network.node_ids().external(i)));
    texts(blob, "lid", (0..network.link_count()).map(|i| network.link_ids().external(i)));
    let links: Vec<LinkId> = (0..network.link_count()).map(LinkId::new).collect();
    let car = |l: LinkId| network.modes(l).contains(Mode::Car);
    let capacity = |l: LinkId| network.capacity(l).as_veh_per_hour();
    let on_car = |l: LinkId, f: &dyn Fn(LinkId) -> f64| if car(l) { f32_of(f(l)) } else { 0.0 };
    let modes = |l: LinkId| {
        let m = network.modes(l);
        u8::from(m.contains(Mode::Car))
            | u8::from(m.contains(Mode::Bike)) << 1
            | u8::from(m.contains(Mode::Walk)) << 2
    };
    blob.u32("lu", &links.iter().map(|&l| network.upstream(l).raw()).collect::<Vec<_>>());
    blob.u32("ld", &links.iter().map(|&l| network.downstream(l).raw()).collect::<Vec<_>>());
    blob.u8("lm", &links.iter().map(|&l| modes(l)).collect::<Vec<_>>());
    let classes = add_classes(blob, network);
    if classes.is_none() {
        let by_capacity = links.iter().map(|&l| rules::class(car(l), capacity(l)));
        blob.u8("cls", &by_capacity.collect::<Vec<_>>());
    }
    blob.f32("len", &links.iter().map(|&l| f32_of(network.length(l).get())).collect::<Vec<_>>());
    let free_flow = |l: LinkId| network.free_flow_time(l, Mode::Car).get();
    blob.f32("ff", &links.iter().map(|&l| on_car(l, &free_flow)).collect::<Vec<_>>());
    let limit = |l: LinkId| if capacity(l).is_finite() { capacity(l) } else { 0.0 };
    blob.f32("cap", &links.iter().map(|&l| on_car(l, &limit)).collect::<Vec<_>>());
    blob.u8("lanes", &links.iter().map(|&l| network.lanes(l)).collect::<Vec<_>>());
    let km_h = |l: LinkId| network.free_flow_speed(l, Mode::Car).get() * 3.6;
    blob.f32("spd", &links.iter().map(|&l| on_car(l, &km_h)).collect::<Vec<_>>());
    Ok(classes)
}

/// Add each link's shape: `vs`, `dx`, `dy` (see [`add_network`]).
fn add_shapes(blob: &mut Blob, network: &Network, origin: (f64, f64)) -> Result<(), MapError> {
    let (mut start, mut dx, mut dy) = (vec![0_u32], Vec::new(), Vec::new());
    let mut last = (0, 0);
    for l in (0..network.link_count()).map(LinkId::new) {
        for p in network.shape(l) {
            let at = (decimetres(p.x, origin.0)?, decimetres(p.y, origin.1)?);
            let step = |a: i32, b: i32| {
                i32::try_from(i64::from(a) - i64::from(b))
                    .map_err(|_| MapError::TooFar { metres: f64::from(a) / 10.0 })
            };
            dx.push(step(at.0, last.0)?);
            dy.push(step(at.1, last.1)?);
            last = at;
        }
        start.push(u32::try_from(dx.len()).expect("fewer than 2^32 shape points"));
    }
    blob.u32("vs", &start);
    blob.i32("dx", &dx);
    blob.i32("dy", &dy);
    Ok(())
}

/// When every link has a road class (a network with links): add each link's drawing class
/// (`cls`, its rank) and its name's place among the distinct names (`rc`), and return those names
/// in sorted order; else add nothing and return `None`.
fn add_classes(blob: &mut Blob, network: &Network) -> Option<Vec<String>> {
    let named: Option<Vec<&str>> =
        (0..network.link_count()).map(|l| network.road_class(LinkId::new(l))).collect();
    let named = named.filter(|n| !n.is_empty())?;
    let mut names: Vec<&str> = named.clone();
    names.sort_unstable();
    names.dedup();
    let place = |n: &str| u32::try_from(names.binary_search(&n).expect("listed")).expect("few");
    blob.u8("cls", &named.iter().map(|n| rules::road_rank(n)).collect::<Vec<_>>());
    blob.u32("rc", &named.iter().map(|n| place(n)).collect::<Vec<_>>());
    Some(names.into_iter().map(str::to_owned).collect())
}

/// Add the traffic: the rows with cars, by bin then link (`bs`, per bin of the window the first
/// row and one past the last; `rl` the link, `rp` the cars that entered it, `rs` the sum of their
/// crossing times, s); and per link the bikes (`bv`) and walkers (`wv`) that entered it over the
/// window. Returns how many rows with cars.
///
/// # Errors
///
/// [`MapError::NotTheRunsNetwork`] for a row of a link the network does not have.
pub(crate) fn add_traffic(
    blob: &mut Blob,
    kept: &Kept,
    network: &Network,
) -> Result<usize, MapError> {
    let r = &kept.rows;
    let n = network.link_count() as usize;
    if let Some(&l) = r.link.iter().find(|l| l.index() >= n) {
        return Err(MapError::NotTheRunsNetwork { link: l.raw(), links: network.link_count() });
    }
    let (mut bikes, mut walkers) = (vec![0_u32; n], vec![0_u32; n]);
    for i in 0..r.link.len() {
        bikes[r.link[i].index()] += r.bikes[i];
        walkers[r.link[i].index()] += r.walkers[i];
    }
    let mut cars: Vec<usize> = (0..r.link.len()).filter(|&i| r.cars[i] > 0).collect();
    cars.sort_by_key(|&i| (r.bin[i], r.link[i]));
    let bins = kept.window.n_steps() as usize;
    let mut start = vec![0_u32; bins + 1];
    for &i in &cars {
        start[r.bin[i] as usize + 1] += 1;
    }
    for b in 0..bins {
        start[b + 1] += start[b];
    }
    blob.u32("bs", &start);
    blob.u32("rl", &cars.iter().map(|&i| r.link[i].raw()).collect::<Vec<_>>());
    blob.u32("rp", &cars.iter().map(|&i| r.cars[i]).collect::<Vec<_>>());
    blob.f32("rs", &cars.iter().map(|&i| f32_of(r.car_time[i])).collect::<Vec<_>>());
    blob.u32("bv", &bikes);
    blob.u32("wv", &walkers);
    Ok(cars.len())
}
