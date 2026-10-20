//! The engine's blend sorter: which draw it puts into its sorted queue (engine-rules R2,
//! R21). `NiAlphaAccumulator::RegisterObject` reads two bits of the alpha property of a
//! shape: blend on, and the no-sorter bit off. The blend it reads is the shape's own, the
//! blend of the first pass of its material; a later pass that blends (the textures of the
//! terrain) is not a reason. A stream that marks the draws of the queue (the shape flag
//! 0x800) does not need this rule; an older capture does. [I] The world Click displays the
//! sky with the accumulator set (0x41C654, 0x6CC7B0), so its blends register unless their
//! alpha property has the no-sorter bit; not read for the sky meshes.

use super::scene::sky_class;

/// True when the engine sorts the draw: a world draw of a class outside the sky whose first
/// pass blends and whose alpha property has no no-sorter bit. `blends` are the blend words
/// of the passes of the material (the first word of a pass is 0 for no blend).
pub(crate) fn sorted_by_engine(class: u32, blends: &[[u32; 3]], no_sorter: bool) -> bool {
    !sky_class(class) && blends.first().is_some_and(|b| b[0] != 0) && !no_sorter
}

#[cfg(test)]
mod tests {
    use super::sorted_by_engine;

    const BLEND: [u32; 3] = [1, 5, 6];
    const NONE: [u32; 3] = [0, 0, 0];

    #[test]
    fn a_blended_object_is_sorted() {
        // A particle system, an object of the world: its first pass blends.
        assert!(sorted_by_engine(12, &[BLEND], false));
        assert!(sorted_by_engine(9, &[BLEND, BLEND], false));
    }

    #[test]
    fn the_no_sorter_bit_keeps_it_in_place() {
        assert!(!sorted_by_engine(12, &[BLEND], true));
    }

    #[test]
    fn the_clouds_are_not_sorted() {
        // The sky's clouds blend, but the sky is not drawn through the queue.
        assert!(!sorted_by_engine(3, &[BLEND], false));
    }

    #[test]
    fn a_later_pass_that_blends_is_not_a_reason() {
        // The terrain: the base pass does not blend, the passes of its textures do.
        assert!(!sorted_by_engine(11, &[NONE, BLEND, BLEND], false));
        assert!(!sorted_by_engine(11, &[], false));
    }
}
