//! The replay of the kept world draws of a frame for an extension ([`Replay`]), and a draw as
//! the player keeps it ([`Draw`]).

use super::*;

/// How the renderer draws the kept world draws of the frame again. See [`Replay`].
pub struct ReplayView {
    /// A change of the world before the view of the game, in the space of the world of the
    /// game and with the layout of Direct3D: a mirror for a reflection. [`None`]: no change.
    pub world: Option<[f64; 16]>,
    /// Another view matrix and not that of the game, for a camera at another place: the
    /// light of a shadow map. [`None`]: the view of each draw.
    pub view: Option<[f32; 16]>,
    /// Another projection. [`None`]: that of each draw.
    pub projection: Option<[f32; 16]>,
    /// The size of the target of the pass.
    pub target_size: (u32, u32),
    /// The samples for each pixel of the targets of the pass: [`SceneView::samples`] for the
    /// pass of the game, 1 for a texture of the extension.
    pub samples: u32,
    /// True for a mirror: the front side of a triangle is its other side.
    pub flip_cull: bool,
    /// `[a, b, c, d]` in the space of the world of the game: only where
    /// `a * x + b * y + c * z + d >= 0` is drawn.
    pub clip_plane: Option<[f64; 4]>,
    /// Only the draws that write depth and do not blend.
    pub opaque_only: bool,
    /// Only the draws that blend and do not write depth: those that a surface drawn after
    /// them hides, whatever is nearer.
    pub translucent_only: bool,
    /// The depth test is on for each draw (less or equal), whatever the draw has: the sky
    /// of the game has it off, and in a reflection it must stay behind the land.
    pub force_depth_test: bool,
    /// The classes of the game to draw again, one bit for each (MGE_LINK_CLASS_*): the
    /// world draws of a frame have the sky too (classes 1 to 7). [`ReplayView::WORLD`] is
    /// all but the sky.
    pub classes: u32,
}

impl ReplayView {
    /// All classes but the sky (1 to 7) and the glare of the sun (8).
    pub const WORLD: u32 = !0x1FE;
    pub const SKY: u32 = 0xFE;
}

/// Draws the world draws of the frame again: those of the game in its world scene, its
/// scene of sorted draws and its scene of the water, into the back buffer, with the sky and
/// without the water plane of the game. The pass must have a colour target and a depth
/// target with the formats of [`SceneView`].
pub trait Replay {
    fn replay(&mut self, pass: &mut wgpu::RenderPass<'static>, view: &ReplayView);
    /// The number of kept draws.
    fn count(&self) -> usize;
}

/// One draw of the game, ready for the pass.
#[derive(Clone)]
pub(crate) struct Draw {
    /// The class of the object of the draw: MGE_LINK_CLASS_*.
    pub(crate) class: u32,
    /// A draw of the world of the game into the back buffer: one of the draws of [`Replay`].
    pub(crate) world: bool,
    pub(crate) key: PipelineKey,
    pub(crate) stencil_reference: u32,
    pub(crate) data: DrawData,
    /// The origin that the world matrices of the draw are relative to, or [`None`].
    pub(crate) origin: Option<[f64; 3]>,
    pub(crate) vertices: Rc<wgpu::Buffer>,
    pub(crate) indexes: Rc<wgpu::Buffer>,
    /// The handles of the textures of the stages, as the stream has them: the order of the
    /// opaque draws first (`MGE_XE_ORDER=opaque`) uses these and not the addresses of the
    /// buffers, so a replay of a capture gives the same order in each build.
    pub(crate) texture_handles: [u32; STAGES],
    /// The first index in the index buffer and the number of indexes, and the base vertex.
    pub(crate) range: (u32, u32),
    pub(crate) base_vertex: i32,
    /// The instances: the first in the instance buffer of the frame and the number. A draw of
    /// its own has one, with its world matrix in its record.
    pub(crate) instances: (u32, u32),
}

impl Draw {
    /// The draw blends and writes no depth: a surface drawn after it hides it.
    pub(crate) fn translucent(&self) -> bool {
        self.key.blend[0] != 0 && self.key.depth[1] == 0
    }
}

/// The kept draws with the renderer, for [`Replay`].
pub(crate) struct Replayer<'a> {
    pub(crate) renderer: &'a mut Renderer,
    pub(crate) kept: &'a [Rc<Draw>],
    /// The instances of the frame: the world matrices of the kept draws' instances.
    pub(crate) instances: &'a [Instance],
    /// The retained instances, drawn after the kept draws, and the indirect arguments of
    /// the frame that their draws join.
    pub(crate) retained: &'a mut RetainedSet,
    pub(crate) args: &'a mut Vec<IndirectArgs>,
    pub(crate) retained_on: bool,
}

/// A plane in the space of the world of the game, in the space of a draw whose world is
/// relative to `origin`.
pub(crate) fn plane_of_draw(plane: [f64; 4], origin: Option<[f64; 3]>) -> [f32; 4] {
    let origin = origin.unwrap_or([0.0; 3]);
    let d = plane[3] + plane[0] * origin[0] + plane[1] * origin[1] + plane[2] * origin[2];
    [plane[0] as f32, plane[1] as f32, plane[2] as f32, d as f32]
}

/// `a * b` for two matrices with the layout of Direct3D.
pub(crate) fn multiply64(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    core::array::from_fn(|i| (0..4).map(|k| a[i / 4 * 4 + k] * b[k * 4 + i % 4]).sum())
}

impl Replay for Replayer<'_> {
    fn count(&self) -> usize {
        self.kept.len()
    }

    fn replay(&mut self, pass: &mut wgpu::RenderPass<'static>, view: &ReplayView) {
        let mut bound = BoundGroups::default();
        let mut arrays_bound = false;
        for kept in self.kept {
            if view.classes & (1 << kept.class.min(31)) == 0 || (view.opaque_only && (kept.key.blend[0] != 0 || kept.key.depth[1] == 0)) || (view.translucent_only && !kept.translucent()) {
                continue;
            }
            let mut data = kept.data;
            let origin = kept.origin.unwrap_or([0.0; 3]);
            // The clusters are those of the view of the world: not for another view.
            data.light_range[3] &= !(1 | crate::WORLD_LIGHTS_BIT);
            if let Some(matrix) = view.view {
                data.view = matrix;
            }
            if let Some(world) = view.world {
                // The change is in the space of the world of the game. The world of the draw
                // can be relative to an origin: to that space, the change, and back.
                let mut to = [0.0f64; 16];
                let mut from = [0.0f64; 16];
                for i in 0..4 {
                    to[i * 5] = 1.0;
                    from[i * 5] = 1.0;
                }
                for axis in 0..3 {
                    to[12 + axis] = origin[axis];
                    from[12 + axis] = -origin[axis];
                }
                let change = multiply64(&multiply64(&to, &world), &from);
                let old: [f64; 16] = data.view.map(|v| v as f64);
                data.view = multiply64(&change, &old).map(|v| v as f32);
            }
            if let Some(projection) = view.projection {
                data.projection = crate::depth::gpu_projection(&projection);
            }
            data.pixel = [1.0 / view.target_size.0.max(1) as f32, -1.0 / view.target_size.1.max(1) as f32, 0.0, 0.0];
            if let Some(plane) = view.clip_plane {
                data.clip_plane = plane_of_draw(plane, kept.origin);
            }
            let mut key = kept.key;
            key.samples = view.samples;
            if view.force_depth_test {
                // D3DCMP_LESSEQUAL is 4.
                key.depth = [1, key.depth[1], 4];
            }
            if view.flip_cull {
                // D3DCULL_CW is 2 and D3DCULL_CCW is 3.
                key.cull = match key.cull {
                    2 => 3,
                    3 => 2,
                    other => other,
                };
            }
            let Some(pipeline) = self.renderer.pipeline(key) else { continue };
            let (start, n) = (kept.instances.0 as usize, kept.instances.1 as usize);
            let Some(first_instance) = self.renderer.copy_of_draw(&data, &self.instances[start..start + n]) else {
                self.renderer.counts.no_record += 1;
                continue;
            };
            pass.set_pipeline(&pipeline);
            pass.set_stencil_reference(kept.stencil_reference);
            bound.set(pass, 0, &self.renderer.frame_group);
            if !arrays_bound {
                // The pass of the extension draws into a texture of its own, not into a
                // texture of the game.
                let group = self.renderer.array_group_for(None);
                pass.set_bind_group(1, &*group, &[]);
                crate::gpu::note_array_bind(crate::gpu::BindOrigin::Replay);
                arrays_bound = true;
            }
            pass.set_vertex_buffer(0, kept.vertices.slice(..));
            pass.set_vertex_buffer(1, self.renderer.defaults.slice(..));
            pass.set_vertex_buffer(2, self.renderer.instance_buffer.slice(..));
            let (first, count) = kept.range;
            let instances = first_instance..first_instance + kept.instances.1;
            // `MGE_XE_REPLAY_DEBUG=1`: each draw that a replay issues, for a comparison of
            // the two paths of the shapes.
            if switches().replay_debug {
                println!(
                    "replay class {} blend {:?} depth {:?} cull {} fill {} origin {:?} range {:?} instances {:?} world {:?} view {:?} flags {:?} light_range {:?}",
                    kept.class, key.blend, key.depth, key.cull, key.fill, kept.origin, kept.range, kept.instances, data.world[0], &data.view[12..16], data.flags, data.light_range
                );
            }
            pass.set_index_buffer(kept.indexes.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(first..first + count, kept.base_vertex, instances);
            self.renderer.counts.replayed += 1;
        }
        self.replay_retained(pass, view);
    }
}
