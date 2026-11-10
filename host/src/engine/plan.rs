//! The decisions that order the items of a frame: when the world starts and ends, where
//! each sorted-blends item goes and which generation a deferred blend gets, when a draw is
//! held for the start of the world, when the post-processing is due (engine-rules R19,
//! R22). A state of seven words and no device: the player asks it at each scene event and
//! draw and does the steps it returns, in their order.

use super::scene;
use crate::ViewKind;

/// What the player does at a scene event or before a draw, in this order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Step {
    /// The world of the frame starts here: the extension draws before the world.
    StartWorld,
    /// The table's main view draws here, in a world scene whose world a main view with a
    /// root of its own began earlier in the frame: the world does not start again.
    MainView,
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
    /// The scenes of the world camera are done (the world and the water scene): the
    /// extension's classes that no draw brought get their call here, once in a frame.
    MissingClasses,
    /// A read of the back buffer ended the last picture, and this world scene starts another
    /// before the Present: the player starts its state of a picture (the plan reset itself).
    NewPicture,
}

/// The state of the plan within a frame.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Plan {
    /// The kind of the scene in progress (`scene::SCENE_*`; 0 between scenes).
    pub(crate) scene_kind: u32,
    /// The kind of the view of the scene in progress. The steps of the extension (the start
    /// and the end of the world, the post) belong to the main view only. A scene that the
    /// plugin names [`scene::SCENE_OTHER`] with the main view's camera is a main view with a
    /// root of its own (a mod's subject on the world camera): see [`Plan::main_other_root`].
    pub(crate) view: ViewKind,
    pub(crate) world_begun: bool,
    pub(crate) world_ended: bool,
    /// The world of the frame began in a main view with a root of its own, not in the world
    /// scene.
    world_by_other: bool,
    /// A world scene follows a main view with a root of its own: [`Step::MainView`] is due
    /// where the world would start in it.
    main_view_due: bool,
    /// [`Step::MainView`] became due in this frame.
    main_view_given: bool,
    /// A draw of a sky class went into the back buffer in the world scene of this frame.
    pub(crate) sky_seen: bool,
    /// The number of sorted-blends items of the frame so far: the generation of the next.
    generation: u32,
    /// A deferred draw waits for the item of `generation`.
    waiting: bool,
    /// The world ended with the engine's sorted queue open: the table's blends wait for the
    /// scene that flushes it (R19).
    queue_open: bool,
    /// [`Step::MissingClasses`] was given in this frame.
    classes_checked: bool,
    /// The post-processing of this picture is in the items ([`Plan::take_post`]).
    posted: bool,
    /// A read of the back buffer ended the picture: the next world scene starts a new one
    /// ([`Step::NewPicture`]).
    picture_ended: bool,
    /// The user interface scene of this picture began in the back buffer ([`Step::Post`] given).
    ui_given: bool,
    /// The read that ended the picture came after its user interface scene, as the game's
    /// screenshot reads: a main view with a root of its own then also starts a new picture.
    read_after_ui: bool,
}

impl Plan {
    /// The start of a frame: nothing began.
    pub(crate) fn frame_start(&mut self) {
        *self = Plan::default();
    }

    /// The draws of a scene go to the back buffer: the game does not draw into a texture of the
    /// helper (`in_target`), and its draws go somewhere (`stream_back_buffer`: not into a texture
    /// that the helper did not make). The `in_back_buffer` of [`Plan::scene_begin_view`].
    pub(crate) fn scene_in_back_buffer(in_target: bool, stream_back_buffer: bool) -> bool {
        !in_target && stream_back_buffer
    }

    /// The start of a scene whose camera the plan does not know: [`Plan::scene_begin_view`] with
    /// the kind that the plugin's scene kind gives.
    #[cfg(test)]
    pub(crate) fn scene_begin(&mut self, kind: u32) -> Vec<Step> {
        self.scene_begin_view(kind, if kind == scene::SCENE_OTHER { ViewKind::OtherScreen } else { ViewKind::Main }, true)
    }

    /// The start of a scene whose view has the kind `view` (from its camera and its target).
    /// `in_back_buffer`: the scene's draws go to the back buffer (not into a texture, known or
    /// not). At the user interface scene of the game, in the back buffer, the world views are
    /// done and the post-processing is due; an interface scene into a texture is not that
    /// scene, as on Direct3D 9 (`rendertargetNormal`). The table's blends that still wait for the flush of the queue go first, at the
    /// first-person or the user interface scene: the engine flushes the queue in its water
    /// scene, so this is for a frame without one. The classes that no draw of the world camera
    /// brought go before them, as the engine draws its water before it flushes the queue.
    /// A world scene after a read of the back buffer starts a new picture: the game draws again
    /// after its clear (`renderMainScene` 0x41C535), and the world starts and ends again in it.
    /// So does a main view with a root of its own (Isolate Subject) when the read came after the
    /// picture's user interface scene, as the game's screenshot reads (`takeScreenshot`
    /// 0x42E620). After a read between the world and the user interface, such a scene is part
    /// of the same picture.
    pub(crate) fn scene_begin_view(&mut self, kind: u32, view: ViewKind, in_back_buffer: bool) -> Vec<Step> {
        let mut steps = Vec::new();
        let own_root_main_view = kind == scene::SCENE_OTHER && view == ViewKind::Main;
        if self.picture_ended && (kind == scene::SCENE_WORLD || (own_root_main_view && self.read_after_ui)) {
            self.frame_start();
            steps.push(Step::NewPicture);
        }
        self.scene_kind = kind;
        self.view = view;
        if self.world_ended && !self.classes_checked && (kind == scene::SCENE_FIRST_PERSON || kind == scene::SCENE_UI) {
            self.classes_checked = true;
            steps.push(Step::MissingClasses);
        }
        if self.queue_open && (kind == scene::SCENE_FIRST_PERSON || kind == scene::SCENE_UI) {
            steps.push(self.sorted_blends());
        }
        if kind == scene::SCENE_UI && in_back_buffer {
            self.ui_given = true;
            steps.push(Step::Post);
        }
        // The world scene after a main view with a root of its own: the world began there and
        // did not wait for this scene's shapes. The table's main view draws where the world
        // would start in this scene (after the sky, at the walk event, or at the scene's end).
        if kind == scene::SCENE_WORLD && view == ViewKind::Main && self.world_by_other && !self.main_view_given {
            self.main_view_given = true;
            self.main_view_due = true;
        }
        steps
    }

    /// A read of the back buffer (a screenshot of the game or of a mod): the picture is done.
    /// The plan resets at the next world scene, not here: a read between the world and the
    /// user interface keeps the picture's post-processing and its classes' calls.
    pub(crate) fn back_buffer_read(&mut self) {
        self.picture_ended = true;
        self.read_after_ui = self.ui_given;
    }

    /// The post-processing item goes now: the world began, the frame is drawn, the extension
    /// wants it (`wanted`), and the picture has none yet.
    pub(crate) fn take_post(&mut self, drawing: bool, wanted: bool) -> bool {
        if self.posted || !self.world_begun || !drawing || !wanted {
            return false;
        }
        self.posted = true;
        true
    }

    /// A clear of the depth by the game. After the world, it starts the game's scenes past the
    /// world camera (the first person, the sun glare): the classes that no draw brought get
    /// their call before it, while the depth still holds the world. After the clear, a late
    /// sea plane would test an empty depth and draw over everything below the horizon (G7's
    /// report of 2026-10-26: a player levitating high, the engine's water plane culled).
    /// The table's sorted blends that still wait for the flush of the engine's queue go
    /// before such a clear too, when it is outside the scenes of the world camera: after it
    /// they test an empty depth, and blends behind walls show (R19). Inside a scene of the
    /// world camera they keep waiting for the flush (R22: above the water, after the plane).
    pub(crate) fn depth_clear(&mut self) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.world_ended && !self.classes_checked {
            self.classes_checked = true;
            steps.push(Step::MissingClasses);
        }
        if self.world_ended && self.queue_open && !scene::world_camera(self.scene_kind) {
            steps.push(self.sorted_blends());
        }
        steps
    }

    /// The end of the scene in progress. `extension`: the player has an extension;
    /// `in_back_buffer`: the game draws into the back buffer; `drawing`: the frame is
    /// drawn; `queue_open`: the engine's sorted queue stays open after the scene;
    /// `flush_marked`: the stream marks the end of the engine's flush with an event
    /// (`queue_flushed`): the sorted blends go there, not at a scene's end.
    pub(crate) fn scene_end(&mut self, extension: bool, in_back_buffer: bool, drawing: bool, queue_open: bool, flush_marked: bool) -> Vec<Step> {
        let mut steps = Vec::new();
        let kind = self.scene_kind;
        let main_other = self.main_other_root();
        let world_scene = (kind == scene::SCENE_WORLD || main_other) && in_back_buffer && drawing;
        // A world scene in which the game drew nothing of the world (the console command
        // ToggleWorld culls the world roots; a view with nothing in it): the world starts at
        // the end of the scene, so that the extension draws its distant land under the sky.
        if extension && world_scene && !self.world_begun {
            self.world_begun = true;
            self.world_by_other = main_other;
            steps.push(Step::StartWorld);
        } else if extension && world_scene && self.main_view_due {
            self.main_view_due = false;
            steps.push(Step::MainView);
        }
        if kind == scene::SCENE_WORLD || (main_other && self.world_begun) {
            steps.push(Step::ReleaseHeld);
        }
        // A blend of the engine's path that came in a later scene of the world camera (the
        // water scene, where the engine flushes its queue after the water) waits past the
        // first item: an item of its own runs at the end of that scene (R22). The table's
        // blends that wait for the flush go into the same item.
        // With the flush marked, the item is at the flush; a draw deferred after it still gets
        // one here.
        let due = if flush_marked { self.waiting } else { self.waiting || self.queue_open };
        if scene::later_world_scene(kind) && in_back_buffer && self.world_begun && due && !queue_open {
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
            } else if (!flush_marked || self.waiting) && (!main_other || self.waiting) {
                // A main view with a root of its own has the sorted blends of its view
                // (`views`), and an item here only for a blend of the engine's path.
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

    /// The scene in progress is the main view with a root that is not the world's: the world
    /// camera into the back buffer, named [`scene::SCENE_OTHER`] by its root (Isolate Subject
    /// of Joy of Painting). Direct3D 9 takes the kind from the camera, so MGE XE adds its parts
    /// to this scene as to the world scene.
    pub(crate) fn main_other_root(&self) -> bool {
        self.scene_kind == scene::SCENE_OTHER && self.view == ViewKind::Main
    }

    /// The first place of a retained shape in a main view with a root of its own, when the
    /// stream does not mark the walk of its root (`walk_marked`; the root has a `Display` of a
    /// mod): the world starts here, as at the first draw of a world scene.
    pub(crate) fn view_first_place(&mut self, extension: bool, in_back_buffer: bool, walk_marked: bool) -> Vec<Step> {
        if extension && in_back_buffer && !walk_marked && self.main_other_root() && !self.world_begun {
            self.world_begun = true;
            self.world_by_other = true;
            return vec![Step::StartWorld, Step::ReleaseHeld];
        }
        Vec::new()
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
    /// `walk_marked`: the stream marks the world's start with an event (`world_walk`); no
    /// draw starts the world then, and a draw before the event that is not the sky is held.
    pub(crate) fn world_draw(&mut self, extension: bool, in_back_buffer: bool, sky: bool, sky_hidden: bool, walk_marked: bool) -> (Vec<Step>, bool) {
        let main_other = self.main_other_root();
        let world_scene = in_back_buffer && (self.scene_kind == scene::SCENE_WORLD || main_other);
        if world_scene && sky {
            self.sky_seen = true;
        }
        let mut steps = Vec::new();
        // A scene with the walk marked starts its world at the event. Without it, a main view
        // with a root of its own (no sky) starts its world at its first draw.
        let starts_here = extension && !walk_marked && world_scene && !sky && (self.sky_seen || sky_hidden || main_other);
        if starts_here && !self.world_begun {
            self.world_begun = true;
            self.world_by_other = main_other;
            steps.push(Step::StartWorld);
            steps.push(Step::ReleaseHeld);
        } else if starts_here && self.main_view_due {
            self.main_view_due = false;
            steps.push(Step::MainView);
        }
        let hold = extension && !self.world_begun && world_scene && !sky;
        (steps, hold)
    }

    /// The walk of the scene's root left the sky root, or reached its first child when the root
    /// has no sky (the plugin's event `EVENT_WORLD_WALK`): the world starts here, in the world
    /// scene in the back buffer or in a main view with a root of its own. The engine walks the
    /// sky root first (`TES3Game::initGame` attaches it as the first child of the world camera's
    /// root), so the sky's draws came before.
    pub(crate) fn world_walk(&mut self, extension: bool, in_back_buffer: bool) -> Vec<Step> {
        let main_other = self.main_other_root();
        if extension && in_back_buffer && (self.scene_kind == scene::SCENE_WORLD || main_other) {
            if !self.world_begun {
                self.world_begun = true;
                self.world_by_other = main_other;
                return vec![Step::StartWorld, Step::ReleaseHeld];
            }
            if self.main_view_due {
                self.main_view_due = false;
                return vec![Step::MainView];
            }
        }
        Vec::new()
    }

    /// The engine drew the main sorted queue of a Click (the plugin's event
    /// `EVENT_QUEUE_FLUSHED`): the sorted blends of the table and of the engine's path go here,
    /// in a scene of the world camera in the back buffer after the world began. Below the water
    /// that is before the water plane, above it after the plane, as the engine does (R22).
    pub(crate) fn queue_flushed(&mut self, in_back_buffer: bool) -> Vec<Step> {
        if in_back_buffer && self.world_begun && scene::world_camera(self.scene_kind) {
            return vec![self.sorted_blends()];
        }
        Vec::new()
    }

    /// A blend of the engine's path waits for the next sorted-blends item: its generation.
    pub(crate) fn defer(&mut self) -> u32 {
        self.waiting = true;
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::scene::{SCENE_FIRST_PERSON, SCENE_OTHER, SCENE_SORTED_ALPHA, SCENE_UI, SCENE_WATER, SCENE_WORLD};
    use super::{Plan, Step};
    use crate::engine::views::Views;
    use crate::ViewKind;

    /// A world scene with the sky first, then the world.
    fn world_scene(plan: &mut Plan) -> Vec<Step> {
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![]);
        assert_eq!(plan.world_draw(true, true, true, false, false), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![Step::StartWorld, Step::ReleaseHeld], false));
        plan.scene_end(true, true, true, false, false)
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
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::SortedBlends(1)]);
        // A later scene with no deferred draw gives no item.
        plan.scene_begin(SCENE_SORTED_ALPHA);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![]);
        // Two deferred draws of one scene share its item.
        plan.scene_begin(SCENE_SORTED_ALPHA);
        assert_eq!((plan.defer(), plan.defer()), (2, 2));
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::SortedBlends(2)]);
    }

    /// The ghost near water: the world Click keeps the queue open, so the table's blends go
    /// after the extension's grass, into the item of the water scene that flushes the queue,
    /// together with the blends of the engine's path deferred there (R19).
    #[test]
    fn the_queue_open_at_the_world_end_moves_the_table_to_the_flush() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true, false);
        assert_eq!(plan.scene_end(true, true, true, true, false), vec![Step::ReleaseHeld, Step::EndWorld]);
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.defer(), 0);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::SortedBlends(0)]);
        // The next deferred draw has an item of its own.
        plan.scene_begin(SCENE_SORTED_ALPHA);
        assert_eq!(plan.defer(), 1);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::SortedBlends(1)]);
    }

    /// A water Click that keeps the queue open too (the lake pass of the shader water) gives
    /// no item; the Click after it flushes.
    #[test]
    fn a_later_scene_with_the_queue_open_gives_no_item() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true, false);
        plan.scene_end(true, true, true, true, false);
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.scene_end(true, true, true, true, false), vec![]);
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::SortedBlends(0)]);
    }

    /// Without a scene that flushes the queue, the table's blends go before the first-person
    /// scene, so that they are not lost.
    #[test]
    fn the_table_blends_without_a_flush_go_before_the_first_person() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true, false);
        plan.scene_end(true, true, true, true, false);
        assert_eq!(plan.scene_begin(SCENE_FIRST_PERSON), vec![Step::MissingClasses, Step::SortedBlends(0)]);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::FirstPersonEnd]);
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::Post]);
    }

    /// The classes that no draw brought get their call once, after the scenes of the world
    /// camera: at the first-person scene, or at the interface without one; not before the
    /// world ended.
    #[test]
    fn the_missing_classes_come_once_after_the_world_camera() {
        let mut plan = Plan::default();
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::Post]);
        plan.scene_end(true, true, true, false, false);
        let mut plan = Plan::default();
        world_scene(&mut plan);
        plan.scene_begin(SCENE_WATER);
        plan.scene_end(true, true, true, false, false);
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::MissingClasses, Step::Post]);
        plan.scene_end(true, true, true, false, false);
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::Post]);
    }

    /// A clear of the depth after the world gives the missing classes before it, once; the
    /// scene after it gives them no more. Before the world ended a clear gives nothing.
    #[test]
    fn the_missing_classes_come_before_a_depth_clear_after_the_world() {
        let mut plan = Plan::default();
        assert_eq!(plan.depth_clear(), vec![]);
        world_scene(&mut plan);
        assert_eq!(plan.depth_clear(), vec![Step::MissingClasses]);
        assert_eq!(plan.depth_clear(), vec![]);
        assert_eq!(plan.scene_begin(SCENE_FIRST_PERSON), vec![]);
    }

    #[test]
    fn the_blends_of_an_open_queue_come_before_a_depth_clear_after_the_world_scenes() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true, false);
        // The world scene ends with the engine's queue open: the blends wait.
        assert_eq!(plan.scene_end(true, true, true, true, false), vec![Step::ReleaseHeld, Step::EndWorld]);
        // A clear in a scene of the world camera does not take them.
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.depth_clear(), vec![Step::MissingClasses]);
        plan.scene_end(true, true, true, true, false);
        // A clear between the scenes, before the first person: they go before it, once.
        assert_eq!(plan.depth_clear(), vec![Step::SortedBlends(0)]);
        assert_eq!(plan.depth_clear(), vec![]);
        assert_eq!(plan.scene_begin(SCENE_FIRST_PERSON), vec![]);
    }

    #[test]
    fn a_blend_deferred_in_the_world_scene_goes_with_the_first_item() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true, false);
        assert_eq!(plan.defer(), 0);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
    }

    #[test]
    fn a_draw_before_the_sky_is_held() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![], true));
        assert_eq!(plan.world_draw(true, true, true, false, false), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![Step::StartWorld, Step::ReleaseHeld], false));
        // A draw into a rendered texture is never the world's.
        assert_eq!(plan.world_draw(true, false, false, false, false), (vec![], false));
    }

    #[test]
    fn with_the_walk_marked_the_event_starts_the_world_and_no_draw_does() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        // The sky draws, then a draw that is not the sky: no start, the draw is held.
        assert_eq!(plan.world_draw(true, true, true, false, true), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false, true), (vec![], true));
        // The end of the sky's walk.
        assert_eq!(plan.world_walk(true, true), vec![Step::StartWorld, Step::ReleaseHeld]);
        assert!(plan.world_begun);
        assert_eq!(plan.world_draw(true, true, false, false, true), (vec![], false));
        // One start a frame.
        assert_eq!(plan.world_walk(true, true), vec![]);
        // The sky hidden: the event still starts the world before any draw.
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_walk(true, true), vec![Step::StartWorld, Step::ReleaseHeld]);
    }

    #[test]
    fn the_walk_event_counts_only_in_the_world_scene_in_the_back_buffer() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.world_walk(true, true), vec![]);
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_walk(true, false), vec![]);
        assert_eq!(plan.world_walk(false, true), vec![]);
        assert!(!plan.world_begun);
    }

    #[test]
    fn the_sky_hidden_starts_the_world_at_the_first_draw() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_draw(true, true, false, true, false), (vec![Step::StartWorld, Step::ReleaseHeld], false));
    }

    #[test]
    fn an_empty_world_scene_starts_the_world_at_its_end() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::StartWorld, Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
    }

    #[test]
    fn without_an_extension_nothing_starts() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_draw(false, true, false, true, false), (vec![], false));
        assert_eq!(plan.scene_end(false, true, true, false, false), vec![Step::ReleaseHeld]);
        assert!(!plan.world_begun);
    }

    #[test]
    fn the_first_person_scene_after_the_world_gives_its_depth() {
        let mut plan = Plan::default();
        // Before the world ended (a menu frame), no step.
        plan.scene_begin(SCENE_FIRST_PERSON);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![]);
        world_scene(&mut plan);
        plan.scene_begin(SCENE_FIRST_PERSON);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::FirstPersonEnd]);
        // Into a rendered texture, no step.
        plan.scene_begin(SCENE_FIRST_PERSON);
        assert_eq!(plan.scene_end(true, false, true, false, false), vec![]);
    }

    /// With the flush marked, the world's sorted blends go at the flush in the world scene,
    /// before the world's end, and the end of the scene adds no item.
    #[test]
    fn the_marked_flush_in_the_world_scene_places_the_item() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_draw(true, true, false, true, true);
        assert_eq!(plan.world_walk(true, true), vec![Step::StartWorld, Step::ReleaseHeld]);
        assert_eq!(plan.defer(), 0);
        assert_eq!(plan.queue_flushed(true), vec![Step::SortedBlends(0)]);
        assert_eq!(plan.scene_end(true, true, true, false, true), vec![Step::ReleaseHeld, Step::EndWorld]);
    }

    /// The water shown: the world Click keeps the queue open, and the water Click flushes it.
    /// The item goes at the flush in the water scene, wherever the flush is in that scene
    /// (before the plane below the water, after it above), not at the scene's end.
    #[test]
    fn the_marked_flush_in_the_water_scene_places_the_item() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.world_walk(true, true);
        assert_eq!(plan.scene_end(true, true, true, true, true), vec![Step::ReleaseHeld, Step::EndWorld]);
        plan.scene_begin(SCENE_WATER);
        assert_eq!(plan.defer(), 0);
        assert_eq!(plan.queue_flushed(true), vec![Step::SortedBlends(0)]);
        assert_eq!(plan.scene_end(true, true, true, false, true), vec![]);
        // The queue was flushed: the first-person scene gives no sorted-blends item.
        assert_eq!(plan.scene_begin(SCENE_FIRST_PERSON), vec![Step::MissingClasses]);
    }

    /// A flush outside a world view (the first person, a rendered texture, before the world)
    /// gives no item; a draw deferred after the flush still gets one at the scene's end.
    #[test]
    fn a_marked_flush_outside_the_world_gives_no_item() {
        let mut plan = Plan::default();
        assert_eq!(plan.queue_flushed(true), vec![]);
        plan.scene_begin(SCENE_WORLD);
        plan.world_walk(true, true);
        assert_eq!(plan.queue_flushed(false), vec![]);
        assert_eq!(plan.queue_flushed(true), vec![Step::SortedBlends(0)]);
        assert_eq!(plan.defer(), 1);
        assert_eq!(plan.scene_end(true, true, true, false, true), vec![Step::ReleaseHeld, Step::SortedBlends(1), Step::EndWorld]);
        plan.scene_begin(SCENE_FIRST_PERSON);
        assert_eq!(plan.queue_flushed(true), vec![]);
    }

    /// A save's picture at the head of a frame: its scenes go to the plan state of the target
    /// group, which starts no world; the main plan then starts and ends the world of the frame.
    #[test]
    fn a_target_group_leaves_the_main_world_its_start() {
        let (mut main, mut target) = (Plan::default(), Plan::default());
        assert_eq!(target.scene_begin(SCENE_WORLD), vec![]);
        assert_eq!(target.world_draw(true, false, false, true, false), (vec![], false));
        assert_eq!(target.world_walk(true, false), vec![]);
        assert_eq!(target.queue_flushed(false), vec![]);
        assert_eq!(target.scene_end(true, false, true, false, true), vec![Step::ReleaseHeld]);
        assert_eq!(target.scene_begin(SCENE_FIRST_PERSON), vec![]);
        assert_eq!(target.scene_end(true, false, true, false, true), vec![]);
        assert!(!target.world_begun && !main.world_begun);
        assert_eq!(world_scene(&mut main), vec![Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
    }

    /// An interface scene drawn into a texture (the menu camera of a mod's own render) is not the
    /// interface of the picture: it gives no post, also after the world began and before the
    /// first-person scene. The interface scene of the back buffer still gives the one post.
    /// Direct3D 9 posts only in the back buffer (`rendertargetNormal`).
    #[test]
    fn an_interface_scene_in_a_target_gives_no_post() {
        let (mut main, mut target) = (Plan::default(), Plan::default());
        world_scene(&mut main);
        assert_eq!(target.scene_begin_view(SCENE_UI, ViewKind::Picture, false), vec![]);
        target.scene_end(true, false, true, false, false);
        main.scene_begin(SCENE_FIRST_PERSON);
        main.scene_end(true, true, true, false, false);
        assert_eq!(main.scene_begin(SCENE_UI), vec![Step::Post]);
        assert!(main.take_post(true, true));
        assert!(!main.take_post(true, true));
    }

    /// An interface scene whose draws go to a texture that the helper did not make (the main plan
    /// has its view as a main one) gives no post either; the one of the back buffer after it does.
    #[test]
    fn an_interface_scene_into_a_texture_that_is_not_known_gives_no_post() {
        let mut main = Plan::default();
        world_scene(&mut main);
        assert_eq!(main.scene_begin_view(SCENE_UI, ViewKind::Main, false), vec![Step::MissingClasses]);
        main.scene_end(true, false, true, false, false);
        assert_eq!(main.scene_begin_view(SCENE_UI, ViewKind::Main, true), vec![Step::Post]);
    }

    /// A scene is in the back buffer when the game draws into no texture of the helper and its draws
    /// go somewhere.
    #[test]
    fn a_scene_is_in_the_back_buffer_only_without_a_texture_and_with_a_destination() {
        assert!(Plan::scene_in_back_buffer(false, true));
        assert!(!Plan::scene_in_back_buffer(true, true));
        assert!(!Plan::scene_in_back_buffer(false, false));
        assert!(!Plan::scene_in_back_buffer(true, false));
    }

    /// A mod's mirror drawn after the world, with the engine's queue still open: a clear of the
    /// mirror's depth goes to the target group's state, and the main plan's blends and classes
    /// still come at the first-person scene of the back buffer, not into the mirror.
    #[test]
    fn a_depth_clear_of_a_target_after_the_world_leaves_the_main_plan() {
        let (mut main, mut target) = (Plan::default(), Plan::default());
        main.scene_begin(SCENE_WORLD);
        main.world_draw(true, true, false, true, false);
        assert_eq!(main.scene_end(true, true, true, true, false), vec![Step::ReleaseHeld, Step::EndWorld]);
        assert_eq!(target.depth_clear(), vec![]);
        assert_eq!(main.scene_begin(SCENE_FIRST_PERSON), vec![Step::MissingClasses, Step::SortedBlends(0)]);
    }

    /// The world scene of a second picture: its sky, its world, its end.
    fn second_world_scene(plan: &mut Plan) {
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![Step::NewPicture]);
        assert!(!plan.world_begun && !plan.world_ended);
        assert_eq!(plan.world_draw(true, true, true, false, false), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![Step::StartWorld, Step::ReleaseHeld], false));
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
    }

    /// The screenshot key: `takeScreenshot` reads the back buffer, then `renderNextFrame` draws
    /// the frame again after its clear (`mainLoop` 0x41B098, 0x41BE56; 0x41C535), in the same
    /// frame of the helper.
    #[test]
    fn a_back_buffer_read_ends_the_picture_and_the_next_world_scene_starts_the_world() {
        let mut plan = Plan::default();
        world_scene(&mut plan);
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::MissingClasses, Step::Post]);
        plan.scene_end(true, true, true, false, false);
        plan.back_buffer_read();
        second_world_scene(&mut plan);
        // One read starts one picture: a later world scene is of the same picture.
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![]);
    }

    /// The world start is the item that calls the extension's `before_world` (`start_world`,
    /// the player's items): Ryker's run lost the second picture's sky with it.
    #[test]
    fn the_picture_after_a_back_buffer_read_gets_its_world_start_and_its_post() {
        let mut plan = Plan::default();
        world_scene(&mut plan);
        plan.scene_begin(SCENE_UI);
        assert!(plan.take_post(true, true));
        assert!(!plan.take_post(true, true), "a second post in one picture");
        plan.scene_end(true, true, true, false, false);
        plan.back_buffer_read();
        second_world_scene(&mut plan);
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::MissingClasses, Step::Post]);
        assert!(plan.take_post(true, true), "the second picture has no post");
    }

    /// The reset touches only the picture after the read (Ryker's run 20261108-072733-risk72b):
    /// the next frame starts from `begin_frame`, and its world starts once.
    #[test]
    fn the_frame_after_the_reset_picture_starts_its_world_once() {
        let mut plan = Plan::default();
        world_scene(&mut plan);
        plan.back_buffer_read();
        second_world_scene(&mut plan);
        plan.scene_begin(SCENE_UI);
        // The Present; the next frame's records.
        plan.frame_start();
        assert_eq!(world_scene(&mut plan), vec![Step::ReleaseHeld, Step::SortedBlends(0), Step::EndWorld]);
        // Its water scene, a later scene of the world camera, starts nothing.
        assert_eq!(plan.scene_begin(SCENE_WATER), vec![]);
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![], false));
        assert!(!plan.scene_end(true, true, true, false, false).contains(&Step::StartWorld));
    }

    /// The reset waits for the next world scene: a read before the user interface scene
    /// keeps the picture's classes' calls and its post.
    #[test]
    fn a_read_between_the_world_and_the_ui_keeps_the_pictures_post() {
        let mut plan = Plan::default();
        world_scene(&mut plan);
        plan.back_buffer_read();
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::MissingClasses, Step::Post]);
        assert!(plan.take_post(true, true));
        // The next frame's start clears the mark: its world scene starts no second picture.
        plan.frame_start();
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![]);
    }

    /// Brief 17 (W105), test 1: today's rule stays. A world scene after a read starts a new
    /// picture.
    #[test]
    fn a_world_scene_after_a_read_starts_a_new_picture() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.back_buffer_read();
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![Step::NewPicture]);
    }

    /// Test 2: the game's screenshot with Isolate Subject. The picture is a main view with a
    /// root of its own; the screenshot reads after its user interface scene; the frame's own
    /// picture then starts with the same kind of view, and that view starts a new picture.
    #[test]
    fn a_main_view_with_its_own_root_after_a_read_after_the_ui_starts_a_new_picture() {
        let mut plan = Plan::default();
        plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true);
        assert_eq!(plan.scene_begin_view(SCENE_UI, ViewKind::Main, true), vec![Step::Post]);
        plan.back_buffer_read();
        assert_eq!(plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true), vec![Step::NewPicture]);
        // The new picture gets its own post.
        assert_eq!(plan.scene_begin_view(SCENE_UI, ViewKind::Main, true), vec![Step::Post]);
    }

    /// Test 3 (the Reviewer's): a read between the world and the user interface, then a main
    /// view with a root of its own before the user interface, is one picture: no reset in its
    /// middle.
    #[test]
    fn a_main_view_with_its_own_root_after_a_read_before_the_ui_starts_no_picture() {
        let mut plan = Plan::default();
        plan.scene_begin(SCENE_WORLD);
        plan.back_buffer_read();
        assert_eq!(plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true), vec![]);
        assert!(!plan.scene_begin_view(SCENE_UI, ViewKind::Main, true).contains(&Step::NewPicture));
    }

    /// Test 4: a read, then a user interface scene, starts no picture; nor do the water scene
    /// and a scene of another view after a read after the user interface.
    #[test]
    fn a_ui_or_water_scene_or_another_view_after_a_read_starts_no_picture() {
        let mut plan = Plan::default();
        plan.back_buffer_read();
        assert!(!plan.scene_begin(SCENE_UI).contains(&Step::NewPicture));
        plan.frame_start();
        plan.scene_begin(SCENE_UI);
        plan.back_buffer_read();
        assert!(!plan.scene_begin(SCENE_WATER).contains(&Step::NewPicture));
        assert!(!plan.scene_begin_view(SCENE_OTHER, ViewKind::OtherScreen, true).contains(&Step::NewPicture));
    }

    #[test]
    fn the_user_interface_scene_asks_for_the_post() {
        let mut plan = Plan::default();
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::Post]);
        plan.frame_start();
        assert_eq!(plan.scene_kind, 0);
    }

    /// The same main view with its root walked (W298 fix B): the event starts its world; its
    /// first place and its first draw do not.
    #[test]
    fn a_main_view_with_a_root_of_its_own_starts_the_world_at_the_walk_event() {
        let mut plan = Plan::default();
        plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true);
        assert_eq!(plan.view_first_place(true, true, true), vec![]);
        assert_eq!(plan.world_draw(true, true, false, true, true), (vec![], true));
        assert_eq!(plan.world_walk(true, true), vec![Step::StartWorld, Step::ReleaseHeld]);
        assert!(plan.world_by_other);
        assert_eq!(plan.world_walk(true, true), vec![]);
    }

    /// A mod's subject on the world camera (Isolate Subject): the scene has the plugin's kind
    /// `SCENE_OTHER` and the main view's camera. In a stream that does not walk its root (a root
    /// with a `Display` of a mod), its world starts at its first place, with no sky, and the post
    /// after the user interface scene follows.
    #[test]
    fn a_main_view_with_a_root_of_its_own_starts_the_world_at_its_first_place() {
        let mut plan = Plan::default();
        assert_eq!(plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true), vec![]);
        assert!(plan.main_other_root());
        assert_eq!(plan.view_first_place(true, true, false), vec![Step::StartWorld, Step::ReleaseHeld]);
        assert_eq!(plan.view_first_place(true, true, false), vec![]);
        // Its sorted blends belong to its view: no item of the plan, the world ends.
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::ReleaseHeld, Step::EndWorld]);
        assert!(plan.world_begun && plan.world_ended);
        assert_eq!(plan.scene_begin(SCENE_UI), vec![Step::MissingClasses, Step::Post]);
    }

    #[test]
    fn a_draw_of_the_engine_starts_the_world_of_a_main_view_with_a_root_of_its_own() {
        let mut plan = Plan::default();
        plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true);
        // The stream does not walk this root (its record has no SCENE_WALK_DUE: a root with a
        // `Display` of a mod): its first draw starts the world.
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![Step::StartWorld, Step::ReleaseHeld], false));
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![], false));
    }

    #[test]
    fn an_empty_main_view_with_a_root_of_its_own_starts_the_world_at_its_end() {
        let mut plan = Plan::default();
        plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::StartWorld, Step::ReleaseHeld, Step::EndWorld]);
    }

    /// A world scene that comes after a main view with a root of its own: the world began in
    /// the earlier scene, so the world scene gets the table's main view and no second start.
    #[test]
    fn a_world_scene_after_a_main_view_with_a_root_of_its_own_still_draws_the_main_view() {
        let mut plan = Plan::default();
        plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true);
        plan.view_first_place(true, true, false);
        plan.scene_end(true, true, true, false, false);
        // The step comes where the world would start: after the sky, at its first draw.
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![]);
        assert_eq!(plan.world_draw(true, true, true, false, false), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![Step::MainView], false));
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![], false));
        plan.scene_end(true, true, true, false, false);
        // Once in a frame.
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![]);
        assert_eq!(plan.world_draw(true, true, false, false, false), (vec![], false));
        plan.scene_end(true, true, true, false, false);
        // With the walk marked, it comes at the walk event.
        plan.frame_start();
        plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true);
        plan.view_first_place(true, true, false);
        plan.scene_end(true, true, true, false, true);
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.world_draw(true, true, true, false, true), (vec![], false));
        assert_eq!(plan.world_draw(true, true, false, false, true), (vec![], false));
        assert_eq!(plan.world_walk(true, true), vec![Step::MainView]);
        assert_eq!(plan.world_walk(true, true), vec![]);
        // An empty world scene: at its end.
        plan.frame_start();
        plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true);
        plan.view_first_place(true, true, false);
        plan.scene_end(true, true, true, false, false);
        plan.scene_begin(SCENE_WORLD);
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![Step::MainView, Step::ReleaseHeld]);
        // A world scene that began the world itself needs no such step.
        plan.frame_start();
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![]);
        plan.world_draw(true, true, false, true, false);
        plan.scene_end(true, true, true, false, false);
        assert_eq!(plan.scene_begin(SCENE_WORLD), vec![]);
    }

    /// An Isolate Subject frame (another screen with the world camera) is a main view.
    #[test]
    fn an_isolate_frame_is_a_main_view() {
        let kind = ViewKind::classify(false, Views::camera_is_world(SCENE_OTHER, true));
        assert_eq!(kind, ViewKind::Main);
        // Without the flag (a mod's camera), another screen.
        assert_ne!(ViewKind::classify(false, Views::camera_is_world(SCENE_OTHER, false)), ViewKind::Main);
    }

    /// The additions of MGE XE are the main view's: another screen, a picture and a frame
    /// with no extension start no world.
    #[test]
    fn only_a_main_view_starts_the_world() {
        let mut plan = Plan::default();
        assert_eq!(plan.scene_begin(SCENE_OTHER), vec![]);
        assert!(!plan.main_other_root());
        assert_eq!(plan.view_first_place(true, true, false), vec![]);
        assert_eq!(plan.world_draw(true, true, false, false, true), (vec![], false));
        assert_eq!(plan.scene_end(true, true, true, false, false), vec![]);
        assert!(!plan.world_begun);
        let mut plan = Plan::default();
        plan.scene_begin_view(SCENE_OTHER, ViewKind::Main, true);
        assert_eq!(plan.view_first_place(false, true, false), vec![]);
        assert_eq!(plan.view_first_place(true, false, false), vec![]);
        assert!(!plan.world_begun);
    }
}
