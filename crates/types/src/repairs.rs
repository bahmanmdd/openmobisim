//! What a reader set right instead of refusing: per kind of repair, how many times it was made
//! and the ids of the first five things it was made on, in the order met.
//!
//! A reader (of an OpenStreetMap extract, a GTFS feed, a table, a demand) or a run refuses only
//! what it cannot repair; everything else it repairs and notes here, one kind per rule, so that
//! nothing is set right silently: its caller reports every kind with its count and first ids
//! (the Python binding: one `UserWarning` per kind; the run's record: the count). A kind is named
//! by a key (`tags_unread`, `trips_outside_window`), the same in the record and in Python, and
//! described by a phrase that completes "N …" (`ways whose access, one-way or lane value was not
//! read`).

/// The most ids kept per kind.
pub const FIRST: usize = 5;

/// One kind of repair: its key, its phrase, how many, and the first ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repair {
    /// The key, `snake_case`: the record's key and the Python dict's.
    pub key: &'static str,
    /// What was repaired, a phrase completing "N …".
    pub what: &'static str,
    /// How many times.
    pub count: u64,
    /// The ids of the first [`FIRST`] things repaired, in the order met.
    pub first: Vec<String>,
}

/// The repairs a reader or a run made, in the order their kinds were first met.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repairs {
    kinds: Vec<Repair>,
}

impl Repairs {
    /// No repair.
    #[must_use]
    pub const fn new() -> Self {
        Self { kinds: Vec::new() }
    }

    /// Note one repair of kind `key` (described by `what`) made on `id`.
    pub fn note(&mut self, key: &'static str, what: &'static str, id: impl Into<String>) {
        let at = match self.kinds.iter().position(|k| k.key == key) {
            Some(at) => at,
            None => {
                self.kinds.push(Repair { key, what, count: 0, first: Vec::new() });
                self.kinds.len() - 1
            }
        };
        let kind = &mut self.kinds[at];
        kind.count += 1;
        if kind.first.len() < FIRST {
            kind.first.push(id.into());
        }
    }

    /// Every kind noted, in the order first met.
    #[must_use]
    pub fn kinds(&self) -> &[Repair] {
        &self.kinds
    }

    /// How many repairs of kind `key` (0 when none).
    #[must_use]
    pub fn count(&self, key: &str) -> u64 {
        self.kinds.iter().find(|k| k.key == key).map_or(0, |k| k.count)
    }

    /// Whether no repair was made.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// The repairs of `other` added to these, kind by kind (the first ids kept up to [`FIRST`]).
    pub fn extend(&mut self, other: &Repairs) {
        for k in &other.kinds {
            for id in &k.first {
                self.note(k.key, k.what, id.clone());
            }
            let extra = k.count - k.first.len() as u64;
            if let Some(mine) = self.kinds.iter_mut().find(|m| m.key == k.key) {
                mine.count += extra;
            }
        }
    }
}
