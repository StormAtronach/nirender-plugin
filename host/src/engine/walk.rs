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

/// The places of a frame.
#[derive(Default)]
pub(crate) struct Walk {
    /// The head of the stream says that the plugin writes the places. Without it nothing
    /// changes: the blends out of the sort draw by bucket at the start of the world.
    pub(crate) on: bool,
    /// The runs of places: the ids of places with no other item between them.
    runs: Vec<Vec<u32>>,
    /// The ids placed in the frame, in the walk's order, once each.
    order: Vec<u32>,
    placed: HashSet<u32>,
    /// The ids that the main view took as placed, and the entries it left out for each.
    taken: HashSet<u32>,
    left_out: HashMap<u32, Vec<Entry>>,
}

impl Walk {
    /// The start of a frame: no place.
    pub(crate) fn frame_start(&mut self) {
        self.runs.clear();
        self.order.clear();
        self.placed.clear();
        self.taken.clear();
        self.left_out.clear();
    }

    /// A place of the frame. `known`: the table has the instance; `adjacent`: the last item
    /// of the frame is a run of places, so the place joins it and draws in the same call.
    pub(crate) fn place(&mut self, id: u32, known: bool, adjacent: bool) -> Place {
        if !known {
            return Place::Unknown;
        }
        if self.placed.insert(id) {
            self.order.push(id);
        }
        match self.runs.last_mut() {
            Some(run) if adjacent => {
                run.push(id);
                Place::Joined
            }
            _ => {
                self.runs.push(vec![id]);
                Place::New(self.runs.len() - 1)
            }
        }
    }

    /// The main view of the table: takes the blends out of the sort of the placed instances
    /// out of `order` (sorted by the keys of the buckets), for their places. `id_of` gives the
    /// instance of a slot. Returns the number of places whose walk order differs from their
    /// bucket order.
    pub(crate) fn leave_out(&mut self, order: &mut Vec<Entry>, id_of: impl Fn(u32) -> Option<u32>) -> u32 {
        self.taken.clone_from(&self.placed);
        self.left_out.clear();
        if self.placed.is_empty() {
            return 0;
        }
        let (placed, left_out) = (&self.placed, &mut self.left_out);
        let mut bucket_order = Vec::new();
        order.retain(|entry| {
            let Some(id) = id_of(entry.3).filter(|id| entry.0 >> 32 == GROUP_IN_PLACE && placed.contains(id)) else { return true };
            let entries = left_out.entry(id).or_default();
            if entries.is_empty() {
                bucket_order.push(id);
            }
            entries.push(*entry);
            false
        });
        let walk_order = self.order.iter().filter(|id| self.left_out.contains_key(id));
        walk_order.zip(&bucket_order).filter(|(walk, bucket)| walk != bucket).count() as u32
    }

    /// The entries of a run in the walk's order, for its item, and the number of its places
    /// that the main view did not take (a part of the frame ran before they came: the view
    /// drew those with the other blends).
    pub(crate) fn take_run(&mut self, run: usize) -> (Vec<Entry>, u32) {
        let ids = self.runs.get_mut(run).map(core::mem::take).unwrap_or_default();
        let mut entries = Vec::new();
        let mut late = 0;
        for id in ids {
            if !self.taken.contains(&id) {
                late += 1;
                continue;
            }
            if let Some(left) = self.left_out.remove(&id) {
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
        let mut walk = Walk { on: true, ..Default::default() };
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
        let mut walk = Walk { on: true, ..Default::default() };
        let mut order = order();
        assert_eq!(walk.leave_out(&mut order, id_of), 0);
        assert_eq!(order, self::order());
    }

    #[test]
    fn adjacent_places_draw_as_one_run() {
        let mut walk = Walk { on: true, ..Default::default() };
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
        let mut walk = Walk { on: true, ..Default::default() };
        assert_eq!(walk.place(99, false, false), Place::Unknown);
        let mut order = order();
        assert_eq!(walk.leave_out(&mut order, id_of), 0);
        assert_eq!(order, self::order());
    }

    #[test]
    fn an_opaque_slot_of_a_placed_instance_stays() {
        let mut walk = Walk { on: true, ..Default::default() };
        walk.place(12, true, false);
        let mut order = order();
        walk.leave_out(&mut order, id_of);
        assert_eq!(order, self::order());
        assert_eq!(walk.take_run(0), (vec![], 0));
    }

    #[test]
    fn a_place_after_the_main_view_is_late() {
        // A part of the frame ran before the place came: the view drew the blend by bucket.
        let mut walk = Walk { on: true, ..Default::default() };
        let mut order = order();
        walk.leave_out(&mut order, id_of);
        walk.place(10, true, false);
        assert_eq!(walk.take_run(0), (vec![], 1));
    }

    #[test]
    fn the_places_in_bucket_order_count_no_change() {
        let mut walk = Walk { on: true, ..Default::default() };
        walk.place(10, true, false);
        walk.place(11, true, false);
        walk.place(13, true, false);
        let mut order = order();
        assert_eq!(walk.leave_out(&mut order, id_of), 0);
        assert_eq!(order, vec![opaque(5, 2)]);
    }
}
