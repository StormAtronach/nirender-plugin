//! The decisions that order the items of a frame: when the world starts and ends, where
//! each sorted-blends item goes and which generation a deferred blend gets, when a draw is
//! held for the start of the world, when the post-processing is due (engine-rules R19,
//! R22). A state of six words and no device: the player asks it at each scene event and
//! draw and does the steps it returns, in their order.

use super::scene;

/// What the player does at a scene event or before a draw, in this order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Step {
    /// The world of the frame starts here: the extension draws before the world.
    StartWorld,
    /// The draws held for the start of the world go into the items, in their order: the
    /// extension draws its distant land under them and starts the depth of the world, so a
    /// draw before the start would lose its depth.
    ReleaseHeld,
    /// The sorted blends of this generation: the table's and the deferred draws of the
    /// engine's path that wait for it.
    SortedBlends(u32),
    /// The world of the frame ends here: the extension draws after the world.
    EndWorld,
    /// The post-processing is due (the player does it once, after the world began).
    Post,
}

/// The state of the plan within a frame.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Plan {
    /// The kind of the scene in progress (`scene::SCENE_*`; 0 between scenes).
    pub(crate) scene_kind: u32,
    pub(crate) world_begun: bool,
    pub(crate) world_ended: bool,
    /// A draw of a sky class went into the back buffer in the world scene of this frame.
    pub(crate) sky_seen: bool,
    /// The number of sorted-blends items of the frame so far: the generation of the next.
    generation: u32,
    /// A deferred draw waits for the item of `generation`.
    waiting: bool,
}

impl Plan {
    /// The start of a frame: nothing began.
    pub(crate) fn frame_start(&mut self) {
        *self = Plan::default();
    }

    /// The start of a scene. At the user interface scene of the game the world views are
    /// done and the post-processing is due.
    pub(crate) fn scene_begin(&mut self, kind: u32) -> Option<Step> {
        self.scene_kind = kind;
        (kind == scene::SCENE_UI).then_some(Step::Post)
    }

    /// The end of the scene in progress. `extension`: the player has an extension;
    /// `in_back_buffer`: the game draws into the back buffer; `drawing`: the frame is
    /// drawn.
    pub(crate) fn scene_end(&mut self, extension: bool, in_back_buffer: bool, drawing: bool) -> Vec<Step> {
        let mut steps = Vec::new();
        let kind = self.scene_kind;
        let world_scene = kind == scene::SCENE_WORLD && in_back_buffer && drawing;
        // A world scene in which the game drew nothing of the world (the console command
        // ToggleWorld culls the world roots; a view with nothing in it): the world starts at
        // the end of the scene, so that the extension draws its distant land under the sky.
        // MGE XE on Direct3D 9 does the same (mged3d8device.cpp, stage 0 from `EndScene`).
        if extension && world_scene && !self.world_begun {
            self.world_begun = true;
            steps.push(Step::StartWorld);
        }
        if kind == scene::SCENE_WORLD {
            steps.push(Step::ReleaseHeld);
        }
        // A blend of the engine's path that came in a later scene of the world camera (the
        // water scene, where the engine flushes its queue after the water) waits past the
        // first item: an item of its own runs at the end of that scene (R22).
        if scene::later_world_scene(kind) && in_back_buffer && self.world_begun && self.waiting {
            steps.push(self.sorted_blends());
        }
        // The end of the world scene: the sorted blends of the table, with the depth of the
        // world and before the extension's world end, where the engine flushes its queue
        // (R19); then the world end.
        if world_scene && self.world_begun && !self.world_ended {
            self.world_ended = true;
            steps.push(self.sorted_blends());
            steps.push(Step::EndWorld);
        }
        self.scene_kind = 0;
        steps
    }

    fn sorted_blends(&mut self) -> Step {
        let step = Step::SortedBlends(self.generation);
        self.generation += 1;
        self.waiting = false;
        step
    }

    /// A draw of the game. `sky`: its class is the sky's; `sky_hidden`: the scene has no
    /// sky. Returns the steps to do before the draw, and whether the draw is held for the
    /// start of the world. The world starts with the first draw of the world scene into the
    /// back buffer that is not a part of the sky and comes after the sky: the engine draws
    /// an opaque shape as it comes to it in the scene graph, so a draw can come before the
    /// sky (the lines of the cell borders); it is not the world. With the sky hidden (a
    /// plain interior cell, ToggleSky) there is no sky draw: the world starts at the first
    /// draw.
    pub(crate) fn world_draw(&mut self, extension: bool, in_back_buffer: bool, sky: bool, sky_hidden: bool) -> (Vec<Step>, bool) {
        let world_scene = in_back_buffer && self.scene_kind == scene::SCENE_WORLD;
        if world_scene && sky {
            self.sky_seen = true;
        }
        let mut steps = Vec::new();
        if extension && !self.world_begun && world_scene && !sky && (self.sky_seen || sky_hidden) {
            self.world_begun = true;
            steps.push(Step::StartWorld);
            steps.push(Step::ReleaseHeld);
        }
        let hold = extension && !self.world_begun && world_scene && !sky;
        (steps, hold)
    }

    /// A blend of the engine's path waits for the next sorted-blends item: its generation.
    pub(crate) fn defer(&mut self) -> u32 {
        self.waiting = true;
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::scene::{SCENE_SORTED_ALPHA, SCENE_UI, SCENE_WATER, SCENE_WORLD};
    use super::{Plan, Step};

    /// A world scene with the sky first, then the world.
    fn world_scene(plan: &mut Plan) -> Vec<Step> {
        assert_eq!(plan.scene_begin(SCENE_WORLD), None);
        assert_eq!(plan.world_draw(true, true, true, false), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false), (vec![Step::StartWorld, Step::ReleaseHeld], false));
        plan.scene_end(true, true, true)
    }

    #[test]
    fn the_world_ends_with_its_sorted_blends() {
        let mut plan = Plan::default();
        assert_eq!(world_scene(&mut plan), vec![Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
        assert!(plan.world_begun && plan.world_ended && plan.scene_kind == 0);
    }

    /// The aura near water: a blend deferred in the water scene, after the world's end,
    /// belongs to the item after its arrival, not to the first (R22).
    #[test]
    fn a_blend_deferred_in_the_water_scene_waits_for_its_own_item() {
        let mut plan = Plan::default();
        world_scene(&mut plan);
        assert_eq!(plan.scene_begin(SCENE_WATER), None);
        assert_eq!(plan.defer(), 1);
        assert_eq!(plan.scene_end(true, true, true), vec![Step::SortedBlends(1)]);
        // A later scene with no deferred draw gives no item.
        plan.scene_begin(SCENE_SORTED_ALPHA);
        assert_eq!(plan.scene_end(true, true, true), vec![]);
        // Two deferred draws of one scene share its item.
        plan.scene_begin(SCENE_SORTED_ALPHA);
        assert_eq!((plan.defer(), plan.defer()), (2, 2));
        assert_eq!(plan.scene_end(true, true, true), vec![Step::SortedBlends(2)]);
    }

    #[test]
    fn a_blend_deferred_in_the_world_scene_goes_with_the_first_item() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true);
        assert_eq!(plan.defer(), 0);
        assert_eq!(plan.scene_end(true, true, true), vec![Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
    }

    #[test]
    fn a_draw_before_the_sky_is_held() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_draw(true, true, false, false), (vec![], true));
        assert_eq!(plan.world_draw(true, true, true, false), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false), (vec![Step::StartWorld, Step::ReleaseHeld], false));
        // A draw into a rendered texture is never the world's.
        assert_eq!(plan.world_draw(true, false, false, false), (vec![], false));
    }

    #[test]
    fn the_sky_hidden_starts_the_world_at_the_first_draw() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_draw(true, true, false, true), (vec![Step::StartWorld, Step::ReleaseHeld], false));
    }

    #[test]
    fn an_empty_world_scene_starts_the_world_at_its_end() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.scene_end(true, true, true), vec![Step::StartWorld, Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
    }

    #[test]
    fn without_an_extension_nothing_starts() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_draw(false, true, false, true), (vec![], false));
        assert_eq!(plan.scene_end(false, true, true), vec![Step::ReleaseHeld]);
        assert!(!plan.world_begun);
    }

    #[test]
    fn the_user_interface_scene_asks_for_the_post() {
        let mut plan = Plan::default();
        assert_eq!(plan.scene_begin(SCENE_UI), Some(Step::Post));
        plan.frame_start();
        assert_eq!(plan.scene_kind, 0);
    }
}
