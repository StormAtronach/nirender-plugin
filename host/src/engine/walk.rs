//! The places of the table's blends in the engine's walk (`tag::INSTANCE_WALK`, the header bit
//! `HEADER_WALK_PLACES`). The engine draws a shape whose alpha property blends at its display
//! when its main sorted queue does not take it (the no-sorter bit, no accumulator): in the
//! order of the walk, among the draws of the engine's path (engine-rules R2, R3). The plugin
//! writes such a place at the display of a retained shape. A state of the frame and no device:
//! the player notes each place at its record, the main view of the table leaves the placed
//! slots out of its blends, and each run of places draws them at its item.

use std::collections::{HashMap, HashSet};

/// An entry of the table's order (`RetainedSet::order`): the key of the bucket, the bucket,
/// the key of the draw in it, the slot.
pub(crate) type Entry = (u64, u32, u32, u32);

/// The group of the buckets that blend out of the sort, in the high word of the key of a
/// bucket (`retained::bucket_key`).
pub(crate) const GROUP_IN_PLACE: u64 = 2;

/// The view slot of the main view (D82): the places of the world camera's scenes into the
/// back buffer.
pub(crate) const MAIN_VIEW: u8 = 0;

/// What a place did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Place {
    /// The table has no such instance: the place is ignored.
    Unknown,
    /// The place joined the run of the last item.
    Joined,
    /// The place starts a run: the number of the run, for a new item.
    New(usize),
}

/// A run of places of one view: the ids of places with no other item between them.
#[derive(Default)]
struct Run {
    view: u8,
    ids: Vec<u32>,
}

/// The places of a frame, for each view (D82 stage 2): a view leaves out of its blends only
/// the instances placed in that view, so a place in one view never takes a blend out of
/// another.
#[derive(Default)]
pub(crate) struct Walk {
    runs: Vec<Run>,
    /// The places of the frame, in the walk's order, once each for each view.
    order: Vec<(u8, u32)>,
    placed: HashSet<(u8, u32)>,
    /// The views of each id placed in the frame, as a mask: an id placed in two views shares
    /// one transform between them (design 3.4a).
    views_of: HashMap<u32, u32>,
    /// The places of an id in a second view of the frame, counted at the place.
    pub(crate) in_two_views: u64,
    /// The places that each view took, and the entries it left out for each.
    taken: HashSet<(u8, u32)>,
    left_out: HashMap<(u8, u32), Vec<Entry>>,
}

impl Walk {
    /// The start of a frame: no place.
    pub(crate) fn frame_start(&mut self) {
        self.runs.clear();
        self.order.clear();
        self.placed.clear();
        self.views_of.clear();
        self.taken.clear();
        self.left_out.clear();
    }

    /// A place of the frame in the main view. `known`: the table has the instance;
    /// `adjacent`: the last item of the frame is a run of places, so the place joins it and
    /// draws in the same call.
    pub(crate) fn place(&mut self, id: u32, known: bool, adjacent: bool) -> Place {
        self.place_in(MAIN_VIEW, id, known, adjacent)
    }

    /// A place of the frame in `view`. A place joins the last run only when the run is of the
    /// same view.
    pub(crate) fn place_in(&mut self, view: u8, id: u32, known: bool, adjacent: bool) -> Place {
        if !known {
            return Place::Unknown;
        }
        if self.placed.insert((view, id)) {
            self.order.push((view, id));
            let views = self.views_of.entry(id).or_insert(0);
            if *views != 0 && *views & 1 << view == 0 {
                self.in_two_views += 1;
            }
            *views |= 1 << view;
        }
        match self.runs.last_mut() {
            Some(run) if adjacent && run.view == view => {
                run.ids.push(id);
                Place::Joined
            }
            _ => {
                self.runs.push(Run { view, ids: vec![id] });
                Place::New(self.runs.len() - 1)
            }
        }
    }

    /// The main view of the table: takes the blends out of the sort of the placed instances
    /// out of `order` (sorted by the keys of the buckets), for their places. `id_of` gives the
    /// instance of a slot. Returns the number of places whose walk order differs from their
    /// bucket order.
    pub(crate) fn leave_out(&mut self, order: &mut Vec<Entry>, id_of: impl Fn(u32) -> Option<u32>) -> u32 {
        self.leave_out_of(MAIN_VIEW, order, id_of)
    }

    /// [`Walk::leave_out`] for the order of `view`: only the instances placed in `view`.
    pub(crate) fn leave_out_of(&mut self, view: u8, order: &mut Vec<Entry>, id_of: impl Fn(u32) -> Option<u32>) -> u32 {
        self.taken.retain(|&(v, _)| v != view);
        self.left_out.retain(|&(v, _), _| v != view);
        self.taken.extend(self.placed.iter().filter(|&&(v, _)| v == view));
        if !self.placed.iter().any(|&(v, _)| v == view) {
            return 0;
        }
        let (placed, left_out) = (&self.placed, &mut self.left_out);
        let mut bucket_order = Vec::new();
        order.retain(|entry| {
            let Some(id) = id_of(entry.3).filter(|id| entry.0 >> 32 == GROUP_IN_PLACE && placed.contains(&(view, *id)))
            else {
                return true;
            };
            let entries = left_out.entry((view, id)).or_default();
            if entries.is_empty() {
                bucket_order.push(id);
            }
            entries.push(*entry);
            false
        });
        let walk_order = self
            .order
            .iter()
            .filter(|&&(v, id)| v == view && self.left_out.contains_key(&(v, id)))
            .map(|&(_, id)| id);
        walk_order.zip(&bucket_order).filter(|(walk, bucket)| walk != *bucket).count() as u32
    }

    /// The entries of a run in the walk's order, for its item, and the number of its places
    /// that its view did not take (a part of the frame ran before they came: the view drew
    /// those with the other blends).
    pub(crate) fn take_run(&mut self, run: usize) -> (Vec<Entry>, u32) {
        let Some(Run { view, ids }) = self.runs.get_mut(run).map(core::mem::take) else {
            return (Vec::new(), 0);
        };
        let mut entries = Vec::new();
        let mut late = 0;
        for id in ids {
            if !self.taken.contains(&(view, id)) {
                late += 1;
                continue;
            }
            if let Some(left) = self.left_out.remove(&(view, id)) {
                entries.extend(left);
            }
        }
        (entries, late)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The blend out of the sort of bucket `b` in slot `slot`, as the main view orders it.
    fn blend(b: u32, sequence: u32, slot: u32) -> Entry {
        (GROUP_IN_PLACE << 32 | b as u64, b, sequence, slot)
    }

    /// An opaque entry of bucket `b` (group 0).
    fn opaque(b: u32, slot: u32) -> Entry {
        (0, b, 0, slot)
    }

    /// Slot 0 is instance 10 in bucket 0, slot 1 instance 11 in bucket 1, slot 2 instance 12
    /// (opaque), slot 3 instance 13 in bucket 2.
    fn order() -> Vec<Entry> {
        vec![opaque(5, 2), blend(0, 1, 0), blend(1, 2, 1), blend(2, 3, 3)]
    }

    fn id_of(slot: u32) -> Option<u32> {
        Some(10 + slot)
    }

    #[test]
    fn the_places_give_the_walk_order() {
        // The walk has B1, an engine draw, then B0: each draws at its own item, B1 first.
        let mut walk = Walk::default();
        assert_eq!(walk.place(11, true, false), Place::New(0));
        assert_eq!(walk.place(10, true, false), Place::New(1));
        let mut order = order();
        assert_eq!(walk.leave_out(&mut order, id_of), 2);
        assert_eq!(order, vec![opaque(5, 2), blend(2, 3, 3)]);
        assert_eq!(walk.take_run(0), (vec![blend(1, 2, 1)], 0));
        assert_eq!(walk.take_run(1), (vec![blend(0, 1, 0)], 0));
    }

    #[test]
    fn no_place_keeps_the_old_order() {
        let mut walk = Walk::default();
        let mut order = order();
        assert_eq!(walk.leave_out(&mut order, id_of), 0);
        assert_eq!(order, self::order());
    }

    #[test]
    fn adjacent_places_draw_as_one_run() {
        let mut walk = Walk::default();
        assert_eq!(walk.place(11, true, false), Place::New(0));
        assert_eq!(walk.place(10, true, true), Place::Joined);
        let mut order = order();
        walk.leave_out(&mut order, id_of);
        assert_eq!(walk.take_run(0), (vec![blend(1, 2, 1), blend(0, 1, 0)], 0));
        // A run draws once.
        assert_eq!(walk.take_run(0), (vec![], 0));
    }

    #[test]
    fn a_place_of_an_unknown_instance_is_ignored() {
        let mut walk = Walk::default();
        assert_eq!(walk.place(99, false, false), Place::Unknown);
        let mut order = order();
        assert_eq!(walk.leave_out(&mut order, id_of), 0);
        assert_eq!(order, self::order());
    }

    #[test]
    fn an_opaque_slot_of_a_placed_instance_stays() {
        let mut walk = Walk::default();
        walk.place(12, true, false);
        let mut order = order();
        walk.leave_out(&mut order, id_of);
        assert_eq!(order, self::order());
        assert_eq!(walk.take_run(0), (vec![], 0));
    }

    #[test]
    fn a_place_after_the_main_view_is_late() {
        // A part of the frame ran before the place came: the view drew the blend by bucket.
        let mut walk = Walk::default();
        let mut order = order();
        walk.leave_out(&mut order, id_of);
        walk.place(10, true, false);
        assert_eq!(walk.take_run(0), (vec![], 1));
    }

    #[test]
    fn the_places_in_bucket_order_count_no_change() {
        let mut walk = Walk::default();
        walk.place(10, true, false);
        walk.place(11, true, false);
        walk.place(13, true, false);
        let mut order = order();
        assert_eq!(walk.leave_out(&mut order, id_of), 0);
        assert_eq!(order, vec![opaque(5, 2)]);
    }

    /// A place in a view of its own takes no blend out of the main view, and the view's own
    /// order leaves out only its places; the runs of two views never join.
    #[test]
    fn a_place_in_one_view_leaves_the_other_view_alone() {
        let mut walk = Walk::default();
        assert_eq!(walk.place_in(3, 11, true, false), Place::New(0));
        // The last item is view 3's run: a place of view 0 starts its own run.
        assert_eq!(walk.place(10, true, true), Place::New(1));
        let mut main = order();
        assert_eq!(walk.leave_out(&mut main, id_of), 0);
        assert_eq!(main, vec![opaque(5, 2), blend(1, 2, 1), blend(2, 3, 3)]);
        let mut mirror = order();
        assert_eq!(walk.leave_out_of(3, &mut mirror, id_of), 0);
        assert_eq!(mirror, vec![opaque(5, 2), blend(0, 1, 0), blend(2, 3, 3)]);
        assert_eq!(walk.take_run(0), (vec![blend(1, 2, 1)], 0));
        assert_eq!(walk.take_run(1), (vec![blend(0, 1, 0)], 0));
    }

    /// An id placed in two views of a frame is counted once for each further view: they share
    /// one transform (design 3.4a). The next frame starts again.
    #[test]
    fn an_id_placed_in_two_views_is_counted() {
        let mut walk = Walk::default();
        walk.place(10, true, false);
        walk.place(10, true, true);
        assert_eq!(walk.in_two_views, 0);
        walk.place_in(2, 10, true, false);
        walk.place_in(2, 10, true, true);
        assert_eq!(walk.in_two_views, 1);
        walk.place_in(5, 10, true, false);
        assert_eq!(walk.in_two_views, 2);
        walk.frame_start();
        walk.place_in(2, 10, true, false);
        assert_eq!(walk.in_two_views, 2);
    }

    /// A second leave-out of the same view in a frame (a second click of its camera) replaces
    /// the first one's entries; another view's stay.
    #[test]
    fn a_second_leave_out_of_a_view_replaces_its_entries() {
        let mut walk = Walk::default();
        walk.place(10, true, false);
        walk.place_in(4, 11, true, false);
        let mut main = order();
        walk.leave_out(&mut main, id_of);
        let mut view = order();
        walk.leave_out_of(4, &mut view, id_of);
        let mut main_again = order();
        walk.leave_out(&mut main_again, id_of);
        assert_eq!(walk.take_run(1), (vec![blend(1, 2, 1)], 0));
        assert_eq!(walk.take_run(0), (vec![blend(0, 1, 0)], 0));
    }
}
