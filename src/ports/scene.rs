//! The display of the shapes, ported 1:1 from the engine: the states, the accumulator, the
//! controllers and the render call of the renderer for each kind of shape, and the destructor
//! of a geometry. The display of a retained shape is skipped
//! (`instance_stream::skipped_display`). The accumulators themselves stay engine code: the
//! ports call them. The traversal of the nodes is the engine's own code.

use crate::engine::{thiscall1, thiscall3, thiscall4, vcall0, vcall1, virtual_at};
use crate::gen::layouts;
use crate::gen::layouts::{NiAVObject as AV, NiCamera as C, NiGeometry as G, NiGeometryData as GD, NiRenderer as R, NiTimeController as TC};
use crate::main_thread::MainThread;
use crate::mem::{rd, wr};
use crate::shadow::Regions;

/// `global_NiTriShape::countRenderCalls`, `global_NiGeometry::countPrimsRendered` and
/// `countVertsRendered`: the statistics of the frame.
const COUNT_RENDER_CALLS: usize = 0x7DE968;
const COUNT_PRIMS_RENDERED: usize = 0x7DE96C;
const COUNT_VERTS_RENDERED: usize = 0x7DE970;

const SLOT_DELETING_DTOR: usize = layouts::InterfaceNiObject::deleting_dtor;
const SLOT_GET_WORLD_BOUND: usize = layouts::InterfaceNiAVObject::GetWorldBound;
const SLOT_RENDER_SHAPE: usize = layouts::InterfaceNiRenderer::RenderShape;
const SLOT_RENDER_TRISTRIPS: usize = layouts::InterfaceNiRenderer::RenderTristrips;
const SLOT_RENDER_POINTS: usize = layouts::InterfaceNiRenderer::RenderPoints;
const SLOT_RENDER_LINES: usize = layouts::InterfaceNiRenderer::RenderLines;
const SLOT_ON_PRE_DISPLAY: usize = layouts::InterfaceNiTimeController::OnPreDisplay;
const SLOT_GET_ACTIVE_VERTEX_COUNT: usize = layouts::InterfaceNiGeometryData::GetActiveVertexCount;
const SLOT_GET_ACTIVE_TRIANGLE_COUNT: usize = layouts::InterfaceNiTriBasedGeomData::GetActiveTriangleCount;

/// Bit 4 of the flags of a node: the node was displayed in this frame.
pub(crate) const NODE_DISPLAYED: u16 = 0x10;
/// The byte of a `BSAnimationNode` with `BSAnimFlags_Displayed`.
pub(crate) const ANIM_FLAGS: usize = layouts::BSAnimationNode::animFlags;
pub(crate) const ANIM_DISPLAYED: u8 = 8;

const PUSH_STATE: usize = 0x6F3F40;
const RENDERER_REGISTER_OBJECT: usize = 0x6F5070;

/// Takes one reference from a `NiObject`; at zero the object deletes itself.
unsafe fn release_ref(object: usize) {
    let count = rd::<i32>(object, 4) - 1;
    wr(object, 4, count);
    if count == 0 {
        vcall1(object, SLOT_DELETING_DTOR, 1);
    }
}

unsafe fn add_ref(object: usize) {
    wr(object, 4, rd::<i32>(object, 4) + 1);
}

// ---------------------------------------------------------------------------------------
// The shapes
// ---------------------------------------------------------------------------------------

port_thiscall!(push_state_port, 0x6F3F40, "NiGeometry::pushStateToRenderer",
    fn(this, renderer: usize) -> (),
    compare_eax = false,
    regions = |set| push_state_regions(set, this, renderer),
    rust = push_state_to_renderer);

/// Makes the shape's property state and effect state the renderer's current ones, with
/// the references counted. A shape without a property state leaves the renderer's.
///
/// # Safety
///
/// `this` points at a live shape (its property state and effect state are read), and `renderer` at
/// the live renderer, whose current states the function writes; the references of the states are
/// counted on both sides.
pub unsafe fn push_state_to_renderer(this: usize, renderer: usize) {
    crate::shadow::touch(&push_state_port::STATE);
    let property = rd::<usize>(this, G::spPropertyState);
    if property != 0 {
        let current = rd::<usize>(renderer, R::currentPropertyState);
        if current != property {
            if current != 0 {
                release_ref(current);
            }
            wr(renderer, R::currentPropertyState, property);
            add_ref(property);
        }
    }
    let effect = rd::<usize>(this, G::spEffectState);
    let current = rd::<usize>(renderer, R::currentEffectState);
    if current != effect {
        if current != 0 {
            release_ref(current);
        }
        wr(renderer, R::currentEffectState, effect);
        if effect != 0 {
            add_ref(effect);
        }
    }
}

unsafe fn push_state_regions(set: &mut Regions, this: usize, renderer: usize) {
    set.add(renderer + R::currentPropertyState, 8);
    for state in [
        rd::<usize>(renderer, R::currentPropertyState),
        rd::<usize>(renderer, R::currentEffectState),
        rd::<usize>(this, G::spPropertyState),
        rd::<usize>(this, G::spEffectState),
    ] {
        if state != 0 {
            set.add(state + 4, 4);
        }
    }
    // A state that the renderer alone held is deleted.
    set.allocates();
}

/// The start of the display of a shape: the states to the renderer, the accumulator, and
/// the controllers' `OnPreDisplay`. Returns the renderer when the shape draws now, or
/// [`None`] when the accumulator took it.
unsafe fn shape_display_start(this: usize, camera: usize) -> Option<usize> {
    let renderer = rd::<usize>(camera, C::spRenderer);
    thiscall1(PUSH_STATE, this, renderer as u32);
    if thiscall1(RENDERER_REGISTER_OBJECT, renderer, this as u32) & 0xFF != 0 {
        return None;
    }
    let mut controller = rd::<usize>(this, AV::controllers);
    while controller != 0 {
        vcall0(controller, SLOT_ON_PRE_DISPLAY);
        controller = rd::<usize>(controller, TC::nextController);
    }
    Some(renderer)
}

/// The display of a shape the helper draws from its table (mode gpu): true when the port
/// returns without a draw. In the flush of the engine's main sorted queue the shape's place
/// goes into the stream (`INSTANCE_QUEUED`): the helper draws it there, in the engine's
/// order. During the walk a sorted blend goes into the main queue, as the engine's display
/// puts it there (`shape_display_start`: the state, then `RegisterObject`), so that the
/// engine sorts it with its own blends. A blend that the queue does not take (the no-sorter
/// bit, no accumulator) draws at its display in the engine: its place in the walk goes into
/// the stream (`INSTANCE_WALK`). A shape under another accumulator keeps the helper's own
/// order.
unsafe fn retained_display(t: &MainThread, this: usize, camera: usize, kind: u32) -> bool {
    let flush = crate::ports::base::in_main_flush(t);
    if flush && crate::instance_stream::queued_now(t, this) {
        crate::instance_stream::flush_place(t, this, true);
        return true;
    }
    if !crate::instance_stream::skipped_display(t, this, kind) {
        return false;
    }
    if !crate::instance_stream::may_queue(t) {
        crate::instance_stream::walk_place(t, this);
        return true;
    }
    if flush {
        // A shape under a node the queue took (a sort adjust node in mode 64): the node
        // displays its subtree in the flush.
        crate::instance_stream::flush_place(t, this, false);
        return true;
    }
    let renderer = rd::<usize>(camera, C::spRenderer);
    let accumulator = if renderer != 0 { rd::<usize>(renderer, R::accumulator) } else { 0 };
    if accumulator == 0 {
        crate::instance_stream::walk_place(t, this);
    } else if crate::ports::base::is_main_accumulator(t, accumulator) {
        thiscall1(PUSH_STATE, this, renderer as u32);
        if thiscall1(RENDERER_REGISTER_OBJECT, renderer, this as u32) & 0xFF != 0 {
            crate::instance_stream::note_queued(t, this);
        } else {
            crate::instance_stream::walk_place(t, this);
        }
    }
    true
}

unsafe fn count_render_call(primitives: u32, vertices: u32) {
    wr(COUNT_RENDER_CALLS, 0, rd::<u32>(COUNT_RENDER_CALLS, 0) + 1);
    wr(COUNT_PRIMS_RENDERED, 0, rd::<u32>(COUNT_PRIMS_RENDERED, 0) + primitives);
    wr(COUNT_VERTS_RENDERED, 0, rd::<u32>(COUNT_VERTS_RENDERED, 0) + vertices);
}

port_thiscall!(tri_shape_display_port, 0x6E4F90, "NiTriShape::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = tri_shape_display);

/// The states, the accumulator, the controllers, then `RenderShape` of the renderer with
/// the shape's data, skin, world transform and world bound; and the counts of the frame.
///
/// # Safety
///
/// `this` points at a live `NiTriShape` with its model data, and `camera` at a live camera whose
/// renderer is set: the function takes the renderer from the camera and calls its virtual functions
/// with the shape's data, skin, world transform and world bound.
pub unsafe fn tri_shape_display(t: &MainThread, this: usize, camera: usize) {
    crate::shadow::touch(&tri_shape_display_port::STATE);
    // A shape the helper draws from its table: the plugin notes its changes, and the engine
    // does not display it.
    if rd::<u16>(this, AV::flags) & crate::instance_stream::RETAINED_BIT != 0 && retained_display(t, this, camera, crate::link::DRAW_SHAPE) {
        return;
    }
    if crate::link_client::water_plane_display_skipped(t, this) {
        return;
    }
    let Some(renderer) = shape_display_start(this, camera) else { return };
    let skin = rd::<usize>(this, G::skinInstance);
    let data = rd::<usize>(this, G::spModelData);
    let bound = vcall0(this, SLOT_GET_WORLD_BOUND) as usize;
    thiscall4(virtual_at(renderer, SLOT_RENDER_SHAPE), renderer, data as u32, skin as u32, (this + AV::worldTransform) as u32, bound as u32);
    let data = rd::<usize>(this, G::spModelData);
    count_render_call(vcall0(data, SLOT_GET_ACTIVE_TRIANGLE_COUNT), vcall0(data, SLOT_GET_ACTIVE_VERTEX_COUNT));
}

port_thiscall!(tri_strips_display_port, 0x742660, "NiTriStrips::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = tri_strips_display);

/// As `NiTriShape::Display`, with `RenderTristrips`; the counts take the triangle count and
/// the vertex count of the data as they are.
///
/// # Safety
///
/// `this` points at a live `NiTriStrips` with its model data, and `camera` at a live camera whose
/// renderer is set: the function takes the renderer from the camera and calls its virtual functions
/// with the shape's data, skin, world transform and world bound.
pub unsafe fn tri_strips_display(t: &MainThread, this: usize, camera: usize) {
    crate::shadow::touch(&tri_strips_display_port::STATE);
    if rd::<u16>(this, AV::flags) & crate::instance_stream::RETAINED_BIT != 0 && retained_display(t, this, camera, crate::link::DRAW_STRIPS) {
        return;
    }
    if crate::link_client::water_plane_display_skipped(t, this) {
        return;
    }
    let Some(renderer) = shape_display_start(this, camera) else { return };
    let skin = rd::<usize>(this, G::skinInstance);
    let data = rd::<usize>(this, G::spModelData);
    let bound = vcall0(this, SLOT_GET_WORLD_BOUND) as usize;
    thiscall4(virtual_at(renderer, SLOT_RENDER_TRISTRIPS), renderer, data as u32, skin as u32, (this + AV::worldTransform) as u32, bound as u32);
    let data = rd::<usize>(this, G::spModelData);
    count_render_call(rd::<u16>(data, GD::SIZE) as u32, rd::<u16>(data, GD::vertexCount) as u32);
}

/// The number of active particles of a `NiParticlesData`, at the end of its geometry data.
const PARTICLES_NUM_ACTIVE: usize = layouts::NiParticlesData::numActive;

port_thiscall!(particles_display_port, 0x6DD3E0, "NiParticles::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = particles_display);

/// As `NiTriShape::Display`, with `RenderPoints` (no skin); the counts take the active
/// particles for both.
///
/// # Safety
///
/// `this` points at a live `NiParticles` with its model data, and `camera` at a live camera whose
/// renderer is set (the function takes it from the camera and calls its virtual functions).
pub unsafe fn particles_display(this: usize, camera: usize) {
    crate::shadow::touch(&particles_display_port::STATE);
    let Some(renderer) = shape_display_start(this, camera) else { return };
    render_points(this, renderer);
}

unsafe fn render_points(this: usize, renderer: usize) {
    let data = rd::<usize>(this, G::spModelData);
    let bound = vcall0(this, SLOT_GET_WORLD_BOUND) as usize;
    thiscall3(virtual_at(renderer, SLOT_RENDER_POINTS), renderer, data as u32, (this + AV::worldTransform) as u32, bound as u32);
    let active = rd::<u16>(rd::<usize>(this, G::spModelData), PARTICLES_NUM_ACTIVE) as u32;
    count_render_call(active, active);
}

port_thiscall!(auto_normal_particles_display_port, 0x6D3540, "NiAutoNormalParticles::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = auto_normal_particles_display);

/// As `NiParticles::Display`, with the normal of each active particle set to the direction
/// towards the camera first.
///
/// # Safety
///
/// `this` points at a live `NiAutoNormalParticles` with its model data, and `camera` at a live
/// camera whose renderer is set. The normals of the data, when not 0, hold at least as many entries
/// (12 bytes each) as the data's count of active particles: the function writes that many.
pub unsafe fn auto_normal_particles_display(this: usize, camera: usize) {
    crate::shadow::touch(&auto_normal_particles_display_port::STATE);
    let Some(renderer) = shape_display_start(this, camera) else { return };
    let normal = [-rd::<f32>(camera, C::kWorldDir), -rd::<f32>(camera, C::kWorldDir + 4), -rd::<f32>(camera, C::kWorldDir + 8)];
    let data = rd::<usize>(this, G::spModelData);
    let normals = rd::<usize>(data, GD::normals);
    if normals != 0 {
        let active = rd::<u16>(data, PARTICLES_NUM_ACTIVE) as usize;
        for i in 0..active {
            for (k, value) in normal.iter().enumerate() {
                wr(normals, i * 12 + k * 4, *value);
            }
        }
    }
    render_points(this, renderer);
}

port_thiscall!(lines_display_port, 0x6DA850, "NiLines::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = lines_display);

/// As `NiTriShape::Display`, with `RenderLines` (no skin) and no counts.
///
/// # Safety
///
/// `this` points at a live `NiLines` with its model data, and `camera` at a live camera whose
/// renderer is set (the function takes it from the camera and calls its virtual functions).
pub unsafe fn lines_display(this: usize, camera: usize) {
    crate::shadow::touch(&lines_display_port::STATE);
    let Some(renderer) = shape_display_start(this, camera) else { return };
    let data = rd::<usize>(this, G::spModelData);
    let bound = vcall0(this, SLOT_GET_WORLD_BOUND) as usize;
    thiscall3(virtual_at(renderer, SLOT_RENDER_LINES), renderer, data as u32, (this + AV::worldTransform) as u32, bound as u32);
}

// ---------------------------------------------------------------------------------------
// The end of a geometry
// ---------------------------------------------------------------------------------------

port_thiscall!(geometry_dtor_port, 0x6F3DB0, "NiGeometry::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = geometry_dtor);

/// The destructor of a geometry: the plugin forgets the shape (its retained instance of
/// `instance_stream`), then the engine code runs.
///
/// # Safety
///
/// `this` points at a live geometry that the engine destroys: the plugin forgets its instance, then
/// the engine function runs. The port is installed (the function calls its engine function).
pub unsafe fn geometry_dtor(t: &MainThread, this: usize) {
    crate::shadow::touch(&geometry_dtor_port::STATE);
    crate::instance_stream::forget(t, this);
    geometry_dtor_port::engine(this);
}

pub static SITES: &[usize] = &[];

pub static PORTS: &[&crate::shadow::PortState] = &[
    &push_state_port::STATE,
    &tri_shape_display_port::STATE,
    &tri_strips_display_port::STATE,
    &particles_display_port::STATE,
    &auto_normal_particles_display_port::STATE,
    &lines_display_port::STATE,
    &geometry_dtor_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook;

    #[test]
    fn the_functions_are_in_the_table() {
        for port in PORTS {
            assert!(hook::find(port.addr).is_some(), "{:#x} {}", port.addr, port.name);
        }
    }
}
