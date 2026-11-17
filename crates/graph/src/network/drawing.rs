//! What a link looks like on a map, for drawing only: its shape (the points between its two
//! ends, in its direction of travel) and its road class (a name). No run reads either: they are
//! not in [`Network::fingerprint`], and a link's length is the row's (or the straight line
//! between its ends), never measured along the shape here.

use openmobisim_types::ids::{EntityId, LinkId};

use super::{Network, NetworkBuilder};
use crate::geometry::{Position, Projection, Xy};
use openmobisim_types::repairs::Repairs;

/// A link's drawing: the points between its upstream and its downstream end, in order of travel
/// (empty: straight between them), each of the nodes' kind (longitude/latitude, projected as the
/// nodes are, or metres); and its road class, if any.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct LinkDrawing {
    /// The points between the two ends, in order of travel.
    pub shape: Vec<Position>,
    /// The road class's name (`motorway`, `residential`, …), or `None`.
    pub road_class: Option<String>,
}

/// The key and phrase of a shape that cannot be drawn: the graph's, and the one a table reader
/// uses for a shape text it cannot read, so that one count says how many links are drawn
/// straight for want of a usable shape.
pub const SHAPES_UNREAD: (&str, &str) = (
    "shapes_unread",
    "shapes that cannot be read as points of the nodes' kind, finite and projectable: drawn straight",
);

impl NetworkBuilder {
    /// Give a link its drawing: it belongs to the row last given with this link id, so a link
    /// is declared first. A drawing given before any row of its link is ignored
    /// (`drawings_unplaced`), a second one for one row too (`drawings_repeated`). For drawing
    /// only: nothing a run reads changes, and the fingerprint does not see it.
    pub fn add_drawing(&mut self, link: impl Into<String>, drawing: LinkDrawing) {
        let link = link.into();
        match self.last_row.get(&link) {
            None => self.stray_drawings.push(link),
            Some(&row) if self.links[row].drawing.is_some() => self.second_drawings.push(link),
            Some(&row) => self.links[row].drawing = Some(drawing),
        }
    }
}

/// The drawings set aside as they were given: of a link no row was given for, and second ones.
pub(super) struct Given {
    pub stray: Vec<String>,
    pub second: Vec<String>,
}

/// The drawings of a built network, by link id: each link's shape points in metres (`start`,
/// one past the last per link, `points`), and its road class (an index into `classes`, the
/// distinct names in sorted order; `u32::MAX` for none).
#[derive(Debug, Default)]
pub(super) struct Drawings {
    start: Vec<u32>,
    points: Vec<Xy>,
    class_of: Vec<u32>,
    classes: Vec<String>,
}

/// The drawings of the links held (`by_link`, in link id order, the kept row's), placed; those
/// set aside when given (`given`) noted: `drawings_unplaced`, `drawings_repeated`. A shape with
/// a point not of the nodes' kind, not finite, or not projectable is drawn straight
/// ([`SHAPES_UNREAD`]). A road class is kept as given. A drawing of a row left out goes with it,
/// its row's fault noted once, there.
pub(super) fn build(
    by_link: &[(String, Option<LinkDrawing>)],
    given: Given,
    projection: Option<Projection>,
    repairs: &mut Repairs,
) -> Drawings {
    let mut stray = given.stray;
    stray.sort();
    for link in stray {
        repairs.note(
            "drawings_unplaced",
            "drawings given for a link no row was given for: ignored",
            link,
        );
    }
    let mut second = given.second;
    second.sort();
    for link in second {
        repairs.note("drawings_repeated", "links given two drawings: the first kept", link);
    }
    let n = by_link.len();
    let mut classes: Vec<String> =
        by_link.iter().filter_map(|(_, d)| d.as_ref()?.road_class.clone()).collect();
    classes.sort_unstable();
    classes.dedup();
    let mut out = Drawings { start: Vec::with_capacity(n + 1), classes, ..Drawings::default() };
    out.start.push(0);
    for (link, drawing) in by_link {
        let (shape, class) =
            drawing.as_ref().map_or((&[][..], None), |d| (&d.shape[..], d.road_class.as_ref()));
        let points: Option<Vec<Xy>> = shape
            .iter()
            .map(|&at| match (at, projection) {
                (Position::LonLat(p), Some(proj)) if p.is_projectable() => Some(proj.project(p)),
                (Position::Metres(xy), None) if xy.x.is_finite() && xy.y.is_finite() => Some(xy),
                _ => None,
            })
            .collect();
        match points {
            Some(points) => out.points.extend(points),
            None => repairs.note(SHAPES_UNREAD.0, SHAPES_UNREAD.1, link.clone()),
        }
        out.start.push(u32::try_from(out.points.len()).expect("fewer than 2^32 shape points"));
        let index = class.map(|c| out.classes.binary_search(c).expect("collected above"));
        let index = index.map(|k| u32::try_from(k).expect("fewer than 2^32 classes"));
        out.class_of.push(index.unwrap_or(u32::MAX));
    }
    out
}

impl Network {
    /// A link's shape: the points between its upstream and its downstream end, in metres, in
    /// order of travel; empty when it is drawn straight. For drawing only.
    #[must_use]
    pub fn shape(&self, link: LinkId) -> &[Xy] {
        let d = &self.drawings;
        &d.points[d.start[link.index()] as usize..d.start[link.index() + 1] as usize]
    }

    /// A link's road class, if it has one. For drawing only.
    #[must_use]
    pub fn road_class(&self, link: LinkId) -> Option<&str> {
        let k = self.drawings.class_of[link.index()];
        (k != u32::MAX).then(|| self.drawings.classes[k as usize].as_str())
    }
}
