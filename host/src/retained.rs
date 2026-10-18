//! The retained instances of the plugin drawn from a table of the helper (plan 0.5 entry
//! T4). An `INSTANCE` record with the flag 0x100 goes into the table: its world matrix and
//! its bound, and the record of its draw (`DrawData`) made once from its material and its
//! geometry, in a segment of the record buffer and of the instance buffer of the frame
//! that stays from frame to frame. The plugin then sends no draw for it: a change of its
//! lights (`INSTANCE_LIGHTS`), of its visibility (`INSTANCE_STATE`), of its values (the
//! `INSTANCE` record again) or its end (`INSTANCE_DELETE`).
//!
//! At the start of the world of a frame, and in each view that the extension replays (a
//! mirror, the layers of the shadow map), the helper tests the bound of each visible
//! instance against the planes of the view on the CPU and draws the ones in view with one
//! indirect call for each bucket (the instances with one pipeline, one stencil reference,
//! one pair of pool buffers, and, without the arrays of the frame, one set of textures).
//! The values of the view (its matrices, the fog, the ambient light, the pixel step, the
//! clip plane) are in a `Scene` uniform with a dynamic offset: the record of a retained
//! instance has the flag 4 in `light_range.w`, and the shader takes those values from the
//! scene instead of the record. The lights of an instance are indexes into a table of the
//! lights the retained instances use (flag 8): a light that changes is written once.
//!
//! Not here yet: the cull on the GPU with a depth pyramid (the second part of T4), and
//! instancing of the instances that share a geometry and a material.

use super::*;

/// The size of a `Scene` uniform, the step of the dynamic offset (a multiple of the
/// alignment of a uniform offset, 256).
pub const SCENE_SIZE: usize = 512;
/// The scenes a frame can have: the world, and the views the extension replays.
pub const SCENE_SLOTS: usize = 32;
/// The room of the segments at first.
pub const RETAINED_AT_FIRST: usize = 2048;
pub const LIGHT_INDEXES_AT_FIRST: usize = 8192;
pub const RETAINED_LIGHTS_AT_FIRST: usize = 1024;
/// The flag of an instance record: the helper draws the instance from its table.
pub const INSTANCE_BLOCK: u32 = 0x100;
/// The bits of `light_range.w` of the record of a retained instance: the world matrix from
/// the instance, the values of the view from the scene, the lights through the indexes.
const RETAINED_RECORD_BITS: u32 = 2 | 4 | 8;

/// The values of a view for the retained draws: `Scene` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SceneData {
    pub view: [f32; 16],
    pub projection: [f32; 16],
    pub fog_color: [f32; 4],
    pub fog: [f32; 4],
    pub scene_ambient: [f32; 4],
    pub pixel: [f32; 4],
    pub clip_plane: [f32; 4],
    /// The sun of the view for the shade of the extension, and the bits of `counts.w` the
    /// extension sets for a draw of the frame (`Extension::shade`).
    pub extension5: [f32; 4],
    pub extension7: [f32; 4],
    pub counts: [[u32; 4]; 2],
    /// The origin of the set in the world: the positions of its draws are relative to it.
    /// The stage of a texture effect that takes the position adds it (`stage_coordinate`).
    pub origin: [f32; 4],
    pub padding: [u32; 56],
}
const _: () = assert!(core::mem::size_of::<SceneData>() == SCENE_SIZE);

/// The buffers of the retained draws: the scenes, the light indexes, the lights.
pub fn scene_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some("scenes"), size: (SCENE_SLOTS * SCENE_SIZE) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

pub fn light_index_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
    storage_buffer(device, "retained light indexes", count, 4)
}

pub fn retained_light_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
    storage_buffer(device, "retained lights", count, core::mem::size_of::<LightData>())
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct BucketKey {
    class: u32,
    key: PipelineKey,
    stencil: u32,
    vertices: usize,
    indexes: usize,
    textures: [u32; STAGES],
    samplers: [[u32; 5]; STAGES],
}

struct Bucket {
    /// The class of the object of the instances (MGE_LINK_CLASS_*): a view draws the
    /// classes it has.
    class: u32,
    key: PipelineKey,
    stencil: u32,
    vertices: Rc<wgpu::Buffer>,
    indexes: Rc<wgpu::Buffer>,
    slots: Vec<u32>,
}

/// One pass of one instance: a record and an instance of the segments.
struct Slot {
    #[allow(dead_code)]
    id: u32,
    /// The handles, the class, the flags and the colours the record was made from.
    geometry: u32,
    material: u32,
    class: u32,
    flags: u32,
    colours: [f32; 14],
    /// The exact translation of the instance: the record's for a relative record, the
    /// stored one as f64 for the others. `world` has it less the origin of the set.
    exact: [f64; 3],
    world: [f32; 16],
    bound: [f32; 4],
    bucket: u32,
    data: DrawData,
    /// The first index and the number of indexes of each draw of the geometry (one for a
    /// triangle list, one for each strip).
    ranges: Vec<(u32, u32)>,
    base_vertex: i32,
    visible: bool,
    /// The draw blends and the engine would sort it: the table draws it back to front with
    /// the other sorted blends where the engine flushes its queue (plan step 6).
    sorted: bool,
    /// The texture handles the slot pinned (`Renderer::pin_texture`), one use each: their
    /// uses go when the slot goes.
    pinned: [u32; STAGES],
}

#[derive(Default)]
pub struct RetainedCounts {
    pub instances: u32,
    /// The sorted blends drawn, in all views.
    pub sorted: u64,
    pub slots: u32,
    pub buckets: u32,
    pub drawn: u64,
    pub culled: u64,
    pub hidden: u64,
    pub calls: u64,
    pub views: u64,
    pub no_geometry: u32,
    pub no_material: u32,
    /// The instance records that moved an instance in place.
    pub moves: u64,
    /// The times the origin of the set moved to the camera.
    pub rebases: u32,
    /// The slots made again for a geometry sent again under its handle (a live shape).
    pub refreshed: u64,
}

/// The origin of the set moves to the camera when the camera is farther from it than this,
/// in units of the world. A 32-bit float has steps of 0.0005 units at 4096.
const REBASE_DISTANCE: f64 = 4096.0;

pub struct RetainedSet {
    slots: Vec<Option<Slot>>,
    free: Vec<u32>,
    by_id: HashMap<u32, Vec<u32>>,
    buckets: Vec<Bucket>,
    bucket_of: HashMap<BucketKey, u32>,
    /// The lights the instances use: the slot of each key in the retained light buffer,
    /// their values, and the slots whose values changed.
    light_slot_of: HashMap<u32, u32>,
    light_values: Vec<LightData>,
    light_dirty: Vec<u32>,
    /// The runs of light indexes of the slots: `light_indexes[first..first + count]` are
    /// slots of `light_values`.
    light_indexes: Vec<u32>,
    light_indexes_live: usize,
    light_indexes_dirty: bool,
    /// The keys of the lights of each instance, for the runs after a compaction.
    keys_of: HashMap<u32, Vec<u32>>,
    /// The slots whose record changed (and so their instance), the slots that only moved
    /// (their instance: the shader reads the world of a retained draw from it), or all.
    dirty: Vec<u32>,
    moved: Vec<u32>,
    all_dirty: bool,
    /// The slots that draw a stage from a slot of the frame in the frame they were made,
    /// with the pinned texture and sampler slots of each stage for the next frame
    /// (`Player::retained_settle_late`).
    late: Vec<(u32, [[u32; 2]; STAGES])>,
    /// The generations of the record and instance buffers the segments were written into.
    record_generation: u32,
    instance_generation: u32,
    /// The draws of a view in their order (`draw_retained`): the key of the bucket, the
    /// bucket, the key of the draw in it, the slot. Kept between the views so that a view
    /// does not allocate.
    order: Vec<(u64, u32, u32, u32)>,
    /// The nearest depth and the group of each bucket in the view in progress.
    bucket_depth: Vec<(f32, u8)>,
    /// The sorted blends of the view in progress, back to front; and those of the main view
    /// kept for the flush of the engine's queue (`Sorted::Keep`), with the view.
    sorted_order: Vec<(u64, u32, u32, u32)>,
    kept_sorted: Vec<(u64, u32, u32, u32)>,
    pub sorted_view: Option<RetainedView>,
    /// The origin of the space of the instances: the translation of a slot and the
    /// position of a light are relative to it. With a relative scene (the packet has the
    /// exact position of the camera) the origin follows the camera (`rebase`), so the
    /// 32-bit translations stay small. Without one the origin is 0: the space of the world.
    origin: [f64; 3],
    relative: bool,
    /// The view of the game for the world of the frame, in the space of the world: the
    /// planes of the cull come from it.
    pub main_view: [f32; 16],
    pub counts: RetainedCounts,
}

impl RetainedSet {
    pub fn new() -> RetainedSet {
        RetainedSet {
            slots: Vec::new(),
            free: Vec::new(),
            by_id: HashMap::new(),
            buckets: Vec::new(),
            bucket_of: HashMap::new(),
            order: Vec::new(),
            bucket_depth: Vec::new(),
            sorted_order: Vec::new(),
            kept_sorted: Vec::new(),
            sorted_view: None,
            origin: [0.0; 3],
            relative: false,
            main_view: [0.0; 16],
            light_slot_of: HashMap::new(),
            light_values: Vec::new(),
            light_dirty: Vec::new(),
            light_indexes: Vec::new(),
            light_indexes_live: 0,
            light_indexes_dirty: false,
            keys_of: HashMap::new(),
            dirty: Vec::new(),
            moved: Vec::new(),
            all_dirty: false,
            late: Vec::new(),
            record_generation: 0,
            instance_generation: 0,
            counts: RetainedCounts::default(),
        }
    }

    /// The most indirect arguments one view of the set can take.
    pub fn most_args(&self) -> usize {
        self.slots.iter().flatten().map(|slot| slot.ranges.len()).sum()
    }

    fn take_slot(&mut self) -> u32 {
        if let Some(slot) = self.free.pop() {
            return slot;
        }
        self.slots.push(None);
        (self.slots.len() - 1) as u32
    }

    /// Removes the instance: its slots go to the free list, and the GPU keeps their stale
    /// records (no draw names them). Returns the texture handles its slots pinned, for
    /// `Renderer::unpin_texture` (`Player::retained_remove`).
    #[must_use]
    pub fn remove(&mut self, id: u32) -> Vec<u32> {
        let mut pinned = Vec::new();
        let Some(slots) = self.by_id.remove(&id) else { return pinned };
        for slot in slots {
            if let Some(taken) = self.slots[slot as usize].take() {
                let bucket = &mut self.buckets[taken.bucket as usize];
                bucket.slots.retain(|&s| s != slot);
                self.light_indexes_live = self.light_indexes_live.saturating_sub(self.keys_of.get(&id).map_or(0, |keys| keys.len()));
                pinned.extend(taken.pinned.iter().copied().filter(|&h| h != 0));
            }
            self.free.push(slot);
        }
        self.keys_of.remove(&id);
        self.counts.instances = self.by_id.len() as u32;
        self.counts.slots = self.slots.iter().flatten().count() as u32;
        pinned
    }

    /// Moves the instance: a new world matrix and bound for its slots, when its draw stays
    /// the same (the handles, the class, the flags). False when the instance is not in the
    /// table or its draw changed: then it is made again.
    pub fn update_place(&mut self, id: u32, r: &Retained) -> bool {
        let Some(slots) = self.by_id.get(&id) else { return false };
        let same = slots.iter().all(|&slot| self.slots[slot as usize].as_ref().is_some_and(|s| s.geometry == r.geometry && s.material == r.material && s.class == r.class && s.flags == r.flags && s.colours == r.colours));
        if !same {
            return false;
        }
        let world = self.world_of(r);
        let exact = r.exact_translation();
        for &slot in slots {
            if let Some(s) = self.slots[slot as usize].as_mut() {
                s.world = world;
                s.exact = exact;
                s.bound = r.bound;
                s.data.world[0] = world;
            }
            self.moved.push(slot);
        }
        self.counts.moves += 1;
        true
    }

    /// The origin that the positions of the lights of the set are relative to.
    fn light_origin(&self) -> Option<[f64; 3]> {
        self.relative.then_some(self.origin)
    }

    /// The world matrix of a record in the space of the set.
    pub fn world_of(&self, r: &Retained) -> [f32; 16] {
        r.world_at(&self.origin)
    }

    /// The origin of the set follows the camera of the world: `camera` is the exact
    /// position of the camera of a relative scene, or `None` for a scene that is not
    /// relative. When the scene changes kind, or the camera is farther than
    /// `REBASE_DISTANCE` from the origin, every slot and every light is made again for the
    /// new origin, and the segments are written whole. Returns true then.
    pub fn rebase(&mut self, camera: Option<[f64; 3]>, known: &HashMap<u32, [u8; 104]>) -> bool {
        let (relative, target) = match camera {
            Some(camera) => (true, camera),
            None => (false, [0.0; 3]),
        };
        let distance = (0..3).map(|i| (target[i] - self.origin[i]).powi(2)).sum::<f64>().sqrt();
        if relative == self.relative && (!relative || distance <= REBASE_DISTANCE) {
            return false;
        }
        self.relative = relative;
        self.origin = target;
        for slot in self.slots.iter_mut().flatten() {
            for i in 0..3 {
                slot.world[12 + i] = (slot.exact[i] - target[i]) as f32;
            }
            slot.data.world[0] = slot.world;
        }
        let origin = self.light_origin();
        for (key, &slot) in &self.light_slot_of {
            if let Some(values) = known.get(key) {
                self.light_values[slot as usize] = light_data(values, origin);
            }
        }
        self.all_dirty = true;
        self.counts.rebases += 1;
        true
    }

    /// The view of a scene for the instances: the view of the game, with its translation
    /// made for the origin of the set. `camera` is the exact position of the camera (the
    /// origin of the scene), or `None` for a scene that is not relative.
    pub fn scene_view(&self, view: &[f32; 16], camera: Option<[f64; 3]>) -> [f32; 16] {
        let Some(camera) = camera.filter(|_| self.relative) else { return *view };
        let d = [self.origin[0] - camera[0], self.origin[1] - camera[1], self.origin[2] - camera[2]];
        let mut out = *view;
        for c in 0..3 {
            out[12 + c] = (d[0] * view[c] as f64 + d[1] * view[4 + c] as f64 + d[2] * view[8 + c] as f64) as f32;
        }
        out
    }

    /// The view of a replayed scene for the instances. `base` is the view of the main scene
    /// (the space of the set); `given` is the view of the replay in the space of the world,
    /// when it has one; `world` is a change in the space of the world (a reflection), when
    /// there is one.
    pub fn replay_view(&self, base: &[f32; 16], given: Option<[f32; 16]>, world: Option<[f64; 16]>) -> [f32; 16] {
        let o = self.origin;
        let mut v: [f64; 16] = match given {
            Some(m) => {
                let mut m = m.map(f64::from);
                for c in 0..3 {
                    m[12 + c] += o[0] * m[c] + o[1] * m[4 + c] + o[2] * m[8 + c];
                }
                m
            }
            None => base.map(f64::from),
        };
        if let Some(w) = world {
            let mut to = [0.0f64; 16];
            let mut from = [0.0f64; 16];
            for i in 0..4 {
                to[i * 5] = 1.0;
                from[i * 5] = 1.0;
            }
            for axis in 0..3 {
                to[12 + axis] = o[axis];
                from[12 + axis] = -o[axis];
            }
            v = multiply64(&multiply64(&multiply64(&to, &w), &from), &v);
        }
        v.map(|x| x as f32)
    }

    /// A plane of the space of the world in the space of the set.
    pub fn plane_of(&self, plane: [f64; 4]) -> [f32; 4] {
        let o = self.origin;
        [plane[0] as f32, plane[1] as f32, plane[2] as f32, (plane[3] + plane[0] * o[0] + plane[1] * o[1] + plane[2] * o[2]) as f32]
    }

    /// The lights of each slot of a geometry as the table resolves them: the run of the
    /// slot and the shaded values behind the indexes (`MGE_XE_DRAW_DUMP`).
    pub fn dump_lights(&self, geometry: u32, lights: &[LightData]) {
        for slot in self.slots.iter().flatten() {
            if slot.geometry != geometry {
                continue;
            }
            let (first, count) = (slot.data.light_range[0] as usize, slot.data.light_range[1] as usize);
            let indexes: Vec<u32> = self.light_indexes[first..(first + count).min(self.light_indexes.len())].to_vec();
            println!("light dump retained table slot {} id {}: light_range {:?} counts {:?} indexes {:?} origin {:?} relative {}", slot.bucket, slot.id, slot.data.light_range, slot.data.counts, indexes, self.origin, self.relative);
            let values: Vec<LightData> = indexes.iter().filter_map(|&i| lights.get(i as usize).copied()).collect();
            crate::light_dump("retained table", &values);
        }
    }

    pub fn set_visible(&mut self, id: u32, visible: bool) {
        if let Some(slots) = self.by_id.get(&id) {
            for &slot in slots {
                if let Some(s) = self.slots[slot as usize].as_mut() {
                    s.visible = visible;
                }
            }
        }
    }

    /// The light of the game with this key changed: its retained value, when the instances
    /// use it.
    pub fn light_changed(&mut self, key: u32, values: &[u8]) {
        if let Some(&slot) = self.light_slot_of.get(&key) {
            // In the space of the set, as the slot was made (a flickering light moved to its
            // absolute position before, 2026-10-16).
            let origin = self.light_origin();
            self.light_values[slot as usize] = light_data(values, origin);
            self.light_dirty.push(slot);
        }
    }

    fn light_slot(&mut self, key: u32, known: &HashMap<u32, [u8; 104]>) -> Option<u32> {
        if let Some(&slot) = self.light_slot_of.get(&key) {
            return Some(slot);
        }
        let values = known.get(&key)?;
        let slot = self.light_values.len() as u32;
        let origin = self.light_origin();
        self.light_values.push(light_data(values, origin));
        self.light_slot_of.insert(key, slot);
        self.light_dirty.push(slot);
        Some(slot)
    }

    /// The lights of the instance: a run of indexes for its slots.
    pub fn set_lights(&mut self, id: u32, keys: &[u32], known: &HashMap<u32, [u8; 104]>) {
        if !self.by_id.contains_key(&id) {
            return;
        }
        let old = self.keys_of.insert(id, keys.to_vec()).map_or(0, |keys| keys.len());
        self.light_indexes_live = self.light_indexes_live + keys.len() - old;
        let first = self.light_indexes.len() as u32;
        let mut count = 0;
        for &key in keys {
            if let Some(slot) = self.light_slot(key, known) {
                self.light_indexes.push(slot);
                count += 1;
            }
        }
        self.light_indexes_dirty = true;
        let slots = self.by_id.get(&id).cloned().unwrap_or_default();
        for slot in slots {
            if let Some(s) = self.slots[slot as usize].as_mut() {
                s.data.light_range[0] = first;
                s.data.light_range[1] = count;
                s.data.counts[0] = count;
                self.dirty.push(slot);
            }
        }
        // The runs that went away stay in the list until it is twice the live runs: then
        // all runs are made again.
        if self.light_indexes.len() > 2 * self.light_indexes_live + 1024 {
            self.compact_light_runs(known);
        }
    }

    fn compact_light_runs(&mut self, known: &HashMap<u32, [u8; 104]>) {
        let ids: Vec<u32> = self.keys_of.keys().copied().collect();
        self.light_indexes.clear();
        self.light_indexes_live = 0;
        for id in ids {
            let keys = self.keys_of.get(&id).cloned().unwrap_or_default();
            self.light_indexes_live += keys.len();
            let first = self.light_indexes.len() as u32;
            let mut count = 0;
            for key in keys {
                if let Some(slot) = self.light_slot(key, known) {
                    self.light_indexes.push(slot);
                    count += 1;
                }
            }
            for slot in self.by_id.get(&id).cloned().unwrap_or_default() {
                if let Some(s) = self.slots[slot as usize].as_mut() {
                    s.data.light_range[0] = first;
                    s.data.light_range[1] = count;
                    s.data.counts[0] = count;
                }
            }
        }
        self.all_dirty = true;
        self.light_indexes_dirty = true;
    }
}

/// The planes of a view from its combined matrix (the layout of Direct3D, a row vector
/// times the matrix): left, right, bottom, top, near, far, each `(n, d)` with `n` of
/// length 1 and the inside where `n . p + d >= 0`.
pub fn frustum_planes(m: &[f32; 16]) -> [[f32; 4]; 6] {
    let col = |c: usize| [m[c], m[4 + c], m[8 + c], m[12 + c]];
    let (c0, c1, c2, c3) = (col(0), col(1), col(2), col(3));
    let add = |a: [f32; 4], b: [f32; 4], sign: f32| [a[0] + sign * b[0], a[1] + sign * b[1], a[2] + sign * b[2], a[3] + sign * b[3]];
    let raw = [add(c3, c0, 1.0), add(c3, c0, -1.0), add(c3, c1, 1.0), add(c3, c1, -1.0), c2, add(c3, c2, -1.0)];
    raw.map(|p| {
        let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-20);
        [p[0] / length, p[1] / length, p[2] / length, p[3] / length]
    })
}

/// `a * b` for two matrices with the layout of Direct3D.
pub fn multiply32(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    core::array::from_fn(|i| (0..4).map(|k| a[i / 4 * 4 + k] * b[k * 4 + i % 4]).sum())
}

fn in_view(bound: &[f32; 4], planes: &[[f32; 4]; 6]) -> bool {
    planes.iter().all(|p| p[0] * bound[0] + p[1] * bound[1] + p[2] * bound[2] + p[3] >= -bound[3])
}

/// A light as the shader has it, from the `D3DLIGHT8` of 26 words of its record (after
/// the key). `origin`: the position is made relative to it.
/// Engine-rules R12 (the skill wgpu-renderer-dev).
pub fn light_data(l: &[u8], origin: Option<[f64; 3]>) -> LightData {
    let mut position: [f32; 3] = floats(l, 13 * 4);
    if let Some(origin) = origin {
        for axis in 0..3 {
            position[axis] = (position[axis] as f64 - origin[axis]) as f32;
        }
    }
    let direction: [f32; 3] = floats(l, 16 * 4);
    let attenuation: [f32; 3] = floats(l, 21 * 4);
    LightData {
        diffuse: floats(l, 4),
        ambient: floats(l, 9 * 4),
        position: [position[0], position[1], position[2], word(l, 0) as f32],
        direction: [direction[0], direction[1], direction[2], float(l, 19 * 4)],
        attenuation: [attenuation[0], attenuation[1], attenuation[2], float(l, 20 * 4)],
        // The plugin puts the radius of a light of the game into its first specular word.
        cone: [(float(l, 24 * 4) * 0.5).cos(), (float(l, 25 * 4) * 0.5).cos(), float(l, 5 * 4), 0.0],
    }
}

/// What a view does with the sorted blends of the table (plan step 6).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sorted {
    /// Draws them after the rest, back to front: a reflection, a shadow map.
    Together,
    /// Draws the rest and keeps them for `Only`: the main view at the start of the world.
    Keep,
    /// Draws the kept blends: at the point where the engine flushes its sorted queue.
    Only,
}

/// How a view changes the keys and the matrices of the retained draws.
#[derive(Clone, Copy)]
pub struct RetainedView {
    pub scene_slot: u32,
    /// The classes the view draws, one bit for each.
    pub classes: u32,
    pub planes: [[f32; 4]; 6],
    pub flip_cull: bool,
    pub force_depth_test: bool,
    pub target: Option<u32>,
    pub sorted: Sorted,
}

/// A texture with a slot of the arrays that stays from frame to frame (`Renderer::pin_texture`):
/// its slot is its index plus 1. `uses` counts the stages of the retained slots that name it;
/// a free entry has the handle 0 and the white view.
pub struct PinnedTexture {
    pub handle: u32,
    pub view: Rc<wgpu::TextureView>,
    pub uses: u32,
}

impl Renderer {
    /// A slot of the arrays of the frame that a texture keeps from frame to frame, for the
    /// records of the retained instances: one more use of it. 0 for no texture, a texture
    /// not known, or no room. The lowest free slot is taken before the room grows, so that
    /// the free slots gather at the end, where `unpin_texture` drops them. The pinned slots
    /// and the frame's own textures share the room; `FRAME_RESERVE` slots stay the frame's.
    pub fn pin_texture(&mut self, handle: u32) -> u32 {
        if handle == 0 {
            return 0;
        }
        if let Some(&index) = self.pinned_index.get(&handle) {
            self.pinned_textures[index].uses += 1;
            return index as u32 + 1;
        }
        let Some(view) = self.textures.get(&handle).cloned() else { return 0 };
        // `pinned_free` goes from the highest slot to the lowest: `pop` takes the lowest.
        let index = if let Some(index) = self.pinned_free.pop() {
            self.pinned_textures[index] = PinnedTexture { handle, view: view.clone(), uses: 1 };
            index
        } else {
            if self.pinned_textures.len() + 1 >= self.array_cap.saturating_sub(crate::FRAME_RESERVE) {
                if self.counts.pinned_over == 0 {
                    println!(
                        "texture slots: the room of {} is full ({} kept, {} for the frame's own draws); textures past it draw white: raise [runtime] renderer_texture_slots of mgeXE.toml",
                        self.array_cap,
                        self.pinned_textures.len(),
                        crate::FRAME_RESERVE
                    );
                }
                self.counts.over_array_cap += 1;
                self.counts.pinned_over += 1;
                return 0;
            }
            self.pinned_textures.push(PinnedTexture { handle, view: view.clone(), uses: 1 });
            self.pinned_textures.len() - 1
        };
        // The arrays of the frame in progress hold the pinned slots of its start at their
        // start: a slot among them taken again gets its new view there too. A slot past them
        // is in the arrays from the next frame on: in this frame the frame's own textures
        // hold that place (`texture_slot_in_frame`, `Player::retained_settle_late`).
        let slot = index + 1;
        if index < self.pinned_in_frame && slot < self.frame_texture_views.len() {
            self.frame_texture_views[slot] = view;
            self.frame_texture_slots.entry(handle).or_insert(slot as u32);
        }
        self.pinned_index.insert(handle, index);
        self.counts.pinned_most = self.counts.pinned_most.max(self.pinned_textures.len() as u32);
        self.array_group = None;
        self.array_variants.clear();
        slot as u32
    }

    /// One use less of a pinned texture (a retained slot that named it went). At no use the
    /// slot is free: the white view in its place, and no handle names it. The free slots at
    /// the end of the room go: the room shrinks.
    pub fn unpin_texture(&mut self, handle: u32) {
        if handle == 0 {
            return;
        }
        let Some(&index) = self.pinned_index.get(&handle) else { return };
        let pinned = &mut self.pinned_textures[index];
        pinned.uses = pinned.uses.saturating_sub(1);
        if pinned.uses != 0 {
            return;
        }
        pinned.handle = 0;
        pinned.view = self.white.clone();
        self.pinned_index.remove(&handle);
        let at = self.pinned_free.partition_point(|&free| free > index);
        self.pinned_free.insert(at, index);
        if self.frame_texture_slots.get(&handle) == Some(&(index as u32 + 1)) {
            self.frame_texture_slots.remove(&handle);
        }
        self.counts.textures_unpinned += 1;
        while self.pinned_textures.last().is_some_and(|p| p.handle == 0) {
            let last = self.pinned_textures.len() - 1;
            self.pinned_textures.pop();
            // The highest free slot is the first of the list.
            if self.pinned_free.first() == Some(&last) {
                self.pinned_free.remove(0);
            }
            self.counts.pinned_dropped += 1;
        }
    }

    /// A slot of the sampler array of the frame that a sampler state keeps.
    pub fn pin_sampler(&mut self, values: [u32; 5]) -> u32 {
        if let Some(slot) = self.pinned_samplers.iter().position(|(v, _)| *v == values) {
            return slot as u32 + 1;
        }
        if self.pinned_samplers.len() + 1 >= self.sampler_cap / 2 {
            return 0;
        }
        let sampler = self.sampler(values);
        self.pinned_samplers.push((values, sampler));
        self.array_group = None;
        self.array_variants.clear();
        self.pinned_samplers.len() as u32
    }

    /// A slot of `pin_texture` is in the arrays of the frame in progress. A slot that grew
    /// the room after `pin_into_frame` is not.
    pub fn texture_slot_in_frame(&self, slot: u32) -> bool {
        (slot as usize) <= self.pinned_in_frame
    }

    /// A slot of `pin_sampler` is in the sampler array of the frame in progress.
    pub fn sampler_slot_in_frame(&self, slot: u32) -> bool {
        (slot as usize) <= self.pinned_samplers_in_frame
    }

    /// The pinned textures and samplers at the start of the arrays of a frame, in their
    /// slots. After `begin_frame_arrays` cleared the arrays.
    pub fn pin_into_frame(&mut self) {
        self.pinned_in_frame = self.pinned_textures.len();
        self.pinned_samplers_in_frame = self.pinned_samplers.len();
        for pinned in &self.pinned_textures {
            let slot = self.frame_texture_views.len() as u32;
            self.frame_texture_views.push(pinned.view.clone());
            if pinned.handle != 0 {
                self.frame_texture_slots.insert(pinned.handle, slot);
            }
        }
        for (values, sampler) in &self.pinned_samplers {
            let slot = self.frame_samplers.len() as u32;
            self.frame_samplers.push(sampler.clone());
            self.frame_sampler_slots.insert(*values, slot);
        }
    }

    /// Room in the segments of the records and the instances for `slots` slots: larger
    /// buffers when short, with the frame's part after the segment. Between frames only.
    fn retained_room(&mut self, slots: usize) {
        if slots <= self.retained_records {
            return;
        }
        let frame_records = self.record_capacity - self.retained_records;
        // At most the device's limit of a storage binding with the frame's part: the slots
        // past it are not uploaded nor drawn (counted, and said once).
        let segment = slots.next_power_of_two().min(self.record_limit.saturating_sub(frame_records));
        if slots > segment {
            if self.counts.retained_over_limit == 0 {
                println!("draw records: {slots} retained slots, room for {segment}; the slots past it are not drawn");
            }
            self.counts.retained_over_limit = (slots - segment) as u32;
        }
        if segment <= self.retained_records {
            return;
        }
        self.retained_records = segment;
        self.record_capacity = segment + frame_records;
        self.record_buffer = Rc::new(record_buffer(&self.device, self.record_capacity));
        self.record_written = 0;
        self.record_extras = 0;
        self.record_generation = self.record_generation.wrapping_add(1);
        let frame_instances = self.instance_capacity - self.retained_instances;
        self.retained_instances = segment;
        self.instance_capacity = segment + frame_instances;
        self.instance_buffer = instance_buffer(&self.device, self.instance_capacity);
        self.defaults = defaults_buffer(&self.device, self.instance_capacity);
        self.instance_extras = 0;
        self.instance_generation = self.instance_generation.wrapping_add(1);
    }

    /// Writes the segments of the retained instances that changed, before the uploads of
    /// the frame. A new buffer gets the whole segment.
    pub fn retained_upload(&mut self, set: &mut RetainedSet) {
        self.retained_room(set.slots.len());
        let all = set.all_dirty || self.record_generation != set.record_generation || self.instance_generation != set.instance_generation;
        set.record_generation = self.record_generation;
        set.instance_generation = self.instance_generation;
        set.all_dirty = false;
        // The slots that have room (all of them but past the device's limit).
        let room = set.slots.len().min(self.retained_records);
        set.dirty.retain(|&i| (i as usize) < room);
        set.moved.retain(|&i| (i as usize) < room);
        if all {
            let mut records = vec![0u8; room * RECORD_SIZE];
            let mut instances = vec![0u8; room * INSTANCE_SIZE];
            for (i, slot) in set.slots.iter().take(room).enumerate() {
                if let Some(slot) = slot {
                    records[i * RECORD_SIZE..(i + 1) * RECORD_SIZE].copy_from_slice(bytemuck::bytes_of(&slot.data));
                    instances[i * INSTANCE_SIZE..(i + 1) * INSTANCE_SIZE].copy_from_slice(bytemuck::bytes_of(&Instance { world: slot.world, record: [i as u32, 0, 0, 0] }));
                }
            }
            if !records.is_empty() {
                self.queue.write_buffer(&self.record_buffer, 0, &records);
                self.queue.write_buffer(&self.instance_buffer, 0, &instances);
            }
            set.dirty.clear();
            set.moved.clear();
        } else {
            set.dirty.sort_unstable();
            set.dirty.dedup();
            for &i in &set.dirty {
                if let Some(slot) = set.slots[i as usize].as_ref() {
                    self.queue.write_buffer(&self.record_buffer, (i as usize * RECORD_SIZE) as u64, bytemuck::bytes_of(&slot.data));
                }
            }
            // The instances of the changed and the moved slots: one write of the segment when
            // there are many (an animated scene moves hundreds a frame), else one each.
            set.moved.sort_unstable();
            set.moved.dedup();
            let instance_of = |i: usize, slot: &Slot| Instance { world: slot.world, record: [i as u32, 0, 0, 0] };
            if set.dirty.len() + set.moved.len() > 64 {
                let mut instances = vec![0u8; room * INSTANCE_SIZE];
                for (i, slot) in set.slots.iter().take(room).enumerate() {
                    if let Some(slot) = slot {
                        instances[i * INSTANCE_SIZE..(i + 1) * INSTANCE_SIZE].copy_from_slice(bytemuck::bytes_of(&instance_of(i, slot)));
                    }
                }
                if !instances.is_empty() {
                    self.queue.write_buffer(&self.instance_buffer, 0, &instances);
                }
            } else {
                for &i in set.dirty.iter().chain(set.moved.iter()) {
                    if let Some(slot) = set.slots[i as usize].as_ref() {
                        self.queue.write_buffer(&self.instance_buffer, (i as usize * INSTANCE_SIZE) as u64, bytemuck::bytes_of(&instance_of(i as usize, slot)));
                    }
                }
            }
            set.dirty.clear();
            set.moved.clear();
        }
        // Room for the lights (their values go up at the start of the world, shaded by the
        // extension as the lights of a draw) and their indexes.
        if set.light_values.len() > self.retained_light_capacity {
            self.retained_light_capacity = set.light_values.len().next_power_of_two();
            self.retained_light_buffer = retained_light_buffer(&self.device, self.retained_light_capacity);
        }
        set.light_dirty.clear();
        if set.light_indexes_dirty {
            if set.light_indexes.len() > self.light_index_capacity {
                self.light_index_capacity = set.light_indexes.len().next_power_of_two();
                self.light_index_buffer = light_index_buffer(&self.device, self.light_index_capacity);
            }
            if !set.light_indexes.is_empty() {
                self.queue.write_buffer(&self.light_index_buffer, 0, bytemuck::cast_slice(&set.light_indexes));
            }
            set.light_indexes_dirty = false;
        }
        set.counts.buckets = set.buckets.len() as u32;
    }

    /// Writes the lights of the retained instances, as the shade of the extension made them
    /// for the frame.
    pub fn upload_retained_lights(&self, lights: &[LightData]) {
        if !lights.is_empty() && lights.len() <= self.retained_light_capacity {
            self.queue.write_buffer(&self.retained_light_buffer, 0, bytemuck::cast_slice(lights));
        }
    }

    /// Writes the values of a view into a scene slot.
    pub fn write_scene(&self, slot: u32, scene: &SceneData) {
        self.queue.write_buffer(&self.scene_buffer, (slot as usize * SCENE_SIZE) as u64, bytemuck::bytes_of(scene));
    }
}

/// Draws the instances of the set in view, one indirect call for each run of draws of one
/// bucket. The pass state is not known after it: the caller forgets its bound groups. With
/// `Sorted::Only` the view draws the sorted blends kept by the main view and nothing else.
pub fn draw_retained(pass: &mut wgpu::RenderPass<'static>, renderer: &mut Renderer, set: &mut RetainedSet, args: &mut Vec<IndirectArgs>, view: &RetainedView) {
    if set.by_id.is_empty() {
        return;
    }
    set.counts.views += 1;
    let mut arrays_bound = false;
    let mut shared_bound = false;
    let mut frame_bound = false;
    let order = if view.sorted == Sorted::Only {
        core::mem::take(&mut set.kept_sorted)
    } else {
        set.collect_order(view);
        core::mem::take(&mut set.order)
    };
    let mut at = 0;
    while at < order.len() {
        let b = order[at].1 as usize;
        let start = args.len();
        while at < order.len() && order[at].1 as usize == b {
            let slot = order[at].3;
            at += 1;
            // A slot past the device's limit has no record (`retained_room`).
            if slot as usize >= renderer.retained_records {
                continue;
            }
            let Some(s) = set.slots[slot as usize].as_ref() else { continue };
            for &(first, count) in &s.ranges {
                args.push(IndirectArgs { index_count: count, instance_count: 1, first_index: first, base_vertex: s.base_vertex, first_instance: slot });
            }
        }
        let count = args.len() - start;
        if count == 0 {
            continue;
        }
        let bucket = &set.buckets[b];
        let mut key = bucket.key;
        if view.force_depth_test {
            key.depth = [1, key.depth[1], 4];
        }
        if view.flip_cull {
            key.cull = match key.cull {
                2 => 3,
                3 => 2,
                other => other,
            };
        }
        let Some(pipeline) = renderer.pipeline(key) else {
            args.truncate(start);
            continue;
        };
        pass.set_pipeline(&pipeline);
        pass.set_stencil_reference(bucket.stencil);
        if !frame_bound {
            pass.set_bind_group(0, &*renderer.frame_group, &[view.scene_slot * SCENE_SIZE as u32]);
            frame_bound = true;
        }
        if !arrays_bound {
            let group = renderer.array_group_for(view.target);
            pass.set_bind_group(1, &*group, &[]);
            arrays_bound = true;
        }
        pass.set_vertex_buffer(0, bucket.vertices.slice(..));
        if !shared_bound {
            pass.set_vertex_buffer(1, renderer.defaults.slice(..));
            pass.set_vertex_buffer(2, renderer.instance_buffer.slice(..));
            shared_bound = true;
        }
        pass.set_index_buffer(bucket.indexes.slice(..), wgpu::IndexFormat::Uint16);
        if renderer.indirect && args.len() <= renderer.indirect_capacity {
            pass.multi_draw_indexed_indirect(&renderer.indirect_buffer, (start * ARGS_SIZE) as u64, count as u32);
            renderer.counts.indirect_calls += 1;
            renderer.counts.indirect_draws += count as u32;
        } else {
            for a in &args[start..] {
                pass.draw_indexed(a.first_index..a.first_index + a.index_count, a.base_vertex, a.first_instance..a.first_instance + 1);
            }
            args.truncate(start);
        }
        set.counts.calls += 1;
    }
    if view.sorted == Sorted::Only {
        set.kept_sorted = order;
    } else {
        set.order = order;
    }
}

impl RetainedSet {
    /// The draws of a view in the order they go to the GPU, into `order`: the modern order
    /// (the owner's decision of 2026-10-16).
    /// 1. The buckets that do not blend and have no alpha test, front to back by their
    ///    nearest draw in view, and the draws of each front to back: the early depth test
    ///    then rejects the pixels behind what is drawn.
    /// 2. The buckets with an alpha test, the same way after them (a discard in the shader
    ///    can turn the early depth test off).
    /// 3. The buckets that blend out of the sort (the no-sorter bit), in the order they were
    ///    made, their draws in the order of their slots. A shape draws a base pass and a
    ///    blend pass that both write depth and test less-or-equal, in different buckets: the
    ///    base must come first, or it overwrites the blend at the same depth (the game draws
    ///    the two in a row).
    /// The depth of a draw is that of the nearest point of its bound: the distance from the
    /// near plane of the view less the radius, at least 0.
    /// 4. The blends the engine would sort, into `sorted_order`: back to front by the depth
    ///    of the bound centre along the view, as the engine's accumulator sorts them, ties
    ///    by the order of the slots (plan step 6). A view draws them after the rest
    ///    (`Sorted::Together`) or keeps them for the flush of the engine's queue
    ///    (`Sorted::Keep`, then `Only`).
    /// Engine-rules R3, R4, R20 (the skill wgpu-renderer-dev).
    fn collect_order(&mut self, view: &RetainedView) {
        self.order.clear();
        self.sorted_order.clear();
        self.bucket_depth.clear();
        self.bucket_depth.resize(self.buckets.len(), (f32::MAX, 0));
        let near = view.planes[4];
        let mut sequence = 0u32;
        for (b, bucket) in self.buckets.iter().enumerate() {
            if view.classes & (1 << bucket.class.min(31)) == 0 {
                continue;
            }
            let blends = bucket.key.blend[0] != 0;
            for &slot in &bucket.slots {
                let Some(s) = self.slots[slot as usize].as_ref() else { continue };
                if !s.visible {
                    self.counts.hidden += 1;
                    continue;
                }
                if !in_view(&s.bound, &view.planes) {
                    self.counts.culled += 1;
                    continue;
                }
                self.counts.drawn += 1;
                let centre = near[0] * s.bound[0] + near[1] * s.bound[1] + near[2] * s.bound[2] + near[3];
                sequence += 1;
                if s.sorted {
                    self.counts.sorted += 1;
                    // The far draw first: the bits of the depth inverted; ties by the creation
                    // order of the instances, as the engine's accumulator registered them.
                    let key = (!centre.max(0.0).to_bits() as u64) << 32 | s.id as u64;
                    self.sorted_order.push((key, b as u32, sequence, slot));
                    continue;
                }
                let depth = (centre - s.bound[3]).max(0.0);
                let entry = &mut self.bucket_depth[b];
                entry.0 = entry.0.min(depth);
                if blends {
                    entry.1 = 2;
                } else if s.data.more[2] != 0 {
                    entry.1 = entry.1.max(1);
                }
                // A depth of at least 0 orders as its bits; a blended draw keeps its place.
                let within = if blends { sequence } else { depth.to_bits() };
                self.order.push((0, b as u32, within, slot));
            }
        }
        for entry in &mut self.order {
            let (depth, group) = self.bucket_depth[entry.1 as usize];
            entry.0 = if group == 2 {
                2 << 32 | entry.1 as u64
            } else {
                (group as u64) << 32 | depth.to_bits() as u64
            };
        }
        self.order.sort_unstable();
        self.sorted_order.sort_unstable();
        match view.sorted {
            Sorted::Together => self.order.append(&mut self.sorted_order),
            Sorted::Keep => {
                self.kept_sorted.clear();
                self.kept_sorted.append(&mut self.sorted_order);
                self.sorted_view = Some(*view);
            }
            Sorted::Only => {}
        }
    }
}

/// The kind of a pass for the shade bits of the scene: the blend form (0 none, 1 by the alpha,
/// 2 a source of ONE, 3 a destination of ONE: the extension's shade reads the two factors
/// and the depth write) times 2, plus 1 with a depth write. `draw_retained_main` shades a
/// dummy draw of each kind once a frame, and the shader takes the bits by this index.
/// Engine-rules R20 (the skill wgpu-renderer-dev).
pub fn slot_kind(blend: [u32; 3], depth_write: bool) -> u32 {
    let form = if blend[0] == 0 {
        0
    } else if blend[2] == 2 {
        3
    } else if blend[1] == 2 {
        2
    } else {
        1
    };
    form * 2 + depth_write as u32
}

/// The stages of the texture effects of a material for the table, as `Pass::new` takes them
/// from a shape record: the generation from a vector of the world in the high word of the
/// coordinate index (5 the normal, 6 the position, 7 the reflection, as the shader reads
/// them), the transform flags, and the texture matrix from the effect's world projection.
/// The engine's callbacks make the same stage for a camera by turning the vector of camera
/// space into one of the world first: here the shader does the turn, so the stage holds for
/// every view. The position of a retained draw is relative to the origin of the set: the
/// shader adds `scene.origin` before the matrix. `None` for a material without effects.
/// Engine-rules R7 (the skill wgpu-renderer-dev).
fn effect_stages(m: &material::Material) -> Option<[[u32; EFFECT_STAGE_WORDS]; 2]> {
    if m.environment_effect.is_none() && m.fog_map_effect.is_none() {
        return None;
    }
    let stage = |effect: Option<material::Effect>, mode: u32| -> [u32; EFFECT_STAGE_WORDS] {
        let mut words = [0u32; EFFECT_STAGE_WORDS];
        let Some(e) = effect else { return words };
        words[0] = match mode {
            0 | 1 => 0x60000,
            2 | 3 => 0x70000,
            _ => 0x50000,
        };
        words[1] = e.flags;
        // The texture matrix of Direct3D, row by row: the coordinate is a row vector times
        // the matrix, so a column of the effect's matrix is a row here, and the translation
        // is the fourth row.
        for row in 0..3 {
            for column in 0..3 {
                words[2 + row * 4 + column] = e.matrix[column * 3 + row].to_bits();
            }
        }
        for column in 0..3 {
            words[2 + 12 + column] = e.translation[column].to_bits();
        }
        words
    };
    Some([stage(m.environment_effect, m.environment_coordinates), stage(m.fog_map_effect, m.fog_map_coordinates)])
}

impl Player {
    /// Removes an instance from the table, and the uses of the textures its slots pinned.
    pub(super) fn retained_remove(&mut self, id: u32) {
        for handle in self.retained.remove(id) {
            self.renderer.unpin_texture(handle);
        }
    }

    /// The slots that drew a stage from a slot of the frame (its pinned texture or sampler
    /// came after `pin_into_frame`) get their pinned slots, which this frame's arrays now
    /// hold. After `pin_into_frame`.
    pub(super) fn retained_settle_late(&mut self) {
        let set = &mut self.retained;
        for (slot, pinned_slots) in set.late.drain(..) {
            let Some(s) = set.slots.get_mut(slot as usize).and_then(Option::as_mut) else { continue };
            for stage in 0..STAGES {
                s.data.stages[stage].slots[0] = pinned_slots[stage][0];
                s.data.stages[stage].slots[1] = pinned_slots[stage][1];
            }
            set.dirty.push(slot);
        }
    }

    /// Puts an instance record with the flag 0x100 into the table: a slot for each pass of
    /// its material, with the record of the draw made as for a shape record. An instance
    /// that is in the table already (its material or geometry handle changed) keeps its
    /// lights: the plugin sends them again only when the set of its lights changes.
    /// Engine-rules R13 (the skill wgpu-renderer-dev).
    pub(super) fn retained_insert(&mut self, id: u32, r: &Retained) {
        let keys = self.retained.keys_of.get(&id).cloned();
        self.retained_remove(id);
        let Some((fvf, stride, entries, strips, vertex, index, base_vertex, first_index)) =
            self.renderer.geometries.get(&r.geometry).map(|g| (g.fvf, g.stride, g.entries, g.strips.clone(), g.vertex.clone(), g.index.clone(), g.base_vertex, g.first_index))
        else {
            self.retained.counts.no_geometry += 1;
            return;
        };
        let Some(m) = self.renderer.materials.get(&r.material).cloned() else {
            self.retained.counts.no_material += 1;
            return;
        };
        let sets = (fvf >> 8) & 0xF;
        let derived = match self.renderer.derived.get(&(r.material, sets)) {
            Some(d) => d.clone(),
            None => {
                let d = Rc::new(material::derive(&m, sets));
                self.renderer.derived.insert((r.material, sets), d.clone());
                d
            }
        };
        // The shape in progress as the record needs it; the lights, the fog, the matrices
        // and the ambient light come from the scene of each view.
        {
            let shape = &mut self.stream.shape;
            shape.geometry = r.geometry;
            shape.fvf = fvf;
            shape.stride = stride;
            shape.world = self.retained.world_of(r);
            shape.relative = false;
            shape.material = material::d3d_material(&r.colours);
            shape.fill = derived.fill;
            shape.lights.clear();
        }
        self.stream.shape_fog = None;
        self.stream.shape_bones = None;
        self.stream.sky = None;
        self.stream.object_class = r.class;
        if let Some(source) = derived.sources[2] {
            self.stream.ambient_source = source;
        }
        let ranges: Vec<(u32, u32)> = if strips.is_empty() {
            vec![(first_index, entries / 3 * 3)]
        } else {
            let mut start = first_index;
            strips
                .iter()
                .map(|&length| {
                    let range = (start, (length as u32).max(2));
                    start += length as u32;
                    range
                })
                .collect()
        };
        let topology = if strips.is_empty() { 4 } else { 5 };
        let stencil = derived.stencil;
        let effects = effect_stages(&m);
        let mut slots = Vec::new();
        for pass in 0..derived.passes.len() {
            let p = Pass::new(&derived, pass, &m, &self.renderer.textures, effects.as_ref());
            let key = PipelineKey {
                fvf,
                stride,
                topology,
                blend: p.blend,
                depth: [(derived.depth[0] != 0) as u32, (derived.depth[1] != 0) as u32, derived.depth[2]],
                cull: derived.cull,
                fill: derived.fill,
                color_write: p.color_write & 7,
                stencil: if stencil[0] == 0 { [0; 7] } else { [1, stencil[4], stencil[5], stencil[6], stencil[1], stencil[3], !0] },
            };
            let mut dummy = Vec::new();
            let mut data = self.stream.draw_data(&derived, &p, &mut self.renderer.counts, &mut dummy);
            crate::draw_dump("retained table, before the record bits", r.geometry, &data);
            data.world[0] = self.retained.world_of(r);
            // The kind of the pass for the shade bits of the scene.
            let kind = slot_kind(p.blend, derived.depth[1] != 0);
            data.light_range = [0, 0, kind, RETAINED_RECORD_BITS | crate::flat_shading_bit(&derived)];
            data.counts[0] = 0;
            // The slot's fog: on or off (z), and by the distance (w) for a RANGE_SQ material.
            // A shape with a fog property of its own keeps its colour and range (z 2).
            data.fog = [0.0, 0.0, derived.fog as u32 as f32, derived.range_fog as u32 as f32];
            if let (true, Some((colour, start, end))) = (derived.fog, derived.own_fog) {
                data.fog_color = color(colour);
                data.fog = [start, end, 2.0, derived.range_fog as u32 as f32];
            }
            crate::draw_dump("retained table, with the record bits", r.geometry, &data);
            let mut handles = [0u32; STAGES];
            let mut sampler_values = [[0u32; 5]; STAGES];
            for stage in 0..STAGES {
                let handle = p.stages[stage].texture;
                if stage < data.counts[1] as usize && handle != 0 && self.renderer.textures.contains_key(&handle) {
                    handles[stage] = handle;
                    sampler_values[stage] = p.stages[stage].sampler;
                }
            }
            if r.geometry == crate::dump_wanted() {
                println!("key dump retained table: fvf {} stride {} topology {} blend {:?} depth {:?} cull {} fill {} color_write {} stencil {:?} reference {} textures {:?} samplers {:?} stages {} passes {} ranges {:?} base {}", key.fvf, key.stride, key.topology, key.blend, key.depth, key.cull, key.fill, key.color_write, key.stencil, stencil[2], handles, sampler_values, data.counts[1], derived.passes.len(), ranges, base_vertex);
            }
            let mut pinned = [0u32; STAGES];
            // The pinned slots of the stages, and whether one of them is not in the arrays of
            // the frame yet: then this frame draws the stage from a slot of the frame, and
            // the next frame puts the pinned slots back (`retained_settle_late`).
            let mut pinned_slots = [[0u32; 2]; STAGES];
            let mut late = false;
            for stage in 0..STAGES {
                let mut texture = self.renderer.pin_texture(handles[stage]);
                if texture != 0 {
                    pinned[stage] = handles[stage];
                }
                let mut sampler = self.renderer.pin_sampler(sampler_values[stage]);
                pinned_slots[stage] = [texture, sampler];
                if !self.renderer.texture_slot_in_frame(texture) {
                    texture = self.renderer.frame_texture_slot(handles[stage]);
                    late = true;
                    self.renderer.counts.pinned_late += 1;
                }
                if !self.renderer.sampler_slot_in_frame(sampler) {
                    sampler = self.renderer.frame_sampler_slot(sampler_values[stage]);
                    late = true;
                    self.renderer.counts.pinned_late += 1;
                }
                data.stages[stage].slots = [texture, sampler, 0, 0];
            }
            let bucket_key = BucketKey { class: r.class, key, stencil: stencil[2], vertices: Rc::as_ptr(&vertex) as usize, indexes: Rc::as_ptr(&index) as usize, textures: handles, samplers: sampler_values };
            let world = self.retained.world_of(r);
            let exact = r.exact_translation();
            let set = &mut self.retained;
            let bucket = match set.bucket_of.get(&bucket_key) {
                Some(&b) => b,
                None => {
                    set.buckets.push(Bucket { class: r.class, key, stencil: stencil[2], vertices: vertex.clone(), indexes: index.clone(), slots: Vec::new() });
                    let b = (set.buckets.len() - 1) as u32;
                    set.bucket_of.insert(bucket_key, b);
                    b
                }
            };
            let slot = set.take_slot();
            // A late entry of an earlier instance in this slot has no meaning now.
            set.late.retain(|&(s, _)| s != slot);
            if late {
                set.late.push((slot, pinned_slots));
            }
            set.slots[slot as usize] = Some(Slot { id, geometry: r.geometry, material: r.material, class: r.class, flags: r.flags, colours: r.colours, exact, world, bound: r.bound, bucket, data, ranges: ranges.clone(), base_vertex, visible: true, sorted: p.blend[0] != 0 && !derived.no_sorter, pinned });
            set.buckets[bucket as usize].slots.push(slot);
            set.dirty.push(slot);
            slots.push(slot);
        }
        let set = &mut self.retained;
        set.by_id.insert(id, slots);
        set.counts.instances = set.by_id.len() as u32;
        set.counts.slots = set.slots.iter().flatten().count() as u32;
        // The lights the instance had, when the record came again (`remove` dropped them).
        if let Some(keys) = keys {
            let known = &self.renderer.lights_known;
            self.retained.set_lights(id, &keys, known);
        }
    }

    /// The values of the world of the frame for the retained draws, at its start: the
    /// ambient light and the fog from the `RETAINED_SCENE` record of the frame (the plugin
    /// computes them for a retained shape as the engine would), else those of the stream.
    pub(super) fn retained_scene_main(&self, facts: &WorldFacts) -> SceneData {
        let (ambient, fog) = match self.retained_values {
            Some((ambient, fog)) => (color(known(ambient, 0)), fog.unwrap_or([0; 4])),
            None => ([self.world_ambient[0], self.world_ambient[1], self.world_ambient[2], 1.0], self.stream.fog),
        };
        let mut data = SceneData {
            view: self.retained.scene_view(&facts.view, facts.origin),
            projection: facts.projection,
            fog_color: color(fog[0]),
            fog: [f32::from_bits(fog[1]), f32::from_bits(fog[2]), 1.0, (fog[3] != 0) as u32 as f32],
            scene_ambient: ambient,
            pixel: [1.0 / facts.target_size.0.max(1) as f32, -1.0 / facts.target_size.1.max(1) as f32, 0.0, 0.0],
            clip_plane: [0.0; 4],
            extension5: [0.0; 4],
            extension7: [0.0; 4],
            counts: [[0; 4]; 2],
            origin: {
                let o = self.retained.light_origin().unwrap_or([0.0; 3]);
                [o[0] as f32, o[1] as f32, o[2] as f32, 0.0]
            },
            padding: [0; 56],
        };
        // The fog of the extension for the world, as a draw of the world gets it.
        let fog_range = self.parameters.as_ref().and_then(|p| p.fog_range);
        if let (Some((start, end)), true) = (fog_range, facts.projection[11] == 1.0) {
            data.fog[0] = start;
            data.fog[1] = end;
            data.fog[3] = 1.0;
        }
        data
    }

    /// Draws the retained instances in the world of the frame: after the extension drew
    /// before the world, before the draws of the game. The extension shades the lights of
    /// the instances and the values of the scene as it shades a draw of the world that
    /// writes depth and does not blend (`Extension::shade`), one time for the frame.
    /// Engine-rules R10, R20 (the skill wgpu-renderer-dev).
    pub(super) fn draw_retained_main(&mut self, facts: &WorldFacts) {
        // The near plane of the world's view, for the depth of the sort of the blends.
        self.world_near = Some(frustum_planes(&multiply32(&facts.view, &facts.projection))[4]);
        if self.retained.by_id.is_empty() || !self.retained_on {
            return;
        }
        let mut scene = self.retained_scene_main(facts);
        let mut lights = self.retained.light_values.clone();
        if self.parameters.as_ref().is_some_and(|p| p.shade) {
            if let Some(extension) = self.extension.as_mut() {
                // The shade of an opaque draw gives the lights and the values of the scene;
                // the shade of a blended and of an additive draw gives their bits (the lights
                // of those calls are copies).
                // The kinds of a slot (`slot_kind`): the four blend forms, each without and
                // with a depth write, as the shade reads them.
                let forms = [(false, 0u32, 0u32), (true, 5, 6), (true, 2, 6), (true, 5, 2)];
                for k in 0..8 {
                    let (blend, source, destination) = forms[k / 2];
                    let write = k % 2 == 1;
                    let mut data = DrawData::zeroed();
                    data.view = facts.view;
                    data.scene_ambient = scene.scene_ambient;
                    let draw_facts = DrawFacts { class: 12, blend_enable: blend, source_blend: source, destination_blend: destination, depth_write: write, specular_power: 0.0, emissive: [0.0; 4] };
                    if k == 1 {
                        extension.shade(&mut data, &mut lights, &draw_facts);
                        scene.scene_ambient = data.scene_ambient;
                        scene.extension5 = data.extension[5];
                        scene.extension7 = data.extension[7];
                        // The shade puts the fog of a draw on the distance from the camera
                        // (`fog.w`): the scene's fog takes it too, or a retained shape off the
                        // axis of the view gets less fog than the same shape on the engine's
                        // path (the far red boxes of the NIF test level, 2026-10-16).
                        scene.fog[3] = scene.fog[3].max(data.fog[3]);
                    } else {
                        let mut copies = self.retained.light_values.clone();
                        extension.shade(&mut data, &mut copies, &draw_facts);
                    }
                    scene.counts[k / 4][k % 4] = data.counts[3];
                }
            }
        }
        if crate::dump_wanted() != 0 {
            self.retained.dump_lights(crate::dump_wanted(), &lights);
        }
        self.renderer.upload_retained_lights(&lights);
        self.renderer.scene_main = scene;
        self.renderer.write_scene(0, &scene);
        if crate::dump_wanted() != 0 {
            println!("scene dump: fog_color {:?} fog {:?} ambient {:?} counts {:?} extension5 {:?} extension7 {:?}", scene.fog_color, scene.fog, scene.scene_ambient, scene.counts, scene.extension5, scene.extension7);
        }
        // The cull is in the space of the world: the bounds of the slots are the engine's.
        self.retained.main_view = facts.view;
        let planes = frustum_planes(&multiply32(&facts.view, &facts.projection));
        let Some(pass) = self.pass.as_mut() else { return };
        let view = RetainedView { scene_slot: 0, classes: !0, planes, flip_cull: false, force_depth_test: false, target: self.pass_target, sorted: Sorted::Keep };
        pass.push_debug_group("retained instances");
        draw_retained(pass, &mut self.renderer, &mut self.retained, &mut self.frame_args, &view);
        pass.pop_debug_group();
        self.forget_bound();
    }

    /// Draws the sorted blends of the frame where the engine flushes its sorted queue (plan
    /// step 6): the kept blends of the table and the blended draws of the engine's path that
    /// waited (`frame_sorted`), as one list back to front by the depth of the bound centre
    /// along the view. A run of the table's entries is one `draw_retained` call; a draw of
    /// the engine's path is issued between them.
    /// Engine-rules R21, R22 (the skill wgpu-renderer-dev).
    pub(super) fn draw_retained_sorted(&mut self, generation: u32) {
        let (deferred, later): (Vec<_>, Vec<_>) = core::mem::take(&mut self.frame_sorted).into_iter().partition(|e| e.3 == generation);
        self.frame_sorted = later;
        let view = if generation == 0 { self.retained.sorted_view.take() } else { self.retained.sorted_view };
        let kept = if generation == 0 && self.retained_on && view.is_some() { core::mem::take(&mut self.retained.kept_sorted) } else { Vec::new() };
        if crate::dump_wanted() != 0 {
            println!("sorted blends: {} kept, {} of the engine's path, pass open {}, frame {}", kept.len(), deferred.len(), self.pass.is_some(), self.renderer.counts.frames);
            for &(key, bucket, _, slot) in &kept {
                if let Some(s) = self.retained.slots[slot as usize].as_ref() {
                    println!("  kept slot {} id {} class {} bucket {} bound {:?} key {:x}", slot, s.id, s.class, bucket, s.bound, key);
                }
            }
        }
        if kept.is_empty() && deferred.is_empty() {
            return;
        }
        let near = self.world_near.unwrap_or([0.0; 4]);
        let mut deferred: Vec<(u64, Rc<Draw>)> = deferred
            .into_iter()
            .map(|(c, sequence, draw, _)| {
                let depth = (near[0] * c[0] + near[1] * c[1] + near[2] * c[2] + near[3]).max(0.0);
                ((!depth.to_bits() as u64) << 32 | sequence as u64, draw)
            })
            .collect();
        deferred.sort_unstable_by_key(|entry| entry.0);
        self.renderer.counts.merged_blends += deferred.len() as u64;
        if self.pass.is_none() {
            self.open_pass(None, None, None);
        }
        let (mut i, mut j) = (0, 0);
        while i < kept.len() || j < deferred.len() {
            let table_next = j >= deferred.len() || (i < kept.len() && kept[i].0 <= deferred[j].0);
            if table_next {
                let start = i;
                while i < kept.len() && (j >= deferred.len() || kept[i].0 <= deferred[j].0) {
                    i += 1;
                }
                let Some(mut v) = view else { break };
                v.sorted = Sorted::Only;
                v.target = self.pass_target;
                self.end_run();
                self.retained.kept_sorted = kept[start..i].to_vec();
                let Some(pass) = self.pass.as_mut() else { return };
                pass.push_debug_group("retained sorted blends");
                draw_retained(pass, &mut self.renderer, &mut self.retained, &mut self.frame_args, &v);
                pass.pop_debug_group();
                self.forget_bound();
            } else {
                let draw = deferred[j].1.clone();
                j += 1;
                self.issue(&draw);
            }
        }
        self.retained.kept_sorted = kept;
    }
}

impl Retained {
    /// The exact translation of the record: its own for a relative record (flag 1), the
    /// stored one of the engine as f64 for the others.
    pub fn exact_translation(&self) -> [f64; 3] {
        if self.flags & 1 != 0 {
            self.exact
        } else {
            self.stored.map(f64::from)
        }
    }

    /// The world matrix of the instance in a space with this origin: its rotation rows and
    /// its exact translation less the origin.
    pub fn world_at(&self, origin: &[f64; 3]) -> [f32; 16] {
        let mut world = [0f32; 16];
        world[..12].copy_from_slice(&self.rotation);
        let exact = self.exact_translation();
        for i in 0..3 {
            world[12 + i] = (exact[i] - origin[i]) as f32;
        }
        world[15] = 1.0;
        world
    }
}

impl Replayer<'_> {
    /// The retained instances in a view the extension replays, after the kept draws.
    pub(super) fn replay_retained(&mut self, pass: &mut wgpu::RenderPass<'static>, view: &ReplayView) {
        if self.retained.by_id.is_empty() || view.translucent_only || !self.retained_on {
            return;
        }
        let slot = self.renderer.scene_slots_used;
        if slot as usize >= SCENE_SLOTS {
            return;
        }
        self.renderer.scene_slots_used += 1;
        let mut scene = self.renderer.scene_main;
        // The view of the replay in the space of the world, for the cull; the same view in
        // the space of the set, for the draws.
        let mut absolute = view.view.unwrap_or(self.retained.main_view);
        if let Some(world) = view.world {
            let old: [f64; 16] = absolute.map(|v| v as f64);
            absolute = multiply64(&world, &old).map(|v| v as f32);
        }
        scene.view = self.retained.replay_view(&scene.view, view.view, view.world);
        if let Some(projection) = view.projection {
            scene.projection = projection;
        }
        scene.pixel = [1.0 / view.target_size.0.max(1) as f32, -1.0 / view.target_size.1.max(1) as f32, 0.0, 0.0];
        scene.clip_plane = view.clip_plane.map_or([0.0; 4], |p| self.retained.plane_of(p));
        self.renderer.write_scene(slot, &scene);
        let planes = frustum_planes(&multiply32(&absolute, &scene.projection));
        let retained_view = RetainedView { scene_slot: slot, classes: view.classes, planes, flip_cull: view.flip_cull, force_depth_test: view.force_depth_test, target: None, sorted: Sorted::Together };
        draw_retained(pass, self.renderer, self.retained, self.args, &retained_view);
    }
}

/// The text of the shader of the extension with the reads of the record of the draw that a
/// retained draw takes from its scene or its light indexes (`draw.view` is already
/// `draws[record].view` here) sent through the functions of `shader.wgsl`.
pub fn rewrite_extension_text(text: &str) -> String {
    let s = text
        .replace("draws[record].view", "draw_view()")
        .replace("draws[record].projection", "draw_projection()")
        .replace("draws[record].pixel", "draw_pixel()")
        .replace("draws[record].scene_ambient", "draw_scene_ambient()")
        .replace("draws[record].fog_color", "draw_fog_color()")
        .replace("draws[record].fog.", "draw_fog().")
        .replace("draws[record].clip_plane", "draw_clip_plane()")
        .replace("draws[record].counts.w", "draw_counts_w()");
    let s = rewrite_index(&s, "draws[record].extension[", "draw_extension(");
    rewrite_index(&s, "lights[", "light_at(")
}

/// Replaces each `open ... ]` (the bracket that matches) with `call ... )`.
fn rewrite_index(text: &str, open: &str, call: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(open) {
        out.push_str(&rest[..at]);
        out.push_str(call);
        let start = at + open.len();
        let bytes = rest.as_bytes();
        let mut depth = 1;
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        out.push_str(&rest[start..i.min(rest.len())]);
        out.push(')');
        rest = &rest[(i + 1).min(rest.len())..];
    }
    out.push_str(rest);
    out
}
