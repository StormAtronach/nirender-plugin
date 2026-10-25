//! Named GPU time stamps for the parts of a frame, with `MGE_XE_GPU_TIME=1`. The helper and its
//! extension both write them: a pair of stamps around a piece of work, with a name and a kind.
//! The end counts print the time of each name for each frame (`Player::print_counts`).
//!
//! A stamp outside a pass needs `TIMESTAMP_QUERY_INSIDE_ENCODERS`, one inside a pass
//! `TIMESTAMP_QUERY_INSIDE_PASSES`; without the feature the call does nothing.

use std::cell::RefCell;

/// What a named span covers, for the sums of the report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StampKind {
    /// Work outside the frame's own passes: a copy, a resolve.
    Outside,
    /// A part of the frame: it can span passes, and its time holds the copies inside it.
    Part,
    /// A piece inside a part, or in an encoder of its own: a post shader, the statics cull.
    Detail,
}

/// A started span: the pair it took, or none.
#[must_use]
pub struct Stamp(Option<u32>);

impl Stamp {
    pub const NONE: Stamp = Stamp(None);
}

struct Inner {
    set: wgpu::QuerySet,
    /// The first query of the named pairs, and how many pairs a frame has.
    first: u32,
    room: u32,
    in_encoders: bool,
    in_passes: bool,
    names: Vec<(&'static str, StampKind)>,
}

/// The named stamps of the frame in progress. Off without `MGE_XE_GPU_TIME=1`.
pub struct Stamps(RefCell<Option<Inner>>);

impl Stamps {
    pub(crate) fn off() -> Stamps {
        Stamps(RefCell::new(None))
    }

    pub(crate) fn new(set: wgpu::QuerySet, first: u32, room: u32, in_encoders: bool, in_passes: bool) -> Stamps {
        Stamps(RefCell::new(Some(Inner { set, first, room, in_encoders, in_passes, names: Vec::new() })))
    }

    /// True when the stamps are on.
    pub fn on(&self) -> bool {
        self.0.borrow().is_some()
    }

    fn take(&self, name: &'static str, kind: StampKind, inside_pass: bool) -> Option<(wgpu::QuerySet, u32)> {
        let mut inner = self.0.borrow_mut();
        let inner = inner.as_mut()?;
        let index = inner.names.len() as u32;
        if index >= inner.room || !(if inside_pass { inner.in_passes } else { inner.in_encoders }) {
            return None;
        }
        inner.names.push((name, kind));
        Some((inner.set.clone(), inner.first + 2 * index))
    }

    fn end_query(&self, stamp: &Stamp) -> Option<(wgpu::QuerySet, u32)> {
        let index = stamp.0?;
        let inner = self.0.borrow();
        let inner = inner.as_ref()?;
        Some((inner.set.clone(), inner.first + 2 * index + 1))
    }

    /// Starts a span in an encoder, with no pass open on it.
    pub fn begin(&self, encoder: &mut wgpu::CommandEncoder, name: &'static str, kind: StampKind) -> Stamp {
        match self.take(name, kind, false) {
            Some((set, query)) => {
                encoder.write_timestamp(&set, query);
                Stamp(Some((query - self.first()) / 2))
            }
            None => Stamp(None),
        }
    }

    /// Ends a span that [`Stamps::begin`] started.
    pub fn end(&self, encoder: &mut wgpu::CommandEncoder, stamp: Stamp) {
        if let Some((set, query)) = self.end_query(&stamp) {
            encoder.write_timestamp(&set, query);
        }
    }

    /// Starts a span inside a render pass.
    pub fn begin_in_pass(&self, pass: &mut wgpu::RenderPass<'_>, name: &'static str, kind: StampKind) -> Stamp {
        match self.take(name, kind, true) {
            Some((set, query)) => {
                pass.write_timestamp(&set, query);
                Stamp(Some((query - self.first()) / 2))
            }
            None => Stamp(None),
        }
    }

    /// Ends a span inside a render pass. A span may start in one pass and end in another.
    pub fn end_in_pass(&self, pass: &mut wgpu::RenderPass<'_>, stamp: Stamp) {
        if let Some((set, query)) = self.end_query(&stamp) {
            pass.write_timestamp(&set, query);
        }
    }

    fn first(&self) -> u32 {
        self.0.borrow().as_ref().map_or(0, |inner| inner.first)
    }

    /// The start of a frame: no span yet.
    pub(crate) fn clear(&self) {
        if let Some(inner) = self.0.borrow_mut().as_mut() {
            inner.names.clear();
        }
    }

    /// The names of the frame's spans, in the order of their pairs. The list is then empty.
    pub(crate) fn take_names(&self) -> Vec<(&'static str, StampKind)> {
        self.0.borrow_mut().as_mut().map_or_else(Vec::new, |inner| std::mem::take(&mut inner.names))
    }
}

/// A name for a span made at run time (a post shader's name): one copy of each distinct name
/// for the life of the program, so a name made again (a shader that is loaded again) takes no
/// more memory.
pub fn name(text: &str) -> &'static str {
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(&known) = names.iter().find(|&&known| known == text) {
        return known;
    }
    let made: &'static str = Box::leak(text.to_owned().into_boxed_str());
    names.push(made);
    made
}
