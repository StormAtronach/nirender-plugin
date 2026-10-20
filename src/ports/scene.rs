//! The display of the shapes, ported 1:1 from the engine: the states, the accumulator, the
//! controllers and the render call of the renderer for each kind of shape, and the destructor
//! of a geometry. The display of a retained shape is skipped
//! (`instance_stream::skipped_display`). The accumulators themselves stay engine code: the
//! ports call them. The traversal of the nodes is the engine's own code.

use crate::engine::{thiscall1, thiscall3, thiscall4, vcall0, vcall1, virtual_at};
use crate::gen::layouts::{NiAVObject as AV, NiCamera as C, NiGeometry as G, NiGeometryData as GD, NiRenderer as R, NiTimeController as TC};
use crate::main_thread::MainThread;
use crate::mem::{rd, wr};
use crate::shadow::Regions;

/// `global_NiTriShape::countRenderCalls`, `global_NiGeometry::countPrimsRendered` and
/// `countVertsRendered`: the statistics of the frame.
const COUNT_RENDER_CALLS: usize = 0x7DE968;
const COUNT_PRIMS_RENDERED: usize = 0x7DE96C;
const COUNT_VERTS_RENDERED: usize = 0x7DE970;

/// The vtable slots of `NiObject` and `NiAVObject`.
const SLOT_DELETING_DTOR: usize = 0x0;
const SLOT_GET_WORLD_BOUND: usize = 0x34;
/// The vtable slots of `NiRenderer`.
const SLOT_RENDER_SHAPE: usize = 0xBC;
const SLOT_RENDER_TRISTRIPS: usize = 0xC0;
const SLOT_RENDER_POINTS: usize = 0xC4;
const SLOT_RENDER_LINES: usize = 0xC8;
/// The vtable slot of `NiTimeController::OnPreDisplay`.
const SLOT_ON_PRE_DISPLAY: usize = 0x40;
/// The vtable slots of `NiGeometryData` and `NiTriBasedGeomData`.
const SLOT_GET_ACTIVE_VERTEX_COUNT: usize = 0x30;
const SLOT_GET_ACTIVE_TRIANGLE_COUNT: usize = 0x3C;

/// Bit 4 of the flags of a node: the node was displayed in this frame.
pub(crate) const NODE_DISPLAYED: u16 = 0x10;
/// `BSAnimFlags_Displayed` in the byte at 0xB0 of a `BSAnimationNode`.
pub(crate) const ANIM_FLAGS: usize = 0xB0;
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
/// `None` when the accumulator took it.
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
pub unsafe fn tri_shape_display(t: &MainThread, this: usize, camera: usize) {
    crate::shadow::touch(&tri_shape_display_port::STATE);
    // A shape the helper draws from its table: the plugin notes its changes, and the engine
    // does not display it.
    if rd::<u16>(this, AV::flags) & crate::instance_stream::RETAINED_BIT != 0 && crate::instance_stream::skipped_display(t, this, crate::link::DRAW_SHAPE) {
        return;
    }
    if crate::link_client::water_plane_display_skipped(this) {
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
pub unsafe fn tri_strips_display(t: &MainThread, this: usize, camera: usize) {
    crate::shadow::touch(&tri_strips_display_port::STATE);
    if rd::<u16>(this, AV::flags) & crate::instance_stream::RETAINED_BIT != 0 && crate::instance_stream::skipped_display(t, this, crate::link::DRAW_STRIPS) {
        return;
    }
    if crate::link_client::water_plane_display_skipped(this) {
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
const PARTICLES_NUM_ACTIVE: usize = 0x3C;

port_thiscall!(particles_display_port, 0x6DD3E0, "NiParticles::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = particles_display);

/// As `NiTriShape::Display`, with `RenderPoints` (no skin); the counts take the active
/// particles for both.
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
