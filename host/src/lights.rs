//! The lights of a frame on the GPU: the clusters of the lights of the draws (binned on
//! the CPU into a grid of tiles and depth slices) and the lights of the frame for the
//! extension, with clusters of their own. The buffers and the binning only; the light
//! values come from the records ([`LightData`]).

use crate::objects::Create;
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use crate::{light_reach, Counts, FrameLights, LightData};

/// The grid of the clusters: tiles across and down the screen, slices of depth.
pub(crate) const CLUSTER_GRID: [u32; 3] = [16, 9, 24];
/// The most lights of a cluster.
pub(crate) const CLUSTER_LIGHTS: usize = 64;

/// The values of the clusters for the shader: the grid (w: 1 for the clusters of the draws, 2
/// for those of the extension, 3 for the lights of the world, 0 when they are off), the tiles
/// for each pixel across and down, the scale, the bias and the least depth of the slice of a
/// depth, and for the lights of the world the translation from the space of the table's
/// lights to the space of the stream's draws relative to the camera.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct ClusterData {
    pub(crate) grid: [u32; 4],
    pub(crate) screen: [f32; 4],
    pub(crate) depth: [f32; 4],
    pub(crate) offset: [f32; 4],
    /// For the lights of the world: the origin of the retained set in the world, the
    /// translation to the space of a draw in world space ([`crate::WORLD_SPACE_LIGHTS_BIT`]).
    pub(crate) origin: [f32; 4],
}

/// [`ClusterData::grid`] w of the clusters of the lights of the world.
pub(crate) const GRID_WORLD_LIGHTS: u32 = 3;

/// Bins lights into the grid of the clusters of a projection of Direct3D. Each item is the
/// entry of a light, its centre in the space of the view ([`None`] for a directional light,
/// which is in each cluster) and its reach. `lists` has a list for each cluster. Returns the
/// table (the first entry and the number for each cluster), the entries, and the values for
/// the shader with `w` in the grid. An entry that finds its cluster full goes into `dropped`.
pub(crate) fn bin_lights(
    lists: &mut [Vec<u32>],
    items: impl Iterator<Item = (u32, Option<[f32; 3]>, f32)>,
    projection: &[f32; 16],
    depth_range: [f32; 2],
    size: (u32, u32),
    w: u32,
    dropped: &mut u64,
) -> (Vec<[u32; 2]>, Vec<u32>, ClusterData) {
    for list in lists.iter_mut() {
        list.clear();
    }
    let (gx, gy, gz) = (CLUSTER_GRID[0] as usize, CLUSTER_GRID[1] as usize, CLUSTER_GRID[2] as usize);
    // The near and far planes of the camera ([`crate::WorldFacts::depth_range`]); a frustum of
    // no depth (a camera whose record has none) still gives slices.
    let near = depth_range[0].max(f32::MIN_POSITIVE);
    let far = depth_range[1].max(near * 2.0);
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
                // A light whose sphere is outside a side plane of the view is in no cluster: the
                // clamps below would put it into the clusters at the edge, or into all of them
                // when it reaches the near plane. A side plane of the view is `x * sx = z`.
                let (across, down) = ((sx * sx + 1.0).sqrt(), (sy * sy + 1.0).sqrt());
                if (sx * c[0] - c[2]) / across > reach
                    || (-sx * c[0] - c[2]) / across > reach
                    || (sy * c[1] - c[2]) / down > reach
                    || (-sy * c[1] - c[2]) / down > reach
                {
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
                    } else {
                        *dropped += 1;
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
        offset: [0.0; 4],
        origin: [0.0; 4],
    };
    (table, indexes, data)
}

/// A storage buffer with room for `count` items of `item` bytes.
pub(crate) fn storage_buffer(device: &wgpu::Device, label: &str, count: usize, item: usize) -> wgpu::Buffer {
    device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (count.max(1) * item) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The clusters of the lights of the frame: for each cluster of the grid, the lights of
/// the frame whose sphere touches it, as indexes into the light buffer of the frame.
/// Binned on the CPU in one space, relative to the origin of the scene, for the view and
/// the projection of the scene (the start of the world).
pub(crate) struct Clusters {
    pub(crate) on: bool,
    /// `MGE_XE_CLUSTERS` (0 or 1): it overrides `FrameParameters::clustered_lights`.
    pub(crate) forced: Option<bool>,
    /// `MGE_XE_LIGHTS=world`: the clusters hold the point lights of the world
    /// (`build_world`) and a draw takes them without a mask.
    pub(crate) world: bool,
    /// The clusters of this frame hold the lights of the world.
    pub(crate) world_built: bool,
    /// The point lights of the world with no end that the last `build_world` left out of the
    /// clusters ([`world_bin_reach`]): the next frame's draws lit for each pixel take them in
    /// their own lists when this is not 0. 0 after a frame without the world's clusters.
    pub(crate) world_no_end: u32,
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
            // the masks the clusters give the picture of the lights of the draws. Off by
            // default: the binning costs CPU time.
            on: forced.unwrap_or(false),
            forced,
            world: crate::switches().world_lights,
            world_built: false,
            world_no_end: 0,
            uniform: device.counted_buffer(&wgpu::BufferDescriptor {
                label: Some("clusters"),
                size: core::mem::size_of::<ClusterData>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
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
    /// Returns the slot of the light in the masks of the draws, or [`None`] when the frame
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
    pub(crate) fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lights: &[LightData],
        view: Option<([f32; 16], [f32; 16], [f32; 2])>,
        size: (u32, u32),
        counts: &mut Counts,
    ) -> bool {
        self.world_built = false;
        self.world_no_end = 0;
        let overflow = core::mem::replace(&mut self.overflow, false);
        if overflow {
            counts.cluster_frames_many += 1;
        }
        let Some((mut view, projection, depth_range)) = view.filter(|_| self.on && !self.unique.is_empty() && !overflow)
        else {
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
        let mut candidates: Vec<(u32, u32, bool)> = self
            .unique
            .values()
            .map(|&(index, slot, relative)| (slot, index, relative))
            .collect();
        candidates.sort_unstable();
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
            // The reach: the cut-off of the extension (its fade), or where the light's own
            // attenuation ends (`cluster_reach`). These clusters have masks: a draw takes only
            // the lights of its own set, so a reach past the objects the game attached the light
            // to lights nothing more.
            Some((index | slot << 16, centre, crate::extension::cluster_reach(light)))
        });
        let (table, indexes, data) = bin_lights(
            &mut self.lists,
            items,
            &projection,
            depth_range,
            size,
            1,
            &mut counts.cluster_dropped,
        );
        self.unique.clear();
        self.write(device, queue, &table, &indexes, &data, counts);
        true
    }

    /// Bins the point lights of the world (`MGE_XE_LIGHTS=world`) for the view and the
    /// projection of the scene. `lights` is the table's light buffer as the extension shaded
    /// it for the main view, `slots` the slots of the lights of the world in it. `camera` is
    /// the origin of the scene when its draws are relative to the camera; `to_scene` the
    /// translation from the space of `lights` to that space. An entry is a slot: the shader
    /// reads `lights_retained` and moves a light of a draw of the stream by `to_scene`, or by
    /// `to_world` (the origin of the set) for a draw in the world's own space.
    /// Returns false when the frame has no start of the world.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_world(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lights: &[LightData],
        slots: &[u32],
        view: Option<([f32; 16], [f32; 16], [f32; 2])>,
        camera: Option<[f64; 3]>,
        to_scene: [f64; 3],
        to_world: [f64; 3],
        size: (u32, u32),
        counts: &mut Counts,
    ) -> bool {
        self.unique.clear();
        self.overflow = false;
        self.world_built = false;
        self.world_no_end = 0;
        let Some((mut view, projection, depth_range)) = view else {
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&ClusterData::zeroed()));
            return false;
        };
        self.world_built = true;
        if camera.is_some() {
            view[12] = 0.0;
            view[13] = 0.0;
            view[14] = 0.0;
        }
        counts.cluster_lights += slots.len() as u64;
        let no_end = slots
            .iter()
            .filter_map(|&slot| lights.get(slot as usize))
            .filter(|light| world_bin_reach(light).is_none())
            .count() as u32;
        self.world_no_end = no_end;
        counts.world_no_end_lights += u64::from(no_end);
        let items = slots.iter().filter_map(|&slot| {
            let light = lights.get(slot as usize)?;
            let reach = world_bin_reach(light)?;
            let p = light.position;
            let centre = [
                (p[0] as f64 + to_scene[0]) as f32,
                (p[1] as f64 + to_scene[1]) as f32,
                (p[2] as f64 + to_scene[2]) as f32,
                p[3],
            ];
            Some((slot, Some(view_point(&view, centre)), reach))
        });
        let (table, indexes, mut data) = bin_lights(
            &mut self.lists,
            items,
            &projection,
            depth_range,
            size,
            GRID_WORLD_LIGHTS,
            &mut counts.cluster_dropped,
        );
        data.offset = [to_scene[0] as f32, to_scene[1] as f32, to_scene[2] as f32, 0.0];
        data.origin = [to_world[0] as f32, to_world[1] as f32, to_world[2] as f32, 0.0];
        self.write(device, queue, &table, &indexes, &data, counts);
        true
    }

    fn write(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        table: &[[u32; 2]],
        indexes: &[u32],
        data: &ClusterData,
        counts: &mut Counts,
    ) {
        if indexes.len() > self.index_capacity {
            self.index_capacity = indexes.len().next_power_of_two();
            self.indexes = storage_buffer(device, "cluster lights", self.index_capacity, 4);
        }
        counts.cluster_entries += indexes.len() as u64;
        queue.write_buffer(&self.table, 0, bytemuck::cast_slice(table));
        if !indexes.is_empty() {
            queue.write_buffer(&self.indexes, 0, bytemuck::cast_slice(indexes));
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(data));
    }
}

/// The reach of a light of the world in the world's clusters, which have no masks: a pixel
/// takes each light of its cluster. Only a light with an end (the cut-off of the extension's
/// fade, `cone[3]`) goes into them, with that cut-off as its reach. A point light with no end
/// gives [`None`]: in the clusters it would light each pixel of its reach, as far as its
/// attenuation shows (one cell). It reaches a draw through the draw's own list instead: the
/// engine's lights, then the world's lights whose sphere meets the draw's bound
/// ([`world_lights_in_reach`]). The shader takes a light with no end only from that list
/// (W587).
pub(crate) fn world_bin_reach(light: &LightData) -> Option<f32> {
    (light.position[3] == 3.0 || light.cone[3] > 0.0).then(|| crate::extension::cluster_reach(light))
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
    /// The number of lights in `lights` (its capacity can be more).
    pub(crate) count: u32,
}

/// The buffers of [`ExtensionLights`] for one call of the extension. They are handles of
/// their own, so that the caller can use the renderer mutably while it gives them.
pub(crate) struct LightHandles {
    lights: wgpu::Buffer,
    clusters: wgpu::Buffer,
    table: wgpu::Buffer,
    indexes: wgpu::Buffer,
    origin: [f64; 3],
    count: u32,
}

impl LightHandles {
    /// The lights as the extension sees them ([`crate::SceneView::lights`]).
    pub(crate) fn frame_lights(&self) -> FrameLights<'_> {
        FrameLights {
            lights: &self.lights,
            clusters: &self.clusters,
            table: &self.table,
            indexes: &self.indexes,
            origin: self.origin,
            count: self.count,
        }
    }
}

impl ExtensionLights {
    pub(crate) fn new(device: &wgpu::Device) -> ExtensionLights {
        let cells = (CLUSTER_GRID[0] * CLUSTER_GRID[1] * CLUSTER_GRID[2]) as usize;
        ExtensionLights {
            noted: HashMap::new(),
            lights: storage_buffer(device, "extension lights", 64, core::mem::size_of::<LightData>()),
            light_capacity: 64,
            uniform: device.counted_buffer(&wgpu::BufferDescriptor {
                label: Some("extension clusters"),
                size: core::mem::size_of::<ClusterData>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            table: storage_buffer(device, "extension cluster table", cells, 8),
            indexes: storage_buffer(device, "extension cluster lights", 1 << 12, 4),
            index_capacity: 1 << 12,
            lists: (0..cells).map(|_| Vec::new()).collect(),
            scene_origin: None,
            built: false,
            origin: [0.0; 3],
            count: 0,
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
    pub(crate) fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame_lights: &[LightData],
        view: [f32; 16],
        projection: [f32; 16],
        depth_range: [f32; 2],
        size: (u32, u32),
        counts: &mut Counts,
    ) {
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
            let Some(&light) = frame_lights.get(index as usize) else {
                continue;
            };
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
        let (table, indexes, data) = bin_lights(
            &mut self.lists,
            items.into_iter(),
            &projection,
            depth_range,
            size,
            2,
            &mut counts.cluster_dropped,
        );
        if lights.len() > self.light_capacity {
            self.light_capacity = lights.len().next_power_of_two();
            self.lights = storage_buffer(
                device,
                "extension lights",
                self.light_capacity,
                core::mem::size_of::<LightData>(),
            );
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
        self.count = lights.len() as u32;
        self.built = true;
    }

    /// The handles of the lights of this frame, when [`ExtensionLights::build`] made them.
    pub(crate) fn handles(&self) -> Option<LightHandles> {
        self.built.then(|| LightHandles {
            lights: self.lights.clone(),
            clusters: self.uniform.clone(),
            table: self.table.clone(),
            indexes: self.indexes.clone(),
            origin: self.origin,
            count: self.count,
        })
    }
}

/// The most world lights in the list of one draw lit for each vertex: the plugin's cap of the
/// lights of one record draw (`MAX_LIGHTS`, src/light_stream.rs of the plugin).
pub(crate) const VERTEX_WORLD_CAP: usize = 64;

/// A point light of the world for the lists by place: the key of the game's light, its slot in
/// the table's lights, its place in absolute world space, and its reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlacedLight {
    pub(crate) key: u32,
    pub(crate) slot: u32,
    pub(crate) place: [f64; 3],
    pub(crate) reach: f32,
}

impl PlacedLight {
    /// A light of the game's record (the `D3DLIGHT8` after the key, the radius in the first
    /// specular word: player/feed.rs, `tag::LIGHT`). [`None`] for a directional light. The
    /// place is the record's own absolute place; the reach is the radius of the engine's light
    /// (its choice of the lights of a node is by that radius), or without one the distance of
    /// its attenuation ([`crate::extension::cluster_reach`]).
    pub(crate) fn of_record(key: u32, slot: u32, record: &[u8]) -> Option<PlacedLight> {
        let light = crate::retained::light_data(record, None);
        if light.position[3] == 3.0 {
            return None;
        }
        let reach = if light.cone[2] > 0.0 {
            light.cone[2].min(crate::extension::LIGHT_REACH_MAX)
        } else {
            crate::extension::cluster_reach(&light)
        };
        Some(PlacedLight {
            key,
            slot,
            place: core::array::from_fn(|axis| light.position[axis] as f64),
            reach,
        })
    }

    /// The light's sphere meets the sphere at `centre` (absolute) with `radius`.
    pub(crate) fn meets(&self, centre: [f64; 3], radius: f32) -> bool {
        self.gap(centre, radius) <= 0.0
    }

    /// How far the light's sphere is from a sphere at `centre` with `radius`: under 0 when they
    /// overlap.
    fn gap(&self, centre: [f64; 3], radius: f32) -> f64 {
        let distance = (0..3).map(|i| (self.place[i] - centre[i]).powi(2)).sum::<f64>().sqrt();
        distance - self.reach as f64 - radius as f64
    }
}

/// The space of the positions of a draw, for the world's lights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DrawSpace {
    /// Relative to the camera at this place of the world.
    Camera([f64; 3]),
    /// The world's own space: the shadow casters' scene, which has no origin of the camera.
    World,
}

impl DrawSpace {
    /// The place in the world of a point of the draw's space.
    pub(crate) fn to_world(self, point: [f32; 3]) -> [f64; 3] {
        match self {
            DrawSpace::Camera(camera) => core::array::from_fn(|i| point[i] as f64 + camera[i]),
            DrawSpace::World => point.map(f64::from),
        }
    }

    /// The camera that [`crate::retained::light_data`] makes a light relative to: none in the
    /// world's own space.
    pub(crate) fn camera(self) -> Option<[f64; 3]> {
        match self {
            DrawSpace::Camera(camera) => Some(camera),
            DrawSpace::World => None,
        }
    }
}

/// The world's point lights by place for the record draws lit for each vertex, made at the
/// first such draw of a frame, and the list of a draw to reuse.
#[derive(Default)]
pub(crate) struct VertexWorld {
    pub(crate) lights: Vec<PlacedLight>,
    /// The entry in `lights` of each key.
    pub(crate) by_key: HashMap<u32, usize>,
    /// The frame they were made in, plus 1 (0: never).
    pub(crate) made_in: u64,
    pub(crate) in_reach: Vec<usize>,
    /// The keys of the own point lights of the draw in progress.
    pub(crate) own_keys: Vec<u32>,
}

/// The engine's own point lights of the draws that took a world list. Each stays in the list;
/// against the draw's bound it is within the reach of its sphere, beyond it, or not among the
/// world's lights. `largest_gap` is the largest gap of a light beyond the reach, as a share of
/// its reach.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct OwnLights {
    pub(crate) lights: u32,
    pub(crate) in_reach: u32,
    pub(crate) not_in_set: u32,
    pub(crate) beyond: u32,
    pub(crate) largest_gap: f32,
}

impl VertexWorld {
    /// Makes the index of `lights` by key, after `lights` changed.
    pub(crate) fn index_lights(&mut self) {
        self.by_key.clear();
        self.by_key
            .extend(self.lights.iter().enumerate().map(|(i, light)| (light.key, i)));
    }

    /// Counts the draw's own point lights (`keys`) against its bound at `centre` (absolute) with
    /// `radius`.
    pub(crate) fn tally_own(&self, keys: impl Iterator<Item = u32>, centre: [f64; 3], radius: f32, tally: &mut OwnLights) {
        for key in keys {
            tally.lights += 1;
            let Some(&entry) = self.by_key.get(&key) else {
                tally.not_in_set += 1;
                continue;
            };
            let light = &self.lights[entry];
            let gap = light.gap(centre, radius);
            if gap <= 0.0 {
                tally.in_reach += 1;
            } else {
                tally.beyond += 1;
                tally.largest_gap = tally.largest_gap.max((gap / light.reach.max(1.0) as f64) as f32);
            }
        }
    }
}

/// The room for the world's lights in the list of a draw lit for each vertex that keeps
/// `own_points` point lights of its own: the cap less those, so that the cap never drops a
/// light that the engine chose.
pub(crate) fn world_room(own_points: usize) -> usize {
    VERTEX_WORLD_CAP.saturating_sub(own_points)
}

/// The world's point lights that a draw lit for each vertex adds to its own (W554 addendum 17):
/// those whose sphere meets the bound (`centre` in absolute world space, `radius`) and that are
/// not its own (`own`), into `out` as entries of `lights`: at most `room`
/// ([`world_room`]), the nearest by the gap of the spheres. Returns the number left out by the
/// room.
///
/// The draw keeps its own lights in front of these: the engine gives a light to every shape of a
/// reference when the light reaches any shape of it, so a light of the draw can be beyond the
/// draw's own bound.
pub(crate) fn world_lights_in_reach(
    lights: &[PlacedLight],
    own: impl Fn(&PlacedLight) -> bool,
    room: usize,
    centre: [f64; 3],
    radius: f32,
    out: &mut Vec<usize>,
) -> u32 {
    out.clear();
    out.extend(
        lights
            .iter()
            .enumerate()
            .filter(|(_, light)| light.gap(centre, radius) <= 0.0 && !own(light))
            .map(|(i, _)| i),
    );
    if out.len() <= room {
        return 0;
    }
    let dropped = (out.len() - room) as u32;
    out.sort_by(|&a, &b| lights[a].gap(centre, radius).total_cmp(&lights[b].gap(centre, radius)));
    out.truncate(room);
    dropped
}

/// A buffer for the lights of a frame.
pub(crate) fn light_buffer(device: &wgpu::Device, lights: usize) -> wgpu::Buffer {
    device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some("lights"),
        size: (lights * core::mem::size_of::<LightData>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Direct3D projection of a square view with a field of 90 degrees, near 1, far 1000.
    fn projection() -> [f32; 16] {
        let (near, far) = (1.0f32, 1000.0f32);
        let mut p = [0.0f32; 16];
        p[0] = 1.0;
        p[5] = 1.0;
        p[10] = far / (far - near);
        p[11] = 1.0;
        p[14] = -near * far / (far - near);
        p
    }

    /// The entries that a number of lights at one place make, and the entries dropped.
    fn bin_at_one_place(lights: u32, centre: [f32; 3], reach: f32) -> (usize, u64) {
        let cells = (CLUSTER_GRID[0] * CLUSTER_GRID[1] * CLUSTER_GRID[2]) as usize;
        let mut lists: Vec<Vec<u32>> = (0..cells).map(|_| Vec::new()).collect();
        let mut dropped = 0;
        let (_, indexes, _) = bin_lights(
            &mut lists,
            (0..lights).map(|entry| (entry, Some(centre), reach)),
            &projection(),
            [1.0, 1000.0],
            (1024, 1024),
            1,
            &mut dropped,
        );
        (indexes.len(), dropped)
    }

    fn entries(centre: [f32; 3], reach: f32) -> usize {
        bin_at_one_place(1, centre, reach).0
    }

    #[test]
    fn a_full_cluster_counts_the_light_that_it_drops() {
        let (centre, reach) = ([0.0, 0.0, 100.0], 10.0);
        let clusters = entries(centre, reach);
        assert!(clusters > 0);
        let full = CLUSTER_LIGHTS as u32;
        assert_eq!(bin_at_one_place(full, centre, reach), (clusters * CLUSTER_LIGHTS, 0));
        // The next light finds each of its clusters full: one drop for each cluster.
        assert_eq!(
            bin_at_one_place(full + 1, centre, reach),
            (clusters * CLUSTER_LIGHTS, clusters as u64)
        );
    }

    #[test]
    fn a_light_in_the_view_is_in_clusters() {
        assert!(entries([0.0, 0.0, 100.0], 10.0) > 0);
    }

    #[test]
    fn a_light_beside_the_view_is_in_no_cluster() {
        // At depth 300 the view reaches 300 across; the light's sphere ends at 700 across.
        assert_eq!(entries([800.0, 0.0, 300.0], 100.0), 0);
        assert_eq!(entries([-800.0, 0.0, 300.0], 100.0), 0);
        assert_eq!(entries([0.0, 800.0, 300.0], 100.0), 0);
        assert_eq!(entries([0.0, -800.0, 300.0], 100.0), 0);
    }

    #[test]
    fn a_light_beside_the_camera_that_reaches_the_near_plane_is_in_no_cluster() {
        // The sphere crosses the near plane, but it is 400 to the side of a view that is 50
        // wide at its depth.
        assert_eq!(entries([400.0, 0.0, 50.0], 100.0), 0);
    }

    #[test]
    fn a_light_that_reaches_into_the_view_from_beside_it_is_in_clusters() {
        assert!(entries([150.0, 0.0, 100.0], 80.0) > 0);
    }

    fn placed(slot: u32, x: f64, reach: f32) -> PlacedLight {
        PlacedLight {
            key: slot,
            slot,
            place: [150000.0 + x, -90000.0, 2000.0],
            reach,
        }
    }

    const CENTRE: [f64; 3] = [150000.0, -90000.0, 2000.0];

    /// The world's lights in reach of a draw with no lights of its own.
    fn in_reach(lights: &[PlacedLight], centre: [f64; 3], radius: f32, out: &mut Vec<usize>) -> u32 {
        world_lights_in_reach(lights, |_| false, world_room(0), centre, radius, out)
    }

    #[test]
    fn a_world_light_in_reach_is_in_the_list() {
        let mut out = Vec::new();
        // 300 units away, reach 400, a bound of 50: in. 451 away: out.
        let lights = [placed(7, 300.0, 400.0), placed(8, 451.0, 400.0)];
        assert_eq!(in_reach(&lights, CENTRE, 50.0, &mut out), 0);
        assert_eq!(out, [0]);
        // Just in at 449.
        let lights = [placed(9, 449.0, 400.0)];
        in_reach(&lights, CENTRE, 50.0, &mut out);
        assert_eq!(out, [0]);
        // The null: a reach of 0 and a bound of 0 give no light.
        let lights = [placed(10, 300.0, 0.0)];
        in_reach(&lights, CENTRE, 0.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn the_cap_keeps_the_nearest_lights() {
        let lights: Vec<PlacedLight> = (0..70).map(|i| placed(i, 100.0 + i as f64, 400.0)).collect();
        let mut out = Vec::new();
        assert_eq!(in_reach(&lights, CENTRE, 50.0, &mut out), 6);
        let mut kept = out.clone();
        kept.sort_unstable();
        assert_eq!(kept, (0..64).collect::<Vec<usize>>());
    }

    /// W554 addendum 17, the Architect's condition 1: with more than 64 candidates the room is
    /// the cap less the draw's own lights, so the cap drops world lights only. An own light is
    /// never added twice, also when it is in reach.
    #[test]
    fn the_cap_never_drops_an_own_light() {
        // 70 world lights in reach; the draw owns 10 of them (keys 60 to 69, the farthest) and 2
        // that are not in reach (keys 100, 101).
        let mut lights: Vec<PlacedLight> = (0..70).map(|i| placed(i, 100.0 + i as f64, 400.0)).collect();
        lights.push(placed(100, 5000.0, 400.0));
        lights.push(placed(101, 6000.0, 400.0));
        let own = |light: &PlacedLight| (60..70).contains(&light.key) || light.key >= 100;
        let mut out = Vec::new();
        let dropped = world_lights_in_reach(&lights, own, world_room(12), CENTRE, 50.0, &mut out);
        // 60 world lights in reach are not own; the room is 52: the 8 farthest go.
        assert_eq!((out.len(), dropped), (52, 8));
        let mut added = out.clone();
        added.sort_unstable();
        assert_eq!(added, (0..52).collect::<Vec<usize>>());
        // The 12 own lights and the 52 added fill the cap exactly.
        assert_eq!(12 + out.len(), VERTEX_WORLD_CAP);
        assert!(out.iter().all(|&i| !own(&lights[i])));
        // The null: a room that does not count the own lights lets in all 60 world lights, and
        // with the 12 own lights in front the list would hold 72, over the cap.
        let mut full = Vec::new();
        world_lights_in_reach(&lights, own, world_room(0), CENTRE, 50.0, &mut full);
        assert_eq!(12 + full.len(), 72);
        // More own lights than the cap: no room, and every own light stays with its caller.
        assert_eq!(world_room(70), 0);
        world_lights_in_reach(&lights, own, world_room(70), CENTRE, 50.0, &mut out);
        assert!(out.is_empty());
    }

    /// A light record of 104 bytes: the type, the place and the radius (the first specular
    /// word), with a range.
    fn record(kind: u32, place: [f32; 3], radius: f32) -> Vec<u8> {
        let mut words = [0u32; 26];
        words[0] = kind;
        words[5] = radius.to_bits();
        for axis in 0..3 {
            words[13 + axis] = place[axis].to_bits();
        }
        words[19] = 10000.0f32.to_bits();
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// W587: the world's clusters take a light with an end, with its cut-off as the reach, and
    /// leave out a point light with no end: the fade off, or a light with no radius (made in
    /// Lua). The null: the reach of the masked clusters bins the light with no end, as far as
    /// its attenuation shows.
    #[test]
    fn the_worlds_clusters_take_only_lights_with_an_end() {
        let mut light = crate::retained::light_data(&record(1, [0.0; 3], 256.0), None);
        light.diffuse = [0.96, 0.55, 0.16, 0.0];
        light.attenuation = [0.0, 3.0 / 256.0, 0.0, 0.0];
        assert_eq!(world_bin_reach(&light), None);
        assert_eq!(crate::extension::cluster_reach(&light), crate::extension::LIGHT_REACH_MAX);
        light.cone[3] = 1.0 / 512.0;
        assert_eq!(world_bin_reach(&light), Some(512.0));
        let mut sky = crate::retained::light_data(&record(3, [0.0; 3], 0.0), None);
        sky.direction = [0.0, 0.0, -1.0, 0.0];
        assert!(world_bin_reach(&sky).is_some());
    }

    #[test]
    fn a_directional_light_is_never_in_the_list() {
        assert!(PlacedLight::of_record(1, 1, &record(3, [0.0; 3], 0.0)).is_none());
        let point = PlacedLight::of_record(2, 2, &record(1, [150300.0, -90000.0, 2000.0], 400.0)).unwrap();
        assert_eq!((point.place, point.reach), ([150300.0, -90000.0, 2000.0], 400.0));
    }

    /// Addendum 11: the test takes the record's absolute place, so the answer is the same at
    /// each origin of the table. The null: the place relative to an origin, taken as absolute.
    #[test]
    fn the_test_by_absolute_place_does_not_change_with_the_origin() {
        let record = record(1, [150300.0, -90000.0, 2000.0], 400.0);
        let light = PlacedLight::of_record(0, 0, &record).unwrap();
        let mut out = Vec::new();
        in_reach(&[light], CENTRE, 50.0, &mut out);
        assert_eq!(out, [0]);
        for origin in [[0.0; 3], CENTRE] {
            let relative = crate::retained::light_data(&record, Some(origin));
            let moved = PlacedLight {
                place: core::array::from_fn(|i| relative.position[i] as f64 + origin[i]),
                ..light
            };
            in_reach(&[moved], CENTRE, 50.0, &mut out);
            assert_eq!(out, [0], "origin {origin:?}");
            // The null: the relative place taken as absolute misses at the far origin.
            let wrong = PlacedLight {
                place: core::array::from_fn(|i| relative.position[i] as f64),
                ..light
            };
            in_reach(&[wrong], CENTRE, 50.0, &mut out);
            assert_eq!(out.is_empty(), origin == CENTRE, "origin {origin:?}");
        }
    }

    #[test]
    fn a_draw_in_the_worlds_space_keeps_its_places() {
        let point = [12.5f32, -3.0, 40.0];
        let camera = DrawSpace::Camera(CENTRE);
        assert_eq!(camera.to_world(point), [150012.5, -90003.0, 2040.0]);
        assert_eq!(camera.camera(), Some(CENTRE));
        assert_eq!(DrawSpace::World.to_world(point), [12.5, -3.0, 40.0]);
        assert_eq!(DrawSpace::World.camera(), None);
    }

    #[test]
    fn the_own_lights_of_a_draw_are_told_apart_by_the_reach() {
        // 300 away with a reach of 400: in reach. 600 away: beyond, by 150 of a reach of 400.
        // Key 99: not among the world's lights.
        let mut world = VertexWorld {
            lights: vec![placed(1, 300.0, 400.0), placed(2, 600.0, 400.0)],
            ..VertexWorld::default()
        };
        world.index_lights();
        let mut tally = OwnLights::default();
        world.tally_own([1, 2, 99].into_iter(), CENTRE, 50.0, &mut tally);
        assert_eq!(
            tally,
            OwnLights {
                lights: 3,
                in_reach: 1,
                not_in_set: 1,
                beyond: 1,
                largest_gap: 150.0 / 400.0,
            }
        );
        // The null: without the index every key is "not among the world's lights".
        world.by_key.clear();
        let mut blind = OwnLights::default();
        world.tally_own([1, 2].into_iter(), CENTRE, 50.0, &mut blind);
        assert_eq!(blind.not_in_set, 2);
    }
}
