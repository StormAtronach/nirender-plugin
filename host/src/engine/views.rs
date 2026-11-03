//! The views of the table of their own (the header bit `HEADER_VIEW_PLACES`). In a scene into a
//! rendered texture (the picture of a save, a mod's mirror) and in a scene of another camera or
//! root into the back buffer (`scene::SCENE_OTHER`), the plugin writes the place of every
//! retained shape the engine displays: `INSTANCE_WALK` at its display, `INSTANCE_QUEUED` at the
//! flush of the engine's sorted queue. The helper draws the scene's shapes only there, with the
//! view of that scene in a scene slot of its own, and its sorted blends in their order of
//! arrival at the flush. A state of the frame and no device: the player notes each scene,
//! place and flush, and each item draws what this state gives it.
//!
//! The state of the world stays the frame's: the frame's `RETAINED_SCENE`, the instance
//! records and the light records reach the table as they come, whichever scene carries them.
//! The shade of the table ([`shade_choice`]) is made once in a frame, only from the world
//! camera's view; a view of another camera takes the latest one.

use super::scene;

/// The view of a scene, at its first place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ViewFacts {
    pub(crate) view: [f32; 16],
    pub(crate) projection: [f32; 16],
    pub(crate) origin: Option<[f64; 3]>,
    pub(crate) target_size: (u32, u32),
}

/// The places of one scene.
#[derive(Debug, Default)]
pub(crate) struct Group {
    /// The scene is of the world camera ([`scene::SCENE_WORLD`]): its view can make the shade
    /// of the frame.
    pub(crate) world_camera: bool,
    pub(crate) facts: Option<ViewFacts>,
    /// The scene slot of the view once it is written; `dropped` when all slots were taken.
    pub(crate) slot: Option<u32>,
    pub(crate) dropped: bool,
    /// The runs of places: the ids with no other item between them.
    runs: Vec<Vec<u32>>,
    /// The places in the flush of the engine's queue, in their order of arrival.
    sorted: Vec<u32>,
}

/// What a place did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Place {
    /// No scene of a view of its own is open, or the table has no such instance.
    Outside,
    Unknown,
    /// The place joined the run of the last item.
    Joined,
    /// The place starts a run: the group and the run, for a new item.
    New(usize, usize),
}

/// The views of a frame.
#[derive(Debug, Default)]
pub(crate) struct Views {
    /// The head of the stream says that the plugin writes the places.
    pub(crate) on: bool,
    groups: Vec<Group>,
    /// The group of the scene in progress.
    open: Option<usize>,
}

impl Views {
    /// The start of a frame: no group.
    pub(crate) fn frame_start(&mut self) {
        self.groups.clear();
        self.open = None;
    }

    /// The start of a scene. `in_target`: the game draws into a rendered texture of the helper.
    /// True when the scene has a view of its own.
    pub(crate) fn scene_begin(&mut self, kind: u32, in_target: bool) -> bool {
        self.open = None;
        if !self.on || !(in_target || kind == scene::SCENE_OTHER) {
            return false;
        }
        self.groups.push(Group { world_camera: kind == scene::SCENE_WORLD, ..Group::default() });
        self.open = Some(self.groups.len() - 1);
        true
    }

    /// The end of the scene: the group whose sorted places wait for an item, if any.
    pub(crate) fn scene_end(&mut self) -> Option<usize> {
        let open = self.open.take()?;
        (!self.groups[open].sorted.is_empty()).then_some(open)
    }

    /// The flush of the engine's queue in the scene: the group whose sorted places are due now.
    /// A later flush of the scene gives the places that came after this one.
    pub(crate) fn queue_flushed(&mut self) -> Option<usize> {
        let open = self.open?;
        (!self.groups[open].sorted.is_empty()).then_some(open)
    }

    /// The group of the scene in progress.
    pub(crate) fn open(&self) -> Option<usize> {
        self.open
    }

    pub(crate) fn group(&self, group: usize) -> Option<&Group> {
        self.groups.get(group)
    }

    pub(crate) fn group_mut(&mut self, group: usize) -> Option<&mut Group> {
        self.groups.get_mut(group)
    }

    /// The view of the open scene at its first place.
    pub(crate) fn note_facts(&mut self, facts: impl FnOnce() -> ViewFacts) {
        if let Some(group) = self.open.and_then(|open| self.groups.get_mut(open)) {
            if group.facts.is_none() {
                group.facts = Some(facts());
            }
        }
    }

    /// A place in the walk of the open scene. `known`: the table has the instance;
    /// `adjacent`: the last item of the frame is a run of this group.
    pub(crate) fn place(&mut self, id: u32, known: bool, adjacent: bool) -> Place {
        let Some(open) = self.open else { return Place::Outside };
        if !known {
            return Place::Unknown;
        }
        let runs = &mut self.groups[open].runs;
        match runs.last_mut() {
            Some(run) if adjacent => {
                run.push(id);
                Place::Joined
            }
            _ => {
                runs.push(vec![id]);
                Place::New(open, runs.len() - 1)
            }
        }
    }

    /// A place in the flush of the engine's queue in the open scene. False when no scene of a
    /// view of its own is open.
    pub(crate) fn queued(&mut self, id: u32) -> bool {
        let Some(open) = self.open else { return false };
        self.groups[open].sorted.push(id);
        true
    }

    /// The ids of a run, for its item; a run draws once.
    pub(crate) fn take_run(&mut self, group: usize, run: usize) -> Vec<u32> {
        self.groups.get_mut(group).and_then(|g| g.runs.get_mut(run)).map(core::mem::take).unwrap_or_default()
    }

    /// The sorted places of a group that came so far, for its item.
    pub(crate) fn take_sorted(&mut self, group: usize) -> Vec<u32> {
        self.groups.get_mut(group).map(|g| core::mem::take(&mut g.sorted)).unwrap_or_default()
    }
}

/// Where the shade of a view comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShadeChoice {
    /// The view makes the frame's shade from its own view: the first view of the world camera.
    Make,
    /// The frame's shade, made before by a view of the world camera.
    ThisFrame,
    /// The last frame's: a view of another camera before any view of the world camera. One
    /// frame late \[I: not visible\]; counted.
    LastFrame,
    /// No shade yet.
    None,
}

/// The shade of a view. `world_camera`: the view is of the world camera; `this_frame`: the
/// frame's shade is made; `last_frame`: a shade of an earlier frame is kept. A view of another
/// camera never makes the shade: its camera would light the main view.
pub(crate) fn shade_choice(world_camera: bool, this_frame: bool, last_frame: bool) -> ShadeChoice {
    if this_frame {
        ShadeChoice::ThisFrame
    } else if world_camera {
        ShadeChoice::Make
    } else if last_frame {
        ShadeChoice::LastFrame
    } else {
        ShadeChoice::None
    }
}

/// A scene slot for a view: the next free one of the frame, or [`None`] when all `capacity`
/// are taken. Slot 0 is the main view's: the count of a frame starts at 1.
pub(crate) fn take_slot(used: &mut u32, capacity: usize) -> Option<u32> {
    if *used == 0 {
        *used = 1;
    }
    if *used as usize >= capacity {
        return None;
    }
    *used += 1;
    Some(*used - 1)
}

/// The view turns the winding of its triangles (a mirror's reflection). The engine's view of
/// a camera takes the right, up and direction axes of the world into the x, y and z of the
/// view, so its rotation has the determinant -1; a reflection makes it +1. The rows of a view
/// of the stream are the first three of `view`.
pub(crate) fn mirrored(view: &[f32; 16]) -> bool {
    let (a, b, c) = (&view[0..3], &view[4..7], &view[8..11]);
    let determinant = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]);
    determinant > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::scene::{SCENE_FIRST_PERSON, SCENE_OTHER, SCENE_UI, SCENE_WORLD};

    fn on() -> Views {
        Views { on: true, ..Views::default() }
    }

    /// The scenes of a save's picture: the world and the first person into the texture, then
    /// the main frame into the back buffer. Each target scene has a group; the back buffer's
    /// world scene has none.
    #[test]
    fn a_scene_into_a_target_or_of_another_root_has_a_view_of_its_own() {
        let mut views = on();
        assert!(views.scene_begin(SCENE_WORLD, true));
        assert!(views.group(0).unwrap().world_camera);
        assert_eq!(views.scene_end(), None);
        assert!(views.scene_begin(SCENE_FIRST_PERSON, true));
        assert!(!views.group(1).unwrap().world_camera);
        views.scene_end();
        assert!(!views.scene_begin(SCENE_WORLD, false));
        assert_eq!(views.place(5, true, false), Place::Outside);
        assert!(!views.queued(5));
        assert!(!views.scene_begin(SCENE_UI, false));
        // A mod's subject on another root, into the back buffer.
        assert!(views.scene_begin(SCENE_OTHER, false));
        // Without the header bit nothing changes.
        let mut off = Views::default();
        assert!(!off.scene_begin(SCENE_WORLD, true));
        assert_eq!(off.place(5, true, false), Place::Outside);
    }

    #[test]
    fn the_places_of_a_scene_draw_in_runs_and_its_sorted_places_at_the_flush() {
        let mut views = on();
        views.scene_begin(SCENE_WORLD, true);
        views.note_facts(|| ViewFacts { view: [1.0; 16], projection: [2.0; 16], origin: None, target_size: (128, 128) });
        views.note_facts(|| ViewFacts { view: [3.0; 16], projection: [3.0; 16], origin: None, target_size: (1, 1) });
        assert_eq!(views.group(0).unwrap().facts.unwrap().target_size, (128, 128));
        assert_eq!(views.place(10, true, false), Place::New(0, 0));
        assert_eq!(views.place(11, true, true), Place::Joined);
        assert_eq!(views.place(99, false, true), Place::Unknown);
        assert_eq!(views.place(12, true, false), Place::New(0, 1));
        assert!(views.queued(20));
        assert_eq!(views.queue_flushed(), Some(0));
        assert_eq!(views.take_sorted(0), vec![20]);
        assert!(views.queued(21));
        assert_eq!(views.scene_end(), Some(0));
        assert_eq!(views.take_sorted(0), vec![21]);
        assert_eq!(views.take_run(0, 0), vec![10, 11]);
        assert_eq!(views.take_run(0, 0), Vec::<u32>::new());
        assert_eq!(views.take_run(0, 1), vec![12]);
        // The next frame starts empty.
        views.frame_start();
        assert!(views.group(0).is_none() && views.open().is_none());
    }

    /// The shade is the world camera's: the save's world view makes it when it comes first;
    /// a mod's view first in a frame never makes it, and takes the last frame's.
    #[test]
    fn only_a_view_of_the_world_camera_makes_the_shade() {
        assert_eq!(shade_choice(true, false, true), ShadeChoice::Make);
        assert_eq!(shade_choice(true, true, true), ShadeChoice::ThisFrame);
        assert_eq!(shade_choice(false, false, true), ShadeChoice::LastFrame);
        assert_eq!(shade_choice(false, true, true), ShadeChoice::ThisFrame);
        assert_eq!(shade_choice(false, false, false), ShadeChoice::None);
    }

    /// Slot 0 stays the main view's; a full set of slots gives none.
    #[test]
    fn a_view_never_takes_the_main_slot() {
        let mut used = 0;
        assert_eq!(take_slot(&mut used, 3), Some(1));
        assert_eq!(take_slot(&mut used, 3), Some(2));
        assert_eq!(take_slot(&mut used, 3), None);
        assert_eq!(used, 3);
    }

    #[test]
    fn a_reflection_turns_the_winding() {
        // The engine's view of a camera that looks north: right x, up z, direction y
        // (`NiDX8Renderer::SetCameraData`, the plugin's test of its view matrix).
        let mut view = [0.0f32; 16];
        view[0] = 1.0;
        view[6] = 1.0;
        view[9] = 1.0;
        view[15] = 1.0;
        assert!(!mirrored(&view));
        // A mirror's reflection of it: the direction turned.
        view[9] = -1.0;
        assert!(mirrored(&view));
    }
}
