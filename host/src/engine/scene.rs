//! The kinds of a scene and the classes of a draw, as the plugin's records carry them.
//! The values are the contract of the stream (`src/link.rs` of the plugin, `SCENE_*` and
//! `CLASS_*`); this crate does not depend on the plugin crate, so they are repeated here.

/// The world scene of the game: the sky at its start, then the world (one Click of the
/// world camera).
pub(crate) const SCENE_WORLD: u32 = 1;
/// The sorted blends of the engine's queue in a scene of their own (MGE XE's second Click
/// of the world camera, when the frame has no water and no shadows).
pub(crate) const SCENE_SORTED_ALPHA: u32 = 2;
/// The water scene: the water plane and the flush of the engine's queue, after the plane
/// with the camera above the water, before it below.
pub(crate) const SCENE_WATER: u32 = 3;
/// The first-person scene: the arms and the weapon, after the world views and a clear of
/// the depth, with the depth test on.
pub(crate) const SCENE_FIRST_PERSON: u32 = 6;
/// The user interface scene: the world views are done.
pub(crate) const SCENE_UI: u32 = 7;

/// The sky's classes, 1 to 8: the dome, the night, the clouds, the sun, the moons, their
/// shadows, the rest of the sky, and the sun glare.
pub(crate) const CLASS_SKY_FIRST: u32 = 1;
pub(crate) const CLASS_SKY_LAST: u32 = 8;
/// The water plane of the game: not a thing of the world (the extension draws its water).
pub(crate) const CLASS_WATER_PLANE: u32 = 10;

/// A scene of the world camera: the world, the sorted blends, the water.
pub(crate) fn world_camera(kind: u32) -> bool {
    (SCENE_WORLD..=SCENE_WATER).contains(&kind)
}

/// A scene of the world camera after the world scene, where the engine flushes its sorted
/// queue (engine-rules R22).
pub(crate) fn later_world_scene(kind: u32) -> bool {
    kind == SCENE_SORTED_ALPHA || kind == SCENE_WATER
}

/// A class of the sky.
pub(crate) fn sky_class(class: u32) -> bool {
    (CLASS_SKY_FIRST..=CLASS_SKY_LAST).contains(&class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenes_of_the_world_camera() {
        assert!(world_camera(SCENE_WORLD) && world_camera(SCENE_SORTED_ALPHA) && world_camera(SCENE_WATER));
        assert!(!world_camera(0) && !world_camera(SCENE_UI));
        assert!(!later_world_scene(SCENE_WORLD) && later_world_scene(SCENE_SORTED_ALPHA) && later_world_scene(SCENE_WATER));
    }

    #[test]
    fn the_classes_of_the_sky() {
        assert!(sky_class(1) && sky_class(8));
        assert!(!sky_class(0) && !sky_class(9) && !sky_class(CLASS_WATER_PLANE));
    }
}
