//! The decisions that order the items of a frame: when the world starts and ends, where
//! each sorted-blends item goes and which generation a deferred blend gets, when a draw is
//! held for the start of the world, when the post-processing is due (engine-rules R19,
//! R22). A state of seven words and no device: the player asks it at each scene event and
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
    /// The first-person scene ends: its depth goes into the depth of the world that the
    /// post-processing reads, before the game clears the depth for its next scene.
    FirstPersonEnd,
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
    /// The world ended with the engine's sorted queue open: the table's blends wait for the
    /// scene that flushes it (R19).
    queue_open: bool,
}

impl Plan {
    /// The start of a frame: nothing began.
    pub(crate) fn frame_start(&mut self) {
        *self = Plan::default();
    }

    /// The start of a scene. At the user interface scene of the game the world views are
    /// done and the post-processing is due. The table's blends that still wait for the flush
    /// of the queue go first, at the first-person or the user interface scene: the engine
    /// flushes the queue in its water scene, so this is for a frame without one.
    pub(crate) fn scene_begin(&mut self, kind: u32) -> Vec<Step> {
        self.scene_kind = kind;
        let mut steps = Vec::new();
        if self.queue_open && (kind == scene::SCENE_FIRST_PERSON || kind == scene::SCENE_UI) {
            steps.push(self.sorted_blends());
        }
        if kind == scene::SCENE_UI {
            steps.push(Step::Post);
        }
        steps
    }

    /// The end of the scene in progress. `extension`: the player has an extension;
    /// `in_back_buffer`: the game draws into the back buffer; `drawing`: the frame is
    /// drawn; `queue_open`: the engine's sorted queue stays open after the scene.
    pub(crate) fn scene_end(&mut self, extension: bool, in_back_buffer: bool, drawing: bool, queue_open: bool) -> Vec<Step> {
        let mut steps = Vec::new();
        let kind = self.scene_kind;
        let world_scene = kind == scene::SCENE_WORLD && in_back_buffer && drawing;
        // A world scene in which the game drew nothing of the world (the console command
        // ToggleWorld culls the world roots; a view with nothing in it): the world starts at
        // the end of the scene, so that the extension draws its distant land under the sky.
        if extension && world_scene && !self.world_begun {
            self.world_begun = true;
            steps.push(Step::StartWorld);
        }
        if kind == scene::SCENE_WORLD {
            steps.push(Step::ReleaseHeld);
        }
        // A blend of the engine's path that came in a later scene of the world camera (the
        // water scene, where the engine flushes its queue after the water) waits past the
        // first item: an item of its own runs at the end of that scene (R22). The table's
        // blends that wait for the flush go into the same item.
        if scene::later_world_scene(kind) && in_back_buffer && self.world_begun && (self.waiting || self.queue_open) && !queue_open {
            steps.push(self.sorted_blends());
        }
        // The end of the world scene: the sorted blends of the table where the engine
        // flushes its queue (R19), then the world end. With the queue flushed in the world
        // scene, they go before the extension's world end, with the depth of the world.
        // With the queue open (the water shown, the shadow manager on), they wait for the
        // scene that flushes it, after the extension's grass.
        if world_scene && self.world_begun && !self.world_ended {
            self.world_ended = true;
            if queue_open {
                self.queue_open = true;
            } else {
                steps.push(self.sorted_blends());
            }
            steps.push(Step::EndWorld);
        }
        // The end of the first-person scene after the world: the depth of the arms is in the
        // depth target until the game clears it for its next scene.
        if kind == scene::SCENE_FIRST_PERSON && in_back_buffer && drawing && self.world_ended {
            steps.push(Step::FirstPersonEnd);
        }
        self.scene_kind = 0;
        steps
    }

    fn sorted_blends(&mut self) -> Step {
        let step = Step::SortedBlends(self.generation);
        self.generation += 1;
        self.waiting = false;
        self.queue_open = false;
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
    use super::scene::{SCENE_FIRST_PERSON, SCENE_SORTED_ALPHA, SCENE_UI, SCENE_WATER, SCENE_WORLD};
    use super::{Plan, Step};

    /// A world scene with the sky first, then the world.
    fn world_scene(plan: &mut Plan) -> Vec<Step> {
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![]);
        assert_eq!(plan.world_draw(true, true, true, false), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false), (vec![Step::StartWorld, Step::ReleaseHeld], false));
        plan.scene_end(true, true, true, false)
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
        assert_eq!(plan.scene_begin(SCENE_WATER), vec![]);
        assert_eq!(plan.defer(), 1);
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::SortedBlends(1)]);
        // A later scene with no deferred draw gives no item.
        plan.scene_begin(SCENE_SORTED_ALPHA);
        assert_eq!(plan.scene_end(true, true, true, false), vec![]);
        // Two deferred draws of one scene share its item.
        plan.scene_begin(SCENE_SORTED_ALPHA);
        assert_eq!((plan.defer(), plan.defer()), (2, 2));
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::SortedBlends(2)]);
    }

    /// The ghost near water: the world Click keeps the queue open, so the table's blends go
    /// after the extension's grass, into the item of the water scene that flushes the queue,
    /// together with the blends of the engine's path deferred there (R19).
    #[test]
    fn the_queue_open_at_the_world_end_moves_the_table_to_the_flush() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true);
        assert_eq!(plan.scene_end(true, true, true, true), vec![Step::ReleaseHeld, Step::EndWorld]);
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.defer(), 0);
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::SortedBlends(0)]);
        // The next deferred draw has an item of its own.
        plan.scene_begin(SCENE_SORTED_ALPHA);
        assert_eq!(plan.defer(), 1);
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::SortedBlends(1)]);
    }

    /// A water Click that keeps the queue open too (the lake pass of the shader water) gives
    /// no item; the Click after it flushes.
    #[test]
    fn a_later_scene_with_the_queue_open_gives_no_item() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true);
        plan.scene_end(true, true, true, true);
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.scene_end(true, true, true, true), vec![]);
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::SortedBlends(0)]);
    }

    /// Without a scene that flushes the queue, the table's blends go before the first-person
    /// scene, so that they are not lost.
    #[test]
    fn the_table_blends_without_a_flush_go_before_the_first_person() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true);
        plan.scene_end(true, true, true, true);
        assert_eq!(plan.scene_begin(SCENE_FIRST_PERSON), vec![Step::SortedBlends(0)]);
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::FirstPersonEnd]);
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::Post]);
    }

    #[test]
    fn a_blend_deferred_in_the_world_scene_goes_with_the_first_item() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true);
        assert_eq!(plan.defer(), 0);
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
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
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::StartWorld, Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
    }

    #[test]
    fn without_an_extension_nothing_starts() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_draw(false, true, false, true), (vec![], false));
        assert_eq!(plan.scene_end(false, true, true, false), vec![Step::ReleaseHeld]);
        assert!(!plan.world_begun);
    }

    #[test]
    fn the_first_person_scene_after_the_world_gives_its_depth() {
        let mut plan = Plan::default();
        // Before the world ended (a menu frame), no step.
        plan.scene_begin(SCENE_FIRST_PERSON);
        assert_eq!(plan.scene_end(true, true, true, false), vec![]);
        world_scene(&mut plan);
        plan.scene_begin(SCENE_FIRST_PERSON);
        assert_eq!(plan.scene_end(true, true, true, false), vec![Step::FirstPersonEnd]);
        // Into a rendered texture, no step.
        plan.scene_begin(SCENE_FIRST_PERSON);
        assert_eq!(plan.scene_end(true, false, true, false), vec![]);
    }

    #[test]
    fn the_user_interface_scene_asks_for_the_post() {
        let mut plan = Plan::default();
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::Post]);
        plan.frame_start();
        assert_eq!(plan.scene_kind, 0);
    }
}
