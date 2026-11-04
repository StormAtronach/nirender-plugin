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

use super::full_tier::FullTier;
use super::scene;
use crate::stream_format::VIEW_SLOTS;
use crate::{Additions, ViewKind};

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
    /// The kind of the view and its additions ([`ViewKind::additions`]).
    pub(crate) kind: ViewKind,
    pub(crate) additions: Additions,
    pub(crate) facts: Option<ViewFacts>,
    /// The scene slot of the view once it is written; `dropped` when all slots were taken.
    pub(crate) slot: Option<u32>,
    pub(crate) dropped: bool,
    /// The call for the parts of the full tier of the view was queued in this scene
    /// ([`Additions::TIER_DRAWN`]).
    pub(crate) tier_begun: bool,
    /// The same for the call at the end of the world ([`Additions::TIER_AFTER`]).
    pub(crate) tier_ended: bool,
    /// The name of the view's GPU span and CPU time, made once its facts are known
    /// (`Player::view_group_name`).
    pub(crate) name: core::cell::Cell<Option<&'static str>>,
    /// The plugin's view slot of the group's scene (`scene_view`): see [`Group::view_slot`].
    view_slot: Option<u8>,
    /// The place of the full tier that the view holds in this scene (`FullTier::scene`).
    pub(crate) place: Option<u8>,
    /// The runs of places: the ids with no other item between them.
    runs: Vec<Vec<u32>>,
    /// The places in the flush of the engine's queue, in their order of arrival.
    sorted: Vec<u32>,
}

impl Group {
    /// The plugin's view slot of the group's scene (the view field of its `SCENE_BEGIN` less
    /// 1), 0 to 31. `None` for a scene without the field: an older plugin, or a stream without
    /// [`HEADER_SCENE_VIEWS`](crate::stream_format::HEADER_SCENE_VIEWS). A group is one scene,
    /// so it never has two slots; a view into the back buffer with several scenes in a frame
    /// has a group for each, all with its slot.
    pub(crate) fn view_slot(&self) -> Option<u32> {
        self.view_slot.map(u32::from)
    }
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

/// The slot of the world camera into the back buffer (the plugin's `view_slots::MAIN_VIEW`): a
/// new world camera takes it with no retirement.
const MAIN_VIEW: u8 = 0;
/// The offset of the view field in a scene record (the plugin's `SceneV1::view`).
const SCENE_VIEW_AT: usize = 48;
/// The size of a scene record with the view field (the plugin's `SceneV1`, design section
/// 3.4c): a shorter record, such as the 48 bytes of an older plugin, has no view.
const SCENE_WITH_VIEW: usize = 56;

/// The view slot of a scene record: the field holds the slot plus 1, 0 for a scene that is not
/// a view. `None` for a record without the field, for 0, or for a slot past the table.
pub(crate) fn scene_view(data: &[u8]) -> Option<u8> {
    if data.len() < SCENE_WITH_VIEW {
        return None;
    }
    let view = crate::data::word(data, SCENE_VIEW_AT) & 0xFF;
    (view != 0 && view as usize <= VIEW_SLOTS).then(|| (view - 1) as u8)
}

/// The key of a view slot, as its last scene record gave it: the camera, and for a view into a
/// texture the root and the target (0 for the back buffer). D82 stage 1 keeps it; the views
/// of stage 2 draw from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewSlot {
    camera: u32,
    root: u32,
    target: u32,
}

impl ViewSlot {
    /// The plugin keeps a key's slot until it retires: another key in the slot without a
    /// retirement is a fault of the stream. A view into the back buffer has many roots.
    fn same_key(&self, other: &ViewSlot) -> bool {
        self.camera == other.camera && self.target == other.target && (self.target == 0 || self.root == other.root)
    }
}

/// The counts of the view slots.
#[derive(Debug, Default, Clone, Copy)]
struct SlotCounts {
    scenes: u64,
    view_scenes: u64,
    /// Scenes whose slot held another key with no retirement between: a fault of the stream.
    rekeyed: u64,
    /// Scenes of a new world camera in the main slot, which it takes with no retirement.
    main_cameras: u64,
    retired: u64,
    /// Retirements of a slot that no scene named.
    retired_unknown: u64,
    /// Flushes that the plugin forced because every slot was open or used since the last flush.
    forced_flushes: u64,
}

/// The views of a frame.
#[derive(Debug, Default)]
pub(crate) struct Views {
    /// The head of the stream says that the plugin writes the places.
    pub(crate) on: bool,
    groups: Vec<Group>,
    /// The group of the scene in progress.
    open: Option<usize>,
    /// The view slot of the scene record just read (`note_scene_record`), for the group of
    /// the scene's start.
    scene_slot: Option<u8>,
    /// The head of the stream says that the scene records carry the view slot
    /// (`HEADER_SCENE_VIEWS`).
    pub(crate) slots_on: bool,
    /// The view slots of D82, for the whole run: a retirement frees one.
    slots: [Option<ViewSlot>; VIEW_SLOTS],
    slot_counts: SlotCounts,
    /// The user interface scene of the frame began: a view after it has no additions.
    ui_seen: bool,
    /// The places of the full tier and the parts that the mods asked for the view slots.
    pub(crate) tier: FullTier,
}

impl Views {
    /// A scene record of a stream with view slots. The camera, the root and the target are its
    /// words 2, 3 and 4. The target is 0 when the record's flag `SCENE_BACK_BUFFER` (word 5) is
    /// set: the plugin keys a view by the device's target, and word 4 is the last target that
    /// the renderer set.
    pub(crate) fn note_scene_record(&mut self, data: &[u8]) {
        let word = |at| crate::data::word(data, at);
        let target = if word(20) & crate::data::SCENE_BACK_BUFFER != 0 { 0 } else { word(16) };
        self.note_scene(scene_view(data), word(8), word(12), target);
    }

    /// A scene of a stream with view slots: the scene's slot keeps its key.
    fn note_scene(&mut self, view: Option<u8>, camera: u32, root: u32, target: u32) {
        self.scene_slot = view;
        self.slot_counts.scenes += 1;
        let Some(view) = view else { return };
        self.slot_counts.view_scenes += 1;
        let key = ViewSlot { camera, root, target };
        let slot = &mut self.slots[view as usize];
        if slot.is_some_and(|old| !old.same_key(&key)) {
            if view == MAIN_VIEW {
                self.slot_counts.main_cameras += 1;
            } else {
                self.slot_counts.rekeyed += 1;
            }
        }
        *slot = Some(key);
    }

    /// The frame event `EVENT_VIEW_RETIRE`: the slot is free.
    pub(crate) fn retire(&mut self, slot: u32) {
        self.tier.retire(slot);
        match self.slots.get_mut(slot as usize).and_then(Option::take) {
            Some(_) => self.slot_counts.retired += 1,
            None => self.slot_counts.retired_unknown += 1,
        }
    }

    /// The frame event `EVENT_FLUSH`: the plugin had no free slot.
    pub(crate) fn note_forced_flush(&mut self) {
        self.slot_counts.forced_flushes += 1;
    }

    /// The report line of the view slots.
    pub(crate) fn slots_line(&self) -> String {
        let c = &self.slot_counts;
        let known = self.slots.iter().flatten().count();
        let textures = self.slots.iter().flatten().filter(|slot| slot.target != 0).count();
        format!(
            "view slots: {} scenes, {} of them of a view; {known} slots known now ({textures} into a texture); {} scenes found another key in their slot, {} a new world camera in the main slot; {} slots retired, {} retirements of a slot not known; {} flushes forced",
            c.scenes, c.view_scenes, c.rekeyed, c.main_cameras, c.retired, c.retired_unknown, c.forced_flushes
        )
    }

    /// The start of a frame: no group.
    pub(crate) fn frame_start(&mut self) {
        self.groups.clear();
        self.open = None;
        self.ui_seen = false;
        self.tier.frame_start();
    }

    /// The camera of a scene is the world camera: the scene is one of the world camera's
    /// scenes, or a [`scene::SCENE_OTHER`] with the world camera `world_camera`. The second
    /// case needs the places of the view groups in the stream (`on`): a stream without them
    /// (a capture from before D82) has no shapes to draw for such a scene, so the plan keeps
    /// treating it as another screen.
    pub(crate) fn camera_is_world(&self, kind: u32, camera: u32, world_camera: u32) -> bool {
        scene::world_camera(kind) || (self.on && kind == scene::SCENE_OTHER && camera != 0 && camera == world_camera)
    }

    /// The start of a scene. `in_target`: the game draws into a rendered texture of the helper;
    /// `camera_is_world`: the camera of the scene is the world camera. True when the scene has
    /// a view of its own.
    pub(crate) fn scene_begin(&mut self, kind: u32, in_target: bool, camera_is_world: bool) -> bool {
        self.open = None;
        let view_slot = self.scene_slot.take();
        if kind == scene::SCENE_UI {
            self.ui_seen = true;
        }
        if !self.on || !(in_target || kind == scene::SCENE_OTHER) {
            return false;
        }
        let view = ViewKind::classify(in_target, camera_is_world);
        // The full tier is for the views of a mod: the main view has every part already.
        let grant = view_slot.filter(|_| view != ViewKind::Main).and_then(|slot| self.tier.scene(slot));
        let additions = view.additions(self.ui_seen).with(Additions::from_option_parts(grant.map_or(0, |grant| grant.parts)));
        let place = grant.map(|grant| grant.place);
        self.groups.push(Group { world_camera: kind == scene::SCENE_WORLD, kind: view, additions, view_slot, place, ..Group::default() });
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
        assert!(views.scene_begin(SCENE_WORLD, true, true));
        assert!(views.group(0).unwrap().world_camera);
        assert_eq!(views.scene_end(), None);
        assert!(views.scene_begin(SCENE_FIRST_PERSON, true, false));
        assert!(!views.group(1).unwrap().world_camera);
        views.scene_end();
        assert!(!views.scene_begin(SCENE_WORLD, false, true));
        assert_eq!(views.place(5, true, false), Place::Outside);
        assert!(!views.queued(5));
        assert!(!views.scene_begin(SCENE_UI, false, false));
        // A mod's subject on another root, into the back buffer.
        assert!(views.scene_begin(SCENE_OTHER, false, false));
        // Without the header bit nothing changes.
        let mut off = Views::default();
        assert!(!off.scene_begin(SCENE_WORLD, true, true));
        assert_eq!(off.place(5, true, false), Place::Outside);
    }

    #[test]
    fn the_places_of_a_scene_draw_in_runs_and_its_sorted_places_at_the_flush() {
        let mut views = on();
        views.scene_begin(SCENE_WORLD, true, true);
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

    /// A view slot keeps its key across frames until its retirement. Another key in a slot
    /// with no retirement is counted; a texture view with a new root is another key, the back
    /// buffer's view is not.
    #[test]
    fn a_view_slot_keeps_its_key_until_it_retires() {
        let mut views = Views::default();
        views.note_scene(Some(0), 10, 100, 0);
        views.note_scene(Some(3), 20, 200, 7);
        views.note_scene(None, 30, 300, 0);
        views.frame_start();
        assert_eq!(views.slots[3], Some(ViewSlot { camera: 20, root: 200, target: 7 }));
        views.note_scene(Some(0), 10, 101, 0);
        assert_eq!(views.slot_counts.rekeyed, 0);
        views.note_scene(Some(3), 20, 201, 7);
        assert_eq!(views.slot_counts.rekeyed, 1);
        views.retire(3);
        views.retire(3);
        views.retire(99);
        assert_eq!(views.slots[3], None);
        assert_eq!((views.slot_counts.retired, views.slot_counts.retired_unknown), (1, 2));
        views.note_scene(Some(3), 40, 400, 9);
        assert_eq!(views.slot_counts.rekeyed, 1);
        assert_eq!((views.slot_counts.scenes, views.slot_counts.view_scenes), (6, 5));
        assert!(views.slots_line().starts_with("view slots: 6 scenes, 5 of them of a view; 2 slots known now (1 into a texture)"));
        // A new world camera takes the main slot with no retirement: no fault of the stream.
        views.note_scene(Some(MAIN_VIEW), 11, 100, 0);
        assert_eq!((views.slot_counts.rekeyed, views.slot_counts.main_cameras), (1, 1));
    }

    /// The scene record's flag `SCENE_BACK_BUFFER` gives target 0, as the plugin keys the view,
    /// whatever target the renderer set last (word 4).
    #[test]
    fn a_scene_into_the_back_buffer_has_target_0() {
        let record = |camera: u32, root: u32, target: u32, flags: u32| {
            let mut data = vec![0u8; 56];
            for (at, value) in [(8, camera), (12, root), (16, target), (20, flags), (48, 1)] {
                data[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            data
        };
        let mut views = Views::default();
        views.note_scene_record(&record(10, 100, 0x4000, crate::data::SCENE_BACK_BUFFER));
        assert_eq!(views.slots[0], Some(ViewSlot { camera: 10, root: 100, target: 0 }));
        // The last target set differs, and so does the root: the same back-buffer view.
        views.note_scene_record(&record(10, 101, 0x5000, crate::data::SCENE_BACK_BUFFER));
        assert_eq!((views.slot_counts.rekeyed, views.slot_counts.main_cameras), (0, 0));
        // Without the flag, word 4 is the target.
        views.note_scene_record(&record(10, 101, 0x5000, 0));
        assert_eq!(views.slots[0], Some(ViewSlot { camera: 10, root: 101, target: 0x5000 }));
    }

    /// A mod's options (`mge.camera.setViewOptions`) add parts to the view of its slot, within the
    /// places of the full tier. The view over the number and the main view get none.
    #[test]
    fn a_view_with_options_gets_the_parts_of_the_full_tier() {
        let mut views = on();
        views.tier.options(3, 0b11);
        views.tier.options(4, 0b1);
        views.tier.options(0, 0b1);
        views.note_scene(Some(3), 20, 200, 7);
        assert!(views.scene_begin(SCENE_WORLD, true, true));
        let picture = views.group(0).unwrap();
        assert!(picture.additions.contains(Additions::LAND.with(Additions::STATICS)) && !picture.additions.contains(Additions::GRASS));
        assert_eq!(picture.place, Some(0));
        views.note_scene(Some(4), 21, 200, 8);
        assert!(views.scene_begin(SCENE_WORLD, true, true));
        assert_eq!(views.group(1).unwrap().additions, Additions::NONE);
        assert_eq!(views.group(1).unwrap().place, None);
        views.note_scene(Some(0), 10, 100, 0);
        assert!(views.scene_begin(SCENE_OTHER, false, true));
        assert_eq!(views.group(2).unwrap().additions, Additions::ALL);
        // A scene with no slot (a stream without them) takes no place.
        assert!(views.scene_begin(SCENE_WORLD, true, true));
        assert_eq!(views.group(3).unwrap().additions, Additions::NONE);
        assert_eq!((views.tier.counts.granted, views.tier.counts.refused), (1, 1));
    }

    /// The kind of a view comes from the camera and the target: the world camera on a mod's root
    /// is a main view; another camera into the back buffer is another screen, with the
    /// additions of Direct3D 9's second stage before the user interface scene and none after
    /// it; a texture is a picture.
    #[test]
    fn a_view_has_its_kind_and_additions() {
        let mut views = on();
        assert!(views.scene_begin(SCENE_OTHER, false, true));
        assert_eq!(views.group(0).unwrap().kind, ViewKind::Main);
        assert_eq!(views.group(0).unwrap().additions, Additions::ALL);
        assert!(views.scene_begin(SCENE_OTHER, false, false));
        let screen = views.group(1).unwrap();
        assert_eq!(screen.kind, ViewKind::OtherScreen);
        assert!(screen.additions.contains(Additions::SHADOW_RECEIVE) && !screen.additions.contains(Additions::LAND));
        assert!(views.scene_begin(SCENE_WORLD, true, true));
        assert_eq!((views.group(2).unwrap().kind, views.group(2).unwrap().additions), (ViewKind::Picture, Additions::NONE));
        views.scene_begin(SCENE_UI, false, false);
        assert!(views.scene_begin(SCENE_OTHER, false, false));
        let after = views.group(3).unwrap();
        assert_eq!((after.kind, after.additions), (ViewKind::OtherScreen, Additions::NONE));
        // The next frame starts before the user interface scene again.
        views.frame_start();
        assert!(views.scene_begin(SCENE_OTHER, false, false));
        assert!(views.group(0).unwrap().additions.contains(Additions::POST_COVER));
    }

    /// A group keeps the view slot of its scene record; a scene with no record of a view
    /// (an older plugin) gives none, and a slot does not pass to the next scene.
    #[test]
    fn a_group_keeps_the_view_slot_of_its_scene() {
        let record = |view: u32| {
            let mut data = vec![0u8; 56];
            for (at, value) in [(8, 10), (12, 100), (16, 0x4000), (48, view)] {
                data[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            data
        };
        let mut views = on();
        views.note_scene_record(&record(4));
        assert!(views.scene_begin(SCENE_OTHER, true, false));
        assert_eq!(views.group(0).and_then(Group::view_slot), Some(3));
        views.scene_end();
        assert!(views.scene_begin(SCENE_OTHER, true, false));
        assert_eq!(views.group(1).and_then(Group::view_slot), None);
    }
}
