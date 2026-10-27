//! The groups of texture tier C, kept by their set (`Renderer::stage_group`): the texture
//! handle and the sampler state of each stage, and the pass's target that shows white. A
//! group goes when one of its textures takes another view or goes ([`StageGroups::forget`]),
//! from the sets of all its handles, so that no handle keeps the key of a group that went.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::data::STAGES;

/// The set of a group of tier C: the texture handle and the sampler state of each stage, and
/// the handle of the pass's target, which shows white (a texture cannot be a target and a
/// texture of one pass; 0 for none).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StageKey {
    pub(crate) textures: [u32; STAGES],
    pub(crate) samplers: [[u32; 5]; STAGES],
    pub(crate) hide: u32,
}

impl StageKey {
    /// The texture handles of the set, without the stages that have none.
    fn handles(&self) -> impl Iterator<Item = u32> + '_ {
        self.textures.iter().copied().filter(|&handle| handle != 0)
    }
}

/// The groups by their set, and the sets that hold each texture handle. `G` is the group
/// (`Rc<wgpu::BindGroup>` in the renderer; a plain value in the tests).
pub(crate) struct StageGroups<G> {
    groups: FxHashMap<StageKey, G>,
    sets_of: FxHashMap<u32, FxHashSet<StageKey>>,
}

impl<G> Default for StageGroups<G> {
    fn default() -> Self {
        StageGroups { groups: FxHashMap::default(), sets_of: FxHashMap::default() }
    }
}

impl<G: Clone> StageGroups<G> {
    pub(crate) fn get(&self, key: &StageKey) -> Option<G> {
        self.groups.get(key).cloned()
    }

    pub(crate) fn insert(&mut self, key: StageKey, group: G) {
        for handle in key.handles() {
            self.sets_of.entry(handle).or_default().insert(key);
        }
        self.groups.insert(key, group);
    }

    /// Drops the groups that hold the texture `handle`, and their keys from the sets of their
    /// other handles.
    pub(crate) fn forget(&mut self, handle: u32) {
        let Some(keys) = self.sets_of.remove(&handle) else { return };
        for key in keys {
            self.groups.remove(&key);
            for other in key.handles().filter(|&other| other != handle) {
                if let Some(sets) = self.sets_of.get_mut(&other) {
                    sets.remove(&key);
                    if sets.is_empty() {
                        self.sets_of.remove(&other);
                    }
                }
            }
        }
    }

    /// The number of groups kept.
    pub(crate) fn len(&self) -> usize {
        self.groups.len()
    }

    /// The number of keys in the sets of all the handles: each group counts once for each of
    /// its handles.
    #[cfg(test)]
    fn keys_held(&self) -> usize {
        self.sets_of.values().map(FxHashSet::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(textures: [u32; STAGES]) -> StageKey {
        StageKey { textures, samplers: [[0; 5]; STAGES], hide: 0 }
    }

    #[test]
    fn a_texture_that_goes_takes_its_groups_from_the_sets_of_the_other_handles() {
        let mut groups = StageGroups::default();
        // A long-lived texture 7 with a texture of each cell that comes and goes.
        for cell in 100..200 {
            groups.insert(key([7, cell, 0, 0, 0, 0]), cell);
            groups.forget(cell);
        }
        assert_eq!(groups.len(), 0);
        assert_eq!(groups.keys_held(), 0, "the set of handle 7 kept the keys of groups that went");

        groups.insert(key([7, 1, 0, 0, 0, 0]), 1);
        groups.insert(key([7, 2, 0, 0, 0, 0]), 2);
        groups.insert(key([3, 0, 0, 0, 0, 0]), 3);
        assert_eq!((groups.len(), groups.keys_held()), (3, 5));
        groups.forget(7);
        assert_eq!(groups.get(&key([7, 1, 0, 0, 0, 0])), None);
        assert_eq!(groups.get(&key([3, 0, 0, 0, 0, 0])), Some(3));
        assert_eq!((groups.len(), groups.keys_held()), (1, 1));
    }

    #[test]
    fn a_set_with_one_handle_twice_is_kept_once() {
        let mut groups = StageGroups::default();
        groups.insert(key([5, 5, 0, 0, 0, 0]), 1);
        groups.insert(key([5, 5, 0, 0, 0, 0]), 1);
        assert_eq!((groups.len(), groups.keys_held()), (1, 1));
        groups.forget(5);
        assert_eq!((groups.len(), groups.keys_held()), (0, 0));
    }
}
