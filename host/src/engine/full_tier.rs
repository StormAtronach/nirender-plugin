//! The places of the full tier (D82 stage 3b, section 3.4e of `d82-view-per-render.md`). A mod
//! asks for parts of MGE XE in a view of its own (`mge.camera.setViewOptions`); the plugin
//! writes the parts of each view slot as the frame event `EVENT_VIEW_OPTIONS`. The player
//! limits how many views a frame get them: `views.full_tier_max` (default 1, 0 to 4; 0 refuses
//! every request). The places are sticky, so that the full tier does not jump between views
//! when a mod walks its mirrors in a changing order:
//!
//! - a view that holds a place keeps it while it has options and renders at least once in
//!   [`IDLE_FRAMES`] frames;
//! - a free place goes to the first view with options that renders without one;
//! - a view that waits takes the place of a holder that rendered nothing for [`IDLE_FRAMES`]
//!   frames.
//!
//! The state is plain values with no device: [`FullTier::scene`] answers at each scene of a
//! view with the parts that the view gets in it.

use crate::stream_format::VIEW_SLOTS;

/// The frames without a scene after which a holder loses its place to a waiting view (the
/// plugin's `view_slots::RETIRE_FRAMES`).
pub(crate) const IDLE_FRAMES: u64 = 60;
/// The largest value of `views.full_tier_max`.
pub(crate) const MAX_PLACES: usize = 4;
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
}

#[derive(Debug)]
pub(crate) struct FullTier {
    /// `views.full_tier_max`.
    max: usize,
    /// The parts that the mod asked for each slot.
    wanted: [u32; VIEW_SLOTS],
    /// The frame of the last scene of each slot that holds a place.
    holder: [Option<u64>; VIEW_SLOTS],
    frame: u64,
    pub(crate) counts: TierCounts,
}

impl Default for FullTier {
    fn default() -> FullTier {
        FullTier { max: DEFAULT_PLACES, wanted: [0; VIEW_SLOTS], holder: [None; VIEW_SLOTS], frame: 0, counts: TierCounts::default() }
    }
}

impl FullTier {
    /// Sets `views.full_tier_max`; a value past the range is clamped. Of the holders, the ones
    /// of the lowest slots keep their places up to the new number.
    pub(crate) fn set_max(&mut self, max: usize) {
        self.max = max.min(MAX_PLACES);
        let mut kept = 0;
        for holder in &mut self.holder {
            if holder.is_some() {
                kept += 1;
                if kept > self.max {
                    *holder = None;
                }
            }
        }
    }

    /// The start of a frame.
    pub(crate) fn frame_start(&mut self) {
        self.frame += 1;
    }

    /// The frame event `EVENT_VIEW_OPTIONS`: the mod's parts for a slot. Parts 0 clear them, and
    /// the slot gives up its place.
    pub(crate) fn options(&mut self, slot: u32, parts: u32) {
        let Some(wanted) = self.wanted.get_mut(slot as usize) else { return };
        *wanted = parts;
        if parts == 0 {
            self.counts.cleared += 1;
            self.holder[slot as usize] = None;
        } else {
            self.counts.set += 1;
        }
    }

    /// The frame event `EVENT_VIEW_RETIRE`: the slot has no options and no place.
    pub(crate) fn retire(&mut self, slot: u32) {
        if let Some(wanted) = self.wanted.get_mut(slot as usize) {
            *wanted = 0;
            self.holder[slot as usize] = None;
        }
    }

    fn holding(&self) -> usize {
        self.holder.iter().flatten().count()
    }

    /// A scene of the view in `slot`: the parts that it gets, 0 for a view without options or
    /// without a place.
    pub(crate) fn scene(&mut self, slot: u8) -> u32 {
        let slot = slot as usize;
        let Some(&wanted) = self.wanted.get(slot) else { return 0 };
        if wanted == 0 {
            return 0;
        }
        if self.holder[slot].is_some() || self.holding() < self.max {
            self.holder[slot] = Some(self.frame);
            self.counts.granted += 1;
            return wanted;
        }
        // A holder that rendered nothing for [`IDLE_FRAMES`] frames loses its place to this view.
        let idle = (0..VIEW_SLOTS).find(|&other| self.holder[other].is_some_and(|last| self.frame.saturating_sub(last) > IDLE_FRAMES));
        if let Some(other) = idle.filter(|_| self.max > 0) {
            self.holder[other] = None;
            self.holder[slot] = Some(self.frame);
            self.counts.taken_over += 1;
            self.counts.granted += 1;
            return wanted;
        }
        self.counts.refused += 1;
        0
    }

    /// The report line of the full tier.
    pub(crate) fn line(&self) -> String {
        let c = &self.counts;
        format!(
            "full tier: {} places of at most {}; {} option events set parts, {} cleared them; {} scenes of a view got their parts, {} were refused by the bound (views.full_tier_max); {} places passed to a waiting view",
            self.holding(),
            self.max,
            c.set,
            c.cleared,
            c.granted,
            c.refused,
            c.taken_over
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(tier.scene(5), 0);
        assert_eq!(tier.scene(4), 0b1);
        assert_eq!(tier.scene(3), 0);
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
            assert_eq!(tier.scene(1), 1);
            assert_eq!(tier.scene(2), 0);
        }
        for _ in 0..100 {
            tier.frame_start();
            assert_eq!(tier.scene(2), 0);
            assert_eq!(tier.scene(1), 1);
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
        assert_eq!(tier.scene(1), 1);
        frames(&mut tier, IDLE_FRAMES);
        assert_eq!(tier.scene(2), 0);
        frames(&mut tier, 1);
        assert_eq!(tier.scene(2), 2);
        assert_eq!(tier.counts.taken_over, 1);
        // The old holder is a waiting view now.
        assert_eq!(tier.scene(1), 0);
    }

    /// Clearing the options, or a retirement, frees the place for the next view.
    #[test]
    fn a_cleared_or_retired_slot_frees_its_place() {
        let mut tier = FullTier::default();
        tier.frame_start();
        tier.options(1, 1);
        tier.options(2, 1);
        assert_eq!(tier.scene(1), 1);
        tier.options(1, 0);
        assert_eq!(tier.scene(1), 0);
        assert_eq!(tier.scene(2), 1);
        tier.retire(2);
        assert_eq!(tier.scene(2), 0);
        tier.options(1, 4);
        assert_eq!(tier.scene(1), 4);
        assert_eq!(tier.counts.cleared, 1);
    }

    /// 0 refuses every request; the number is clamped to 4; a lower number takes places back.
    #[test]
    fn the_number_of_places_is_the_players() {
        let mut tier = FullTier::default();
        tier.options(1, 1);
        tier.set_max(0);
        tier.frame_start();
        assert_eq!(tier.scene(1), 0);
        assert_eq!(tier.counts.refused, 1);
        tier.set_max(99);
        assert_eq!(tier.max, MAX_PLACES);
        for slot in 1..=5 {
            tier.options(slot, 1);
        }
        let got: Vec<u32> = (1..=5u8).map(|slot| tier.scene(slot)).collect();
        assert_eq!(got, vec![1, 1, 1, 1, 0]);
        tier.set_max(2);
        assert_eq!(tier.scene(1), 1);
        assert_eq!(tier.scene(2), 1);
        assert_eq!(tier.scene(3), 0);
        assert_eq!(tier.scene(4), 0);
        assert!(tier.line().starts_with("full tier: 2 places of at most 2;"));
    }

    /// A slot past the table and a slot with no options draw as without a tier.
    #[test]
    fn a_view_without_options_gets_nothing() {
        let mut tier = FullTier::default();
        assert_eq!(tier.scene(7), 0);
        assert_eq!(tier.scene(200), 0);
        tier.options(200, 1);
        assert_eq!(tier.counts.set, 0);
        assert_eq!((tier.counts.granted, tier.counts.refused), (0, 0));
    }
}
