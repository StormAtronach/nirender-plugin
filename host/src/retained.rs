//! The retained instances of the plugin drawn from a table of the helper. An `INSTANCE`
//! record with the flag 0x100 goes into the table: its world matrix and
//! its bound, and the record of its draw ([`DrawData`]) made once from its material and its
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
/// The flag of a skinned instance (the shape record's `SKINNED`): its world is the identity,
/// and its bones come in `INSTANCE_BONES` records ([`RetainedSet::set_bones`]).
const INSTANCE_SKINNED: u32 = 4;
/// The bytes of one bone of an `INSTANCE_BONES` record: the matrix of the rotation and the
/// scale (9 floats, by columns) and the exact translation in the world (3 doubles).
pub const BONE_BYTES: usize = 9 * 4 + 3 * 8;
/// The room of the bones of the skinned instances at first, in matrices.
pub const RETAINED_BONES_AT_FIRST: usize = 1024;
/// The first bone of a slot whose bones have not come yet: the slot is not drawn.
const NO_BONES: u32 = u32::MAX;
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
    /// extension sets for a draw of the frame ([`Extension::shade`]).
    pub extension5: [f32; 4],
    pub extension7: [f32; 4],
    pub counts: [[u32; 4]; 2],
    /// The origin of the set in the world: the positions of its draws are relative to it.
    /// The stage of a texture effect that takes the position adds it (`stage_coordinate`).
    pub origin: [f32; 4],
    /// What the extension's shade makes of an ambient light ([`TableShade::ambient_scale`]),
    /// so each retained draw's own ambient ([`DrawData::scene_ambient`], from the instance's
    /// lights record) is scaled as the scene's is.
    pub ambient_scale: [f32; 4],
    /// x: 1 when the view takes the point lights of the world from the clusters (the main
    /// view, `MGE_XE_LIGHTS=world`); a reflection or a shadow map keeps the lights of the draw.
    pub world_lights: [u32; 4],
    pub padding: [u32; 48],
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
    /// the other sorted blends where the engine flushes its queue.
    sorted: bool,
    /// The shape's own alpha property blends and the engine does not sort it: the engine
    /// draws it at its display, in the order of its walk ([`engine::walk`]).
    in_place: bool,
    /// The texture handles the slot pinned ([`Renderer::pin_texture`]), one use each: their
    /// uses go when the slot goes.
    pinned: [u32; STAGES],
    /// A skinned slot: the first of the bones of its instance in the table's palette (its
    /// instance carries it to the shader), or [`NO_BONES`].
    bones_first: u32,
    /// The material is emissive only (`Derived::no_lights`): the slot takes no light of its
    /// instance and a black ambient.
    no_lights: bool,
    /// The facts of the instance's first pass, when the extension claims it
    /// ([`Extension::claims`]): each frame offers the instance to `hold`.
    claim: Option<DrawFacts>,
    /// The extension holds the instance in this frame: no view of the table draws the slot.
    held: bool,
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
    /// The `INSTANCE_BONES` records taken, and the skinned slots not drawn because their
    /// bones had not come.
    pub bone_records: u64,
    pub without_bones: u64,
    /// In the main view with the walk's places on: the blends drawn in place by the engine
    /// that had no place (drawn by bucket), and the places whose walk order differs from
    /// their bucket order.
    pub walk_without_place: u64,
    pub walk_reordered: u64,
}

/// What the cull of a view reads of a slot (`collect_order`), beside the slot in a small
/// record: the slots of a view are mostly culled, and the slot itself is large.
#[derive(Clone, Copy, Default)]
struct CullEntry {
    bound: [f32; 4],
    state: u8,
}

/// The bits of [`CullEntry::state`].
const CULL_LIVE: u8 = 1;
const CULL_VISIBLE: u8 = 2;
const CULL_HELD: u8 = 4;
const CULL_WITHOUT_BONES: u8 = 8;

impl CullEntry {
    fn of(slot: Option<&Slot>) -> CullEntry {
        let Some(s) = slot else { return CullEntry::default() };
        let mut state = CULL_LIVE;
        if s.visible {
            state |= CULL_VISIBLE;
        }
        if s.held {
            state |= CULL_HELD;
        }
        if s.flags & INSTANCE_SKINNED != 0 && s.bones_first == NO_BONES {
            state |= CULL_WITHOUT_BONES;
        }
        CullEntry { bound: s.bound, state }
    }
}

/// The origin of the set moves to the camera when the camera is farther from it than this,
/// in units of the world. A 32-bit float has steps of 0.0005 units at 4096.
const REBASE_DISTANCE: f64 = 4096.0;

pub struct RetainedSet {
    slots: Vec<Option<Slot>>,
    /// The cull's record of each slot (`refresh_cull`), and the draw ranges of all slots.
    cull: Vec<CullEntry>,
    ranges_total: usize,
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
    /// The lights of the world that are on (`WORLD_LIGHTS`): their keys. Each has a slot in
    /// `light_values`, which is the light buffer of the world mode.
    world_keys: Vec<u32>,
    /// The ambient light of each instance's own effect state (the lights record), the
    /// `D3DRS_AMBIENT` of its draws: the shader multiplies it by the scene's `ambient_scale`.
    /// An instance without one takes the scene's ambient at its insert.
    ambient_of: HashMap<u32, u32>,
    /// The slots whose record changed (and so their instance), the slots that only moved
    /// (their instance: the shader reads the world of a retained draw from it), or all.
    dirty: Vec<u32>,
    moved: Vec<u32>,
    all_dirty: bool,
    /// The slots that draw a stage from a slot of the frame in the frame they were made,
    /// with the pinned texture and sampler slots of each stage for the next frame
    /// ([`Player::retained_settle_late`]).
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
    /// kept for the flush of the engine's queue ([`Sorted::Keep`]), with the view.
    sorted_order: Vec<(u64, u32, u32, u32)>,
    kept_sorted: Vec<(u64, u32, u32, u32)>,
    pub sorted_view: Option<RetainedView>,
    /// The places of the table's blends in the engine's walk, and the main view that left
    /// their slots out for them.
    pub(crate) walk: crate::engine::walk::Walk,
    walk_view: Option<RetainedView>,
    /// The origin of the space of the instances: the translation of a slot and the
    /// position of a light are relative to it. With a relative scene (the packet has the
    /// exact position of the camera) the origin follows the camera (`rebase`), so the
    /// 32-bit translations stay small. Without one the origin is 0: the space of the world.
    origin: [f64; 3],
    relative: bool,
    /// The view of the game for the world of the frame, in the space of the world: the
    /// planes of the cull come from it.
    pub main_view: [f32; 16],
    /// The bones of the skinned instances, as their records give them (the matrix of the
    /// rotation and the scale, the exact translation): `bones_of` has the range of each
    /// instance, `bone_free` the free ranges. The GPU's palette is made from them for the
    /// origin of the set when they changed ([`Renderer::retained_upload`]).
    bones: Vec<([f32; 9], [f64; 3])>,
    bones_of: HashMap<u32, (u32, u32)>,
    bone_free: Vec<(u32, u32)>,
    bones_dirty: bool,
    /// The instances that the extension claims ([`Slot::claim`]).
    claimed: std::collections::HashSet<u32>,
    pub counts: RetainedCounts,
}

impl RetainedSet {
    pub fn new() -> RetainedSet {
        RetainedSet {
            claimed: std::collections::HashSet::new(),
            slots: Vec::new(),
            cull: Vec::new(),
            ranges_total: 0,
            free: Vec::new(),
            by_id: HashMap::new(),
            buckets: Vec::new(),
            bucket_of: HashMap::new(),
            order: Vec::new(),
            bucket_depth: Vec::new(),
            sorted_order: Vec::new(),
            kept_sorted: Vec::new(),
            sorted_view: None,
            walk: Default::default(),
            walk_view: None,
            origin: [0.0; 3],
            relative: false,
            main_view: [0.0; 16],
            bones: Vec::new(),
            bones_of: HashMap::new(),
            bone_free: Vec::new(),
            bones_dirty: false,
            light_slot_of: HashMap::new(),
            light_values: Vec::new(),
            light_dirty: Vec::new(),
            light_indexes: Vec::new(),
            light_indexes_live: 0,
            light_indexes_dirty: false,
            keys_of: HashMap::new(),
            world_keys: Vec::new(),
            ambient_of: HashMap::new(),
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
        self.ranges_total
    }

    fn take_slot(&mut self) -> u32 {
        if let Some(slot) = self.free.pop() {
            return slot;
        }
        self.slots.push(None);
        self.cull.push(CullEntry::default());
        (self.slots.len() - 1) as u32
    }

    /// The cull's record of a slot after a change of the slot.
    fn refresh_cull(&mut self, slot: u32) {
        self.cull[slot as usize] = CullEntry::of(self.slots[slot as usize].as_ref());
    }

    /// Removes the instance: its slots go to the free list, and the GPU keeps their stale
    /// records (no draw names them). Returns the texture handles its slots pinned, for
    /// [`Renderer::unpin_texture`] ([`Player::retained_remove`]).
    #[must_use]
    pub fn remove(&mut self, id: u32) -> Vec<u32> {
        let mut pinned = Vec::new();
        self.claimed.remove(&id);
        let Some(slots) = self.by_id.remove(&id) else { return pinned };
        for slot in slots {
            if let Some(taken) = self.slots[slot as usize].take() {
                self.ranges_total -= taken.ranges.len();
                self.refresh_cull(slot);
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
                // A skinned slot's bound comes with its bones.
                if s.flags & INSTANCE_SKINNED == 0 {
                    s.bound = r.bound;
                }
                s.data.world[0] = world;
            }
            self.cull[slot as usize] = CullEntry::of(self.slots[slot as usize].as_ref());
            self.moved.push(slot);
        }
        self.counts.moves += 1;
        true
    }

    /// The origin that the positions of the lights of the set are relative to.
    fn light_origin(&self) -> Option<[f64; 3]> {
        self.relative.then_some(self.origin)
    }

    /// The world matrix of a record in the space of the set. A skinned instance has the
    /// identity: its bones place it.
    pub fn world_of(&self, r: &Retained) -> [f32; 16] {
        if r.flags & INSTANCE_SKINNED != 0 {
            return IDENTITY;
        }
        r.world_at(&self.origin)
    }

    /// The bones of a skinned instance (`INSTANCE_BONES`): `count` bones of [`BONE_BYTES`] in
    /// `data`, and the world bound of the shape for the cull. The instance keeps its range of
    /// the palette while its number of bones stays the same; its slots take the range and the
    /// bound.
    pub fn set_bones(&mut self, id: u32, count: u32, bound: [f32; 4], data: &[u8]) {
        if count == 0 || data.len() < count as usize * BONE_BYTES {
            return;
        }
        let first = match self.bones_of.get(&id) {
            Some(&(first, kept)) if kept == count => first,
            _ => {
                self.forget_bones(id);
                let first = self.take_bones(count);
                self.bones_of.insert(id, (first, count));
                first
            }
        };
        for b in 0..count as usize {
            let at = b * BONE_BYTES;
            let rotation: [f32; 9] = core::array::from_fn(|i| f32::from_le_bytes(data[at + i * 4..at + i * 4 + 4].try_into().unwrap()));
            let exact: [f64; 3] = core::array::from_fn(|i| f64::from_le_bytes(data[at + 36 + i * 8..at + 44 + i * 8].try_into().unwrap()));
            self.bones[first as usize + b] = (rotation, exact);
        }
        self.bones_dirty = true;
        self.counts.bone_records += 1;
        if let Some(slots) = self.by_id.get(&id) {
            for &slot in slots {
                if let Some(s) = self.slots[slot as usize].as_mut() {
                    s.bound = bound;
                    if s.bones_first != first {
                        s.bones_first = first;
                        self.moved.push(slot);
                    }
                }
                self.cull[slot as usize] = CullEntry::of(self.slots[slot as usize].as_ref());
            }
        }
    }

    /// A range of `count` bones: a free range of that size, else a new one at the end.
    fn take_bones(&mut self, count: u32) -> u32 {
        if let Some(at) = self.bone_free.iter().position(|&(_, n)| n == count) {
            return self.bone_free.swap_remove(at).0;
        }
        let first = self.bones.len() as u32;
        self.bones.resize(self.bones.len() + count as usize, ([0.0; 9], [0.0; 3]));
        first
    }

    /// The instance's range of bones goes to the free ranges: the instance is gone, or its
    /// number of bones changed.
    pub fn forget_bones(&mut self, id: u32) {
        if let Some(range) = self.bones_of.remove(&id) {
            self.bone_free.push(range);
        }
    }

    /// The first bone of a slot of the instance, for a slot made again.
    fn bones_first_of(&self, id: u32) -> u32 {
        self.bones_of.get(&id).map_or(NO_BONES, |&(first, _)| first)
    }

    /// The origin of the set follows the camera of the world: `camera` is the exact
    /// position of the camera of a relative scene, or [`None`] for a scene that is not
    /// relative. When the scene changes kind, or the camera is farther than
    /// [`REBASE_DISTANCE`] from the origin, every slot and every light is made again for the
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
        // The bones are made again for the new origin at the upload; a skinned slot keeps the
        // identity.
        self.bones_dirty = true;
        for slot in self.slots.iter_mut().flatten() {
            if slot.flags & INSTANCE_SKINNED != 0 {
                continue;
            }
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
    /// origin of the scene), or [`None`] for a scene that is not relative.
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
                self.cull[slot as usize] = CullEntry::of(self.slots[slot as usize].as_ref());
            }
        }
    }

    /// The light of the game with this key changed: its retained value, when the instances
    /// use it.
    pub fn light_changed(&mut self, key: u32, values: &[u8]) {
        if let Some(&slot) = self.light_slot_of.get(&key) {
            // In the space of the set, as the slot was made.
            let origin = self.light_origin();
            self.light_values[slot as usize] = light_data(values, origin);
            self.light_dirty.push(slot);
        }
    }

    /// The lights of the world that are on: a slot for each key that has values.
    pub fn set_world_lights(&mut self, keys: &[u32], known: &HashMap<u32, [u8; 104]>) {
        self.world_keys.clear();
        for &key in keys {
            if self.light_slot(key, known).is_some() {
                self.world_keys.push(key);
            }
        }
    }

    /// The slots of the point lights of the world in `light_values` (a directional light
    /// stays with the draws that have it).
    pub fn world_light_slots(&self) -> Vec<u32> {
        self.world_keys
            .iter()
            .filter_map(|key| self.light_slot_of.get(key).copied())
            .filter(|&slot| self.light_values.get(slot as usize).is_some_and(|l| l.position[3] != 3.0))
            .collect()
    }

    /// The translation from the space of the set (the positions of `light_values`) to the
    /// space relative to `camera`, or to the world without a camera.
    pub fn set_to_scene(&self, camera: Option<[f64; 3]>) -> [f64; 3] {
        let o = self.light_origin().unwrap_or([0.0; 3]);
        let c = camera.unwrap_or([0.0; 3]);
        [o[0] - c[0], o[1] - c[1], o[2] - c[2]]
    }

    /// The light of the game with this key is destroyed. Its slot gives no light until a new
    /// light with the key comes (`light_changed`): a run that still names the key adds nothing.
    pub fn light_gone(&mut self, key: u32) {
        if let Some(&slot) = self.light_slot_of.get(&key) {
            let light = &mut self.light_values[slot as usize];
            light.diffuse = [0.0; 4];
            light.ambient = [0.0; 4];
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
    pub fn set_lights(&mut self, id: u32, keys: &[u32], ambient: Option<u32>, known: &HashMap<u32, [u8; 104]>) {
        if let Some(ambient) = ambient {
            self.ambient_of.insert(id, ambient);
        }
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
                // An emissive-only slot keeps no light and a black ambient.
                let count = if s.no_lights { 0 } else { count };
                s.data.light_range[0] = first;
                s.data.light_range[1] = count;
                s.data.counts[0] = count;
                if s.no_lights {
                    s.data.scene_ambient = [0.0, 0.0, 0.0, 1.0];
                } else if let Some(ambient) = ambient {
                    s.data.scene_ambient = crate::d3d::color(ambient);
                }
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
                    let count = if s.no_lights { 0 } else { count };
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
        // The engine keeps the radius of a point light in its specular colour (written by
        // `game_dynamicLightTest` 0x4D3039, read back at 0x4D3150 and by
        // `game_getLightRadiusFromSpecular` 0x4D1B80): the first specular word.
        cone: [(float(l, 24 * 4) * 0.5).cos(), (float(l, 25 * 4) * 0.5).cos(), float(l, 5 * 4), 0.0],
    }
}

/// What a view does with the sorted blends of the table.
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
    /// The samples for each pixel of the targets of the pass ([`PipelineKey::samples`]).
    pub samples: u32,
}

/// A texture with a slot of the arrays that stays from frame to frame ([`Renderer::pin_texture`]):
/// its slot is its index plus 1. `uses` counts the stages of the retained slots that name it;
/// a free entry has the handle 0 and the white view.
pub struct PinnedTexture {
    pub handle: u32,
    pub view: Rc<wgpu::TextureView>,
    pub uses: u32,
}

impl Renderer {
    /// A slot of the arrays of the frame that a texture keeps from frame to frame, for the
    /// records of the retained instances: one more use of it. 0 for no texture or no room.
    /// A texture not known yet takes a slot with the white view: its record can come after
    /// the instance that names it (a capture that starts with a purge of the textures, a
    /// texture made again under the same handle), and `set_texture` then puts the view into
    /// the slot. Without the slot the instance drew white until its next record. The lowest
    /// free slot is taken before the room grows, so that the free slots gather at the end,
    /// where `unpin_texture` drops them. The pinned slots and the frame's own textures share
    /// the room; [`FRAME_RESERVE`] slots stay the frame's.
    pub fn pin_texture(&mut self, handle: u32) -> u32 {
        if handle == 0 {
            return 0;
        }
        if let Some(&index) = self.pinned_index.get(&handle) {
            self.pinned_textures[index].uses += 1;
            return index as u32 + 1;
        }
        let view = match self.textures.get(&handle) {
            Some(view) => view.clone(),
            None => {
                self.counts.pinned_before_record += 1;
                self.white.clone()
            }
        };
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
        // The arrays stay from frame to frame: the slot lets the texture go now.
        if index < self.pinned_in_frame && index + 1 < self.frame_texture_views.len() {
            self.frame_texture_views[index + 1] = self.white.clone();
            self.array_group = None;
            self.array_variants.clear();
        }
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
    /// slots. After `begin_frame_arrays` cleared the arrays (it returned true).
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
                    instances[i * INSTANCE_SIZE..(i + 1) * INSTANCE_SIZE].copy_from_slice(bytemuck::bytes_of(&slot_instance(i, slot)));
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
            let instance_of = |i: usize, slot: &Slot| slot_instance(i, slot);
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
        // The bones of the skinned instances, in the space of the set: all of them in one
        // write when one changed (the actors in view send theirs each frame).
        if set.bones_dirty {
            set.bones_dirty = false;
            if set.bones.len() > self.retained_bone_capacity {
                self.retained_bone_capacity = set.bones.len().next_power_of_two();
                self.retained_bone_buffer = bone_buffer(&self.device, self.retained_bone_capacity);
            }
            let origin = set.light_origin().unwrap_or([0.0; 3]);
            let matrices: Vec<[f32; 16]> = set
                .bones
                .iter()
                .map(|(rotation, exact)| {
                    let mut m = [0f32; 16];
                    for c in 0..3 {
                        m[c * 4..c * 4 + 3].copy_from_slice(&rotation[c * 3..c * 3 + 3]);
                        m[12 + c] = (exact[c] - origin[c]) as f32;
                    }
                    m[15] = 1.0;
                    m
                })
                .collect();
            if !matrices.is_empty() {
                self.queue.write_buffer(&self.retained_bone_buffer, 0, bytemuck::cast_slice(&matrices));
            }
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

/// The instance of a slot: its world, its record, and the first of its bones for a skinned
/// slot.
fn slot_instance(i: usize, slot: &Slot) -> Instance {
    let bones = if slot.bones_first == NO_BONES { 0 } else { slot.bones_first };
    Instance { world: slot.world, record: [i as u32, bones, 0, 0] }
}

/// Draws the instances of the set in view, one indirect call for each run of draws of one
/// bucket. The pass state is not known after it: the caller forgets its bound groups. With
/// [`Sorted::Only`] the view draws the sorted blends kept by the main view and nothing else.
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
        key.samples = view.samples;
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
    /// The centre of the bound of an instance (of its first slot).
    pub(crate) fn centre_of(&self, id: u32) -> Option<[f32; 3]> {
        let &slot = self.by_id.get(&id)?.first()?;
        let s = self.slots[slot as usize].as_ref()?;
        Some([s.bound[0], s.bound[1], s.bound[2]])
    }

    /// Puts the entries of the sorted slots of an instance into `entries` (a run of
    /// [`Sorted::Only`]), in the order of its slots. Returns the number of its slots that the
    /// table does not sort: those drew with the other draws of the table.
    fn placed_entries(&self, id: u32, entries: &mut Vec<(u64, u32, u32, u32)>) -> u32 {
        let mut not_sorted = 0;
        for &slot in self.by_id.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
            let Some(s) = self.slots[slot as usize].as_ref() else { continue };
            if s.held {
                continue;
            }
            if !s.sorted {
                not_sorted += 1;
                continue;
            }
            if s.flags & INSTANCE_SKINNED != 0 && s.bones_first == NO_BONES {
                continue;
            }
            entries.push((0, s.bucket, 0, slot));
        }
        not_sorted
    }

    /// The draws of a view in the order they go to the GPU, into `order`:
    /// 1. The buckets that do not blend and have no alpha test, front to back by their
    ///    nearest draw in view, and the draws of each front to back: the early depth test
    ///    then rejects the pixels behind what is drawn.
    /// 2. The buckets with an alpha test, the same way after them (a discard in the shader
    ///    can turn the early depth test off).
    /// 3. The buckets that blend out of the sort (the no-sorter bit), in the order they were
    ///    made, their draws in the order of their slots. A shape draws a base pass and a
    ///    blend pass that both write depth and test less-or-equal, in different buckets: the
    ///    base must come first, or it overwrites the blend at the same depth (the game draws
    ///    the two in a row). In the main view a slot of an instance with a place in the
    ///    engine's walk ([`tag::INSTANCE_WALK`]) leaves this group: it draws at its place
    ///    ([`engine::walk`]). This order is only for the blends without one.
    /// The depth of a draw is that of the nearest point of its bound: the distance from the
    /// near plane of the view less the radius, at least 0.
    /// 4. The blends the engine would sort, into `sorted_order`: back to front by the depth
    ///    of the bound centre along the view, ties by the order of the slots. This is not
    ///    the engine's order: the engine sorts screen-space boxes (`$2::Sort` 0x6FE830,
    ///    engine-rules R21). A view draws them after the rest
    ///    ([`Sorted::Together`]) or keeps them for the flush of the engine's queue
    ///    ([`Sorted::Keep`], then `Only`). There a kept blend with a place in the engine's
    ///    queue ([`tag::INSTANCE_QUEUED`]) draws at its place, in the engine's order; this
    ///    order is only for the blends without one (`draw_retained_sorted`).
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
                let cull = &self.cull[slot as usize];
                if cull.state & CULL_LIVE == 0 {
                    continue;
                }
                if cull.state & CULL_VISIBLE == 0 {
                    self.counts.hidden += 1;
                    continue;
                }
                // The extension draws a held instance (`Extension::claims`).
                if cull.state & CULL_HELD != 0 {
                    continue;
                }
                if cull.state & CULL_WITHOUT_BONES != 0 {
                    self.counts.without_bones += 1;
                    continue;
                }
                if !in_view(&cull.bound, &view.planes) {
                    self.counts.culled += 1;
                    continue;
                }
                let Some(s) = self.slots[slot as usize].as_ref() else { continue };
                self.counts.drawn += 1;
                let centre = near[0] * s.bound[0] + near[1] * s.bound[1] + near[2] * s.bound[2] + near[3];
                sequence += 1;
                if s.sorted {
                    self.counts.sorted += 1;
                    self.sorted_order.push((sorted_key(centre, s.id), b as u32, sequence, slot));
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
            entry.0 = bucket_key(group, entry.1, depth);
        }
        self.order.sort_unstable();
        self.sorted_order.sort_unstable();
        // The main view: a blend out of the sort with a place in the engine's walk draws there.
        if view.sorted == Sorted::Keep && self.walk.on {
            let slots = &self.slots;
            self.counts.walk_reordered += self.walk.leave_out(&mut self.order, |slot| slots[slot as usize].as_ref().map(|s| s.id)) as u64;
            self.counts.walk_without_place += self.order.iter().filter(|e| e.0 >> 32 == crate::engine::walk::GROUP_IN_PLACE && slots[e.3 as usize].as_ref().is_some_and(|s| s.in_place)).count() as u64;
            self.walk_view = Some(*view);
        }
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
/// and the depth write) times 2, plus 1 with a depth write: the index into [`TABLE_KINDS`].
/// `draw_retained_main` asks the extension for the bits of each kind once a frame
/// ([`Extension::shade_table`]), and the shader takes the bits by this index.
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

/// The stages of the texture effects of a material for the table, as [`Pass::new`] takes them
/// from a shape record: the generation from a vector of the world in the high word of the
/// coordinate index (5 the normal, 6 the position, 7 the reflection, as the shader reads
/// them), the transform flags, and the texture matrix from the effect's world projection.
/// The engine's callbacks make the same stage for a camera by turning the vector of camera
/// space into one of the world first: here the shader does the turn, so the stage holds for
/// every view. The position of a retained draw is relative to the origin of the set: the
/// shader adds `scene.origin` before the matrix. [`None`] for a material without effects.
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
    /// Removes an instance from the table, and the uses of the textures its slots pinned,
    /// and its bones.
    pub(super) fn retained_remove(&mut self, id: u32) {
        self.retained_remove_slots(id);
        self.retained.forget_bones(id);
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

    /// Removes the slots of an instance and the uses of their textures; its bones stay (the
    /// instance is made again).
    fn retained_remove_slots(&mut self, id: u32) {
        for handle in self.retained.remove(id) {
            self.renderer.unpin_texture(handle);
        }
    }

    /// Puts an instance record with the flag 0x100 into the table: a slot for each pass of
    /// its material, with the record of the draw made as for a shape record. An instance
    /// that is in the table already (its material or geometry handle changed) keeps its
    /// lights: the plugin sends them again only when the set of its lights changes.
    /// Engine-rules R13 (the skill wgpu-renderer-dev).
    pub(super) fn retained_insert(&mut self, id: u32, r: &Retained) {
        let keys = self.retained.keys_of.get(&id).cloned();
        self.retained_remove_slots(id);
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
                let d = Rc::new(material::derive(&m, sets, self.stream.envmap_lit));
                self.renderer.derived.insert((r.material, sets), d.clone());
                d
            }
        };
        // The shape in progress as the record needs it; the lights, the fog and the matrices
        // come from the scene of each view. The ambient light is the instance's own (its
        // lights record), the scene's until that record comes.
        {
            let shape = &mut self.stream.shape;
            shape.ambient = self.retained.ambient_of.get(&id).copied().unwrap_or_else(|| self.retained_values.map_or(0xFF00_0000, |v| v.0));
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
        // The extension's claim (`Extension::claims`), from the facts of the first pass as
        // `hold_shape` gives them to `hold`; as there, not for the sky, the water plane of the
        // game or a skinned shape.
        let colours = material::d3d_material(&r.colours);
        let claim = (!derived.passes.is_empty() && r.flags & INSTANCE_SKINNED == 0 && !scene::sky_class(r.class) && r.class != scene::CLASS_WATER_PLANE)
            .then(|| {
                let p = Pass::new(&derived, 0, &m, &self.renderer.textures, effects.as_ref());
                DrawFacts {
                    class: r.class,
                    blend_enable: p.blend[0] != 0,
                    source_blend: p.blend[1],
                    destination_blend: p.blend[2],
                    depth_write: derived.depth[1] != 0,
                    specular_power: colours[16],
                    emissive: colours[12..16].try_into().unwrap(),
                }
            })
            .filter(|facts| self.extension.as_ref().is_some_and(|extension| extension.claims(facts)));
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
                // A draw of the table goes into the pass of the game; a replay into another
                // pass sets the samples of that pass (`draw_retained` with a `ReplayView`).
                samples: self.renderer.samples,
            };
            let mut dummy = Vec::new();
            let mut data = self.stream.draw_data(&derived, &p, &mut self.renderer.counts, &mut dummy);
            crate::draw_dump("retained table, before the record bits", r.geometry, &data);
            data.world[0] = self.retained.world_of(r);
            // A skinned instance blends four bones of its instance for each vertex.
            if r.flags & INSTANCE_SKINNED != 0 {
                data.flags[3] = 4;
            }
            // The kind of the pass for the shade bits of the scene.
            let kind = slot_kind(p.blend, derived.depth[1] != 0);
            let world_lights = if derived.no_lights { 0 } else { crate::WORLD_LIGHTS_BIT };
            data.light_range = [0, 0, kind, RETAINED_RECORD_BITS | crate::flat_shading_bit(&derived) | world_lights];
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
                // A texture not known yet keeps its handle: `pin_texture` gives it a slot
                // with the white view, and its record fills the slot when it comes.
                if stage < data.counts[1] as usize && handle != 0 {
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
            // A skinned instance keeps its range of bones when it is made again; its bound
            // comes with the bones that the plugin sends after the instance record.
            let bones_first = if r.flags & INSTANCE_SKINNED != 0 { set.bones_first_of(id) } else { NO_BONES };
            let sorted = crate::engine::alpha::sorted_by_engine(r.class, &derived.blends, derived.no_sorter);
            let in_place = !sorted && derived.blends.first().is_some_and(|b| b[0] != 0);
            set.slots[slot as usize] = Some(Slot { id, geometry: r.geometry, material: r.material, class: r.class, flags: r.flags, colours: r.colours, exact, world, bound: r.bound, bucket, data, ranges: ranges.clone(), base_vertex, visible: true, sorted, in_place, pinned, bones_first, no_lights: derived.no_lights, claim, held: false });
            set.ranges_total += ranges.len();
            set.cull[slot as usize] = CullEntry::of(set.slots[slot as usize].as_ref());
            set.buckets[bucket as usize].slots.push(slot);
            set.dirty.push(slot);
            slots.push(slot);
        }
        let set = &mut self.retained;
        set.by_id.insert(id, slots);
        if claim.is_some() {
            set.claimed.insert(id);
        }
        set.counts.instances = set.by_id.len() as u32;
        set.counts.slots = set.slots.iter().flatten().count() as u32;
        // The lights the instance had, when the record came again (`remove` dropped them).
        if let Some(keys) = keys {
            let known = &self.renderer.lights_known;
            let ambient = self.retained.ambient_of.get(&id).copied();
            self.retained.set_lights(id, &keys, ambient, known);
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
            ambient_scale: [1.0; 4],
            world_lights: [0; 4],
            padding: [0; 48],
        };
        // The fog of the extension for the world, as a draw of the world gets it: this is the
        // world camera's main view.
        if let Some((start, end)) = self.parameters.as_ref().and_then(|p| p.fog_range) {
            data.fog[0] = start;
            data.fog[1] = end;
            data.fog[3] = 1.0;
        }
        data
    }

    /// Offers the instances that the extension claims ([`Extension::claims`]) to `hold`, one
    /// time in a frame for each one in the world's main view, as the stream's shapes are
    /// offered (`hold_shape`): a held instance is drawn by the extension, and no view of the
    /// table draws its slots in the frame ([`Slot::held`]). Returns the held draws, made from
    /// the instance's first slot as `hold_shape` makes them from the first pass.
    fn retained_hold(&mut self, facts: &WorldFacts, planes: &[[f32; 4]; 6]) -> Vec<HeldDraw> {
        let mut out = Vec::new();
        if self.retained.claimed.is_empty() {
            return out;
        }
        let offer = self.parameters.as_ref().is_some_and(|p| p.hold);
        let Some(mut extension) = self.extension.take() else { return out };
        let origin = self.retained.light_origin();
        let view = self.retained.scene_view(&facts.view, facts.origin);
        let ids: Vec<u32> = self.retained.claimed.iter().copied().collect();
        for id in ids {
            let slots = self.retained.by_id.get(&id).cloned().unwrap_or_default();
            let Some(&first) = slots.first() else { continue };
            let Some(s) = self.retained.slots[first as usize].as_ref() else { continue };
            let Some(claim) = s.claim else { continue };
            let held = offer && s.visible && in_view(&s.bound, planes) && extension.hold(&claim);
            if held {
                let bucket = &self.retained.buckets[s.bucket as usize];
                let key = bucket.key;
                let sets = (key.fvf >> 8) & 0xF;
                let derived = self.renderer.derived.get(&(s.material, sets)).cloned();
                let m = self.renderer.materials.get(&s.material);
                if let (Some(layout), Some(derived), Some(m)) = (VertexLayout::of(key.fvf, key.stride), derived, m) {
                    let mut data = s.data;
                    data.view = view;
                    data.projection = facts.projection;
                    out.push(HeldDraw {
                        facts: claim,
                        vertices: bucket.vertices.clone(),
                        base_vertex: s.base_vertex,
                        indexes: bucket.indexes.clone(),
                        maps: self.renderer.held_maps(m, layout.coordinates.len() as u32),
                        layout,
                        topology: if key.topology == 5 { wgpu::PrimitiveTopology::TriangleStrip } else { wgpu::PrimitiveTopology::TriangleList },
                        parts: s.ranges.clone(),
                        data,
                        origin,
                        lights: Vec::new(),
                        depth: [(derived.depth[0] != 0) as u32, (derived.depth[1] != 0) as u32, derived.depth[2]],
                        cull: derived.cull,
                        stencil: derived.stencil,
                    });
                    self.renderer.counts.held += 1;
                }
            }
            for slot in slots {
                if let Some(s) = self.retained.slots[slot as usize].as_mut() {
                    s.held = held;
                }
                self.retained.cull[slot as usize] = CullEntry::of(self.retained.slots[slot as usize].as_ref());
            }
        }
        self.extension = Some(extension);
        out
    }

    /// The table's lights as the extension shades them for the main view of the frame
    /// ([`Extension::shade_table`]), and the values the shade gives the scene; the lights
    /// unshaded when the extension does not shade.
    pub(super) fn shade_table_main(&mut self, facts: &WorldFacts) -> (Vec<LightData>, Option<TableShade>) {
        let mut lights = self.retained.light_values.clone();
        let mut shade = None;
        if self.parameters.as_ref().is_some_and(|p| p.shade) {
            if let Some(extension) = self.extension.as_mut() {
                // The kinds of a slot (`slot_kind`) are those of `TABLE_KINDS`.
                shade = Some(extension.shade_table(&facts.view, &mut lights, &TABLE_KINDS));
            }
        }
        (lights, shade)
    }

    /// Draws the retained instances in the world of the frame: after the extension drew
    /// before the world, before the draws of the game. The extension shades the lights of
    /// the instances and the values of the scene as it shades a draw of the world that
    /// writes depth and does not blend ([`Extension::shade`]), one time for the frame.
    /// Engine-rules R10, R20 (the skill wgpu-renderer-dev).
    pub(super) fn draw_retained_main(&mut self, facts: &WorldFacts) {
        // The near plane of the world's view, for the depth of the sort of the blends.
        self.world_near = Some(frustum_planes(&multiply32(&facts.view, &facts.projection))[4]);
        if self.retained.by_id.is_empty() || !self.retained_on {
            return;
        }
        let mut scene = self.retained_scene_main(facts);
        let (lights, shade) = match self.table_shaded.take() {
            Some(shaded) => shaded,
            None => {
                let shaded = self.shade_table_main(facts);
                self.renderer.upload_retained_lights(&shaded.0);
                shaded
            }
        };
        // The view takes the point lights of the world when the frame binned them.
        scene.world_lights[0] = (self.renderer.clusters.world && self.renderer.clusters.world_built) as u32;
        {
            if let Some(shade) = shade {
                let s = shade.ambient_scale;
                scene.scene_ambient = [scene.scene_ambient[0] * s[0], scene.scene_ambient[1] * s[1], scene.scene_ambient[2] * s[2], scene.scene_ambient[3]];
                // Each retained draw's own ambient is multiplied by the factor in the shader.
                scene.ambient_scale = [s[0], s[1], s[2], 1.0];
                scene.extension5 = shade.extension5;
                scene.extension7 = shade.extension7;
                // The shade puts the fog of a draw on the distance from the camera: the
                // scene's fog takes it too, or a retained shape off the axis of the view gets
                // less fog than the same shape on the engine's path.
                if shade.radial_fog {
                    scene.fog[3] = 1.0;
                }
                for (k, flags) in shade.flags.iter().enumerate() {
                    scene.counts[k / 4][k % 4] = *flags;
                }
            }
        }
        if crate::dump_wanted() != 0 {
            self.retained.dump_lights(crate::dump_wanted(), &lights);
        }
        self.renderer.scene_main = scene;
        self.renderer.write_scene(0, &scene);
        if crate::dump_wanted() != 0 {
            println!("scene dump: fog_color {:?} fog {:?} ambient {:?} counts {:?} extension5 {:?} extension7 {:?}", scene.fog_color, scene.fog, scene.scene_ambient, scene.counts, scene.extension5, scene.extension7);
        }
        // The cull is in the space of the world: the bounds of the slots are the engine's.
        self.retained.main_view = facts.view;
        let planes = frustum_planes(&multiply32(&facts.view, &facts.projection));
        let held = self.retained_hold(facts, &planes);
        let Some(pass) = self.pass.as_mut() else { return };
        let view = RetainedView { scene_slot: 0, classes: !0, planes, flip_cull: false, force_depth_test: false, target: self.pass_target, sorted: Sorted::Keep, samples: self.pass_samples };
        pass.push_debug_group("retained instances");
        draw_retained(pass, &mut self.renderer, &mut self.retained, &mut self.frame_args, &view);
        pass.pop_debug_group();
        self.forget_bound();
        // The held instances go where the stream's held shapes go: in place (after the
        // table's draws, its place in the frame [I: the engine's walk would place each one
        // among the game's draws]) or with the others at the end of the world scene.
        for h in held {
            if self.extension.as_ref().is_some_and(|extension| extension.held_in_place(&h)) {
                self.give_held_here(h, facts);
            } else {
                self.taken.push(h);
            }
        }
    }

    /// Draws the sorted blends of the frame where the engine flushes its sorted queue: the
    /// blended draws of the engine's path and the places of the table's blends that waited
    /// (`frame_sorted`), and the kept blends of the table without a place. The waiting
    /// entries keep their order of arrival: the engine sorted them, the table's blends with
    /// them (the plugin puts those into the engine's queue, [`tag::INSTANCE_QUEUED`]), and a
    /// node that orders its subtree (a sort adjust node) gives an order that the depth of
    /// each shape does not. A kept blend without a place (an older capture, a queue that did
    /// not take the shape) goes between them by the depth of its bound centre along the view
    /// (`merge_picks`). A run of the table's entries is one `draw_retained` call; a draw of
    /// the engine's path is issued between them.
    /// Engine-rules R21, R22 (the skill wgpu-renderer-dev).
    pub(super) fn draw_retained_sorted(&mut self, generation: u32) {
        let (deferred, later): (Vec<_>, Vec<_>) = core::mem::take(&mut self.frame_sorted).into_iter().partition(|e| e.3 == generation);
        self.frame_sorted = later;
        let view = if generation == 0 { self.retained.sorted_view.take() } else { self.retained.sorted_view };
        let all_kept = if generation == 0 && self.retained_on && view.is_some() { core::mem::take(&mut self.retained.kept_sorted) } else { Vec::new() };
        // The kept blends of the instances with a place in the queue draw at their place.
        let placed: std::collections::HashSet<u32> = deferred.iter().filter_map(|e| if let crate::Waiting::Table(id) = e.2 { Some(id) } else { None }).collect();
        let kept: Vec<(u64, u32, u32, u32)> = all_kept
            .iter()
            .copied()
            .filter(|entry| placed.is_empty() || !self.retained.slots[entry.3 as usize].as_ref().is_some_and(|s| placed.contains(&s.id)))
            .collect();
        self.renderer.counts.sorted_without_place += kept.len() as u64;
        // Risk 34: a frame whose kept blends found no place at all points to a cause of the
        // frame (a scene or a Click that did not queue); a frame with some places, to a cause
        // of the shape.
        if placed.is_empty() {
            self.renderer.counts.sorted_without_place_no_places += kept.len() as u64;
        }
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
        let deferred: Vec<(u64, crate::Waiting)> = deferred
            .into_iter()
            .map(|(c, sequence, waiting, _)| {
                let depth = (near[0] * c[0] + near[1] * c[1] + near[2] * c[2] + near[3]).max(0.0);
                ((!depth.to_bits() as u64) << 32 | sequence as u64, waiting)
            })
            .collect();
        self.renderer.counts.merged_blends += deferred.iter().filter(|e| matches!(e.1, crate::Waiting::Draw(_))).count() as u64;
        if self.pass.is_none() {
            self.open_pass(None, None, None);
        }
        let kept_keys: Vec<u64> = kept.iter().map(|entry| entry.0).collect();
        let deferred_keys: Vec<u64> = deferred.iter().map(|entry| entry.0).collect();
        // The places of the table that follow each other draw as one run.
        let mut run: Vec<(u64, u32, u32, u32)> = Vec::new();
        for pick in merge_picks(&kept_keys, &deferred_keys) {
            match pick {
                Pick::Table(start, end) => {
                    self.draw_sorted_run(view, core::mem::take(&mut run));
                    self.draw_sorted_run(view, kept[start..end].to_vec());
                }
                Pick::Engine(j) => match &deferred[j].1 {
                    crate::Waiting::Draw(draw) => {
                        self.draw_sorted_run(view, core::mem::take(&mut run));
                        let draw = draw.clone();
                        self.issue(&draw);
                    }
                    &crate::Waiting::Table(id) => {
                        self.renderer.counts.queued_places += 1;
                        if self.retained.placed_entries(id, &mut run) != 0 {
                            self.renderer.counts.queued_not_sorted += 1;
                        }
                    }
                },
            }
        }
        self.draw_sorted_run(view, run);
        self.retained.kept_sorted = all_kept;
        // An item after the world end (the engine flushed its queue in a later scene): the
        // post-processing reads the copy of the world depth, so the depth of these blends
        // goes into it. The depth target still has the world, so a new copy is the nearer of
        // the two, as `renderStage2` of the Direct3D 9 path merges its later scenes.
        if self.world_depth_kept {
            self.copy_depth(1, "depth: the world again, after late sorted blends");
            self.open_pass(None, None, None);
        }
    }
}

impl Player {
    /// Draws a run of the table's sorted entries into the pass ([`Sorted::Only`]), with the
    /// view of the main pass. Nothing without entries or without that view.
    fn draw_sorted_run(&mut self, view: Option<RetainedView>, entries: Vec<(u64, u32, u32, u32)>) {
        if entries.is_empty() {
            return;
        }
        let Some(mut v) = view else { return };
        v.sorted = Sorted::Only;
        v.target = self.pass_target;
        self.end_run();
        self.retained.kept_sorted = entries;
        let Some(pass) = self.pass.as_mut() else { return };
        pass.push_debug_group("retained sorted blends");
        draw_retained(pass, &mut self.renderer, &mut self.retained, &mut self.frame_args, &v);
        pass.pop_debug_group();
        self.forget_bound();
    }

    /// Draws a run of places of the table's blends in the engine's walk ([`Item::WalkPlaces`]):
    /// the slots the main view left out for them, in the walk's order, with the main view.
    /// The kept sorted blends stay for their item.
    /// Engine-rules R2, R3 (the skill wgpu-renderer-dev).
    pub(super) fn draw_walk_run(&mut self, run: usize) {
        let (entries, late) = self.retained.walk.take_run(run);
        self.renderer.counts.walk_late += late as u64;
        if entries.is_empty() {
            return;
        }
        let kept = core::mem::take(&mut self.retained.kept_sorted);
        self.draw_sorted_run(self.retained.walk_view, entries);
        self.retained.kept_sorted = kept;
    }
}

/// The key of a sorted blend of the table: the far draw first (the bits of the depth of the
/// bound centre inverted; a depth below 0 counts as 0), ties by the id of the instance, the
/// order in which the engine's accumulator registered them.
fn sorted_key(centre: f32, id: u32) -> u64 {
    (!centre.max(0.0).to_bits() as u64) << 32 | id as u64
}

/// The key of a bucket of the other draws: the groups in order (0 no blend and no alpha test,
/// 1 an alpha test, 2 a blend out of the sort), the first two front to back by the depth of
/// their nearest draw, the blends in the order of their buckets.
fn bucket_key(group: u8, bucket: u32, depth: f32) -> u64 {
    if group == 2 {
        2 << 32 | bucket as u64
    } else {
        (group as u64) << 32 | depth.to_bits() as u64
    }
}

/// One step of the sorted blends: a run of the table's entries, or one draw of the engine's
/// path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pick {
    Table(usize, usize),
    Engine(usize),
}

/// The order of the sorted blends from the keys of the two lists (far first: a smaller key
/// is farther). The table's keys are sorted; the engine's keys are in the engine's order and
/// stay in it. Before each draw of the engine, the table's entries with a key not larger
/// than its key go as one run.
fn merge_picks(kept: &[u64], deferred: &[u64]) -> Vec<Pick> {
    let mut picks = Vec::new();
    let mut i = 0;
    for (j, &key) in deferred.iter().enumerate() {
        let start = i;
        while i < kept.len() && kept[i] <= key {
            i += 1;
        }
        if i > start {
            picks.push(Pick::Table(start, i));
        }
        picks.push(Pick::Engine(j));
    }
    if i < kept.len() {
        picks.push(Pick::Table(i, kept.len()));
    }
    picks
}

#[cfg(test)]
mod merge_tests {
    use super::{merge_picks, Pick};

    #[test]
    fn the_engine_order_stays_when_its_keys_are_not_sorted() {
        // A sort adjust node in mode 64: its shapes come in the order of the tree, and their
        // own depths are not sorted. No table entry.
        assert_eq!(merge_picks(&[], &[30, 10, 20]), vec![Pick::Engine(0), Pick::Engine(1), Pick::Engine(2)]);
    }

    #[test]
    fn the_table_entries_go_before_the_first_nearer_draw_of_the_engine() {
        assert_eq!(
            merge_picks(&[5, 15, 40], &[10, 30]),
            vec![Pick::Table(0, 1), Pick::Engine(0), Pick::Table(1, 2), Pick::Engine(1), Pick::Table(2, 3)]
        );
    }

    #[test]
    fn a_run_of_table_entries_is_one_pick() {
        assert_eq!(merge_picks(&[1, 2, 3], &[9]), vec![Pick::Table(0, 3), Pick::Engine(0)]);
        assert_eq!(merge_picks(&[1, 2], &[]), vec![Pick::Table(0, 2)]);
    }

    #[test]
    fn sorted_engine_keys_give_the_merge_of_before() {
        // The keys of the engine are sorted when no node orders a subtree: the picks are
        // the merge of two sorted lists, as before the change.
        assert_eq!(
            merge_picks(&[2, 4, 6], &[1, 3, 5, 7]),
            vec![
                Pick::Engine(0),
                Pick::Table(0, 1),
                Pick::Engine(1),
                Pick::Table(1, 2),
                Pick::Engine(2),
                Pick::Table(2, 3),
                Pick::Engine(3)
            ]
        );
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    #[test]
    fn a_farther_sorted_blend_has_the_smaller_key() {
        assert!(sorted_key(100.0, 1) < sorted_key(10.0, 1));
        assert!(sorted_key(10.0, 1) < sorted_key(0.0, 1));
        // A depth below 0 counts as 0.
        assert_eq!(sorted_key(-5.0, 1), sorted_key(0.0, 1));
    }

    #[test]
    fn sorted_blends_at_one_depth_keep_the_order_of_their_ids() {
        assert!(sorted_key(10.0, 1) < sorted_key(10.0, 2));
        assert!(sorted_key(10.0, 2) < sorted_key(10.0, 300));
    }

    #[test]
    fn the_groups_of_the_buckets_come_in_order() {
        let opaque = bucket_key(0, 5, 1000.0);
        let alpha_test = bucket_key(1, 3, 1.0);
        let blend = bucket_key(2, 0, 0.0);
        assert!(opaque < alpha_test && alpha_test < blend);
    }

    #[test]
    fn the_walk_finds_the_group_of_the_blends_out_of_the_sort() {
        assert_eq!(bucket_key(2, 7, 1.0) >> 32, crate::engine::walk::GROUP_IN_PLACE);
        assert_ne!(bucket_key(0, 7, f32::MAX) >> 32, crate::engine::walk::GROUP_IN_PLACE);
        assert_ne!(bucket_key(1, 7, f32::MAX) >> 32, crate::engine::walk::GROUP_IN_PLACE);
    }

    #[test]
    fn the_opaque_buckets_go_front_to_back_and_the_blends_by_bucket() {
        assert!(bucket_key(0, 9, 1.0) < bucket_key(0, 2, 50.0));
        assert!(bucket_key(1, 9, 1.0) < bucket_key(1, 2, 50.0));
        assert!(bucket_key(2, 2, 50.0) < bucket_key(2, 9, 1.0));
    }

    const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

    #[test]
    fn the_identity_view_is_the_unit_box_in_front_of_the_near_plane() {
        // The planes of the identity: x and y from -1 to 1, z from 0 to 1.
        let planes = frustum_planes(&IDENTITY);
        assert!(in_view(&[0.0, 0.0, 0.5, 0.1], &planes));
        assert!(!in_view(&[3.0, 0.0, 0.5, 0.1], &planes));
        assert!(!in_view(&[0.0, 0.0, -0.5, 0.1], &planes));
        assert!(!in_view(&[0.0, 0.0, 1.5, 0.1], &planes));
    }

    #[test]
    fn a_bound_that_crosses_a_plane_is_in_view() {
        let planes = frustum_planes(&IDENTITY);
        assert!(in_view(&[1.05, 0.0, 0.5, 0.1], &planes));
        assert!(in_view(&[0.0, 0.0, -0.05, 0.1], &planes));
        assert!(!in_view(&[1.2, 0.0, 0.5, 0.1], &planes));
    }

    #[test]
    fn the_planes_have_unit_normals() {
        let projection = [1.5, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.001, 1.0, 0.0, 0.0, -1.001, 0.0];
        for plane in frustum_planes(&projection) {
            let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            assert!((length - 1.0).abs() < 1e-5, "{plane:?}");
        }
    }

    #[test]
    fn the_kind_of_a_slot_names_its_blend_form_and_its_depth_write() {
        assert_eq!(slot_kind([0, 0, 0], false), 0);
        assert_eq!(slot_kind([0, 5, 6], true), 1);
        assert_eq!(slot_kind([1, 5, 6], false), 2);
        assert_eq!(slot_kind([1, 5, 6], true), 3);
        assert_eq!(slot_kind([1, 2, 6], false), 4);
        assert_eq!(slot_kind([1, 2, 2], true), 7);
        assert_eq!(slot_kind([1, 5, 2], false), 6);
    }

    fn light_words() -> Vec<u8> {
        // A `D3DLIGHT8`: the type, the diffuse, the specular, the ambient, the position, the
        // direction, the range, the falloff, the attenuation, theta, phi.
        let mut words = [0u32; 26];
        words[0] = 1;
        for (i, value) in [0.5f32, 0.25, 0.125, 1.0].iter().enumerate() {
            words[1 + i] = value.to_bits();
        }
        words[5] = 256.0f32.to_bits();
        for (i, value) in [0.1f32, 0.2, 0.3, 1.0].iter().enumerate() {
            words[9 + i] = value.to_bits();
        }
        for (i, value) in [10.0f32, 20.0, 30.0].iter().enumerate() {
            words[13 + i] = value.to_bits();
        }
        for (i, value) in [0.0f32, 0.0, -1.0].iter().enumerate() {
            words[16 + i] = value.to_bits();
        }
        words[19] = 512.0f32.to_bits();
        words[20] = 2.0f32.to_bits();
        for (i, value) in [1.0f32, 0.01, 0.0].iter().enumerate() {
            words[21 + i] = value.to_bits();
        }
        words[24] = 1.0f32.to_bits();
        words[25] = 2.0f32.to_bits();
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn a_light_record_becomes_the_light_of_the_shader() {
        let light = light_data(&light_words(), None);
        assert_eq!(light.diffuse, [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(light.ambient, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(light.position, [10.0, 20.0, 30.0, 1.0]);
        assert_eq!(light.direction, [0.0, 0.0, -1.0, 512.0]);
        assert_eq!(light.attenuation, [1.0, 0.01, 0.0, 2.0]);
        assert!((light.cone[0] - 0.5f32.cos()).abs() < 1e-6 && (light.cone[1] - 1.0f32.cos()).abs() < 1e-6);
        assert_eq!(light.cone[2], 256.0);
    }

    #[test]
    fn the_position_of_a_light_is_relative_to_the_origin() {
        let light = light_data(&light_words(), Some([1.0, 2.0, 3.0]));
        assert_eq!(light.position, [9.0, 18.0, 27.0, 1.0]);
    }

    fn retained(flags: u32) -> Retained {
        Retained {
            class: 0,
            flags,
            geometry: 0,
            material: 0,
            rotation: [1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0, 0.0, 7.0, 8.0, 9.0, 0.0],
            exact: [100.5, 200.25, 300.125],
            stored: [100.0, 200.0, 300.0],
            colours: [0.0; 14],
            bound: [0.0; 4],
        }
    }

    #[test]
    fn a_relative_instance_takes_its_exact_translation_less_the_origin() {
        let world = retained(1).world_at(&[100.0, 200.0, 300.0]);
        assert_eq!(&world[..12], &[1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0, 0.0, 7.0, 8.0, 9.0, 0.0]);
        assert_eq!(&world[12..], &[0.5, 0.25, 0.125, 1.0]);
    }

    #[test]
    fn an_instance_that_is_not_relative_takes_the_stored_translation() {
        assert_eq!(retained(0).exact_translation(), [100.0, 200.0, 300.0]);
        assert_eq!(&retained(0).world_at(&[0.0, 0.0, 0.0])[12..15], &[100.0, 200.0, 300.0]);
    }

    #[test]
    fn the_extension_text_reads_its_lights_and_its_values_through_functions() {
        let text = "let a = lights[idx[i]].diffuse + draws[record].extension[k].x; let v = draws[record].view;";
        assert_eq!(rewrite_extension_text(text), "let a = light_at(idx[i]).diffuse + draw_extension(k).x; let v = draw_view();");
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
        let retained_view = RetainedView { scene_slot: slot, classes: view.classes, planes, flip_cull: view.flip_cull, force_depth_test: view.force_depth_test, target: None, sorted: Sorted::Together, samples: view.samples };
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
