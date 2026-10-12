//! Three functions of `NiDX8Renderer` that other mods change in memory: `RenderPoints`,
//! `RenderLines` and the constructor.
//!
//! Each direct call (`call rel32`) of these functions goes through `site::call_target`. The
//! port calls the target that the instruction in memory has now, with the calling convention
//! and the arguments of the game code. `SITES` lists those instructions. This module does
//! not call a port of another module directly. The constructor also reads the sizes of its
//! hash maps from the instructions in memory, because MWSE changes them there.

use core::arch::asm;
use core::mem::transmute;

use crate::d3d::{
    buffer_lock, buffer_unlock, device_set_indices, device_set_pixel_shader, device_set_stream_source, device_set_vertex_shader,
    D3DRS_VERTEXBLEND,
};
use crate::engine::{self, vcall0, NI_POINTER_CTOR, NI_POINTER_DTOR, VERTEX_BLEND_LOOKUP};
use crate::gen::layouts::{NiDX8RenderState as RS, NiDX8Renderer as R, NiGeometryData as GD, NiPropertyState as PS, THashMap};
use crate::mem::{rd, wr};
use crate::shadow::Regions;
// The macros of `site` call `target` for the address of the callee.
use crate::site::{call_target as target, site_cdecl, site_thiscall};

// ---------------------------------------------------------------------------------------
// The call sites
// ---------------------------------------------------------------------------------------

/// The address of every `call rel32` of the three functions. The ports call the target that is
/// in memory there. The comment of each line gives the callee of the database.
#[rustfmt::skip]
pub static SITES: &[usize] = &[
    // NiDX8Renderer::RenderPoints
    0x6AD0ED, // NiDX8Renderer::SetModelTransform
    0x6AD100, // NiBound_or_NiQuaternion::Copy
    0x6AD160, // NiDX8ConfigurableTexturePipeline::selectAndLinkPass
    0x6AD16F, // NiDX8RenderState::updateD3DState
    0x6AD198, // NiDX8ConfigurableTexturePipeline::buildPass
    0x6AD1C6, // NiRenderStateSetting::SetValue
    0x6AD204, // NiDX8LightManager::setState
    0x6AD221, // NiDX8ConfigurableTexturePipeline::loadBinary
    0x6AD236, // NiDX8IndexBufferManager::getOrBuildQuadIndexBuffer
    0x6AD2F0, // NiDX8VertexBufferManager::CreateVertexBuffer
    0x6AD7C6, // NiDX8ConfigurableTexturePipeline::buildSkinPartitionPass
    0x6AD84B, // patchDrawIndexedPrimitive
    0x6AD874, // NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables
    // NiDX8Renderer::RenderLines
    0x6AD8C2, // NiDX8Renderer::SetModelTransform
    0x6AD8D2, // NiBound_or_NiQuaternion::Copy
    0x6AD909, // NiDX8IndexBufferManager::buildLineSegmentIndexBuffer
    0x6AD975, // NiDX8VertexBufferManager::CreateVertexBuffer
    0x6ADB2E, // NiDX8ConfigurableTexturePipeline::selectAndLinkPass
    0x6ADB3D, // NiDX8RenderState::updateD3DState
    0x6ADB68, // NiDX8ConfigurableTexturePipeline::buildPass
    0x6ADBE8, // NiDX8LightManager::setState
    0x6ADBFC, // NiDX8ConfigurableTexturePipeline::loadBinary
    0x6ADC92, // patchDrawIndexedPrimitive
    0x6ADCBB, // NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables
    // NiDX8Renderer::ctor
    0x6A9A90, // NiRenderer::ctor
    0x6A9AAC, // NiBound_or_NiPlane::ctor
    0x6A9ABC, // NiBound_or_NiPlane::ctor
    0x6A9ADB, // `eh vector constructor iterator'
    0x6A9B11, // operator new
    0x6A9B57, // operator new
    0x6A9B9D, // operator new
    0x6A9BE3, // operator new
    0x6A9C19, // NiDX8Renderer::init
];

/// The call that the Morrowind Code Patch put in the place of `call [edx+11Ch]`
/// (`IDirect3DDevice8::DrawIndexedPrimitive`). The six arguments of the device method are on
/// the stack, the device is in EAX and its vtable is in EDX. The callee removes the arguments.
/// The function passes `min_index` as the caller gives it. At the call, ECX has the address of
/// a block of this function and not the value that the game code has there. The function of
/// the same name in `draw` is different: it passes the ECX of the game and a `min_index` of 0.
unsafe fn site_draw_indexed_primitive(
    site: usize,
    device: usize,
    vtable: usize,
    primitive_type: u32,
    min_index: u32,
    vertex_count: u32,
    start_index: u32,
    primitive_count: u32,
) -> u32 {
    let block = [target(site) as u32, primitive_type, min_index, vertex_count, start_index, primitive_count];
    let vtable = crate::link_client::draw_vtable(vtable);
    let result: u32;
    asm!(
        "push dword ptr [ecx + 20]",
        "push dword ptr [ecx + 16]",
        "push dword ptr [ecx + 12]",
        "push dword ptr [ecx + 8]",
        "push dword ptr [ecx + 4]",
        "push eax",
        "call dword ptr [ecx]",
        inout("eax") device => result,
        inout("ecx") block.as_ptr() => _,
        inout("edx") vtable => _,
        clobber_abi("C"),
    );
    result
}

// ---------------------------------------------------------------------------------------
// Engine data at fixed addresses, and members the layouts do not have
// ---------------------------------------------------------------------------------------

/// The vtables of the four hash maps of the renderer, in the order of the members.
const VTBL_HASH_MAPS: [usize; 4] = [
    engine::VTBL_RENDERER_MAP_GEOM_BUFFERS,
    engine::VTBL_RENDERER_MAP_SKIN_PARTITIONS,
    engine::VTBL_RENDERER_MAP_TEXTURES,
    engine::VTBL_RENDERER_MAP_CUBE_MAPS,
];

/// `NiParticlesData::particleRadius`
const PARTICLES_RADIUS: usize = 0x38;
/// `NiParticlesData::numActive` (16 bits)
const PARTICLES_NUM_ACTIVE: usize = 0x3C;
/// `NiParticlesData::sizes`
const PARTICLES_SIZES: usize = 0x40;
/// `NiLinesData::lineSegmentFlags`
const LINES_SEGMENT_FLAGS: usize = 0x34;
/// `NiRenderStateSetting` of `D3DRS_VERTEXBLEND` in the render state.
const RS_VERTEX_BLEND: usize = RS::renderStateSettings + D3DRS_VERTEXBLEND as usize * 8;

/// `D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX1`
const FVF_POINTS: u32 = 0x152;
/// Bytes of one vertex of a particle quad.
const POINT_STRIDE: u32 = 36;
/// A pass sets this bit in the result of `buildPass` when the lights need no update.
const PASS_NO_LIGHTS: u32 = 0x1000_0000;

/// 1.0
const ONE: u32 = 0x3F80_0000;
/// The value of all three components of the normal of the particle quads: 0.3.
const POINT_NORMAL: u32 = 0x3E99_999A;
const _: () = assert!(RS_VERTEX_BLEND == 0x6B0 && RS::currentVertexShader == 0xA00 && RS::currentPixelShader == 0x9F8);
const _: () = assert!(R::hashGeomBuffers == 0x560 && R::hashmap_maybeRenderedCubemaps == 0x590 && THashMap::SIZE == 0x10);

// ---------------------------------------------------------------------------------------
// Virtual calls: each call reads the slot
// ---------------------------------------------------------------------------------------

/// `NiGeometryData::GetActiveVertexCount`, slot 0x30. The result is 16 bits, and the callers
/// here test only those bits. The function of the same name in `buffers` returns the full
/// `u32` of EAX, and its callers take the low word. The two are different on purpose.
unsafe fn geometry_active_vertex_count(data: usize) -> u16 {
    vcall0(data, 0x30) as u16
}

/// Sets the vertex shader and the pixel shader of a fixed-function draw, as the two draw
/// functions do. The port makes the device call only when the render state holds a different
/// value.
unsafe fn set_fixed_function_shaders(this: usize, fvf: u32) {
    let render_state: usize = rd(this, R::renderState);
    if rd::<u32>(render_state, RS::currentVertexShader) != fvf {
        wr(render_state, RS::currentVertexShader, fvf);
        device_set_vertex_shader(rd(render_state, RS::device), fvf);
    }
    let render_state: usize = rd(this, R::renderState);
    if rd::<u32>(render_state, RS::currentPixelShader) != 0 {
        wr::<u32>(render_state, RS::currentPixelShader, 0);
        device_set_pixel_shader(rd(render_state, RS::device), 0);
    }
}

// ---------------------------------------------------------------------------------------
// NiDX8Renderer::RenderPoints
// ---------------------------------------------------------------------------------------

/// The two diagonals of a particle quad of size 1 (0x6AD247 to 0x6AD2EC):
/// `(right + up) * radius` and `(up - right) * radius`, with the model-space camera axes.
unsafe fn point_diagonals(this: usize, radius: f32) -> ([f32; 3], [f32; 3]) {
    let a = rd::<f32>(this, R::modelCameraUp_x) - rd::<f32>(this, R::modelCameraRight_x);
    let b = rd::<f32>(this, R::modelCameraUp_y) - rd::<f32>(this, R::modelCameraRight_y);
    let c = rd::<f32>(this, R::modelCameraUp_z) - rd::<f32>(this, R::modelCameraRight_z);
    let difference = [a * radius, b * radius, c * radius];
    let d = rd::<f32>(this, R::modelCameraRight_x) + rd::<f32>(this, R::modelCameraUp_x);
    let e = rd::<f32>(this, R::modelCameraRight_y) + rd::<f32>(this, R::modelCameraUp_y);
    let f = rd::<f32>(this, R::modelCameraRight_z) + rd::<f32>(this, R::modelCameraUp_z);
    let sum = [d * radius, e * radius, f * radius];
    (sum, difference)
}

/// Writes one vertex of a particle quad: the position, the fixed normal, the color and the
/// texture coordinates. The order of the writes is the order of the game code.
unsafe fn write_point_vertex(to: usize, position: [f32; 3], color: u32, u: u32, v: u32) {
    wr(to, 0, position[0]);
    wr(to, 4, position[1]);
    wr(to, 8, position[2]);
    wr(to, 0x0C, POINT_NORMAL);
    wr(to, 0x10, POINT_NORMAL);
    wr(to, 0x14, POINT_NORMAL);
    wr(to, 0x18, color);
    wr(to, 0x1C, u);
    wr(to, 0x20, v);
}

/// The vertex loops of `RenderPoints` (0x6AD340 with colors, 0x6AD584 without colors). They
/// write four vertices for each particle. `colors` is 0 when there are no colors, and the
/// color is then white. The port does one `f32` operation for each x87 operation, in the
/// order of the instructions.
unsafe fn write_point_quads(mut to: usize, count: u32, mut vertices: usize, mut sizes: usize, mut colors: usize, sum: [f32; 3], difference: [f32; 3]) {
    let mut left = count;
    while left != 0 {
        let size = rd::<f32>(sizes, 0);
        let color: u32 = if colors != 0 { rd(colors, 0) } else { 0xFFFF_FFFF };
        let p_x = sum[0] * size;
        let p_y = sum[1] * size;
        // The game code keeps this value on the x87 stack until the third vertex.
        let p_z = size * sum[2];
        let size = rd::<f32>(sizes, 0);
        let q = [difference[0] * size, difference[1] * size, difference[2] * size];

        let x = rd::<f32>(vertices, 0) - p_x;
        let y = rd::<f32>(vertices, 4) - p_y;
        let z = rd::<f32>(vertices, 8) - p_z;
        write_point_vertex(to, [x, y, z], color, 0, 0);
        to += POINT_STRIDE as usize;

        let x = rd::<f32>(vertices, 0) - q[0];
        let y = rd::<f32>(vertices, 4) - q[1];
        let z = rd::<f32>(vertices, 8) - q[2];
        write_point_vertex(to, [x, y, z], color, ONE, 0);
        to += POINT_STRIDE as usize;

        let x = p_x + rd::<f32>(vertices, 0);
        let y = p_y + rd::<f32>(vertices, 4);
        let z = p_z + rd::<f32>(vertices, 8);
        write_point_vertex(to, [x, y, z], color, ONE, ONE);
        to += POINT_STRIDE as usize;

        let x = q[0] + rd::<f32>(vertices, 0);
        let y = q[1] + rd::<f32>(vertices, 4);
        let z = q[2] + rd::<f32>(vertices, 8);
        write_point_vertex(to, [x, y, z], color, 0, ONE);
        to += POINT_STRIDE as usize;

        vertices += 12;
        if colors != 0 {
            colors += 4;
        }
        sizes += 4;
        left -= 1;
    }
}

/// The shape record of a particle system (see `shape_stream::send_particles`): the lights
/// of the draw as `RenderPoints` sets them, then the record with the position, the size
/// and the colour of each active particle. The helper makes the quads. Returns true when
/// the pipeline of the device must not run for the draw.
unsafe fn particles_record(this: usize, geometry_data: usize, count: u32, radius: f32, vertices: usize, sizes: usize, colors: usize) -> bool {
    if crate::shape_stream::mode() == crate::shape_stream::OFF || !crate::capture::recording() {
        return false;
    }
    let property_state: usize = rd(this, R::currentPropertyState);
    let vertex_color: usize = rd(property_state, PS::propVertexColor);
    let texturing: usize = rd(property_state, PS::propTexturing);
    let effect_state: usize = rd(this, R::currentEffectState);
    site_thiscall!(0x6AD204, rd::<usize>(this, R::lightManager), effect_state, texturing, vertex_color);
    // D3DRS_AMBIENT (139) as the render state record of the renderer has it.
    let ambient: u32 = rd(rd::<usize>(this, R::renderState), RS::renderStateSettings + 139 * 8);
    let right: [f32; 3] = rd(this, R::modelCameraRight_x);
    let up: [f32; 3] = rd(this, R::modelCameraUp_x);
    let mut particles = Vec::with_capacity(count as usize * crate::shape_stream::PARTICLE_BYTES);
    for i in 0..count as usize {
        particles.extend_from_slice(&rd::<[u8; 12]>(vertices, i * 12));
        particles.extend_from_slice(&rd::<[u8; 4]>(sizes, i * 4));
        particles.extend_from_slice(&if colors != 0 { rd::<[u8; 4]>(colors, i * 4) } else { [0xFF; 4] });
    }
    crate::shape_stream::send_particles(
        this,
        geometry_data as u32,
        crate::shape_stream::current_world(this),
        crate::link_client::world_is_relative(),
        ambient,
        crate::shape_stream::material_values(property_state),
        crate::shape_stream::fog_values(this, property_state),
        radius,
        right,
        up,
        &particles,
    )
}

/// 0x6AD0A0 `NiDX8Renderer::RenderPoints`: draws the particles as quads that face the camera.
/// With the shape records on, the record of the particles goes out after the transform and
/// the bound are set, and the pipeline of the device does not run (see `particles_record`).
pub unsafe fn render_points(this: usize, geometry_data: usize, world_transform: usize, world_bound: usize) {
    crate::shadow::touch(&render_points_port::STATE);
    if rd::<u8>(this, R::bSceneOpen) == 0 {
        return;
    }
    if geometry_active_vertex_count(geometry_data) == 0 {
        return;
    }
    let active: u16 = rd(geometry_data, PARTICLES_NUM_ACTIVE);
    if active == 0 {
        return;
    }
    site_thiscall!(0x6AD0ED, this, world_transform);
    site_thiscall!(0x6AD100, this + R::worldBound, world_bound);
    let colors: usize = rd(geometry_data, GD::colors);
    let sizes: usize = rd(geometry_data, PARTICLES_SIZES);
    let radius: f32 = rd(geometry_data, PARTICLES_RADIUS);
    let count = active as u32;
    let vertex_count = count * 4;
    let vertices: usize = rd(geometry_data, GD::vertices);
    if particles_record(this, geometry_data, count, radius, vertices, sizes, colors) {
        return;
    }

    let pipeline = || rd::<usize>(this, R::configurableTexturePipeline);
    let property_state = || rd::<usize>(this, R::currentPropertyState);
    let effect_state = || rd::<usize>(this, R::currentEffectState);

    site_thiscall!(0x6AD160, pipeline(), property_state(), effect_state(), 1, 1, geometry_data, 0, world_transform, world_bound);
    site_thiscall!(0x6AD16F, rd::<usize>(this, R::renderState), property_state());
    let pass_flags = site_thiscall!(0x6AD198, pipeline(), property_state(), effect_state(), 1, 1, geometry_data, 0, world_transform, world_bound);

    let render_state: usize = rd(this, R::renderState);
    let blend: u32 = rd(VERTEX_BLEND_LOOKUP, 0);
    if rd::<u32>(render_state, RS_VERTEX_BLEND) != blend {
        site_thiscall!(0x6AD1C6, render_state + RS_VERTEX_BLEND, blend, 0);
        engine::device_set_render_state(rd(render_state, RS::device), D3DRS_VERTEXBLEND, blend);
    }
    if pass_flags & PASS_NO_LIGHTS == 0 {
        let state = property_state();
        let vertex_color: usize = rd(state, PS::propVertexColor);
        let texturing: usize = rd(state, PS::propTexturing);
        site_thiscall!(0x6AD204, rd::<usize>(this, R::lightManager), effect_state(), texturing, vertex_color);
    }
    site_thiscall!(0x6AD221, pipeline(), geometry_data, 0, world_transform, world_bound);
    let index_buffer = site_thiscall!(0x6AD236, rd::<usize>(this, R::indexBufferManager), vertex_count, 1, 0, 1) as usize;
    if index_buffer == 0 {
        return;
    }

    let (sum, difference) = point_diagonals(this, radius);
    let size_to_lock = vertex_count.wrapping_mul(9) << 2;
    let vertex_buffer = site_thiscall!(0x6AD2F0, rd::<usize>(this, R::vertexBufferManager), size_to_lock, 0, FVF_POINTS, 1) as usize;
    if vertex_buffer == 0 {
        return;
    }
    let mut memory: usize = 0;
    if buffer_lock(vertex_buffer, 0, size_to_lock, core::ptr::addr_of_mut!(memory) as usize, 0) != 0 {
        return;
    }
    write_point_quads(memory, count, vertices, sizes, colors, sum, difference);
    buffer_unlock(vertex_buffer);

    site_thiscall!(0x6AD7C6, pipeline(), property_state(), effect_state(), 1, 1, geometry_data, 0, 0, world_transform, world_bound);
    device_set_stream_source(rd(this, R::d3dDevice), 0, vertex_buffer, POINT_STRIDE);
    device_set_indices(rd(this, R::d3dDevice), index_buffer, 0);
    set_fixed_function_shaders(this, FVF_POINTS);
    let device: usize = rd(this, R::d3dDevice);
    crate::link_client::draw_state(this, 4, vertex_count, 0, count + count, false, false);
    site_draw_indexed_primitive(0x6AD84B, device, rd(device, 0), 4, 0, vertex_count, 0, count + count);
    site_thiscall!(0x6AD874, pipeline(), property_state(), effect_state(), 1, 1, geometry_data, 0, world_transform, world_bound);
}

/// The call is a skip when the scene is open and the data has active vertices and active
/// particles. Such a draw gets to `buildPass` and `loadBinary` of the texture pass. The
/// texture pass uploads a texture on first use, and a texture upload cannot run twice. Thus
/// the shadow test compares only the calls that return at the first tests. Those calls write
/// nothing. `GetActiveVertexCount` only reads.
unsafe fn render_points_regions(set: &mut Regions, this: usize, geometry_data: usize) {
    set.add(this, R::SIZE);
    if rd::<u8>(this, R::bSceneOpen) == 0 {
        return;
    }
    if geometry_active_vertex_count(geometry_data) == 0 || rd::<u16>(geometry_data, PARTICLES_NUM_ACTIVE) == 0 {
        return;
    }
    set.skip();
}

// ---------------------------------------------------------------------------------------
// NiDX8Renderer::RenderLines
// ---------------------------------------------------------------------------------------

/// The vertex format and the size in bytes of a vertex of `RenderLines` (0x6AD91A to 0x6AD954).
fn lines_format(has_texture_coords: bool, has_colors: bool) -> (u32, u32) {
    let mut fvf = 2;
    let mut stride = 0x0C;
    if has_texture_coords {
        fvf = 0x102;
        stride = 0x14;
    }
    if has_colors {
        fvf |= 0x40;
        stride += 4;
    }
    (fvf, stride)
}

/// The four vertex loops of `RenderLines`. They copy integers. The port returns the texture
/// coordinate pointer as the game code leaves it in EBP. The loop for positions only
/// (0x6ADCE5) does not move the source pointer: it writes the first vertex `count` times.
unsafe fn write_line_vertices(mut to: usize, count: u32, mut vertices: usize, mut colors: usize, mut texture_coords: usize) -> usize {
    let with_colors = colors != 0;
    let with_texture_coords = texture_coords != 0;
    for _ in 0..count {
        wr::<u32>(to, 0, rd(vertices, 0));
        wr::<u32>(to, 4, rd(vertices, 4));
        wr::<u32>(to, 8, rd(vertices, 8));
        to += 12;
        if with_colors || with_texture_coords {
            vertices += 12;
        }
        if with_colors {
            wr::<u32>(to, 0, rd(colors, 0));
            colors += 4;
            to += 4;
        }
        if with_texture_coords {
            wr::<u32>(to, 0, rd(texture_coords, 0));
            wr::<u32>(to, 4, rd(texture_coords, 4));
            texture_coords = texture_coords.wrapping_add(8);
            to += 8;
        }
    }
    texture_coords
}

/// 0x6AD890 `NiDX8Renderer::RenderLines`. No caller reads EAX.
pub unsafe fn render_lines(this: usize, lines_data: usize, transform: usize, world_bound: usize) {
    crate::shadow::touch(&render_lines_port::STATE);
    if rd::<u8>(this, R::bSceneOpen) == 0 {
        return;
    }
    if geometry_active_vertex_count(lines_data) == 0 {
        return;
    }
    site_thiscall!(0x6AD8C2, this, transform);
    site_thiscall!(0x6AD8D2, this + R::worldBound, world_bound);
    let count = rd::<u16>(lines_data, GD::vertexCount) as u32;
    let segment_flags: usize = rd(lines_data, LINES_SEGMENT_FLAGS);
    let vertices: usize = rd(lines_data, GD::vertices);
    let texture_coords: usize = rd(lines_data, GD::textureCoords);
    let colors: usize = rd(lines_data, GD::colors);

    let mut index_count: u32 = 0;
    let index_count_address = &mut index_count as *mut u32 as usize;
    let index_buffer = site_thiscall!(0x6AD909, rd::<usize>(this, R::indexBufferManager), count, segment_flags, index_count_address, 0, 1) as usize;
    if index_buffer == 0 {
        return;
    }
    let (fvf, stride) = lines_format(texture_coords != 0, colors != 0);
    let size = count.wrapping_mul(stride);
    let vertex_buffer = site_thiscall!(0x6AD975, rd::<usize>(this, R::vertexBufferManager), size, 8, fvf, 0) as usize;
    if vertex_buffer == 0 {
        return;
    }
    let mut memory: usize = 0;
    if buffer_lock(vertex_buffer, 0, size, core::ptr::addr_of_mut!(memory) as usize, 0) != 0 {
        return;
    }
    // This is EBP of the game code: the pointer after the loop. The tests below read it.
    let texture_coords = write_line_vertices(memory, count, vertices, colors, texture_coords);
    buffer_unlock(vertex_buffer);
    let texture_sets = if texture_coords != 0 { count } else { 0 };

    let pipeline = || rd::<usize>(this, R::configurableTexturePipeline);
    let property_state = || rd::<usize>(this, R::currentPropertyState);
    let effect_state = || rd::<usize>(this, R::currentEffectState);

    site_thiscall!(0x6ADB2E, pipeline(), property_state(), effect_state(), 0, texture_sets, lines_data, 0, transform, world_bound);
    site_thiscall!(0x6ADB3D, rd::<usize>(this, R::renderState), property_state());
    let pass_flags = site_thiscall!(0x6ADB68, pipeline(), property_state(), effect_state(), 0, texture_sets, lines_data, 0, transform, world_bound);

    // The game code has this block twice.
    for _ in 0..2 {
        let blend: u32 = rd(VERTEX_BLEND_LOOKUP, 0);
        let render_state: usize = rd(this, R::renderState);
        if rd::<u32>(render_state, RS_VERTEX_BLEND) != blend {
            wr(render_state, RS_VERTEX_BLEND, blend);
            engine::device_set_render_state(rd(render_state, RS::device), D3DRS_VERTEXBLEND, blend);
        }
    }
    if pass_flags & PASS_NO_LIGHTS == 0 {
        let state = property_state();
        let vertex_color: usize = rd(state, PS::propVertexColor);
        let texturing: usize = rd(state, PS::propTexturing);
        site_thiscall!(0x6ADBE8, rd::<usize>(this, R::lightManager), effect_state(), texturing, vertex_color);
    }
    site_thiscall!(0x6ADBFC, pipeline(), lines_data, 0, transform, world_bound);
    device_set_indices(rd(this, R::d3dDevice), index_buffer, 0);
    device_set_stream_source(rd(this, R::d3dDevice), 0, vertex_buffer, stride);
    set_fixed_function_shaders(this, fvf);
    let device: usize = rd(this, R::d3dDevice);
    crate::link_client::draw_state(this, 2, count, 0, index_count >> 1, false, false);
    site_draw_indexed_primitive(0x6ADC92, device, rd(device, 0), 2, 0, count, 0, index_count >> 1);
    site_thiscall!(0x6ADCBB, pipeline(), property_state(), effect_state(), 0, texture_sets, lines_data, 0, transform, world_bound);
}

/// The call is a skip when the scene is open and the data has active vertices. Such a draw
/// gets to the texture pass. `render_points_regions` gives the reason.
unsafe fn render_lines_regions(set: &mut Regions, this: usize, lines_data: usize) {
    set.add(this, R::SIZE);
    if rd::<u8>(this, R::bSceneOpen) == 0 || geometry_active_vertex_count(lines_data) == 0 {
        return;
    }
    set.skip();
}

// ---------------------------------------------------------------------------------------
// NiDX8Renderer::ctor
// ---------------------------------------------------------------------------------------

/// The operand of `mov ebp, imm32` at 0x6A9AEC: the bucket count of the four hash maps.
/// It is 37 in the database.
const HASH_MAP_BUCKETS_OPERAND: usize = 0x6A9AED;
/// The operands of the four `push imm32` before `operator new`: the size in bytes of the
/// bucket array of the maps for geometry buffers, skin partitions, rendered textures and
/// rendered cube maps. Each is 0x94 in the database.
const HASH_MAP_BYTES_OPERANDS: [usize; 4] = [0x6A9AF2, 0x6A9B38, 0x6A9B7E, 0x6A9BC4];
/// The calls to `operator new` for the four bucket arrays.
const HASH_MAP_NEW_SITES: [usize; 4] = [0x6A9B11, 0x6A9B57, 0x6A9B9D, 0x6A9BE3];

/// 0x6A9A70 `NiDX8Renderer::ctor`
///
/// MWSE changes the sizes of the hash maps in this code: 37 buckets become 4093. The patch is
/// "Optimize renderer hash map lookups" in PatchUtil.cpp of MWSE. Thus the port reads the
/// bucket count and the four allocation sizes from the instructions in memory, as the code
/// in memory uses them.
pub unsafe fn renderer_ctor(this: usize) -> usize {
    crate::shadow::touch(&renderer_ctor_port::STATE);
    site_thiscall!(0x6A9A90, this);
    wr::<u32>(this, R::propertyStatePtr, 0);
    wr::<u32>(this, R::effectStatePtr, 0);
    site_thiscall!(0x6A9AAC, this + R::worldBound);
    site_thiscall!(0x6A9ABC, this + R::defaultBound);
    let construct_array: unsafe extern "system" fn(usize, u32, u32, usize, usize) = transmute(target(0x6A9ADB));
    construct_array(this + R::defaultTexturePixelData, 4, 4, NI_POINTER_CTOR, NI_POINTER_DTOR);
    wr::<u32>(this, R::backbufferRenderTarget_field_C, 0);
    wr::<u32>(this, R::backbufferRenderTarget_field_18, 0);

    let bucket_count: u32 = rd(HASH_MAP_BUCKETS_OPERAND, 0);
    for index in 0..4 {
        let map = this + R::hashGeomBuffers + index * THashMap::SIZE;
        wr(map, THashMap::vtbl, VTBL_HASH_MAPS[index]);
        wr(map, THashMap::bucketCount, bucket_count);
        wr::<u32>(map, THashMap::count, 0);
        let buckets = site_cdecl!(HASH_MAP_NEW_SITES[index], rd::<u32>(HASH_MAP_BYTES_OPERANDS[index], 0)) as usize;
        // The game code reads the count from the map and clears that many pointers.
        let bytes = rd::<u32>(map, THashMap::bucketCount) << 2;
        wr(map, THashMap::arrayBuckets, buckets);
        core::ptr::write_bytes(buckets as *mut u8, 0, bytes as usize);
    }
    wr(this, R::vtbl, engine::VTBL_NI_DX8_RENDERER);
    site_thiscall!(0x6A9C19, this);
    this
}

// ---------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------

port_thiscall!(render_points_port, 0x6AD0A0, "NiDX8Renderer::RenderPoints",
    fn(this, geometry_data: usize, world_transform: usize, world_bound: usize) -> (),
    compare_eax = false,
    regions = |set| render_points_regions(set, this, geometry_data),
    rust = render_points);

port_thiscall!(render_lines_port, 0x6AD890, "NiDX8Renderer::RenderLines",
    fn(this, lines_data: usize, transform: usize, world_bound: usize) -> (),
    compare_eax = false,
    regions = |set| render_lines_regions(set, this, lines_data),
    rust = render_lines);

// The game runs the constructor one time. With "early" in the config of the mod, the plugin
// installs before the game creates its renderer, and the port then runs in the mode of the
// config. Without "early", the constructor ran as game code before the plugin installed.
//
// The call is always a skip. A comparison needs `set.allocates()` for the four bucket arrays
// of the hash maps. It also needs the blocks that the base constructor `NiRenderer::ctor` and
// `NiDX8Renderer::init` write, and those blocks are not known.
port_thiscall!(renderer_ctor_port, 0x6A9A70, "NiDX8Renderer::ctor",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = renderer_ctor);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &render_points_port::STATE,
    &render_lines_port::STATE,
    &renderer_ctor_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::census;
    use crate::hook;

    const FUNCTIONS: [usize; 3] = [0x6AD0A0, 0x6AD890, 0x6A9A70];

    /// The callee that the database has for each entry of `SITES`, in the same order.
    #[rustfmt::skip]
    const CALLEES: &[usize] = &[
        // NiDX8Renderer::RenderPoints
        0x6AC9C0, // NiDX8Renderer::SetModelTransform
        0x6ED100, // NiBound_or_NiQuaternion::Copy
        0x6B9C40, // NiDX8ConfigurableTexturePipeline::selectAndLinkPass
        0x6B82A0, // NiDX8RenderState::updateD3DState
        0x6B9C20, // NiDX8ConfigurableTexturePipeline::buildPass
        0x65EEB0, // NiRenderStateSetting::SetValue
        0x6BB200, // NiDX8LightManager::setState
        0x6B9C30, // NiDX8ConfigurableTexturePipeline::loadBinary
        0x6BCFE0, // NiDX8IndexBufferManager::getOrBuildQuadIndexBuffer
        0x6BDC70, // NiDX8VertexBufferManager::CreateVertexBuffer
        0x6B9E90, // NiDX8ConfigurableTexturePipeline::buildSkinPartitionPass
        0x736400, // patchDrawIndexedPrimitive
        0x6B9D30, // NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables
        // NiDX8Renderer::RenderLines
        0x6AC9C0, // NiDX8Renderer::SetModelTransform
        0x6ED100, // NiBound_or_NiQuaternion::Copy
        0x6BD190, // NiDX8IndexBufferManager::buildLineSegmentIndexBuffer
        0x6BDC70, // NiDX8VertexBufferManager::CreateVertexBuffer
        0x6B9C40, // NiDX8ConfigurableTexturePipeline::selectAndLinkPass
        0x6B82A0, // NiDX8RenderState::updateD3DState
        0x6B9C20, // NiDX8ConfigurableTexturePipeline::buildPass
        0x6BB200, // NiDX8LightManager::setState
        0x6B9C30, // NiDX8ConfigurableTexturePipeline::loadBinary
        0x736400, // patchDrawIndexedPrimitive
        0x6B9D30, // NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables
        // NiDX8Renderer::ctor
        0x6F4E30, // NiRenderer::ctor
        0x6F25E0, // NiBound_or_NiPlane::ctor
        0x6F25E0, // NiBound_or_NiPlane::ctor
        0x7276B0, // `eh vector constructor iterator'
        0x727692, // operator new
        0x727692, // operator new
        0x727692, // operator new
        0x727692, // operator new
        0x6A96C0, // NiDX8Renderer::init
    ];

    #[test]
    fn every_site_is_a_call_inside_one_of_the_functions() {
        assert_eq!(SITES.len(), CALLEES.len());
        for (&site, &callee) in SITES.iter().zip(CALLEES) {
            let func = FUNCTIONS
                .iter()
                .map(|&ea| hook::find(ea).unwrap())
                .find(|func| site >= func.ea && site + 5 <= func.ea + func.size)
                .unwrap_or_else(|| panic!("{site:#x} is in none of the functions"));
            let code = census::expected(func);
            let at = site - func.ea;
            assert_eq!(code[at], 0xE8, "{site:#x} is not a call");
            let rel = i32::from_le_bytes(code[at + 1..at + 5].try_into().unwrap());
            assert_eq!((site + 5).wrapping_add(rel as usize), callee, "callee of {site:#x}");
        }
        let mut sorted = SITES.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), SITES.len(), "a site is listed twice");
    }

    #[test]
    fn the_hash_map_sizes_are_operands_of_the_constructor() {
        let func = hook::find(0x6A9A70).unwrap();
        let code = census::expected(func);
        let operand = |at: usize, opcode: u8| -> u32 {
            let offset = at - func.ea;
            assert_eq!(code[offset - 1], opcode, "{at:#x} is not the operand of that instruction");
            u32::from_le_bytes(code[offset..offset + 4].try_into().unwrap())
        };
        // mov ebp, 37
        assert_eq!(operand(HASH_MAP_BUCKETS_OPERAND, 0xBD), 37);
        for (index, at) in HASH_MAP_BYTES_OPERANDS.into_iter().enumerate() {
            // push 0x94, then other instructions, then the call to operator new
            assert_eq!(operand(at, 0x68), 37 * 4);
            assert!(at < HASH_MAP_NEW_SITES[index] && HASH_MAP_NEW_SITES[index] < at + 0x24);
            assert!(SITES.contains(&HASH_MAP_NEW_SITES[index]));
        }
    }

    #[test]
    fn the_sites_are_served() {
        assert!(crate::ports::serves_site(0x6AD84B));
        assert!(crate::ports::serves_site(0x6ADC92));
        assert!(!crate::ports::serves_site(0x6AD84A));
    }

    fn floats(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn words(bytes: &[u8]) -> Vec<u32> {
        bytes.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
    }

    fn vertex(position: [f32; 3], color: u32, u: f32, v: f32) -> Vec<u32> {
        let mut out: Vec<u32> = position.iter().map(|p| p.to_bits()).collect();
        out.extend([POINT_NORMAL, POINT_NORMAL, POINT_NORMAL, color, u.to_bits(), v.to_bits()]);
        out
    }

    #[test]
    fn point_diagonals_use_the_model_camera_axes() {
        let mut renderer = vec![0u8; R::SIZE];
        let this = renderer.as_mut_ptr() as usize;
        unsafe {
            for (i, value) in [1.0f32, 2.0, 3.0].into_iter().enumerate() {
                wr(this, R::modelCameraRight + i * 4, value);
            }
            for (i, value) in [5.0f32, 7.0, 11.0].into_iter().enumerate() {
                wr(this, R::modelCameraUp + i * 4, value);
            }
            let (sum, difference) = point_diagonals(this, 0.5);
            assert_eq!(sum, [3.0, 4.5, 7.0]);
            assert_eq!(difference, [2.0, 2.5, 4.0]);
        }
    }

    #[test]
    fn a_particle_is_four_vertices_around_its_position() {
        let vertices = floats(&[10.0, 20.0, 30.0, 100.0, 200.0, 300.0]);
        let sizes = floats(&[2.0, 1.0]);
        let colors: Vec<u8> = [0x1122_3344u32, 0x5566_7788].iter().flat_map(|c| c.to_le_bytes()).collect();
        let sum = [1.0, 2.0, 3.0];
        let difference = [0.5, 0.25, 0.125];
        let mut out = vec![0xEEu8; 2 * 4 * 36];
        unsafe {
            write_point_quads(out.as_mut_ptr() as usize, 2, vertices.as_ptr() as usize, sizes.as_ptr() as usize, colors.as_ptr() as usize, sum, difference);
        }
        let got = words(&out);
        let mut want = Vec::new();
        want.extend(vertex([8.0, 16.0, 24.0], 0x1122_3344, 0.0, 0.0));
        want.extend(vertex([9.0, 19.5, 29.75], 0x1122_3344, 1.0, 0.0));
        want.extend(vertex([12.0, 24.0, 36.0], 0x1122_3344, 1.0, 1.0));
        want.extend(vertex([11.0, 20.5, 30.25], 0x1122_3344, 0.0, 1.0));
        want.extend(vertex([99.0, 198.0, 297.0], 0x5566_7788, 0.0, 0.0));
        want.extend(vertex([99.5, 199.75, 299.875], 0x5566_7788, 1.0, 0.0));
        want.extend(vertex([101.0, 202.0, 303.0], 0x5566_7788, 1.0, 1.0));
        want.extend(vertex([100.5, 200.25, 300.125], 0x5566_7788, 0.0, 1.0));
        assert_eq!(got, want);

        // Without colors every vertex is white.
        let mut out = vec![0u8; 4 * 36];
        unsafe {
            write_point_quads(out.as_mut_ptr() as usize, 1, vertices.as_ptr() as usize, sizes.as_ptr() as usize, 0, sum, difference);
        }
        let got = words(&out);
        assert_eq!(&got[..9], &vertex([8.0, 16.0, 24.0], 0xFFFF_FFFF, 0.0, 0.0)[..]);
        assert_eq!(got[9 * 3 + 6], 0xFFFF_FFFF);
    }

    #[test]
    fn line_formats() {
        assert_eq!(lines_format(false, false), (0x002, 12));
        assert_eq!(lines_format(false, true), (0x042, 16));
        assert_eq!(lines_format(true, false), (0x102, 20));
        assert_eq!(lines_format(true, true), (0x142, 24));
    }

    #[test]
    fn line_vertices_in_the_four_formats() {
        let vertices = floats(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let colors: Vec<u8> = [0xAAu32, 0xBB].iter().flat_map(|c| c.to_le_bytes()).collect();
        let coords = floats(&[0.25, 0.5, 0.75, 1.0]);
        let (v, c, t) = (vertices.as_ptr() as usize, colors.as_ptr() as usize, coords.as_ptr() as usize);
        let f = |x: f32| x.to_bits();
        unsafe {
            let mut out = vec![0u8; 48];
            assert_eq!(write_line_vertices(out.as_mut_ptr() as usize, 2, v, c, t), t + 16);
            assert_eq!(
                words(&out),
                vec![f(1.0), f(2.0), f(3.0), 0xAA, f(0.25), f(0.5), f(4.0), f(5.0), f(6.0), 0xBB, f(0.75), f(1.0)]
            );

            let mut out = vec![0u8; 40];
            assert_eq!(write_line_vertices(out.as_mut_ptr() as usize, 2, v, 0, t), t + 16);
            assert_eq!(words(&out), vec![f(1.0), f(2.0), f(3.0), f(0.25), f(0.5), f(4.0), f(5.0), f(6.0), f(0.75), f(1.0)]);

            let mut out = vec![0u8; 32];
            assert_eq!(write_line_vertices(out.as_mut_ptr() as usize, 2, v, c, 0), 0);
            assert_eq!(words(&out), vec![f(1.0), f(2.0), f(3.0), 0xAA, f(4.0), f(5.0), f(6.0), 0xBB]);

            // Positions only: the game code repeats the first vertex.
            let mut out = vec![0u8; 24];
            assert_eq!(write_line_vertices(out.as_mut_ptr() as usize, 2, v, 0, 0), 0);
            assert_eq!(words(&out), vec![f(1.0), f(2.0), f(3.0), f(1.0), f(2.0), f(3.0)]);
        }
    }
}
