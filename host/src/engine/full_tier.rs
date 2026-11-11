//! The places of the full tier (D82 stage 3b, section 3.4e of `d82-view-per-render.md`). A mod
//! asks for parts of MGE XE in a view of its own (`mge.camera.setViewOptions`); the plugin
//! writes the parts of each view slot as the frame event `EVENT_VIEW_OPTIONS`. The player
//! limits how many views a frame get them: `views.full_tier_max` (default 1; 0 refuses every
//! request; the code takes up to [`MAX_PLACES`]). The places are sticky, so that the full tier
//! does not jump between views
//! when a mod walks its mirrors in a changing order:
//!
//! - a view that holds a place keeps it while it has options and renders at least once in
//!   [`IDLE_FRAMES`] frames;
//! - a free place goes to the first view with options that renders without one;
//! - a view that waits takes the place of a holder that rendered nothing for [`IDLE_FRAMES`]
//!   frames.
//!
//! Each place has a number, 0 to [`MAX_PLACES`] less 1, that it keeps for as long as a view
//! holds it and that a view taking it over inherits. The extension keys what a view needs for
//! its own draws (its view group, its cull sets) to the place, so that a mirror that keeps its
//! place keeps its group.
//!
//! The state is plain values with no device: [`FullTier::scene`] answers at each scene of a
//! view with the place and the parts that the view gets in it.

use crate::stream_format::VIEW_SLOTS;

/// The frames without a scene after which a holder loses its place to a waiting view (the
/// plugin's `view_slots::RETIRE_FRAMES`).
pub(crate) const IDLE_FRAMES: u64 = 60;
/// The most places the code takes, one for each view slot: a place is held by a view slot, so
/// a frame cannot use more. The player's range of `views.full_tier_max` is the setting's own
/// (`FULL_TIER_MAX_RANGE` of mge-config), at most this.
pub(crate) const MAX_PLACES: usize = VIEW_SLOTS;
/// The default of `views.full_tier_max`.
pub(crate) const DEFAULT_PLACES: usize = 1;

/// The counts of the full tier, for the report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TierCounts {
    /// The options events that set parts, and the ones that cleared them.
    pub(crate) set: u64,
    pub(crate) cleared: u64,
    /// The scenes of a view with options that got its parts.
    pub(crate) granted: u64,
    /// The scenes of a view with options that got none: all places were held (or the number is 0).
    pub(crate) refused: u64,
    /// A place passed from an idle holder to a waiting view.
    pub(crate) taken_over: u64,
    /// The scenes that repeated the key of an earlier scene in the same flush and got the
    /// parts and the place of that scene's slot ([`FullTier::scene_of_repeat`]).
    pub(crate) repeats: u64,
}

/// What a view gets in a scene: the number of its place and the parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Grant {
    pub(crate) place: u8,
    pub(crate) parts: u32,
}

/// The holder of a place: its view slot and the frame of its last scene.
#[derive(Clone, Copy, Debug)]
struct Holder {
    slot: u8,
    last: u64,
}

#[derive(Debug)]
pub(crate) struct FullTier {
    /// `views.full_tier_max`.
    max: usize,
    /// The parts that the mod asked for each slot.
    wanted: [u32; VIEW_SLOTS],
    /// The holder of each place; only the places below `max` hold.
    places: [Option<Holder>; MAX_PLACES],
    frame: u64,
    pub(crate) counts: TierCounts,
}

impl Default for FullTier {
    fn default() -> FullTier {
        FullTier {
            max: DEFAULT_PLACES,
            wanted: [0; VIEW_SLOTS],
            places: [None; MAX_PLACES],
            frame: 0,
            counts: TierCounts::default(),
        }
    }
}

impl FullTier {
    /// Sets `views.full_tier_max`; a value above [`MAX_PLACES`] is clamped. The places below
    /// the new number keep their holders and their numbers.
    pub(crate) fn set_max(&mut self, max: usize) {
        self.max = max.min(MAX_PLACES);
        for place in &mut self.places[self.max..] {
            *place = None;
        }
    }

    /// How many places a frame may use now (`views.full_tier_max`).
    pub(crate) fn max(&self) -> usize {
        self.max
    }

    /// The start of a frame.
    pub(crate) fn frame_start(&mut self) {
        self.frame += 1;
    }

    /// The frame event `EVENT_VIEW_OPTIONS`: the mod's parts for a slot. Parts 0 clear them, and
    /// the slot gives up its place.
    pub(crate) fn options(&mut self, slot: u32, parts: u32) {
        let Some(wanted) = self.wanted.get_mut(slot as usize) else {
            return;
        };
        *wanted = parts;
        if parts == 0 {
            self.counts.cleared += 1;
            self.release(slot);
        } else {
            self.counts.set += 1;
        }
    }

    /// The frame event `EVENT_VIEW_RETIRE`: the slot has no options and no place.
    pub(crate) fn retire(&mut self, slot: u32) {
        if let Some(wanted) = self.wanted.get_mut(slot as usize) {
            *wanted = 0;
            self.release(slot);
        }
    }

    /// The slot gives up its place, if it holds one.
    fn release(&mut self, slot: u32) {
        for place in &mut self.places {
            if place.is_some_and(|holder| u32::from(holder.slot) == slot) {
                *place = None;
            }
        }
    }

    fn holding(&self) -> usize {
        self.places.iter().flatten().count()
    }

    /// A scene of the view in `slot`: the place and the parts that it gets; `None` for a view
    /// without options or without a place.
    pub(crate) fn scene(&mut self, slot: u8) -> Option<Grant> {
        let wanted = *self.wanted.get(slot as usize)?;
        if wanted == 0 {
            return None;
        }
        let hold = Holder { slot, last: self.frame };
        let held = self.places[..self.max]
            .iter()
            .position(|place| place.is_some_and(|holder| holder.slot == slot));
        let free = || self.places[..self.max].iter().position(Option::is_none);
        // A holder that rendered nothing for [`IDLE_FRAMES`] frames loses its place to this view.
        let idle = || {
            self.places[..self.max]
                .iter()
                .position(|place| place.is_some_and(|holder| self.frame.saturating_sub(holder.last) > IDLE_FRAMES))
        };
        let (place, taken_over) = match (held, free(), idle()) {
            (Some(place), ..) => (place, false),
            (None, Some(place), _) => (place, false),
            (None, None, Some(place)) => (place, true),
            _ => {
                self.counts.refused += 1;
                return None;
            }
        };
        self.places[place] = Some(hold);
        self.counts.taken_over += u64::from(taken_over);
        self.counts.granted += 1;
        Some(Grant {
            place: place as u8,
            parts: wanted,
        })
    }

    /// A scene that repeats the key of an earlier scene in the same flush (it has a second
    /// slot, which the mod set no options on): it gets what the key's first slot `first`
    /// gets, the same parts in the same place, so that a mirror that renders twice in a
    /// frame keeps its place and does not take another one (D82, design 3.4f). The extension
    /// serves two scenes of one place in a frame, the later ones on spare sets (R2).
    pub(crate) fn scene_of_repeat(&mut self, first: u8) -> Option<Grant> {
        let grant = self.scene(first)?;
        self.counts.repeats += 1;
        Some(grant)
    }

    /// The report line of the full tier.
    pub(crate) fn line(&self) -> String {
        let c = &self.counts;
        format!(
            "full tier: {} places of at most {}; {} option events set parts, {} cleared them; {} scenes of a view got their parts ({} of them repeats of a key in a flush), {} were refused by the bound (views.full_tier_max); {} places passed to a waiting view",
            self.holding(),
            self.max,
            c.set,
            c.cleared,
            c.granted,
            c.repeats,
            c.refused,
            c.taken_over
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts of a scene of the view in `slot`; 0 for none.
    fn parts(tier: &mut FullTier, slot: u8) -> u32 {
        tier.scene(slot).map_or(0, |grant| grant.parts)
    }

    fn frames(tier: &mut FullTier, count: u64) {
        for _ in 0..count {
            tier.frame_start();
        }
    }

    /// One place by default: the first view with options that renders gets it, and the second
    /// gets none and is counted.
    #[test]
    fn the_first_view_with_options_takes_the_place() {
        let mut tier = FullTier::default();
        tier.frame_start();
        tier.options(3, 0b11);
        tier.options(4, 0b1);
        assert_eq!(parts(&mut tier, 5), 0);
        assert_eq!(parts(&mut tier, 4), 0b1);
        assert_eq!(parts(&mut tier, 3), 0);
        assert_eq!((tier.counts.granted, tier.counts.refused), (1, 1));
    }

    /// The order of the scenes does not move a place: a holder keeps it while it renders.
    #[test]
    fn a_holder_keeps_its_place_in_any_order() {
        let mut tier = FullTier::default();
        tier.options(1, 1);
        tier.options(2, 1);
        for _ in 0..100 {
            tier.frame_start();
            assert_eq!(parts(&mut tier, 1), 1);
            assert_eq!(parts(&mut tier, 2), 0);
        }
        for _ in 0..100 {
            tier.frame_start();
            assert_eq!(parts(&mut tier, 2), 0);
            assert_eq!(parts(&mut tier, 1), 1);
        }
    }

    /// A waiting view takes the place of a holder that rendered nothing for 60 frames, and not
    /// before.
    #[test]
    fn a_waiting_view_takes_over_an_idle_holder() {
        let mut tier = FullTier::default();
        tier.frame_start();
        tier.options(1, 1);
        tier.options(2, 2);
        assert_eq!(parts(&mut tier, 1), 1);
        frames(&mut tier, IDLE_FRAMES);
        assert_eq!(parts(&mut tier, 2), 0);
        frames(&mut tier, 1);
        assert_eq!(parts(&mut tier, 2), 2);
        assert_eq!(tier.counts.taken_over, 1);
        // The old holder is a waiting view now.
        assert_eq!(parts(&mut tier, 1), 0);
    }

    /// Clearing the options, or a retirement, frees the place for the next view.
    #[test]
    fn a_cleared_or_retired_slot_frees_its_place() {
        let mut tier = FullTier::default();
        tier.frame_start();
        tier.options(1, 1);
        tier.options(2, 1);
        assert_eq!(parts(&mut tier, 1), 1);
        tier.options(1, 0);
        assert_eq!(parts(&mut tier, 1), 0);
        assert_eq!(parts(&mut tier, 2), 1);
        tier.retire(2);
        assert_eq!(parts(&mut tier, 2), 0);
        tier.options(1, 4);
        assert_eq!(parts(&mut tier, 1), 4);
        assert_eq!(tier.counts.cleared, 1);
    }

    /// 0 refuses every request; the number is clamped to the code's bound; a lower number takes
    /// places back.
    #[test]
    fn the_number_of_places_is_the_players() {
        let mut tier = FullTier::default();
        tier.options(1, 1);
        tier.set_max(0);
        tier.frame_start();
        assert_eq!(parts(&mut tier, 1), 0);
        assert_eq!(tier.counts.refused, 1);
        tier.set_max(4);
        for slot in 1..=5 {
            tier.options(slot, 1);
        }
        let got: Vec<u32> = (1..=5u8).map(|slot| parts(&mut tier, slot)).collect();
        assert_eq!(got, vec![1, 1, 1, 1, 0]);
        tier.set_max(2);
        assert_eq!(parts(&mut tier, 1), 1);
        assert_eq!(parts(&mut tier, 2), 1);
        assert_eq!(parts(&mut tier, 3), 0);
        assert_eq!(parts(&mut tier, 4), 0);
        assert!(tier.line().starts_with("full tier: 2 places of at most 2;"));
    }

    /// Each holder has a place number that it keeps in any order of the scenes, and no two holders
    /// share a number.
    #[test]
    fn a_holder_keeps_its_place_number() {
        let mut tier = FullTier::default();
        tier.set_max(3);
        for slot in [5, 2, 9] {
            tier.options(slot, 1);
        }
        tier.frame_start();
        let first: Vec<u8> = [5, 2, 9].iter().map(|&slot| tier.scene(slot).unwrap().place).collect();
        assert_eq!(first, vec![0, 1, 2]);
        for _ in 0..50 {
            tier.frame_start();
            let again: Vec<u8> = [9, 5, 2].iter().map(|&slot| tier.scene(slot).unwrap().place).collect();
            assert_eq!(again, vec![2, 0, 1]);
        }
    }

    /// A view that takes over an idle holder's place gets that place's number, and a view that
    /// frees a place leaves it free for the next one.
    #[test]
    fn a_new_holder_inherits_the_number_of_the_place_it_takes() {
        let mut tier = FullTier::default();
        tier.set_max(2);
        tier.frame_start();
        tier.options(1, 1);
        tier.options(2, 1);
        tier.options(3, 1);
        assert_eq!(tier.scene(1).map(|g| g.place), Some(0));
        assert_eq!(tier.scene(2).map(|g| g.place), Some(1));
        assert_eq!(tier.scene(3), None);
        // Slot 1 renders no more; slot 2 keeps rendering.
        for _ in 0..=IDLE_FRAMES {
            tier.frame_start();
            tier.scene(2);
        }
        assert_eq!(tier.scene(3).map(|g| g.place), Some(0));
        // The old holder of place 0 waits now; clearing slot 2 frees place 1 for it.
        tier.options(2, 0);
        assert_eq!(tier.scene(1).map(|g| g.place), Some(1));
    }

    /// A lower number takes the places above it back and keeps the numbers below.
    #[test]
    fn a_lower_number_keeps_the_numbers_below_it() {
        let mut tier = FullTier::default();
        tier.set_max(4);
        tier.frame_start();
        for slot in 1..=4u8 {
            tier.options(u32::from(slot), 1);
            assert_eq!(tier.scene(slot).map(|g| g.place), Some(slot - 1));
        }
        tier.set_max(2);
        assert_eq!(tier.scene(2).map(|g| g.place), Some(1));
        assert_eq!(tier.scene(4), None);
        assert!(tier.line().starts_with("full tier: 2 places of at most 2;"));
    }

    /// A number above 4 up to the code's bound is served, and keeps the holders and the numbers
    /// below it when it grows; above the bound it is clamped (the place of D82 caps, design B).
    #[test]
    fn a_number_above_four_up_to_the_bound_is_served_and_a_larger_one_is_clamped() {
        let mut tier = FullTier::default();
        tier.set_max(6);
        tier.frame_start();
        for slot in 1..=8u8 {
            tier.options(u32::from(slot), 1);
        }
        let got: Vec<Option<u8>> = (1..=8u8).map(|slot| tier.scene(slot).map(|grant| grant.place)).collect();
        assert_eq!(got, vec![Some(0), Some(1), Some(2), Some(3), Some(4), Some(5), None, None]);
        // A larger number keeps the holders and their numbers, and serves the waiting views.
        tier.set_max(8);
        assert_eq!(tier.scene(1).map(|grant| grant.place), Some(0));
        assert_eq!(tier.scene(6).map(|grant| grant.place), Some(5));
        assert_eq!(tier.scene(7).map(|grant| grant.place), Some(6));
        assert_eq!(tier.scene(8).map(|grant| grant.place), Some(7));
        // A smaller one takes the places above it back.
        tier.set_max(6);
        assert_eq!(tier.scene(7), None);
        assert_eq!(tier.scene(6).map(|grant| grant.place), Some(5));
        // The bound is the number of view slots.
        tier.set_max(99);
        assert_eq!((tier.max(), MAX_PLACES), (VIEW_SLOTS, VIEW_SLOTS));
        tier.set_max(VIEW_SLOTS);
        assert_eq!(tier.max(), VIEW_SLOTS);
    }

    /// A slot past the table and a slot with no options draw as without a tier.
    #[test]
    fn a_view_without_options_gets_nothing() {
        let mut tier = FullTier::default();
        assert_eq!(parts(&mut tier, 7), 0);
        assert_eq!(parts(&mut tier, 200), 0);
        tier.options(200, 1);
        assert_eq!(tier.counts.set, 0);
        assert_eq!((tier.counts.granted, tier.counts.refused), (0, 0));
    }
}
