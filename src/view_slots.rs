//! The view slots of D82 (stage 1 of `design-d82-view-per-render.md`, section 3.1). Each
//! render of the engine that the retained table serves (a `NiCamera::Click`) is a scene of a
//! view, and each view has a slot. The key of a view into the back buffer is its camera: all
//! the clicks of that camera into the back buffer in one frame are scenes of one view, and each
//! of their roots is part of its content. The key of a view into a texture is its camera, its
//! root and its target. The same texture key again before a flush gets a second slot, which
//! goes at the next flush or at the frame's end. Slot
//! [`MAIN_VIEW`] is the world camera into the back buffer. A slot with no scene for
//! [`RETIRE_FRAMES`] frames goes; the caller sends its retirement to the helper.
//!
//! The slot pressure (design 3.1): no scene is refused. A key with no slot takes a free one,
//! else takes back the slot that closed longest ago (never the main view's, an open one or one
//! used since the last flush), else the table is full and the caller forces a flush. A flush
//! closes the open views into a texture first; a view into the back buffer stays open until the
//! swap (design section 6). The destructor of a texture's renderer data frees the slots of
//! the views into that texture ([`ViewTable::target_gone`]), so that a new texture at the same
//! address does not take an old slot.
//!
//! A swap closes views; it does not end the frame. The frame ends at the device's Present,
//! where the plugin writes `FRAME_END` and the retained table's frame advances: there the slots
//! retire and the flush counts clear ([`ViewTable::present`]). JoP's visibility test swaps
//! with a texture as the target in the middle of a frame (section 7a).
//!
//! The scenes that are not views (the shadow cameras, the first person, the UI, the splash)
//! get no slot and close no view: [`is_view`].
//!
//! The table keeps no state of the engine's sorted queue. The places of the queue belong to the
//! view of the scene whose click draws the list (`NiClusterAccumulator::FinishAccumulating`
//! 0x6CF0E0, called inside `NiCamera::Click` at 0x6CC895), and they come in that scene.

use crate::link::{SCENE_FIRST_PERSON, SCENE_SHADOW_CASTERS, SCENE_SHADOW_VOLUME, SCENE_SORTED_ALPHA, SCENE_SPLASH, SCENE_UI};
use crate::ptr_hash::HashMap;
pub use crate::stream_format::VIEW_SLOTS;

/// The slot of the world camera into the back buffer.
pub const MAIN_VIEW: u8 = 0;
/// The frames without a scene after which a slot goes.
pub const RETIRE_FRAMES: u32 = 60;

/// A scene of this kind is a scene of a view.
pub fn is_view(kind: u32) -> bool {
    !matches!(kind, SCENE_SHADOW_VOLUME | SCENE_SHADOW_CASTERS | SCENE_FIRST_PERSON | SCENE_UI | SCENE_SPLASH | SCENE_SORTED_ALPHA)
}

/// The key of a view.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ViewKey {
    /// A camera into the back buffer.
    Screen { camera: usize },
    /// A camera into a texture. `again` counts the scenes of the same camera, root and target
    /// before this one since the last flush.
    Texture { camera: usize, root: usize, target: usize, again: u32 },
}

/// A scene's start, as the view table reads it.
#[derive(Clone, Copy, Debug)]
pub struct Click {
    /// The scene kind (`link::SCENE_*`).
    pub kind: u32,
    pub camera: usize,
    pub root: usize,
    /// The render target, or `None` for the back buffer.
    pub target: Option<usize>,
    /// The camera is the world camera.
    pub world_camera: bool,
}

/// The slot that a scene opened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Opened {
    pub slot: u8,
    /// The slot was given to the key at this scene.
    pub new: bool,
    /// The slot was taken back from the key that closed longest ago (design 3.1, the slot
    /// pressure, rule 4): the caller writes its retirement before the scene's record, so the
    /// new key never gets the old key's shapes.
    pub taken_back: bool,
}

/// What a scene's start found in the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Opening {
    /// The scene is not a view (`is_view`).
    NotView,
    Opened(Opened),
    /// Every slot but the main view's is open or used since the last flush (rule 5): the
    /// caller flushes ([`ViewTable::flush`] and the helper's flush) and opens again.
    Full,
}

/// A view that closed: its slot, and a root change. A view into the back buffer changes its
/// root when the set of roots of its scenes since its last close differs from the set before;
/// a view into a texture never does (its root is in its key).
pub type Closed = (u8, bool);

/// What a flush or the device's Present ended.
#[derive(Default, PartialEq, Eq, Debug)]
pub struct FrameEnd {
    /// The views that closed: at a flush, the open views into a texture; at Present, the views
    /// that no swap closed.
    pub closed: Vec<Closed>,
    /// The slots that went: the second slots (rule 2), and at Present the slots that had no
    /// scene for [`RETIRE_FRAMES`] frames.
    pub retired: Vec<u8>,
}

struct Slot {
    key: ViewKey,
    /// The frame of its last scene.
    last_frame: u32,
    /// The flush (the count of [`ViewTable::flush`] and Present) of its last scene.
    last_flush: u32,
    /// The order of its last close among all closes, for the take-back of the slot that
    /// closed longest ago.
    closed_order: u64,
    /// The roots of its scenes since its last close, and at that close.
    roots: Vec<usize>,
    last_roots: Vec<usize>,
    /// A scene opened it and nothing closed it yet.
    open: bool,
    /// Its target was destroyed while its items had not run: it has no key, and it goes at
    /// the next flush or at the frame's end (rule 3).
    dead: bool,
}

impl Slot {
    fn new(key: ViewKey, frame: u32, flush: u32) -> Slot {
        Slot { key, last_frame: frame, last_flush: flush, closed_order: 0, roots: Vec::new(), last_roots: Vec::new(), open: false, dead: false }
    }

    /// A second slot: its key's own slot was used in the same flush (rule 2).
    fn second(&self) -> bool {
        matches!(self.key, ViewKey::Texture { again, .. } if again > 0)
    }

    fn targets_texture(&self) -> bool {
        matches!(self.key, ViewKey::Texture { .. })
    }

    /// Closes the view. Returns its root change.
    fn close(&mut self, order: u64) -> bool {
        self.open = false;
        self.closed_order = order;
        let mut roots = core::mem::take(&mut self.roots);
        roots.sort_unstable();
        let changed = !self.last_roots.is_empty() && roots != self.last_roots;
        self.last_roots = roots;
        changed
    }
}

/// The view slots and their keys.
#[derive(Default)]
pub struct ViewTable {
    slots: [Option<Slot>; VIEW_SLOTS],
    by_key: HashMap<ViewKey, u8>,
    /// The scenes of each texture key (camera, root, target) since the last flush.
    since_flush: HashMap<(usize, usize, usize), u32>,
    /// The frames ended at Present.
    frame: u32,
    /// The flushes and Presents so far.
    flushes: u32,
    /// The closes so far.
    closes: u64,
    pub retired: u64,
    pub root_changes: u64,
    /// The counters of the slot pressure (rule 6): the second slots taken, the most of them in
    /// one flush, the slots taken back, and the flushes forced.
    pub second_slots: u64,
    pub most_second_in_flush: u32,
    second_in_flush: u32,
    pub taken_back: u64,
    pub forced_flushes: u64,
    /// The slots freed because their target was destroyed (rule 3).
    pub targets_gone: u64,
}

impl ViewTable {
    /// The slot of a scene. A key that has no slot takes a free one, else takes back the slot
    /// that closed longest ago, never the main view's, an open one or one used since the last
    /// flush. With none of those: [`Opening::Full`].
    pub fn open(&mut self, click: &Click) -> Opening {
        if !is_view(click.kind) {
            return Opening::NotView;
        }
        let key = match click.target {
            None => ViewKey::Screen { camera: click.camera },
            Some(target) => {
                let scenes = self.since_flush.entry((click.camera, click.root, target)).or_insert(0);
                ViewKey::Texture { camera: click.camera, root: click.root, target, again: *scenes }
            }
        };
        let (slot, new, taken_back) = if click.world_camera && click.target.is_none() {
            let (slot, new) = self.take_main(key);
            (slot, new, false)
        } else if let Some(&slot) = self.by_key.get(&key) {
            (slot, false, false)
        } else if let Some(free) = (1..VIEW_SLOTS).find(|&i| self.slots[i].is_none()) {
            (free as u8, true, false)
        } else if let Some(back) = self.oldest_closed() {
            let old = self.slots[back as usize].take().expect("a slot to take back");
            self.by_key.remove(&old.key);
            self.taken_back += 1;
            (back, true, true)
        } else {
            return Opening::Full;
        };
        if let Some(target) = click.target {
            *self.since_flush.entry((click.camera, click.root, target)).or_insert(0) += 1;
        }
        if new && slot != MAIN_VIEW {
            self.slots[slot as usize] = Some(Slot::new(key, self.frame, self.flushes));
            self.by_key.insert(key, slot);
            if matches!(key, ViewKey::Texture { again, .. } if again > 0) {
                self.second_slots += 1;
                self.second_in_flush += 1;
                self.most_second_in_flush = self.most_second_in_flush.max(self.second_in_flush);
            }
        }
        let entry = self.slots[slot as usize].as_mut().expect("an open slot");
        entry.last_frame = self.frame;
        entry.last_flush = self.flushes;
        entry.open = true;
        if click.target.is_none() && !entry.roots.contains(&click.root) {
            entry.roots.push(click.root);
        }
        Opening::Opened(Opened { slot, new, taken_back })
    }

    /// The slot that closed longest ago and that the take-back may use: not the main view's,
    /// not open, not used since the last flush.
    fn oldest_closed(&self) -> Option<u8> {
        (1..VIEW_SLOTS)
            .filter_map(|i| self.slots[i].as_ref().map(|slot| (i, slot)))
            .filter(|(_, slot)| !slot.open && slot.last_flush != self.flushes)
            .min_by_key(|(_, slot)| slot.closed_order)
            .map(|(i, _)| i as u8)
    }

    /// Slot [`MAIN_VIEW`] for the world camera's key. Another world camera takes the slot over.
    fn take_main(&mut self, key: ViewKey) -> (u8, bool) {
        let main = &mut self.slots[MAIN_VIEW as usize];
        let new = main.as_ref().is_none_or(|slot| slot.key != key);
        if new {
            if let Some(old) = main.take() {
                self.by_key.remove(&old.key);
            }
            *main = Some(Slot::new(key, self.frame, self.flushes));
            self.by_key.insert(key, MAIN_VIEW);
        }
        (MAIN_VIEW, new)
    }

    /// A readback, a tile's copy or a forced flush ran the frame's work so far (`forced`: rule
    /// 5). The open views into a texture close first (section 6: a view into the back buffer
    /// stays open until the swap), then the second slots go (rule 2), and a texture key used
    /// again gets its first slot.
    pub fn flush(&mut self, forced: bool) -> FrameEnd {
        let mut end = FrameEnd::default();
        for (index, entry) in self.slots.iter_mut().enumerate() {
            let Some(slot) = entry.as_mut().filter(|slot| slot.open && slot.targets_texture()) else { continue };
            self.closes += 1;
            let changed = slot.close(self.closes);
            self.root_changes += changed as u64;
            end.closed.push((index as u8, changed));
        }
        self.free_second_slots(&mut end.retired);
        self.forced_flushes += forced as u64;
        self.next_flush();
        end
    }

    /// The renderer data of a rendered texture is destroyed (rule 3): the views into `target`
    /// lose their keys, so that a new texture at the same address takes a new slot. A slot
    /// whose items ran (not open, not used since the last flush) is free at once; another one
    /// stays busy until the next flush or the frame's end, after the record at which its items
    /// run. Gives the slots free at once, for their retirements.
    pub fn target_gone(&mut self, target: usize) -> Vec<u8> {
        let mut retired = Vec::new();
        for (index, entry) in self.slots.iter_mut().enumerate() {
            let Some(slot) = entry.as_mut().filter(|slot| !slot.dead && matches!(slot.key, ViewKey::Texture { target: into, .. } if into == target)) else {
                continue;
            };
            self.by_key.remove(&slot.key);
            self.targets_gone += 1;
            if slot.open || slot.last_flush == self.flushes {
                slot.dead = true;
            } else {
                *entry = None;
                self.retired += 1;
                retired.push(index as u8);
            }
        }
        self.since_flush.retain(|&(_, _, into), _| into != target);
        retired
    }

    /// Frees the short-lived slots after their items ran: the second slots (rule 2) and the
    /// slots of a destroyed target (rule 3).
    fn free_second_slots(&mut self, retired: &mut Vec<u8>) {
        for (index, entry) in self.slots.iter_mut().enumerate() {
            if entry.as_ref().is_some_and(|slot| !slot.open && (slot.second() || slot.dead)) {
                let slot = entry.take().expect("a short-lived slot");
                if !slot.dead {
                    self.by_key.remove(&slot.key);
                }
                self.retired += 1;
                retired.push(index as u8);
            }
        }
    }

    /// The flushes and Presents so far.
    pub fn flushes(&self) -> u32 {
        self.flushes
    }

    fn next_flush(&mut self) {
        self.since_flush.clear();
        self.second_in_flush = 0;
        self.flushes = self.flushes.wrapping_add(1);
    }

    /// The renderer's SwapBuffers, with the device's target. The back buffer (`None`) closes
    /// the open views into the back buffer; a texture closes the open views into it. Neither
    /// ends the frame: [`ViewTable::present`] does.
    pub fn swap(&mut self, target: Option<usize>) -> Vec<Closed> {
        let mut closed = Vec::new();
        for (index, entry) in self.slots.iter_mut().enumerate() {
            let Some(slot) = entry else { continue };
            let into = match slot.key {
                ViewKey::Screen { .. } => None,
                ViewKey::Texture { target, .. } => Some(target),
            };
            if slot.open && into == target {
                self.closes += 1;
                let changed = slot.close(self.closes);
                self.root_changes += changed as u64;
                closed.push((index as u8, changed));
            }
        }
        closed
    }

    /// The device's Present, where the frame ends: the views that no swap closed close, the
    /// second slots go, the slots without a scene for [`RETIRE_FRAMES`] frames go, and the
    /// flush counts clear. Slot [`MAIN_VIEW`] stays.
    pub fn present(&mut self) -> FrameEnd {
        let mut end = FrameEnd::default();
        for (index, entry) in self.slots.iter_mut().enumerate() {
            let Some(slot) = entry else { continue };
            if slot.open {
                self.closes += 1;
                let changed = slot.close(self.closes);
                self.root_changes += changed as u64;
                end.closed.push((index as u8, changed));
            } else if index != MAIN_VIEW as usize && !slot.second() && !slot.dead && self.frame.wrapping_sub(slot.last_frame) >= RETIRE_FRAMES {
                self.by_key.remove(&slot.key);
                *entry = None;
                self.retired += 1;
                end.retired.push(index as u8);
            }
        }
        self.free_second_slots(&mut end.retired);
        end.retired.sort_unstable();
        self.next_flush();
        self.frame = self.frame.wrapping_add(1);
        end
    }

    /// The key of a slot in use.
    pub fn key(&self, slot: u8) -> Option<ViewKey> {
        self.slots.get(slot as usize)?.as_ref().map(|slot| slot.key)
    }

    /// The frames that Present ended.
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// The slots in use.
    pub fn in_use(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{SCENE_OTHER, SCENE_WATER, SCENE_WORLD};

    const WORLD_CAMERA: usize = 0x1000;
    const WORLD_ROOT: usize = 0x2000;
    const WATER_ROOT: usize = 0x3000;

    fn world(kind: u32, root: usize) -> Click {
        Click { kind, camera: WORLD_CAMERA, root, target: None, world_camera: true }
    }

    fn texture(camera: usize, root: usize, target: usize) -> Click {
        Click { kind: SCENE_OTHER, camera, root, target: Some(target), world_camera: false }
    }

    /// A slot opened, `new` or not, with no take-back.
    fn opened(slot: u8, new: bool) -> Opening {
        Opening::Opened(Opened { slot, new, taken_back: false })
    }

    /// The slot of an opening.
    fn slot_of(opening: Opening) -> Option<u8> {
        match opening {
            Opening::Opened(opened) => Some(opened.slot),
            _ => None,
        }
    }

    /// A normal frame: the world click, a shadow click, the water click, the swap, Present.
    fn normal_frame(views: &mut ViewTable) -> Vec<Closed> {
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        views.open(&world(SCENE_WATER, WATER_ROOT));
        let closed = views.swap(None);
        views.present();
        closed
    }

    /// The world click and the water click of a frame are view 0, with a shadow click between
    /// them, and their two roots are no root change. A world camera on another root (JoP's
    /// Isolate Subject) is view 0 too, with a root change, and so is the way back.
    #[test]
    fn the_world_and_water_clicks_of_a_frame_are_the_main_view() {
        let mut views = ViewTable::default();
        let shadow = Click { kind: SCENE_SHADOW_CASTERS, camera: 0x5000, root: 0x5100, target: None, world_camera: false };
        for frame in 0..3 {
            assert_eq!(views.open(&world(SCENE_WORLD, WORLD_ROOT)), opened(MAIN_VIEW, frame == 0));
            assert_eq!(views.open(&shadow), Opening::NotView);
            assert_eq!(views.open(&world(SCENE_WATER, WATER_ROOT)), opened(MAIN_VIEW, false));
            assert_eq!(views.swap(None), vec![(MAIN_VIEW, false)], "frame {frame}");
            assert_eq!(views.present(), FrameEnd::default());
        }
        // The NPC's node in place of the world root and the water plane.
        assert_eq!(views.open(&world(SCENE_OTHER, 0x4000)), opened(MAIN_VIEW, false));
        assert_eq!(views.swap(None), vec![(MAIN_VIEW, true)]);
        views.present();
        views.open(&world(SCENE_OTHER, 0x4000));
        assert_eq!(views.swap(None), vec![(MAIN_VIEW, false)]);
        views.present();
        assert_eq!(normal_frame(&mut views), vec![(MAIN_VIEW, true)]);
        assert_eq!(views.root_changes, 2);
        // A frame with no world click (a menu) closes nothing, and the next frame compares
        // with the last close.
        assert_eq!(views.swap(None), vec![]);
        views.present();
        assert_eq!(normal_frame(&mut views), vec![(MAIN_VIEW, false)]);
        assert_eq!(views.in_use(), 1);
        // The eye below the water surface: renderWater clicks the water plane's root two more
        // times (0x51C686, 0x51C6D3, 0x51C713). The set of roots is the same: no root change.
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        for _ in 0..3 {
            views.open(&world(SCENE_WATER, WATER_ROOT));
        }
        assert_eq!(views.swap(None), vec![(MAIN_VIEW, false)]);
        views.present();
        assert_eq!(normal_frame(&mut views), vec![(MAIN_VIEW, false)]);
        assert_eq!(views.root_changes, 2);
    }

    /// The swap of the back buffer closes view 0 and ends no frame; Present ends it. A view
    /// that no swap closed closes at Present.
    #[test]
    fn the_swap_closes_view_0_and_present_ends_the_frame() {
        let mut views = ViewTable::default();
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        assert_eq!(views.swap(None), vec![(MAIN_VIEW, false)]);
        assert_eq!(views.frame(), 0);
        // A second swap before Present has nothing open to close.
        assert_eq!(views.swap(None), vec![]);
        assert_eq!(views.present(), FrameEnd::default());
        assert_eq!(views.frame(), 1);
        // A frame with no swap: Present closes view 0.
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        assert_eq!(views.present(), FrameEnd { closed: vec![(MAIN_VIEW, false)], retired: vec![] });
    }

    /// JoP's visibility test calls `swapBuffers` with a texture as the target (the Reviewer's
    /// N1): that swap closes the texture's open view only. View 0 stays open, the frame does
    /// not advance, and a second texture swap closes nothing.
    #[test]
    fn a_swap_into_a_texture_closes_only_its_view() {
        let mut views = ViewTable::default();
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        let test = texture(WORLD_CAMERA, WORLD_ROOT, 0x9000);
        let other = texture(0x7000, WORLD_ROOT, 0xA000);
        assert_eq!(slot_of(views.open(&test)), Some(1));
        assert_eq!(slot_of(views.open(&other)), Some(2));
        assert_eq!(views.swap(Some(0x9000)), vec![(1, false)]);
        assert_eq!(views.swap(Some(0x9000)), vec![]);
        assert_eq!(views.frame(), 0, "a texture swap advanced the frame");
        // View 0 is still open: the water click joins it, with no root change at its close.
        views.open(&world(SCENE_WATER, WATER_ROOT));
        assert_eq!(views.swap(None), vec![(MAIN_VIEW, false)]);
        // The other texture view had no swap: Present closes it.
        assert_eq!(views.present(), FrameEnd { closed: vec![(2, false)], retired: vec![] });
    }

    /// A texture key keeps its slot: after a flush in the frame, and in the next frame.
    /// Another camera into the back buffer is a view of its own, not view 0.
    #[test]
    fn a_texture_key_keeps_its_slot() {
        let mut views = ViewTable::default();
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        let test = texture(WORLD_CAMERA, WORLD_ROOT, 0x9000);
        assert_eq!(views.open(&test), opened(1, true));
        views.flush(false);
        assert_eq!(views.open(&test), opened(1, false));
        // The same camera and target with another root is another view.
        assert_eq!(views.open(&texture(WORLD_CAMERA, 0x4000, 0x9000)), opened(2, true));
        views.present();
        assert_eq!(views.open(&test), opened(1, false));
        let screen = Click { kind: SCENE_OTHER, camera: 0x6000, root: WORLD_ROOT, target: None, world_camera: false };
        assert_eq!(views.open(&screen), opened(3, true));
        assert_eq!(views.swap(None), vec![(3, false)]);
    }

    /// The same texture key twice before a flush: the second scene gets a second slot, which
    /// goes at the frame's end (rule 2). The next frame takes it again and counts it again.
    #[test]
    fn a_second_slot_goes_at_the_frames_end() {
        let mut views = ViewTable::default();
        let mirror = texture(0x7000, WORLD_ROOT, 0x9000);
        for frame in 0..2 {
            assert_eq!(views.open(&mirror), opened(1, frame == 0));
            assert_eq!(views.open(&mirror), opened(2, true));
            assert_eq!(views.present(), FrameEnd { closed: vec![(1, false), (2, false)], retired: vec![2] });
        }
        assert_eq!(views.in_use(), 1);
        assert_eq!((views.second_slots, views.most_second_in_flush), (2, 1));
    }

    /// A flush (a readback, a tile's copy) closes the open views into a texture first, and
    /// not view 0 or another camera's view into the back buffer (section 6). Then the second
    /// slots go, and the key's next scene uses its first slot: the map's tiles, each with a
    /// copy, never take a second slot.
    #[test]
    fn a_flush_closes_the_texture_views_and_frees_the_second_slots() {
        let mut views = ViewTable::default();
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        let screen = Click { kind: SCENE_OTHER, camera: 0x6000, root: WORLD_ROOT, target: None, world_camera: false };
        views.open(&screen);
        let map = texture(0x8000, WORLD_ROOT, 0x9000);
        let map_water = texture(0x8000, WATER_ROOT, 0x9000);
        assert_eq!(views.open(&map), opened(2, true));
        assert_eq!(views.open(&map_water), opened(3, true));
        assert_eq!(views.open(&map), opened(4, true));
        assert_eq!(views.flush(false), FrameEnd { closed: vec![(2, false), (3, false), (4, false)], retired: vec![4] });
        for _ in 0..6 {
            assert_eq!(views.open(&map), opened(2, false));
            assert_eq!(views.open(&map_water), opened(3, false));
            assert_eq!(views.flush(false).retired, Vec::<u8>::new());
        }
        assert_eq!(views.second_slots, 1);
        // View 0 and the other camera's view are still open: the swap closes them.
        assert_eq!(views.swap(None), vec![(MAIN_VIEW, false), (1, false)]);
    }

    /// A slot with no scene for 60 frames goes, once; the key gets a slot again at its next
    /// scene. The main view stays.
    #[test]
    fn a_slot_without_scenes_retires() {
        let mut views = ViewTable::default();
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        let save = texture(WORLD_CAMERA, WORLD_ROOT, 0x9000);
        views.open(&save);
        views.present();
        for _ in 1..RETIRE_FRAMES {
            assert!(views.present().retired.is_empty());
        }
        assert_eq!(views.present().retired, vec![1]);
        assert_eq!((views.in_use(), views.retired), (1, 1));
        assert!(views.present().retired.is_empty());
        assert_eq!(views.open(&save), opened(1, true));
    }

    /// 32 slots taken: a new key takes back the slot that closed longest ago (rule 4), never
    /// the main view's, an open one or one used since the last flush. With every slot open or
    /// used since the last flush the table is full (rule 5), and a forced flush makes room.
    #[test]
    fn a_full_table_takes_back_the_oldest_closed_slot_and_then_forces_a_flush() {
        let mut views = ViewTable::default();
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        for i in 1..VIEW_SLOTS {
            assert_eq!(slot_of(views.open(&texture(0x7000, WORLD_ROOT, 0x10_0000 + i))), Some(i as u8));
        }
        assert_eq!(views.in_use(), VIEW_SLOTS);
        // Every slot is open or used since the last flush.
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x20_0000)), Opening::Full);
        // The swap of slot 5's target closes it, but it is still used in this flush.
        views.swap(Some(0x10_0005));
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x20_0000)), Opening::Full);
        // The forced flush closes the texture views in slot order; slot 5 closed first.
        let end = views.flush(true);
        assert_eq!((end.closed.len(), end.retired), (VIEW_SLOTS - 2, vec![]));
        assert_eq!(views.forced_flushes, 1);
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x20_0000)), Opening::Opened(Opened { slot: 5, new: true, taken_back: true }));
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x20_0001)), Opening::Opened(Opened { slot: 1, new: true, taken_back: true }));
        assert_eq!(views.taken_back, 2);
        // The old key of slot 5 starts again in another slot at its next scene.
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x10_0005)), Opening::Opened(Opened { slot: 2, new: true, taken_back: true }));
        assert_eq!(views.in_use(), VIEW_SLOTS);
    }

    /// Rule 3: the destructor of a target takes the keys of the views into it, its first and
    /// its second slot, and no other. A slot whose items did not run yet stays busy until the
    /// next flush, so that its retirement comes after the flush's record; a new texture at
    /// the same address takes another slot. A slot whose items ran is free at once.
    #[test]
    fn a_destroyed_target_frees_its_slots_after_their_items_ran() {
        let mut views = ViewTable::default();
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        assert_eq!(slot_of(views.open(&texture(0x7000, WORLD_ROOT, 0x9000))), Some(1));
        views.swap(Some(0x9000));
        assert_eq!(slot_of(views.open(&texture(0x7000, WORLD_ROOT, 0x9000))), Some(2));
        views.swap(Some(0x9000));
        assert_eq!(slot_of(views.open(&texture(0x7000, WORLD_ROOT, 0x9100))), Some(3));
        views.swap(Some(0x9100));
        // Both slots of 0x9000 were used in this flush: no retirement yet.
        assert_eq!(views.target_gone(0x9000), vec![]);
        assert_eq!((views.targets_gone, views.retired, views.in_use()), (2, 0, 4));
        assert_eq!(views.target_gone(0x9000), vec![]);
        assert_eq!(views.targets_gone, 2);
        // A new texture at the same address: a new key and a new slot.
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x9000)), opened(4, true));
        views.swap(Some(0x9000));
        // The flush runs the items; then the two dead slots go, and the second slots too.
        let end = views.flush(false);
        assert_eq!(end.retired, vec![1, 2]);
        assert_eq!(views.key(4), Some(ViewKey::Texture { camera: 0x7000, root: WORLD_ROOT, target: 0x9000, again: 0 }));
        // 0x9100's slot was last used before this flush: free at once.
        assert_eq!(views.target_gone(0x9100), vec![3]);
        assert_eq!((views.targets_gone, views.retired), (3, 3));
    }
}
