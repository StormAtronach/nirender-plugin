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

    /// The additions that a mod asks for a view (`mge.camera.setViewOptions`): the bits of
    /// `MGEApi.h` `ViewPart`, as the plugin's event `EVENT_VIEW_OPTIONS` carries them. Bits
    /// beyond the six are ignored. `shaders` is the post chain of the view
    /// ([`Additions::POST_COVER`]); `distantWater` is the water of MGE XE and its reflection.
    pub const fn from_option_parts(parts: u32) -> Additions {
        const MAP: [(u32, Additions); 6] = [
            (1 << 0, Additions::LAND),
            (1 << 1, Additions::STATICS),
            (1 << 2, Additions::GRASS),
            (1 << 3, Additions::WATER.with(Additions::REFLECTION)),
            (1 << 4, Additions::POST_COVER),
            (1 << 5, Additions::GLOWS),
        ];
        let mut set = Additions::NONE;
        let mut at = 0;
        while at < MAP.len() {
            if parts & MAP[at].0 != 0 {
                set = set.with(MAP[at].1);
            }
            at += 1;
        }
        set
    }

    /// The set with the parts of both.
    pub const fn with(self, other: Additions) -> Additions {
        Additions(self.0 | other.0)
    }

    /// The parts of the full tier that a view of its own gets drawn today: the distant land
    /// and the distant statics (stage 3b, slice 1). The other granted parts stay unused until
    /// their slices come.
    pub const TIER_DRAWN: Additions = Additions::LAND.with(Additions::STATICS);

    /// The parts that are in both sets.
    pub const fn intersect(self, other: Additions) -> Additions {
        Additions(self.0 & other.0)
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

    /// The key of a view of its own from the plugin's view slot of its scene (0 to 31). Slot 0
    /// is the main view's, so the key counts from 1; a scene without a slot (an older plugin)
    /// has no options, so it gets no parts and its key is not used.
    pub fn of_view(view_slot: Option<u32>, kind: ViewKind) -> ViewKey {
        ViewKey { slot: view_slot.map_or(0, |slot| slot + 1), kind }
    }
}

/// The parts of the full tier that the extension draws in a scene of a view of its own
/// ([`Additions::TIER_DRAWN`] of its additions). The main view has none: its parts come from
/// the calls of the frame.
pub(crate) fn tier_parts(kind: ViewKind, additions: Additions) -> Additions {
    if kind == ViewKind::Main {
        Additions::NONE
    } else {
        additions.intersect(Additions::TIER_DRAWN)
    }
}

/// The samples for each pixel of the pass of a scene: a rendered texture has one sample, the
/// back buffer the count of the frame.
pub(crate) fn pass_samples(in_texture: bool, frame_samples: u32) -> u32 {
    if in_texture {
        1
    } else {
        frame_samples
    }
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

    /// A picture with options draws the land and the statics in its own view; the other granted
    /// parts wait for their slices, and the main view has none from this path.
    #[test]
    fn a_view_of_its_own_draws_the_land_and_the_statics_only() {
        let granted = Additions::from_option_parts(0b111111);
        assert_eq!(tier_parts(ViewKind::Picture, granted), Additions::LAND.with(Additions::STATICS));
        assert_eq!(tier_parts(ViewKind::Picture, Additions::from_option_parts(0b100)), Additions::NONE);
        assert_eq!(tier_parts(ViewKind::Picture, Additions::NONE), Additions::NONE);
        assert_eq!(tier_parts(ViewKind::Main, Additions::ALL), Additions::NONE);
        assert_eq!(tier_parts(ViewKind::OtherScreen, Additions::LAND), Additions::LAND);
    }

    /// The key of a view of its own is not the main view's, and the slots stay apart.
    #[test]
    fn a_view_of_its_own_has_a_key_of_its_own() {
        let first = ViewKey::of_view(Some(0), ViewKind::Picture);
        assert_eq!(first, ViewKey { slot: 1, kind: ViewKind::Picture });
        assert_ne!(first, ViewKey::MAIN);
        assert_ne!(ViewKey::of_view(Some(1), ViewKind::Picture), first);
        assert_ne!(ViewKey::of_view(Some(0), ViewKind::OtherScreen), first);
    }

    /// A picture's pass has one sample whatever the frame has; the back buffer keeps the frame's.
    #[test]
    fn a_texture_pass_has_one_sample() {
        assert_eq!(pass_samples(true, 4), 1);
        assert_eq!(pass_samples(false, 4), 4);
        assert_eq!(pass_samples(false, 1), 1);
    }

    #[test]
    fn the_main_key_is_slot_zero() {
        assert_eq!(ViewKey::default(), ViewKey::MAIN);
        assert_eq!(ViewKind::OtherScreen.name(), "other screen");
    }
}
