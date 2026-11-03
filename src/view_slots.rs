//! The view slots of D82 (stage 1 of `design-d82-view-per-render.md`, section 3.1). Each
//! render of the engine that the retained table serves (a `NiCamera::Click`) is a scene of a
//! view, and each view has a slot. The key of a view into the back buffer is its camera: all
//! the clicks of that camera into the back buffer in one frame are scenes of one view, and each
//! of their roots is part of its content. The key of a view into a texture is its camera, its
//! root and its target. The same texture key again before a flush gets another slot. Slot
//! [`MAIN_VIEW`] is the world camera into the back buffer. A slot with no scene for
//! [`RETIRE_FRAMES`] frames goes; the caller sends its retirement to the helper.
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

/// The number of view slots: one bit of a `u32` mask for each.
pub const VIEW_SLOTS: usize = 32;
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
}

/// A view that closed: its slot, and a root change. A view into the back buffer changes its
/// root when the set of roots of its scenes since its last close differs from the set before;
/// a view into a texture never does (its root is in its key).
pub type Closed = (u8, bool);

/// What the device's Present ended.
#[derive(Default, PartialEq, Eq, Debug)]
pub struct FrameEnd {
    /// The views that no swap closed in the frame.
    pub closed: Vec<Closed>,
    /// The slots that had no scene for [`RETIRE_FRAMES`] frames.
    pub retired: Vec<u8>,
}

struct Slot {
    key: ViewKey,
    /// The frame of its last scene.
    last_frame: u32,
    /// The roots of its scenes since its last close, and at that close.
    roots: Vec<usize>,
    last_roots: Vec<usize>,
    /// A scene opened it and nothing closed it yet.
    open: bool,
}

impl Slot {
    fn new(key: ViewKey, frame: u32) -> Slot {
        Slot { key, last_frame: frame, roots: Vec::new(), last_roots: Vec::new(), open: false }
    }

    /// Closes the view. Returns its root change.
    fn close(&mut self) -> bool {
        self.open = false;
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
    /// The scenes that found no free slot.
    pub refused: u64,
    pub retired: u64,
    pub root_changes: u64,
}

impl ViewTable {
    /// The slot of a scene, or `None`: not a view, or all slots taken (counted in `refused`).
    pub fn open(&mut self, click: &Click) -> Option<Opened> {
        if !is_view(click.kind) {
            return None;
        }
        let key = match click.target {
            None => ViewKey::Screen { camera: click.camera },
            Some(target) => {
                let scenes = self.since_flush.entry((click.camera, click.root, target)).or_insert(0);
                let again = *scenes;
                *scenes += 1;
                ViewKey::Texture { camera: click.camera, root: click.root, target, again }
            }
        };
        let (slot, new) = if click.world_camera && click.target.is_none() {
            self.take_main(key)
        } else if let Some(&slot) = self.by_key.get(&key) {
            (slot, false)
        } else {
            let Some(free) = (1..VIEW_SLOTS).find(|&i| self.slots[i].is_none()) else {
                self.refused += 1;
                return None;
            };
            self.slots[free] = Some(Slot::new(key, self.frame));
            self.by_key.insert(key, free as u8);
            (free as u8, true)
        };
        let entry = self.slots[slot as usize].as_mut().expect("an open slot");
        entry.last_frame = self.frame;
        entry.open = true;
        if click.target.is_none() && !entry.roots.contains(&click.root) {
            entry.roots.push(click.root);
        }
        Some(Opened { slot, new })
    }

    /// Slot [`MAIN_VIEW`] for the world camera's key. Another world camera takes the slot over.
    fn take_main(&mut self, key: ViewKey) -> (u8, bool) {
        let main = &mut self.slots[MAIN_VIEW as usize];
        let new = main.as_ref().is_none_or(|slot| slot.key != key);
        if new {
            if let Some(old) = main.take() {
                self.by_key.remove(&old.key);
            }
            *main = Some(Slot::new(key, self.frame));
            self.by_key.insert(key, MAIN_VIEW);
        }
        (MAIN_VIEW, new)
    }

    /// A readback ran the frame's work so far: a texture key used again gets its first slot.
    pub fn flush(&mut self) {
        self.since_flush.clear();
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
                let changed = slot.close();
                self.root_changes += changed as u64;
                closed.push((index as u8, changed));
            }
        }
        closed
    }

    /// The device's Present, where the frame ends: the views that no swap closed close, the
    /// slots without a scene for [`RETIRE_FRAMES`] frames go, and the flush counts clear. Slot
    /// [`MAIN_VIEW`] stays.
    pub fn present(&mut self) -> FrameEnd {
        let mut end = FrameEnd::default();
        for (index, entry) in self.slots.iter_mut().enumerate() {
            let Some(slot) = entry else { continue };
            if slot.open {
                let changed = slot.close();
                self.root_changes += changed as u64;
                end.closed.push((index as u8, changed));
            } else if index != MAIN_VIEW as usize && self.frame.wrapping_sub(slot.last_frame) >= RETIRE_FRAMES {
                self.by_key.remove(&slot.key);
                *entry = None;
                self.retired += 1;
                end.retired.push(index as u8);
            }
        }
        self.since_flush.clear();
        self.frame = self.frame.wrapping_add(1);
        end
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
            assert_eq!(views.open(&world(SCENE_WORLD, WORLD_ROOT)), Some(Opened { slot: MAIN_VIEW, new: frame == 0 }));
            assert_eq!(views.open(&shadow), None);
            assert_eq!(views.open(&world(SCENE_WATER, WATER_ROOT)), Some(Opened { slot: MAIN_VIEW, new: false }));
            assert_eq!(views.swap(None), vec![(MAIN_VIEW, false)], "frame {frame}");
            assert_eq!(views.present(), FrameEnd::default());
        }
        // The NPC's node in place of the world root and the water plane.
        assert_eq!(views.open(&world(SCENE_OTHER, 0x4000)), Some(Opened { slot: MAIN_VIEW, new: false }));
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
        assert_eq!(views.open(&test).map(|o| o.slot), Some(1));
        assert_eq!(views.open(&other).map(|o| o.slot), Some(2));
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
        assert_eq!(views.open(&test), Some(Opened { slot: 1, new: true }));
        views.flush();
        assert_eq!(views.open(&test), Some(Opened { slot: 1, new: false }));
        // The same camera and target with another root is another view.
        assert_eq!(views.open(&texture(WORLD_CAMERA, 0x4000, 0x9000)), Some(Opened { slot: 2, new: true }));
        views.present();
        assert_eq!(views.open(&test), Some(Opened { slot: 1, new: false }));
        let screen = Click { kind: SCENE_OTHER, camera: 0x6000, root: WORLD_ROOT, target: None, world_camera: false };
        assert_eq!(views.open(&screen), Some(Opened { slot: 3, new: true }));
        assert_eq!(views.swap(None), vec![(3, false)]);
    }

    /// The same texture key twice before a flush: the second scene gets a second slot, and
    /// each keeps its slot in the next frame.
    #[test]
    fn a_texture_key_twice_before_a_flush_gets_a_second_slot() {
        let mut views = ViewTable::default();
        let mirror = texture(0x7000, WORLD_ROOT, 0x9000);
        for frame in 0..2 {
            assert_eq!(views.open(&mirror), Some(Opened { slot: 1, new: frame == 0 }));
            assert_eq!(views.open(&mirror), Some(Opened { slot: 2, new: frame == 0 }));
            views.present();
        }
        assert_eq!(views.in_use(), 2);
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
        assert_eq!(views.open(&save), Some(Opened { slot: 1, new: true }));
    }

    /// 32 slots: a key with all of them taken gets none and is counted; a retirement makes
    /// room.
    #[test]
    fn all_slots_taken_refuse_a_new_key_and_count_it() {
        let mut views = ViewTable::default();
        views.open(&world(SCENE_WORLD, WORLD_ROOT));
        for i in 1..VIEW_SLOTS {
            assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x10_0000 + i)).map(|o| o.slot), Some(i as u8));
        }
        assert_eq!(views.in_use(), VIEW_SLOTS);
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x20_0000)), None);
        assert_eq!(views.refused, 1);
        // A second scene of a key before a flush needs a slot of its own: refused too.
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x10_0001)), None);
        assert_eq!(views.refused, 2);
        // After a flush the key gets its slot.
        views.flush();
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x10_0001)).map(|o| o.slot), Some(1));
        for _ in 0..=RETIRE_FRAMES {
            views.present();
        }
        assert_eq!(views.in_use(), 1, "only the main view stays");
        assert_eq!(views.retired, VIEW_SLOTS as u64 - 1);
        assert_eq!(views.open(&texture(0x7000, WORLD_ROOT, 0x20_0000)).map(|o| o.slot), Some(1));
    }
}
