//! The kinds of a view and the parts that MGE XE adds to each ("additions", A(v)): the model
//! of the design of D82 (sections 3.2 and 3.4b). A view is one camera drawn into one target in
//! a frame. The kind comes from the camera and the target only, never from the root of the
//! scene; this is the rule of Direct3D 9 (`isMainView` of MGE XE tests the view matrix and no
//! root).
//!
//! The kind and its parts are plain values with no device: the player decides them at each
//! scene start, and an [`crate::Extension`] reads the key of the view in
//! [`crate::SceneView::view_key`].

/// What a view is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewKind {
    /// The world camera into the back buffer, with any root: normal play, and a mod's subject
    /// on the world camera (Joy of Painting's Isolate Subject). All its clicks in a frame are
    /// one view, and the additions of MGE XE run once in it.
    #[default]
    Main,
    /// Any camera into a texture: the picture of a save, a mod's test render or mirror, the
    /// local map, the head of the character preview. No additions: the engine's own shapes
    /// only.
    Picture,
    /// Another camera into the back buffer (a mod's camera, the head of the race menu).
    OtherScreen,
}

impl ViewKind {
    /// The kind of a view. `in_target`: the game draws into a texture; `camera_is_world`: the
    /// camera of the scene is the world camera.
    pub fn classify(in_target: bool, camera_is_world: bool) -> ViewKind {
        if in_target {
            ViewKind::Picture
        } else if camera_is_world {
            ViewKind::Main
        } else {
            ViewKind::OtherScreen
        }
    }

    /// The additions of a view of this kind. `after_ui`: the view comes after the user
    /// interface scene of the frame; Direct3D 9 adds nothing there (`isFrameComplete`).
    pub fn additions(self, after_ui: bool) -> Additions {
        match self {
            ViewKind::Main => Additions::ALL,
            ViewKind::Picture => Additions::NONE,
            ViewKind::OtherScreen if after_ui => Additions::NONE,
            // The stage-2 parts of Direct3D 9, made with the camera of the view.
            ViewKind::OtherScreen => Additions::SHADOW_RECEIVE.with(Additions::SHADE).with(Additions::POST_COVER),
        }
    }

    /// The name of the kind, for a report line.
    pub fn name(self) -> &'static str {
        match self {
            ViewKind::Main => "main",
            ViewKind::Picture => "picture",
            ViewKind::OtherScreen => "other screen",
        }
    }
}

/// The parts that MGE XE adds to a view, as a set of bits. Each part is one switch or one
/// group of switches of MGE XE; a view has the parts of its kind ([`ViewKind::additions`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Additions(u16);

impl Additions {
    pub const NONE: Additions = Additions(0);
    /// The distant land.
    pub const LAND: Additions = Additions(1 << 0);
    /// The distant statics.
    pub const STATICS: Additions = Additions(1 << 1);
    pub const GRASS: Additions = Additions(1 << 2);
    /// The glows of the distant lights.
    pub const GLOWS: Additions = Additions(1 << 3);
    /// The casting of the shadows: the atlas made from the camera.
    pub const SHADOW_CAST: Additions = Additions(1 << 4);
    /// The receiving of the shadows: the overlay that samples the frame's atlas.
    pub const SHADOW_RECEIVE: Additions = Additions(1 << 5);
    /// The water of MGE XE, in place of the water plane of the game.
    pub const WATER: Additions = Additions(1 << 6);
    /// The reflection of the water, made from the camera.
    pub const REFLECTION: Additions = Additions(1 << 7);
    /// The sky shader.
    pub const SKY_SHADER: Additions = Additions(1 << 8);
    /// The post chain covers the pixels of the view in the back buffer.
    pub const POST_COVER: Additions = Additions(1 << 9);
    /// The per-pixel shade of the draws.
    pub const SHADE: Additions = Additions(1 << 10);
    /// Every part: a main view, each by its switch.
    pub const ALL: Additions = Additions((1 << 11) - 1);

    /// The set with the parts of both.
    pub const fn with(self, other: Additions) -> Additions {
        Additions(self.0 | other.0)
    }

    /// Every part of `other` is in this set.
    pub const fn contains(self, other: Additions) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The key of a view: the slot of its scene (0 is the main view's) and its kind. An
/// extension that draws one view for each key reads it in [`crate::SceneView::view_key`]; it may
/// ignore it, and today the calls are the main view's.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ViewKey {
    pub slot: u32,
    pub kind: ViewKind,
}

impl ViewKey {
    /// The main view of a frame.
    pub const MAIN: ViewKey = ViewKey { slot: 0, kind: ViewKind::Main };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind follows the camera and the target, not the root (the design, 3.2).
    #[test]
    fn the_kind_comes_from_the_camera_and_the_target() {
        assert_eq!(ViewKind::classify(false, true), ViewKind::Main);
        assert_eq!(ViewKind::classify(true, true), ViewKind::Picture);
        assert_eq!(ViewKind::classify(true, false), ViewKind::Picture);
        assert_eq!(ViewKind::classify(false, false), ViewKind::OtherScreen);
    }

    /// Main has every part, a picture none; another screen has the stage-2 parts of Direct3D 9
    /// before the user interface scene and none after it.
    #[test]
    fn each_kind_has_its_additions() {
        for part in [Additions::LAND, Additions::STATICS, Additions::GRASS, Additions::GLOWS, Additions::SHADOW_CAST, Additions::SHADOW_RECEIVE, Additions::WATER, Additions::REFLECTION, Additions::SKY_SHADER, Additions::POST_COVER, Additions::SHADE] {
            assert!(ViewKind::Main.additions(false).contains(part));
            assert!(!ViewKind::Picture.additions(false).contains(part));
            assert!(!ViewKind::OtherScreen.additions(true).contains(part));
        }
        let other = ViewKind::OtherScreen.additions(false);
        assert!(other.contains(Additions::SHADOW_RECEIVE) && other.contains(Additions::SHADE) && other.contains(Additions::POST_COVER));
        for part in [Additions::LAND, Additions::STATICS, Additions::GRASS, Additions::GLOWS, Additions::SHADOW_CAST, Additions::WATER, Additions::REFLECTION, Additions::SKY_SHADER] {
            assert!(!other.contains(part));
        }
        assert!(ViewKind::Picture.additions(true).is_empty());
        // After the interface scene the kind Main keeps its parts: a frame has one main view.
        assert_eq!(ViewKind::Main.additions(true), Additions::ALL);
    }

    #[test]
    fn the_main_key_is_slot_zero() {
        assert_eq!(ViewKey::default(), ViewKey::MAIN);
        assert_eq!(ViewKind::OtherScreen.name(), "other screen");
    }
}
