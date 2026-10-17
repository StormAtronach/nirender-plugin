//! The lights of a frame on the GPU: the clusters of the lights of the draws (binned on
//! the CPU into a grid of tiles and depth slices) and the lights of the frame for the
//! extension, with clusters of their own. The buffers and the binning only; the light
//! values come from the records (`LightData`).

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use crate::{light_reach, Counts, LightData};

/// The grid of the clusters: tiles across and down the screen, slices of depth.
pub(crate) const CLUSTER_GRID: [u32; 3] = [16, 9, 24];
/// The most lights of a cluster.
pub(crate) const CLUSTER_LIGHTS: usize = 64;

/// The values of the clusters for the shader: the grid (w: 1 for the clusters of the draws, 2
/// for those of the extension, 0 when they are off), the tiles for each pixel across and
/// down, then the scale, the bias and the least depth of the slice of a depth.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct ClusterData {
    pub(crate) grid: [u32; 4],
    pub(crate) screen: [f32; 4],
    pub(crate) depth: [f32; 4],
}

/// Bins lights into the grid of the clusters of a projection of Direct3D. Each item is the
/// entry of a light, its centre in the space of the view (`None` for a directional light,
/// which is in each cluster) and its reach. `lists` has a list for each cluster. Returns the
/// table (the first entry and the number for each cluster), the entries, and the values for
/// the shader with `w` in the grid.
pub(crate) fn bin_lights(lists: &mut [Vec<u32>], items: impl Iterator<Item = (u32, Option<[f32; 3]>, f32)>, projection: &[f32; 16], size: (u32, u32), w: u32) -> (Vec<[u32; 2]>, Vec<u32>, ClusterData) {
    for list in lists.iter_mut() {
        list.clear();
    }
    let (gx, gy, gz) = (CLUSTER_GRID[0] as usize, CLUSTER_GRID[1] as usize, CLUSTER_GRID[2] as usize);
    // The near and far planes of a projection of Direct3D.
    let near = if projection[10] != 0.0 { (-projection[14] / projection[10]).max(1.0) } else { 1.0 };
    let far = if projection[10] != 1.0 { (projection[14] / (1.0 - projection[10])).max(near * 2.0) } else { near * 1000.0 };
    let depth_scale = gz as f32 / (far / near).ln();
    let slice_of = |z: f32| ((z.max(near) / near).ln() * depth_scale).floor().clamp(0.0, gz as f32 - 1.0) as usize;
    let (sx, sy) = (projection[0], projection[5]);
    for (entry, centre, reach) in items {
        let (x0, x1, y0, y1, z0, z1);
        match centre {
            None => (x0, x1, y0, y1, z0, z1) = (0, gx - 1, 0, gy - 1, 0, gz - 1),
            Some(c) => {
                if c[2] + reach < near || c[2] - reach > far {
                    continue;
                }
                z0 = slice_of(c[2] - reach);
                z1 = slice_of(c[2] + reach);
                if c[2] - reach < near {
                    (x0, x1, y0, y1) = (0, gx - 1, 0, gy - 1);
                } else {
                    let nearest = c[2] - reach;
                    let (half_w, half_h) = (reach * sx / nearest, reach * sy / nearest);
                    let (cx, cy) = (c[0] * sx / c[2], c[1] * sy / c[2]);
                    let tile_x = |ndc: f32| (((ndc + 1.0) * 0.5 * gx as f32).floor()).clamp(0.0, gx as f32 - 1.0) as usize;
                    let tile_y = |ndc: f32| (((1.0 - ndc) * 0.5 * gy as f32).floor()).clamp(0.0, gy as f32 - 1.0) as usize;
                    (x0, x1) = (tile_x(cx - half_w), tile_x(cx + half_w));
                    (y0, y1) = (tile_y(cy + half_h), tile_y(cy - half_h));
                }
            }
        }
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let list = &mut lists[(z * gy + y) * gx + x];
                    if list.len() < CLUSTER_LIGHTS {
                        list.push(entry);
                    }
                }
            }
        }
    }
    let mut table: Vec<[u32; 2]> = Vec::with_capacity(lists.len());
    let mut indexes: Vec<u32> = Vec::new();
    for list in lists.iter() {
        table.push([indexes.len() as u32, list.len() as u32]);
        indexes.extend_from_slice(list);
    }
    let data = ClusterData {
        grid: [CLUSTER_GRID[0], CLUSTER_GRID[1], CLUSTER_GRID[2], w],
        screen: [gx as f32 / size.0.max(1) as f32, gy as f32 / size.1.max(1) as f32, 0.0, 0.0],
        depth: [depth_scale, -near.ln() * depth_scale, near, 0.0],
    };
    (table, indexes, data)
}

/// A storage buffer with room for `count` items of `item` bytes.
pub(crate) fn storage_buffer(device: &wgpu::Device, label: &str, count: usize, item: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (count.max(1) * item) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

/// The clusters of the lights of the frame: for each cluster of the grid, the lights of
/// the frame whose sphere touches it, as indexes into the light buffer of the frame.
/// Binned on the CPU in one space, relative to the origin of the scene, for the view and
/// the projection of the scene (the start of the world).
pub(crate) struct Clusters {
    pub(crate) on: bool,
    /// `MGE_XE_CLUSTERS` (0 or 1): it overrides `FrameParameters::clustered_lights`.
    pub(crate) forced: Option<bool>,
    pub(crate) uniform: wgpu::Buffer,
    pub(crate) table: wgpu::Buffer,
    pub(crate) indexes: wgpu::Buffer,
    pub(crate) index_capacity: usize,
    /// The lights of the frame that are candidates, by their key: the index in the light
    /// buffer of the frame, the slot of the light in the masks of the draws, and whether
    /// its position is relative to the origin of the scene (the light buffer of the frame
    /// has some relative and some not, as the draws are).
    pub(crate) unique: HashMap<u32, (u32, u32, bool)>,
    /// The origin of the scene of the candidates, which the relative positions are
    /// relative to.
    pub(crate) origin: Option<[f64; 3]>,
    /// The frame has more candidates than a mask has bits, or an index a cluster entry
    /// cannot hold: the frame keeps the lights of the draws.
    pub(crate) overflow: bool,
    pub(crate) lists: Vec<Vec<u32>>,
}

/// The bits of the mask of a draw over the clustered lights of the frame.
pub(crate) const CLUSTER_SLOTS: usize = 256;

impl Clusters {
    pub(crate) fn new(device: &wgpu::Device) -> Clusters {
        let cells = (CLUSTER_GRID[0] * CLUSTER_GRID[1] * CLUSTER_GRID[2]) as usize;
        let index_capacity = 1 << 16;
        let forced = crate::switches().clusters;
        Clusters {
            // On when the extension asks for them (`FrameParameters::clustered_lights`). With
            // the masks the clusters give the picture of the lights of the draws; at Narsis by
            // night (13 lights for a draw at most, before the retained instances) the GPU took
            // the same time either way while the binning cost the helper 0.4 ms a frame.
            on: forced.unwrap_or(false),
            forced,
            uniform: device.create_buffer(&wgpu::BufferDescriptor { label: Some("clusters"), size: core::mem::size_of::<ClusterData>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            table: storage_buffer(device, "cluster table", cells, 8),
            indexes: storage_buffer(device, "cluster lights", index_capacity, 4),
            index_capacity,
            unique: HashMap::new(),
            origin: None,
            overflow: false,
            lists: (0..cells).map(|_| Vec::new()).collect(),
        }
    }

    /// Notes a light of the frame as a candidate, by its key and its index in the light
    /// buffer: the first draw with the key gives the values. `origin` is that of the scene
    /// of the draw; `relative` says that the position of the light is relative to it.
    /// Returns the slot of the light in the masks of the draws, or `None` when the frame
    /// has no slot left.
    pub(crate) fn note(&mut self, key: u32, index: u32, relative: bool, origin: Option<[f64; 3]>) -> Option<u32> {
        let next = self.unique.len() as u32;
        let (_, slot, _) = *self.unique.entry(key).or_insert((index, next, relative));
        self.origin = origin;
        if slot as usize >= CLUSTER_SLOTS || index >= 1 << 16 {
            self.overflow = true;
            return None;
        }
        Some(slot)
    }

    /// Bins the candidates into the grid for the view and the projection of the scene and
    /// writes the buffers. `size` is the size of the picture. Returns false when the
    /// clusters are off for the frame (no start of the world).
    pub(crate) fn build(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, lights: &[LightData], view: Option<([f32; 16], [f32; 16])>, size: (u32, u32), counts: &mut Counts) -> bool {
        let overflow = core::mem::replace(&mut self.overflow, false);
        if overflow {
            counts.cluster_frames_many += 1;
        }
        let Some((mut view, projection)) = view.filter(|_| self.on && !self.unique.is_empty() && !overflow) else {
            self.unique.clear();
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&ClusterData::zeroed()));
            return false;
        };
        // Relative to the origin of the scene, the camera: the view without its move.
        let origin = self.origin;
        if origin.is_some() {
            view[12] = 0.0;
            view[13] = 0.0;
            view[14] = 0.0;
        }
        // The candidates by their slot; an entry of a list is the slot in its high half and
        // the index in its low half.
        let mut candidates: Vec<(u32, u32, bool)> = self.unique.values().map(|&(index, slot, relative)| (slot, index, relative)).collect();
        candidates.sort_unstable();
        // A point light without a cut-off (the fade of the extension is off) has no end:
        // its tail would reach the pixels of objects the game never attached it to. The
        // frame keeps the lights of the draws then.
        if candidates.iter().any(|&(_, index, _)| lights.get(index as usize).is_some_and(|light| light.position[3] != 3.0 && light.cone[0] <= 0.0)) {
            self.unique.clear();
            counts.cluster_frames_off += 1;
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&ClusterData::zeroed()));
            return false;
        }
        counts.cluster_lights += candidates.len() as u64;
        let items = candidates.iter().filter_map(|&(slot, index, relative)| {
            let light = lights.get(index as usize)?;
            let centre = (light.position[3] != 3.0).then(|| {
                let mut position = light.position;
                if let (Some(o), false) = (origin, relative) {
                    for axis in 0..3 {
                        position[axis] = (position[axis] as f64 - o[axis]) as f32;
                    }
                }
                view_point(&view, position)
            });
            // The reach: the cut-off of the extension (its fade).
            Some((index | slot << 16, centre, 1.0 / light.cone[0]))
        });
        let (table, indexes, data) = bin_lights(&mut self.lists, items, &projection, size, 1);
        self.unique.clear();
        if indexes.len() > self.index_capacity {
            self.index_capacity = indexes.len().next_power_of_two();
            self.indexes = storage_buffer(device, "cluster lights", self.index_capacity, 4);
        }
        counts.cluster_entries += indexes.len() as u64;
        queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
        if !indexes.is_empty() {
            queue.write_buffer(&self.indexes, 0, bytemuck::cast_slice(&indexes));
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&data));
        true
    }
}

/// A point in the space of a view (the layout of Direct3D).
pub(crate) fn view_point(view: &[f32; 16], p: [f32; 4]) -> [f32; 3] {
    [
        p[0] * view[0] + p[1] * view[4] + p[2] * view[8] + view[12],
        p[0] * view[1] + p[1] * view[5] + p[2] * view[9] + view[13],
        p[0] * view[2] + p[1] * view[6] + p[2] * view[10] + view[14],
    ]
}

/// The lights of the frame for the extension (`FrameLights`): one copy of each light that a
/// world draw has, its position relative to the origin of the scene (the light buffer of the
/// frame has some relative and some not, as the draws are), and clusters of their own.
pub(crate) struct ExtensionLights {
    /// The lights of the frame by their key: the index of the first copy in the light buffer
    /// of the frame, and whether its position is relative to the origin of the scene.
    pub(crate) noted: HashMap<u32, (u32, bool)>,
    pub(crate) lights: wgpu::Buffer,
    pub(crate) light_capacity: usize,
    pub(crate) uniform: wgpu::Buffer,
    pub(crate) table: wgpu::Buffer,
    pub(crate) indexes: wgpu::Buffer,
    pub(crate) index_capacity: usize,
    pub(crate) lists: Vec<Vec<u32>>,
    /// The origin of the scene of the noted lights, which the positions of the relative ones
    /// are relative to.
    pub(crate) scene_origin: Option<[f64; 3]>,
    /// The lights and the clusters are those of this frame, and the point that the
    /// positions are relative to.
    pub(crate) built: bool,
    pub(crate) origin: [f64; 3],
}

impl ExtensionLights {
    pub(crate) fn new(device: &wgpu::Device) -> ExtensionLights {
        let cells = (CLUSTER_GRID[0] * CLUSTER_GRID[1] * CLUSTER_GRID[2]) as usize;
        ExtensionLights {
            noted: HashMap::new(),
            lights: storage_buffer(device, "extension lights", 64, core::mem::size_of::<LightData>()),
            light_capacity: 64,
            uniform: device.create_buffer(&wgpu::BufferDescriptor { label: Some("extension clusters"), size: core::mem::size_of::<ClusterData>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            table: storage_buffer(device, "extension cluster table", cells, 8),
            indexes: storage_buffer(device, "extension cluster lights", 1 << 12, 4),
            index_capacity: 1 << 12,
            lists: (0..cells).map(|_| Vec::new()).collect(),
            scene_origin: None,
            built: false,
            origin: [0.0; 3],
        }
    }

    /// The start of a frame: no lights.
    pub(crate) fn clear(&mut self) {
        self.noted.clear();
        self.built = false;
    }

    /// Notes a light of a world draw, by its key and its index in the light buffer of the
    /// frame: the first draw with the key gives the values. `origin` is that of the scene of
    /// the draw; `relative` says that the position of the light is relative to it.
    pub(crate) fn note(&mut self, key: u32, index: u32, relative: bool, origin: Option<[f64; 3]>) {
        self.noted.entry(key).or_insert((index, relative));
        self.scene_origin = origin;
    }

    /// Makes the lights of the frame for the extension, one copy of each relative to the
    /// origin of the scene, and bins them for the view and the projection that the extension
    /// gets (`SceneView`).
    pub(crate) fn build(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame_lights: &[LightData], view: [f32; 16], projection: [f32; 16], size: (u32, u32), counts: &mut Counts) {
        self.built = false;
        let noted = core::mem::take(&mut self.noted);
        if noted.is_empty() {
            return;
        }
        let origin = self.scene_origin;
        let mut view = view;
        // Relative to the origin of the scene, the camera: the view without its move.
        if origin.is_some() {
            view[12] = 0.0;
            view[13] = 0.0;
            view[14] = 0.0;
        }
        let mut first: Vec<(u32, bool)> = noted.into_values().collect();
        first.sort_unstable();
        let mut lights: Vec<LightData> = Vec::with_capacity(first.len());
        let mut items = Vec::with_capacity(first.len());
        for (index, relative) in first {
            let Some(&light) = frame_lights.get(index as usize) else { continue };
            let mut light = light;
            let centre = (light.position[3] != 3.0).then(|| {
                if let (Some(o), false) = (origin, relative) {
                    for axis in 0..3 {
                        light.position[axis] = (light.position[axis] as f64 - o[axis]) as f32;
                    }
                }
                view_point(&view, light.position)
            });
            items.push((lights.len() as u32, centre, light_reach(&light)));
            lights.push(light);
        }
        let (table, indexes, data) = bin_lights(&mut self.lists, items.into_iter(), &projection, size, 2);
        if lights.len() > self.light_capacity {
            self.light_capacity = lights.len().next_power_of_two();
            self.lights = storage_buffer(device, "extension lights", self.light_capacity, core::mem::size_of::<LightData>());
        }
        if indexes.len() > self.index_capacity {
            self.index_capacity = indexes.len().next_power_of_two();
            self.indexes = storage_buffer(device, "extension cluster lights", self.index_capacity, 4);
        }
        counts.extension_lights += lights.len() as u64;
        if !lights.is_empty() {
            queue.write_buffer(&self.lights, 0, bytemuck::cast_slice(&lights));
        }
        queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
        if !indexes.is_empty() {
            queue.write_buffer(&self.indexes, 0, bytemuck::cast_slice(&indexes));
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&data));
        self.origin = origin.unwrap_or([0.0; 3]);
        self.built = true;
    }
}

/// A buffer for the lights of a frame.
pub(crate) fn light_buffer(device: &wgpu::Device, lights: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lights"),
        size: (lights * core::mem::size_of::<LightData>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
