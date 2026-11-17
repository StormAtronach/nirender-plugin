//! The kinds of a scene and the classes of a draw, as the plugin's records carry them.
//! The values are the contract of the stream (`src/link.rs` of the plugin, `SCENE_*` and
//! `CLASS_*`); this crate does not depend on the plugin crate, so they are repeated here.

/// A scene of a camera the plugin does not name, or of the world camera on another root
/// (a mod's subject).
pub(crate) const SCENE_OTHER: u32 = 0;
/// The world scene of the game: the sky at its start, then the world (one Click of the
/// world camera).
pub(crate) const SCENE_WORLD: u32 = 1;
/// The sorted blends of the engine's queue in a scene of their own (MGE XE's second Click
/// of the world camera, when the frame has no water and no shadows).
pub(crate) const SCENE_SORTED_ALPHA: u32 = 2;
/// The water scene: the water plane and the flush of the engine's queue, after the plane
/// with the camera above the water, before it below.
pub(crate) const SCENE_WATER: u32 = 3;
/// The shadow casters' scene: the engine draws the actors that cast a stencil shadow, in
/// colour, after the world scene and its shadow volumes, into the depth of the world.
pub(crate) const SCENE_SHADOW_CASTERS: u32 = 5;
/// The first-person scene: the arms and the weapon, after the world views and a clear of
/// the depth, with the depth test on.
pub(crate) const SCENE_FIRST_PERSON: u32 = 6;
/// The user interface scene: the world views are done.
pub(crate) const SCENE_UI: u32 = 7;

/// The sky's classes, 1 to 8: the dome, the night, the clouds, the sun, the moons, their
/// shadows, the rest of the sky, and the sun glare.
pub(crate) const CLASS_SKY_FIRST: u32 = 1;
pub(crate) const CLASS_SKY_LAST: u32 = 8;
/// The sky dome: the shapes under the weather's sky dome node.
pub(crate) const CLASS_SKY_DOME: u32 = 1;
/// The clouds: the shapes under the weather's clouds node (`WeatherController::sgSkyClouds`).
pub(crate) const CLASS_SKY_CLOUDS: u32 = 3;
/// The moon shadow: the one shape "Moon Shadow" under the "Shadow Node" of each moon
/// (`Moon::ctor` 0x4443B0), the disc that keeps the stars out of the moon.
pub(crate) const CLASS_SKY_MOON_SHADOW: u32 = 6;
/// The water plane of the game: not a thing of the world (the extension draws its water).
pub(crate) const CLASS_WATER_PLANE: u32 = 10;

/// A scene of the world camera: the world, the sorted blends, the water.
pub(crate) fn world_camera(kind: u32) -> bool {
    (SCENE_WORLD..=SCENE_WATER).contains(&kind)
}

/// A scene whose draws take the lights of the world: the world camera's scenes, the
/// first-person scene and the shadow casters' scene. A shadowed actor is thus lit as the wall
/// beside it.
pub(crate) fn lit_as_world(kind: u32) -> bool {
    world_camera(kind) || kind == SCENE_FIRST_PERSON || kind == SCENE_SHADOW_CASTERS
}

/// Why a lit draw in the world's own space keeps its own lights in place of the world's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorldSpaceRefusal {
    /// The scene's draws are not lit as the world ([`lit_as_world`]).
    NotLit,
    /// The frame has no view of the world scene before this scene.
    NoWorldView,
    /// The shadow casters' scene has a view other than the world scene's.
    View,
    /// The shadow casters' scene has a projection other than the world scene's.
    Projection,
}

/// Whether a lit draw in the world's own space of a scene of `kind`, with the scene's `view`
/// and `projection`, can take the world's lights: its pixel and its depth must find the
/// clusters of the main view. A scene of the world camera or of the arm camera can. The shadow
/// casters' scene can when its camera has the view and the projection of the frame's world
/// scene (`world`); both draw into the back buffer (`lit` of the draw).
pub(crate) fn world_space_lights(
    kind: u32,
    view: &[f32; 16],
    projection: &[f32; 16],
    world: Option<&([f32; 16], [f32; 16])>,
) -> Result<(), WorldSpaceRefusal> {
    if world_camera(kind) || kind == SCENE_FIRST_PERSON {
        return Ok(());
    }
    if kind != SCENE_SHADOW_CASTERS {
        return Err(WorldSpaceRefusal::NotLit);
    }
    let (world_view, world_projection) = world.ok_or(WorldSpaceRefusal::NoWorldView)?;
    if view != world_view {
        return Err(WorldSpaceRefusal::View);
    }
    if projection != world_projection {
        return Err(WorldSpaceRefusal::Projection);
    }
    Ok(())
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
    fn the_shadow_casters_are_lit_as_the_world() {
        for kind in [
            SCENE_WORLD,
            SCENE_SORTED_ALPHA,
            SCENE_WATER,
            SCENE_SHADOW_CASTERS,
            SCENE_FIRST_PERSON,
        ] {
            assert!(lit_as_world(kind), "kind {kind}");
        }
        for kind in [SCENE_OTHER, 4, SCENE_UI, 8, 9, 10, 11, 12] {
            assert!(!lit_as_world(kind), "kind {kind}");
        }
        // The null: the test before the shadow casters joined leaves kind 5 out.
        let before = |kind: u32| world_camera(kind) || kind == SCENE_FIRST_PERSON;
        assert!(!before(SCENE_SHADOW_CASTERS));
    }

    /// A view with the camera at a place of the world, and a projection.
    fn camera(x: f32) -> ([f32; 16], [f32; 16]) {
        let mut view = [0.0; 16];
        (view[0], view[5], view[10], view[15]) = (1.0, 1.0, 1.0, 1.0);
        view[12] = -x;
        let mut projection = [0.0; 16];
        (projection[0], projection[5], projection[10], projection[11], projection[14]) = (1.0, 1.7, 1.0, 1.0, -1.0);
        (view, projection)
    }

    #[test]
    fn the_shadow_casters_take_the_worlds_lights_only_with_the_worlds_view() {
        let world = camera(-22486.428);
        let (view, projection) = world;
        assert_eq!(
            world_space_lights(SCENE_SHADOW_CASTERS, &view, &projection, Some(&world)),
            Ok(())
        );
        // The null: a view one step of f32 away from the world's is refused.
        let mut moved = view;
        moved[12] = f32::from_bits(view[12].to_bits() + 1);
        assert_eq!(
            world_space_lights(SCENE_SHADOW_CASTERS, &moved, &projection, Some(&world)),
            Err(WorldSpaceRefusal::View)
        );
        let mut wider = projection;
        wider[0] = 0.9;
        assert_eq!(
            world_space_lights(SCENE_SHADOW_CASTERS, &view, &wider, Some(&world)),
            Err(WorldSpaceRefusal::Projection)
        );
        assert_eq!(
            world_space_lights(SCENE_SHADOW_CASTERS, &view, &projection, None),
            Err(WorldSpaceRefusal::NoWorldView)
        );
        // The scenes of the world camera and of the arm camera need no world view.
        for kind in [SCENE_WORLD, SCENE_SORTED_ALPHA, SCENE_WATER, SCENE_FIRST_PERSON] {
            assert_eq!(world_space_lights(kind, &moved, &wider, None), Ok(()), "kind {kind}");
        }
    }

    #[test]
    fn a_scene_not_lit_as_the_world_takes_no_lights_of_the_world_in_the_worlds_space() {
        // The character preview (10), with the world's view: refused. Kinds 8, 11 and 12 too.
        let world = camera(100.0);
        for kind in [SCENE_OTHER, 4, SCENE_UI, 8, 9, 10, 11, 12] {
            assert_eq!(
                world_space_lights(kind, &world.0, &world.1, Some(&world)),
                Err(WorldSpaceRefusal::NotLit),
                "kind {kind}"
            );
        }
    }

    #[test]
    fn the_classes_of_the_sky() {
        assert!(sky_class(1) && sky_class(8));
        assert!(!sky_class(0) && !sky_class(9) && !sky_class(CLASS_WATER_PLANE));
    }
}
