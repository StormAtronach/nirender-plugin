//! This module ports the functions of `NiDX8Renderer` for the frame: begin, end, clear and
//! present. It also ports the camera, the model and bone transforms, and the small accessors.

use core::arch::asm;
use core::mem::transmute;

use crate::d3d::{self, D3DERR_DEVICELOST, D3DERR_DEVICENOTRESET, D3DTS_PROJECTION, D3DTS_VIEW, D3DTS_WORLDMATRIX0};
use crate::engine::{self, thiscall0, thiscall1, thiscall2};
use crate::gen::layouts::{
    NiAVObject, NiDX8ConfigurableTexturePipeline as CTP, NiDX8LightManager as LM, NiDX8RenderState as RS,
    NiDX8RenderTarget as RT, NiDX8Renderer as R, NiDX8TexturePass, NiFrustum, NiMaterialProperty, NiRefObject,
    NiSkinInstance, NiSkinPartition_Partition as Partition, NiTransform as T, D3DVIEWPORT8,
};
use crate::mem::{rd, wr, x87_pass};
use crate::ports::{hash_map, ni, render_state};
use crate::shadow::Regions;
use crate::x87::{ftol_asm, ftol_count_times, ftol_difference_times_int, ftol_product};

// ---------------------------------------------------------------------------------------
// Engine data and functions at fixed addresses
// ---------------------------------------------------------------------------------------

/// `NiRTTI` of `NiRenderedCubeMap`.
const RTTI_NI_RENDERED_CUBE_MAP: usize = 0x7DEE14;
/// `NiRTTI` of `NiLight`.
const RTTI_NI_LIGHT: usize = 0x7DECF8;

// Single-precision constants of the game code, as bits.
/// `f_one_` at 0x74BFDC.
const ONE: u32 = 0x3F80_0000;
/// `flt_74C020`: -2.0.
const MINUS_TWO: u32 = 0xC000_0000;
/// `f_255_sg` at 0x74C0A0: 255.0.
const F255: u32 = 0x437F_0000;
/// `flt_74C08C`: 1 / 255.
const INVERSE_255: u32 = 0x3B80_8081;
/// `flt_74C078`: the smallest max fog value, about 1e-5.
const MIN_MAX_FOG: u32 = 0x3727_C5AC;

/// `NiDX8Renderer` + 0x28C: the second description buffer that `init` empties. It has no name
/// in the database (`field_28C`).
const DESCRIPTION_2: usize = R::field_28C;
/// `NiSkinData::transformRootParentToSkin`
const SKIN_DATA_ROOT_PARENT_TO_SKIN: usize = 0x0C;
/// `NiSkinData::boneData`
const SKIN_DATA_BONE_DATA: usize = 0x44;
/// The size of one `NiSkinData::BoneData`. Its first member is the skin-to-bone `NiTransform`.
const BONE_DATA_SIZE: usize = 76;
/// `TListItem`: data, next, previous.
const LIST_ITEM_SIZE: usize = 12;
const LIST_ITEM_NEXT: usize = 4;

/// 0x6AAE80 `NiDX8Renderer::Recreate`. It resets the device.
unsafe fn renderer_recreate(this: usize) -> u32 {
    thiscall0(0x6AAE80, this)
}

/// 0x6BB190 `NiDX8LightManager::nextFrame`
unsafe fn light_manager_next_frame(manager: usize) -> u32 {
    thiscall0(0x6BB190, manager)
}

/// 0x6BB4E0 `NiDX8LightManager::RemoveLight`
unsafe fn light_manager_remove_light(manager: usize, light: usize) -> u32 {
    thiscall1(0x6BB4E0, manager, light as u32)
}

/// 0x6B9E00 `NiDX8ConfigurableTexturePipeline::nextFrame`
unsafe fn texture_pipeline_next_frame(pipeline: usize) -> u32 {
    thiscall0(0x6B9E00, pipeline)
}

/// 0x6B9D80 `NiDX8ConfigurableTexturePipeline::setCameraData`. The caller passes the location by
/// value.
unsafe fn texture_pipeline_set_camera_data(pipeline: usize, direction: usize, up: usize, right: usize, location: [u32; 3]) {
    let f: unsafe extern "thiscall" fn(usize, usize, usize, usize, u32, u32, u32) -> u32 = transmute(0x6B9D80usize);
    f(pipeline, direction, up, right, location[0], location[1], location[2]);
}

/// 0x6B8600 `NiDX8RenderState::PurgeMaterial`
unsafe fn render_state_purge_material(render_state: usize, material: usize) -> u32 {
    thiscall1(0x6B8600, render_state, material as u32)
}

/// 0x402EB0 `nullsub_29`: `retn 4`. The game code calls it with the render state in ECX.
unsafe fn nullsub_29(render_state: usize, transform: usize) {
    thiscall1(0x402EB0, render_state, transform as u32);
}

/// 0x6E81B0 `matrix33::multiplyValue`
unsafe fn matrix33_multiply_value(matrix: usize, out: usize, factor: u32) {
    thiscall2(0x6E81B0, matrix, out as u32, factor);
}

/// 0x6E8290 `matrix33_static::transpose_mul_vec3`. It is `__cdecl`. It returns `out`.
unsafe fn matrix33_transpose_mul_vec3(out: usize, vector: usize, matrix: usize) -> usize {
    engine::cdecl3(0x6E8290, out as u32, vector as u32, matrix as u32) as usize
}

/// 0x40A7E0 `vec3::copy`
unsafe fn vec3_copy(to: usize, from: usize) {
    thiscall1(0x40A7E0, to, from as u32);
}

/// 0x6FE0A0 `NiTransform::Invert`
unsafe fn transform_invert(transform: usize, out: usize) {
    thiscall1(0x6FE0A0, transform, out as u32);
}

/// 0x6ACE00 `NiTransform::combine`: `out = this * other`. It returns `out`.
unsafe fn transform_combine(transform: usize, out: usize, other: usize) -> usize {
    thiscall2(0x6ACE00, transform, out as u32, other as u32) as usize
}

/// 0x6ACEC0 `NiTransform::copy2`
unsafe fn transform_copy2(to: usize, from: usize) {
    thiscall1(0x6ACEC0, to, from as u32);
}

// ---------------------------------------------------------------------------------------
// x87 sequences
// ---------------------------------------------------------------------------------------

/// The `D3DRECT` of `ClearBuffer` (0x6ABEA2 to 0x6ABF00) from the size of the render target
/// and a `NiRectFloat` (left, right, top, bottom). The width stays on the x87 stack as an
/// exact integer. The height goes through a float in memory.
unsafe fn clear_rect(width: u32, height: u32, rect: usize) -> [u32; 4] {
    // +16 width (64 bits), +24 height (64 bits), +32 height as float, +36 1.0, +40 the result
    let mut s = [0u32, 0, 0, 0, width, 0, height, 0, 0, ONE, 0, 0, 0, 0];
    asm!(
        "fild qword ptr [{s} + 16]",
        "fild qword ptr [{s} + 24]",
        "fstp dword ptr [{s} + 32]",
        "fld st(0)",
        "fmul dword ptr [{r}]",
        ftol_asm!(),
        "mov dword ptr [{s} + 40], eax",
        "fld dword ptr [{s} + 36]",
        "fsub dword ptr [{r} + 8]",
        "fmul dword ptr [{s} + 32]",
        ftol_asm!(),
        "mov dword ptr [{s} + 44], eax",
        "fmul dword ptr [{r} + 4]",
        ftol_asm!(),
        "mov dword ptr [{s} + 48], eax",
        "fld dword ptr [{s} + 36]",
        "fsub dword ptr [{r} + 12]",
        "fmul dword ptr [{s} + 32]",
        ftol_asm!(),
        "mov dword ptr [{s} + 52], eax",
        s = in(reg) s.as_mut_ptr(),
        r = in(reg) rect,
        out("eax") _,
        clobber_abi("C"),
        options(nostack),
    );
    [s[10], s[11], s[12], s[13]]
}

/// `fchs`
#[inline(always)]
fn chs(value: f32) -> f32 {
    f32::from_bits(value.to_bits() ^ 0x8000_0000)
}

#[inline(always)]
unsafe fn float(base: usize, offset: usize) -> f32 {
    rd(base, offset)
}

/// The offset of the element in row `row` and column `column` of a `D3DMATRIX`. The numbers
/// start at 1.
const fn element(matrix: usize, row: usize, column: usize) -> usize {
    matrix + ((row - 1) * 4 + (column - 1)) * 4
}

// ---------------------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------------------

/// The view matrix of `SetCameraData` (0x6AC635 to 0x6AC6FF).
unsafe fn write_view_matrix(this: usize, location: usize, direction: usize, up: usize, right: usize) {
    let view = R::d3dViewMat;
    for row in 0..3 {
        wr::<u32>(this, element(view, row + 1, 1), rd(right, row * 4));
        wr::<u32>(this, element(view, row + 1, 2), rd(up, row * 4));
        wr::<u32>(this, element(view, row + 1, 3), rd(direction, row * 4));
    }
    let mut sum = float(location, 0) * float(right, 0);
    sum = sum + float(right, 8) * float(location, 8);
    sum = sum + float(location, 4) * float(right, 4);
    wr(this, element(view, 4, 1), chs(sum));

    let mut sum = float(location, 8) * float(up, 8);
    sum = sum + float(location, 0) * float(up, 0);
    sum = sum + float(location, 4) * float(up, 4);
    wr(this, element(view, 4, 2), chs(sum));

    let mut sum = float(location, 4) * float(direction, 4);
    sum = sum + float(location, 0) * float(direction, 0);
    let last = float(location, 8) * float(direction, 8);
    wr::<u32>(this, element(view, 4, 4), ONE);
    wr::<u32>(this, element(view, 1, 4), 0);
    wr::<u32>(this, element(view, 2, 4), 0);
    sum = sum + last;
    wr::<u32>(this, element(view, 3, 4), 0);
    wr(this, element(view, 4, 3), chs(sum));
}

/// The projection matrix of `SetCameraData` (0x6AC7A5 to 0x6AC8B6), with the frustum values
/// that the renderer keeps. `left_right_swap` is `NiDX8RenderState::bLeftRightSwap`.
unsafe fn write_projection(this: usize, frustum: usize, left_right_swap: u32) {
    let proj = R::d3dProjMat;
    wr::<u32>(this, R::frustumNear, rd(frustum, NiFrustum::near));
    let depth = float(frustum, NiFrustum::far) - float(frustum, NiFrustum::near);
    wr(this, R::frustumDistance, depth);
    let width = float(frustum, NiFrustum::right) - float(frustum, NiFrustum::left);
    let left_plus_right = float(frustum, NiFrustum::left) + float(frustum, NiFrustum::right);
    let height = float(frustum, NiFrustum::top) - float(frustum, NiFrustum::bottom);
    let top_plus_bottom = float(frustum, NiFrustum::top) + float(frustum, NiFrustum::bottom);
    wr::<u32>(this, element(proj, 4, 1), 0);
    wr::<u32>(this, element(proj, 2, 1), 0);
    let one = f32::from_bits(ONE);
    let inverse_depth = one / depth;
    let inverse_width = one / width;
    let m31 = if left_right_swap != 0 {
        wr(this, element(proj, 1, 1), f32::from_bits(MINUS_TWO) * inverse_width);
        inverse_width * left_plus_right
    } else {
        wr(this, element(proj, 1, 1), inverse_width + inverse_width);
        chs(inverse_width * left_plus_right)
    };
    wr(this, element(proj, 3, 1), m31);
    let inverse_height = one / height;
    wr::<u32>(this, element(proj, 1, 2), 0);
    wr::<u32>(this, element(proj, 4, 2), 0);
    wr::<u32>(this, element(proj, 1, 3), 0);
    wr::<u32>(this, element(proj, 2, 3), 0);
    wr(this, element(proj, 2, 2), inverse_height + inverse_height);
    wr(this, element(proj, 3, 2), chs(inverse_height * top_plus_bottom));
    wr(this, element(proj, 3, 3), inverse_depth * float(frustum, NiFrustum::far));
    let far_near = float(frustum, NiFrustum::far) * float(frustum, NiFrustum::near);
    wr::<u32>(this, element(proj, 1, 4), 0);
    wr::<u32>(this, element(R::screenPolyProj, 3, 3), rd(this, element(proj, 3, 3)));
    wr::<u32>(this, element(proj, 2, 4), 0);
    wr::<u32>(this, element(proj, 4, 4), 0);
    wr::<u32>(this, element(proj, 3, 4), ONE);
    wr(this, element(proj, 4, 3), chs(far_near * inverse_depth));
    wr::<u32>(this, element(R::screenPolyProj, 4, 3), rd(this, element(proj, 4, 3)));
}

/// The `D3DVIEWPORT8` of `SetCameraData` (0x6AC8C6 to 0x6AC96E) from the size of the render
/// target and the `NiRectFloat` of the camera (left, right, top, bottom).
unsafe fn camera_viewport(target: usize, port: usize) -> [u32; 6] {
    let x = ftol_count_times(rd(target, RT::width), rd(port, 0));
    let y = ftol_difference_times_int(ONE, rd(port, 8), rd(target, RT::height));
    let width = ftol_difference_times_int(rd(port, 4), rd(port, 0), rd(target, RT::width));
    let height = ftol_difference_times_int(rd(port, 8), rd(port, 12), rd(target, RT::height));
    [x, y, width, height, 0, ONE]
}

/// 0x6AC620 `NiDX8Renderer::SetCameraData`
pub unsafe fn set_camera_data(this: usize, location: usize, direction: usize, up: usize, right: usize, frustum: usize, port: usize) {
    crate::shadow::touch(&set_camera_data_port::STATE);
    write_view_matrix(this, location, direction, up, right);
    d3d::device_set_transform(rd(this, R::d3dDevice), D3DTS_VIEW, this + R::d3dViewMat);

    wr::<[u32; 3]>(this, R::modelCameraRight, rd(right, 0));
    wr::<[u32; 3]>(this, R::cameraRight, rd(right, 0));
    wr::<[u32; 3]>(this, R::modelCameraUp, rd(up, 0));
    wr::<[u32; 3]>(this, R::cameraUp, rd(up, 0));
    let pipeline: usize = rd(this, R::configurableTexturePipeline);
    if pipeline != 0 {
        texture_pipeline_set_camera_data(pipeline, direction, up, right, rd(location, 0));
    }

    let render_state: usize = rd(this, R::renderState);
    write_projection(this, frustum, rd(render_state, RS::bLeftRightSwap));
    d3d::device_set_transform(rd(this, R::d3dDevice), D3DTS_PROJECTION, this + R::d3dProjMat);

    let viewport = camera_viewport(rd(this, R::currentRenderTarget), port);
    d3d::device_set_viewport(rd(this, R::d3dDevice), viewport.as_ptr() as usize);

    let render_state: usize = rd(this, R::renderState);
    if render_state != 0 {
        let far: u32 = rd(frustum, NiFrustum::far);
        let near: u32 = rd(frustum, NiFrustum::near);
        wr(render_state, RS::frustumNear, x87_pass(near));
        wr(render_state, RS::frustumFar, x87_pass(far));
        wr(render_state, RS::frustumDistance, f32::from_bits(far) - f32::from_bits(near));
    }
}

/// The call is never a skip.
unsafe fn set_camera_data_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
    set.add(rd(this, R::renderState), RS::SIZE);
    set.add(rd(this, R::configurableTexturePipeline), CTP::SIZE);
}

// ---------------------------------------------------------------------------------------
// Model and bone transforms
// ---------------------------------------------------------------------------------------

/// This function makes `currentWorldMatrix` from a `NiTransform`. The matrix has the transpose
/// of the rotation times the scale. The translation is in the last row.
pub(crate) unsafe fn write_world_matrix(this: usize, transform: usize) {
    let world = R::currentWorldMatrix;
    let scaled = |offset: usize| float(transform, offset) * float(transform, T::scale);
    wr(this, element(world, 1, 1), scaled(T::rotation_m0_x));
    wr(this, element(world, 1, 2), scaled(T::rotation_m1_x));
    wr(this, element(world, 1, 3), scaled(T::rotation_m2_x));
    wr(this, element(world, 2, 1), scaled(T::rotation_m0_y));
    wr(this, element(world, 2, 2), scaled(T::rotation_m1_y));
    wr(this, element(world, 2, 3), scaled(T::rotation_m2_y));
    wr(this, element(world, 3, 1), scaled(T::rotation_m0_z));
    wr(this, element(world, 3, 2), scaled(T::rotation_m1_z));
    wr(this, element(world, 3, 3), scaled(T::rotation_m2_z));
    wr::<u32>(this, element(world, 4, 1), rd(transform, T::translation_x));
    wr::<u32>(this, element(world, 4, 2), rd(transform, T::translation_y));
    wr::<u32>(this, element(world, 4, 3), rd(transform, T::translation_z));
    wr::<u32>(this, element(world, 1, 4), 0);
    wr::<u32>(this, element(world, 2, 4), 0);
    wr::<u32>(this, element(world, 3, 4), 0);
    wr::<u32>(this, element(world, 4, 4), ONE);
}

/// The end of `SetModelTransform` and of `SetSkinnedModelTransforms`. It calculates the camera
/// right and up vectors in model space.
pub(crate) unsafe fn update_model_camera(this: usize, transform: usize) {
    nullsub_29(rd(this, R::renderState), transform);
    let mut matrix = [0u32; 9];
    let mut vector = [0u32; 3];
    let (matrix, vector) = (matrix.as_mut_ptr() as usize, vector.as_mut_ptr() as usize);
    matrix33_multiply_value(transform + T::rotation, matrix, rd(transform, T::scale));
    let result = matrix33_transpose_mul_vec3(vector, this + R::cameraRight, matrix);
    wr::<[u32; 3]>(this, R::modelCameraRight, rd(result, 0));
    let result = matrix33_transpose_mul_vec3(vector, this + R::cameraUp, matrix);
    wr::<[u32; 3]>(this, R::modelCameraUp, rd(result, 0));
}

/// 0x6AC9C0 `NiDX8Renderer::SetModelTransform`
pub unsafe fn set_model_transform(this: usize, transform: usize) {
    crate::shadow::touch(&set_model_transform_port::STATE);
    write_world_matrix(this, transform);
    d3d::device_set_transform(rd(this, R::d3dDevice), D3DTS_WORLDMATRIX0, this + R::currentWorldMatrix);
    update_model_camera(this, transform);
}

/// 0x6ACB10 `NiDX8Renderer::SetBoneTransform`
pub unsafe fn set_bone_transform(this: usize, transform: usize, bone_index: u32) {
    crate::shadow::touch(&set_bone_transform_port::STATE);
    write_world_matrix(this, transform);
    d3d::device_set_transform(rd(this, R::d3dDevice), bone_index.wrapping_add(D3DTS_WORLDMATRIX0), this + R::currentWorldMatrix);
}

/// This function copies a `NiTransform` as the game code does at 0x6ACC29 and 0x6ACD1E.
unsafe fn copy_transform(to: usize, from: usize) {
    engine::matrix33_copy(to + T::rotation, from + T::rotation);
    vec3_copy(to + T::translation, from + T::translation);
    wr::<u32>(to, T::scale, rd(from, T::scale));
}

/// 0x6ACBE0 `NiDX8Renderer::SetSkinnedModelTransforms`. The game code does not read `bound`.
pub unsafe fn set_skinned_model_transforms(this: usize, skin: usize, partition: usize, transform: usize, _bound: usize) {
    crate::shadow::touch(&set_skinned_model_transforms_port::STATE);
    let buffer = || [0u32; T::SIZE / 4];
    let (mut inverse_root, mut scratch, mut skin_to_world, mut combined) = (buffer(), buffer(), buffer(), buffer());
    let (mut bone_world, mut bone) = (buffer(), buffer());
    let inverse_root = inverse_root.as_mut_ptr() as usize;
    let scratch = scratch.as_mut_ptr() as usize;
    let skin_to_world = skin_to_world.as_mut_ptr() as usize;
    let combined = combined.as_mut_ptr() as usize;
    let bone_world = bone_world.as_mut_ptr() as usize;
    let bone = bone.as_mut_ptr() as usize;

    transform_invert(rd::<usize>(skin, NiSkinInstance::rootParent) + NiAVObject::worldTransform, inverse_root);
    let result = transform_combine(transform, scratch, rd::<usize>(skin, NiSkinInstance::skinData) + SKIN_DATA_ROOT_PARENT_TO_SKIN);
    copy_transform(skin_to_world, result);
    let result = transform_combine(skin_to_world, scratch, inverse_root);
    transform_copy2(combined, result);

    let mut index: u16 = 0;
    if rd::<u16>(partition, Partition::numBones) > index {
        loop {
            let bone_id = rd::<u16>(rd(partition, Partition::bones), index as usize * 2) as usize;
            let bone_object: usize = rd(rd(skin, NiSkinInstance::bones), bone_id * 4);
            let result = transform_combine(combined, scratch, bone_object + NiAVObject::worldTransform);
            engine::matrix33_copy(bone_world + T::rotation, result + T::rotation);
            wr::<[u32; 3]>(bone_world, T::translation, rd(result, T::translation));
            wr::<u32>(bone_world, T::scale, rd(result, T::scale));
            let skin_data: usize = rd(skin, NiSkinInstance::skinData);
            let bone_data: usize = rd(skin_data, SKIN_DATA_BONE_DATA);
            // The game code gives the buffer of the inverse root transform as the output here.
            let result = transform_combine(bone_world, inverse_root, bone_data + bone_id * BONE_DATA_SIZE);
            copy_transform(bone, result);
            set_bone_transform(this, bone, index as u32);
            index = index.wrapping_add(1);
            if index >= rd::<u16>(partition, Partition::numBones) {
                break;
            }
        }
    }
    update_model_camera(this, transform);
}

// ---------------------------------------------------------------------------------------
// Frame
// ---------------------------------------------------------------------------------------

/// 0x6AC520 `NiDX8Renderer::BeginPaint`. The game code does not read `camera`.
pub unsafe fn begin_paint(this: usize, _camera: usize) {
    crate::shadow::touch(&begin_paint_port::STATE);
    let status = d3d::device_test_cooperative_level(rd(this, R::d3dDevice));
    if status == D3DERR_DEVICENOTRESET {
        renderer_recreate(this);
    } else if status == D3DERR_DEVICELOST {
        return;
    }
    render_state::next_frame(rd(this, R::renderState));
    light_manager_next_frame(rd(this, R::lightManager));
    let pipeline: usize = rd(this, R::configurableTexturePipeline);
    if pipeline != 0 {
        texture_pipeline_next_frame(pipeline);
    }
    if d3d::device_begin_scene(rd(this, R::d3dDevice)) == 0 {
        wr::<u8>(this, R::bSceneOpen, 1);
    }
}

/// Adds each item of the device light list. `NiDX8LightManager::nextFrame` moves these items
/// to the free list.
unsafe fn add_light_list_items(set: &mut Regions, manager: usize) {
    let count: u32 = rd(manager, LM::deviceLights_count);
    let mut item: usize = rd(manager, LM::deviceLights_head);
    let mut done = 0;
    while item != 0 && done < count {
        set.add(item, LIST_ITEM_SIZE);
        done += 1;
        item = rd(item, LIST_ITEM_NEXT);
    }
}

/// The call is a skip when `TestCooperativeLevel` gives `D3DERR_DEVICENOTRESET`. `Recreate`
/// resets the device then.
///
/// The dry device returns 0 for `BeginScene`. `bSceneOpen` depends on that result. Thus a call
/// where the real device fails shows as a mismatch.
unsafe fn begin_paint_regions(set: &mut Regions, this: usize) {
    let status = d3d::device_test_cooperative_level(rd(this, R::d3dDevice));
    if status == D3DERR_DEVICENOTRESET {
        set.skip();
        return;
    }
    set.add(this, R::SIZE);
    if status == D3DERR_DEVICELOST {
        return;
    }
    set.add(rd(this, R::renderState), RS::SIZE);
    let manager: usize = rd(this, R::lightManager);
    set.add(manager, LM::SIZE);
    let pipeline: usize = rd(this, R::configurableTexturePipeline);
    if pipeline != 0 {
        set.add(pipeline, CTP::SIZE);
        // `nextFrame` of the pipeline calls a virtual function of its active texture pass.
        set.add(rd(pipeline, CTP::field_8), NiDX8TexturePass::SIZE);
    }
    add_light_list_items(set, manager);
}

/// 0x6AC590 `NiDX8Renderer::EndPaint`
pub unsafe fn end_paint(this: usize) {
    crate::shadow::touch(&end_paint_port::STATE);
    if rd::<u8>(this, R::bSceneOpen) != 0 && d3d::device_end_scene(rd(this, R::d3dDevice)) == 0 {
        wr::<u8>(this, R::bSceneOpen, 0);
    }
}

/// The call is never a skip. The dry device returns 0 for `EndScene`. `bSceneOpen` depends on
/// that result. Thus a call where the real device fails shows as a mismatch.
unsafe fn end_paint_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
}

/// 0x6ABE40 `NiDX8Renderer::SwapBuffers`. The port presents nothing when the target is a texture.
pub unsafe fn swap_buffers(this: usize) {
    crate::shadow::touch(&swap_buffers_port::STATE);
    let target: usize = rd(this, R::currentRenderTarget);
    if rd::<usize>(target, RT::d3dTexture) == 0 {
        d3d::device_present(rd(this, R::d3dDevice), 0, 0, 0, 0);
    }
}

/// The call is a skip when the target is not a texture. The port does a `Present` then. The
/// recorder runs the frame hook at each `Present`, also at a `Present` that it does not pass on.
/// Two `Present` calls in one frame break the frame comparison.
unsafe fn swap_buffers_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
    let target: usize = rd(this, R::currentRenderTarget);
    if rd::<usize>(target, RT::d3dTexture) == 0 {
        set.skip();
    }
}

/// 0x6ABE60 `NiDX8Renderer::ClearBuffer`. `rect` is a `NiRectFloat`, or 0 for the full
/// target. `flags` has bit 0 for the colour buffer, bit 1 for stencil and bit 2 for depth.
pub unsafe fn clear_buffer(this: usize, rect: usize, flags: u8) {
    crate::shadow::touch(&clear_buffer_port::STATE);
    let status = d3d::device_test_cooperative_level(rd(this, R::d3dDevice));
    if status == D3DERR_DEVICENOTRESET {
        renderer_recreate(this);
    } else if status == D3DERR_DEVICELOST {
        return;
    }
    let target: usize = rd(this, R::currentRenderTarget);
    let width: u32 = rd(target, RT::width);
    let height: u32 = rd(target, RT::height);
    let rects = if rect != 0 { clear_rect(width, height, rect) } else { [0, 0, width, height] };

    let flags = flags as u32;
    let has_depth_stencil = rd::<u32>(target, RT::d3dSurfaceDepthStencil) != 0;
    let mut clear_flags = flags & 1; // D3DCLEAR_TARGET
    if flags & 4 != 0 && has_depth_stencil {
        clear_flags |= 2; // D3DCLEAR_ZBUFFER
    }
    if flags & 2 != 0 && rd::<u8>(this, R::capabilityFlags) & 0x80 != 0 && has_depth_stencil {
        clear_flags |= 4; // D3DCLEAR_STENCIL
    }
    wr::<u32>(this, R::d3dViewport_Width, rd(target, RT::width));
    wr::<u32>(this, R::d3dViewport_Height, rd(target, RT::height));
    d3d::device_set_viewport(rd(this, R::d3dDevice), this + R::d3dViewport);
    d3d::device_clear(
        rd(this, R::d3dDevice),
        1,
        rects.as_ptr() as usize,
        clear_flags,
        rd(this, R::backgroundColour),
        rd(this, R::depthClearValue),
        rd(this, R::stencilClearValue),
    );
}

/// The call is a skip when `TestCooperativeLevel` gives `D3DERR_DEVICENOTRESET`. `Recreate`
/// resets the device then.
unsafe fn clear_buffer_regions(set: &mut Regions, this: usize) {
    if d3d::device_test_cooperative_level(rd(this, R::d3dDevice)) == D3DERR_DEVICENOTRESET {
        set.skip();
    }
    set.add(this, R::SIZE);
}

/// 0x6ABD20 `NiDX8Renderer::SetRenderTarget`. `texture` is a `NiRenderedTexture`, or 0 for
/// the back buffer. The port returns 1 when the device has the new target.
pub unsafe fn set_render_target(this: usize, texture: usize) -> u8 {
    crate::shadow::touch(&set_render_target_port::STATE);
    if rd::<u8>(this, R::bSceneOpen) != 0 {
        return 0;
    }
    let target = if texture == 0 {
        this + R::backbufferRenderTarget
    } else {
        let map = if ni::is_kind_of(texture, RTTI_NI_RENDERED_CUBE_MAP) {
            this + R::hashmap_maybeRenderedCubemaps
        } else {
            this + R::hashmap_maybeRenderedTextures
        };
        let data = hash_map::find_value(map, texture as u32);
        if data == 0 {
            return 0;
        }
        // The virtual function at +0x28 of the renderer data of the texture gives its render target.
        engine::vcall0(data, 0x28) as usize
    };
    let result = d3d::device_set_render_target(rd(this, R::d3dDevice), rd(target, RT::d3dSurfaceBackBuffer), rd(target, RT::d3dSurfaceDepthStencil));
    if result != 0 {
        return 0;
    }
    wr(this, R::currentRenderTarget, target);
    1
}

/// The call is never a skip. The dry device returns 0 for `SetRenderTarget`.
/// `currentRenderTarget` and the return value depend on that result. Thus a call where the
/// real device fails shows as a mismatch.
unsafe fn set_render_target_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
}

// ---------------------------------------------------------------------------------------
// Batch state, purges
// ---------------------------------------------------------------------------------------

/// Adds the memory that `ni::pointer_assign` writes outside the object that has the slot. The
/// call is a skip when the assignment deletes the old object. A virtual deleting destructor
/// cannot run two times.
unsafe fn assign_pointer_regions(set: &mut Regions, slot: usize, object: usize) {
    let old: usize = rd(slot, 0);
    if old == object {
        return;
    }
    if old != 0 && rd::<u32>(old, NiRefObject::refCount) == 1 {
        set.skip();
    }
    set.add(old, NiRefObject::SIZE);
    set.add(object, NiRefObject::SIZE);
}

/// 0x6AE030 `NiDX8Renderer::BeginBatch`
pub unsafe fn begin_batch(this: usize, property_state: usize, effect_state: usize) {
    crate::shadow::touch(&begin_batch_port::STATE);
    ni::pointer_assign(this + R::propertyStatePtr, property_state);
    ni::pointer_assign(this + R::effectStatePtr, effect_state);
}

/// The call is a skip when it deletes a state that it replaces. See `assign_pointer_regions`.
unsafe fn begin_batch_regions(set: &mut Regions, this: usize, property_state: usize, effect_state: usize) {
    set.add(this, R::SIZE);
    assign_pointer_regions(set, this + R::propertyStatePtr, property_state);
    assign_pointer_regions(set, this + R::effectStatePtr, effect_state);
}

/// 0x6AC5C0 `NiDX8Renderer::PurgeMaterial`. It is a jump to the function of the render state.
pub unsafe fn purge_material(this: usize, material: usize) {
    crate::shadow::touch(&purge_material_port::STATE);
    render_state_purge_material(rd(this, R::renderState), material);
}

/// The call is a skip when the material has an entry. The material manager frees the entry.
unsafe fn purge_material_regions(set: &mut Regions, this: usize, material: usize) {
    set.add(this, R::SIZE);
    if material != 0 && rd::<usize>(material, NiMaterialProperty::rendererData) != 0 {
        set.skip();
    }
}

/// 0x6AC5D0 `NiDX8Renderer::PurgeEffect`. Only a light has data in the renderer.
pub unsafe fn purge_effect(this: usize, effect: usize) {
    crate::shadow::touch(&purge_effect_port::STATE);
    if effect != 0 && ni::is_kind_of(effect, RTTI_NI_LIGHT) {
        light_manager_remove_light(rd(this, R::lightManager), effect);
    }
}

/// The call is a skip when the effect is a light. `RemoveLight` frees the entry of the light and
/// its hash map node.
unsafe fn purge_effect_regions(set: &mut Regions, this: usize, effect: usize) {
    set.add(this, R::SIZE);
    if effect != 0 && ni::is_kind_of(effect, RTTI_NI_LIGHT) {
        set.skip();
    }
}

// ---------------------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------------------

/// 0x6A9D70 `NiDX8Renderer::GetFlags`
pub unsafe fn get_flags(this: usize) -> u32 {
    crate::shadow::touch(&get_flags_port::STATE);
    rd(this, R::capabilityFlags)
}

/// 0x6A9C70 `NiDX8Renderer::SetDepthClear`. The port copies the float as an integer.
pub unsafe fn set_depth_clear(this: usize, depth: u32) {
    crate::shadow::touch(&set_depth_clear_port::STATE);
    wr(this, R::depthClearValue, depth);
}

/// 0x6A9D60 `NiDX8Renderer::SetStencilClear`
pub unsafe fn set_stencil_clear(this: usize, stencil: u32) {
    crate::shadow::touch(&set_stencil_clear_port::STATE);
    wr(this, R::stencilClearValue, stencil);
}

/// 0x6A9C80 `NiDX8Renderer::SetBgColor`. `color` is a `NiColor`. Alpha becomes 255.
pub unsafe fn set_bg_color(this: usize, color: usize) -> u8 {
    crate::shadow::touch(&set_bg_color_port::STATE);
    let red = ftol_product(rd(color, 0), F255);
    let mut packed = ((red & 0xFF) | 0xFFFF_FF00) << 8;
    let green = ftol_product(rd(color, 4), F255);
    packed = (packed | (green & 0xFF)) << 8;
    let blue = ftol_product(rd(color, 8), F255);
    packed |= blue & 0xFF;
    wr(this, R::backgroundColour, packed);
    1
}

/// 0x6A9CF0 `NiDX8Renderer::GetBgColor`. The port fills the `NiColor` at `color` and returns it.
pub unsafe fn get_bg_color(this: usize, color: usize) -> usize {
    crate::shadow::touch(&get_bg_color_port::STATE);
    let scale = f32::from_bits(INVERSE_255);
    let red = rd::<u8>(this, R::backgroundColour_r) as f32 * scale;
    let green = rd::<u8>(this, R::backgroundColour_g) as f32 * scale;
    let blue = (rd::<u32>(this, R::backgroundColour) & 0xFF) as f32 * scale;
    wr(color, 0, red);
    wr(color, 4, green);
    wr(color, 8, blue);
    color
}

/// 0x6ABFB0 `NiDX8Renderer::GetLeftRightSwap`
pub unsafe fn get_left_right_swap(this: usize) -> u8 {
    crate::shadow::touch(&get_left_right_swap_port::STATE);
    let render_state: usize = rd(this, R::renderState);
    (rd::<u32>(render_state, RS::bLeftRightSwap) != 0) as u8
}

/// 0x6AC310 `NiDX8Renderer::GetMaxFogValue`. The result is on the x87 stack.
pub unsafe fn get_max_fog_value(this: usize) -> f32 {
    crate::shadow::touch(&get_max_fog_value_port::STATE);
    rd(rd::<usize>(this, R::renderState), RS::maxFogValue)
}

/// 0x6F5670 `NiRenderer::GetMaxFogValue`: the base class gives 1.
pub unsafe fn base_get_max_fog_value(_this: usize) -> f32 {
    crate::shadow::touch(&base_get_max_fog_value_port::STATE);
    1.0
}

/// 0x6F5650 `NiRenderer::SetLeftRightSwap`: the base class cannot swap, and returns 0.
pub unsafe fn base_set_left_right_swap(_this: usize, _swap: u8) -> u8 {
    crate::shadow::touch(&base_set_left_right_swap_port::STATE);
    0
}

/// 0x6AC370 `NiDX8Renderer::PrecacheTexture`: gives the call to the texture manager. The game
/// code jumps to the function of the manager, so the result is the result of that function.
pub unsafe fn precache_texture(this: usize, texture: usize, a3: u32, a4: u32) -> u32 {
    crate::shadow::touch(&precache_texture_port::STATE);
    let manager: usize = rd(this, R::textureManager);
    engine::thiscall3(engine::virtual_at(manager, 0xC), manager, texture as u32, a3, a4)
}

/// 0x6F53E0 `NiRenderer::BatchRenderShape`: the base class gives the call to `RenderShape`.
pub unsafe fn base_batch_render_shape(this: usize, geometry_data: usize, skin: usize, transform: usize, bound: usize) -> u32 {
    crate::shadow::touch(&base_batch_render_shape_port::STATE);
    engine::thiscall4(engine::virtual_at(this, 0xBC), this, geometry_data as u32, skin as u32, transform as u32, bound as u32)
}

/// 0x6F53F0 `NiRenderer::BatchRenderStrips`: the base class gives the call to `RenderTriStrips`.
pub unsafe fn base_batch_render_strips(this: usize, geometry_data: usize, skin: usize, transform: usize, bound: usize) -> u32 {
    crate::shadow::touch(&base_batch_render_strips_port::STATE);
    engine::thiscall4(engine::virtual_at(this, 0xC0), this, geometry_data as u32, skin as u32, transform as u32, bound as u32)
}

/// 0x6ABFD0 `NiDX8Renderer::SetLeftRightSwap`
pub unsafe fn set_left_right_swap(this: usize, swap: u8) -> u8 {
    crate::shadow::touch(&set_left_right_swap_port::STATE);
    let render_state: usize = rd(this, R::renderState);
    wr(render_state, RS::bLeftRightSwap, (swap != 0) as u32);
    1
}

/// 0x6ABFF0 `NiDX8Renderer::SetUseWBuffer`. The port returns 0 when the device has no w-buffer.
pub unsafe fn set_use_w_buffer(this: usize, use_w_buffer: u8) -> u8 {
    crate::shadow::touch(&set_use_w_buffer_port::STATE);
    render_state::set_z_buffer_enable(rd(this, R::renderState), use_w_buffer);
    if use_w_buffer != 0 && rd::<u32>(this, R::capabilityFlags) & 0x100 == 0 {
        return 0;
    }
    1
}

/// 0x6AC030 `NiDX8Renderer::GetBackbufferWidth`. The port returns the width of the current
/// render target.
pub unsafe fn get_backbuffer_width(this: usize) -> u32 {
    crate::shadow::touch(&get_backbuffer_width_port::STATE);
    rd(rd(this, R::currentRenderTarget), RT::width)
}

/// 0x6AC040 `NiDX8Renderer::GetBackbufferHeight`. The port returns the height of the current
/// render target.
pub unsafe fn get_backbuffer_height(this: usize) -> u32 {
    crate::shadow::touch(&get_backbuffer_height_port::STATE);
    rd(rd(this, R::currentRenderTarget), RT::height)
}

/// 0x6AC320 `NiDX8Renderer::SetMaxFogValue`. Zero and negative values become the minimum.
pub unsafe fn set_max_fog_value(this: usize, value: f32) {
    crate::shadow::touch(&set_max_fog_value_port::STATE);
    let render_state: usize = rd(this, R::renderState);
    let mut bits = x87_pass(value.to_bits());
    // A NaN does not go into this branch. The game code tests "less" and "equal" with the parity
    // flag.
    if value < 0.0 || value == 0.0 {
        bits = MIN_MAX_FOG;
    }
    wr(render_state, RS::maxFogValue, bits);
    let one = f32::from_bits(ONE);
    let inverse = one / f32::from_bits(bits);
    wr(render_state, RS::inverseMaxFogValue, inverse - one);
}

// ---------------------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------------------

/// This function copies a C string with its end, as the loops of `init` do.
unsafe fn copy_string(to: usize, from: usize) {
    let mut at = 0;
    loop {
        let byte: u8 = rd(from, at);
        wr(to, at, byte);
        at += 1;
        if byte == 0 {
            break;
        }
    }
}

unsafe fn zero_words(base: usize, offset: usize, words: usize) {
    for word in 0..words {
        wr::<u32>(base, offset + word * 4, 0);
    }
}

/// `init` with the addresses of the data that it reads.
unsafe fn init_from(this: usize, empty_string: usize, x_axis: usize, y_axis: usize) {
    wr::<u32>(this, R::d3dDevice, 0);
    wr::<u32>(this, R::windowDevice, 0);
    copy_string(this + R::driverDesc, empty_string);
    wr::<u32>(this, R::currentAdapterIndex, 0);
    wr::<u32>(this, R::d3dDeviceType, 1); // D3DDEVTYPE_HAL
    wr::<u32>(this, R::focusWindow, 0);
    wr::<u32>(this, R::d3dBehaviorFlags, 0);
    zero_words(this, R::d3dPresentParameters, 13);
    wr::<u32>(this, R::systemDesc, 0);
    wr::<u32>(this, R::countHWBones, 0);
    wr::<u8>(this, R::maybe_bMipmapCubeMaps, 0);
    wr::<u32>(this, R::backgroundColour, 0xFF80_8080);
    wr::<u32>(this, R::depthClearValue, ONE);
    wr::<u32>(this, R::stencilClearValue, 0);
    wr::<u32>(this, R::capabilityFlags, 0);
    copy_string(this + DESCRIPTION_2, empty_string);

    wr::<[u32; 3]>(this, R::cameraRight, rd(x_axis, 0));
    wr::<[u32; 3]>(this, R::cameraUp, rd(y_axis, 0));
    wr::<[u32; 3]>(this, R::modelCameraRight, rd(x_axis, 0));
    wr::<[u32; 3]>(this, R::modelCameraUp, rd(y_axis, 0));

    let (view, proj) = (R::screenPolyView, R::screenPolyProj);
    let two = 2.0f32.to_bits();
    let minus_one = (-1.0f32).to_bits();
    wr::<u32>(this, element(proj, 1, 1), two);
    wr::<u32>(this, element(proj, 3, 1), minus_one);
    wr::<u32>(this, element(proj, 2, 2), two);
    wr::<u32>(this, element(proj, 3, 2), minus_one);
    wr::<u32>(this, R::frustumNear, 0.1f32.to_bits());
    wr::<u32>(this, R::frustumDistance, 100.0f32.to_bits());
    for row in 1..=4 {
        for column in 1..=4 {
            wr::<u32>(this, element(view, row, column), if row == column { ONE } else { 0 });
        }
    }
    for (row, column, value) in
        [(2, 1, 0), (4, 1, 0), (1, 2, 0), (4, 2, 0), (1, 3, 0), (2, 3, 0), (3, 3, ONE), (4, 3, 0), (1, 4, 0), (2, 4, 0), (3, 4, ONE), (4, 4, 0)]
    {
        wr::<u32>(this, element(proj, row, column), value);
    }
    zero_words(this, R::matrixD3DIdentity, 16);
    for diagonal in [1, 4, 3, 2] {
        wr::<u32>(this, element(R::matrixD3DIdentity, diagonal, diagonal), ONE);
    }
    wr::<u32>(this, R::batchedObjectListHead, 0);
    wr::<u32>(this, R::batchedObjectListTail, 0);
    zero_words(this, R::d3dViewport, D3DVIEWPORT8::SIZE / 4);
    wr::<u32>(this, R::d3dViewport_X, 0);
    wr::<u32>(this, R::d3dViewport_Y, 0);
    wr::<u32>(this, R::d3dViewport_Width, 1);
    wr::<u32>(this, R::d3dViewport_Height, 1);
    wr::<u32>(this, R::d3dViewport_MinZ, 0);
    wr::<u32>(this, R::d3dViewport_MaxZ, ONE);
    wr::<u8>(this, R::bSceneOpen, 0);

    // There are four texture format groups. Each group has 14 formats, the default format and
    // its pixel data.
    for group in 0..4 {
        zero_words(this, R::arrayArrayTextureFormats + group * 0x38, 14);
        wr::<u32>(this, R::defaultTextureFormats + group * 4, 0);
        let slot = this + R::defaultTexturePixelData + group * 4;
        ni::release_and_clear(slot);
    }
    zero_words(this, R::depthStencilFormats, 14);
    zero_words(this, R::backbufferRenderTarget, RT::SIZE / 4);
    wr::<u32>(this, R::currentRenderTarget, 0);
    wr::<u32>(this, R::vertexBufferManager, 0);
    wr::<u32>(this, R::indexBufferManager, 0);
    wr::<u32>(this, R::textureManager, 0);
    wr::<u32>(this, R::renderState, 0);
    wr::<u32>(this, R::lightManager, 0);
    wr::<u32>(this, R::configurableTexturePipeline, 0);
    zero_words(this, R::tempGeomBuffer, 16);
}

/// 0x6A96C0 `NiDX8Renderer::init`. The port sets the start values of the members, from the
/// constructor.
pub unsafe fn init(this: usize) {
    crate::shadow::touch(&init_port::STATE);
    init_from(this, engine::EMPTY_STRING, engine::VEC3_X_AXIS, engine::VEC3_Y_AXIS);
}

/// The call is a skip when it deletes the pixel data of a format group. A virtual deleting
/// destructor cannot run two times.
unsafe fn init_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
    for group in 0..4 {
        let object: usize = rd(this, R::defaultTexturePixelData + group * 4);
        if object != 0 {
            if rd::<u32>(object, NiRefObject::refCount) == 1 {
                set.skip();
            }
            set.add(object, NiRefObject::SIZE);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------

port_thiscall!(init_port, 0x6A96C0, "NiDX8Renderer::init",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| init_regions(set, this),
    rust = init);

port_thiscall!(set_depth_clear_port, 0x6A9C70, "NiDX8Renderer::SetDepthClear",
    fn(this, depth: u32) -> (),
    compare_eax = false,
    regions = |set| set.add(this, R::SIZE),
    rust = set_depth_clear);

port_thiscall!(set_bg_color_port, 0x6A9C80, "NiDX8Renderer::SetBgColor",
    fn(this, color: usize) -> u8,
    compare_eax = true,
    regions = |set| set.add(this, R::SIZE),
    rust = set_bg_color);

port_thiscall!(get_bg_color_port, 0x6A9CF0, "NiDX8Renderer::GetBgColor",
    fn(this, color: usize) -> usize,
    compare_eax = true,
    regions = |set| set.add(color, 12),
    rust = get_bg_color);

port_thiscall!(set_stencil_clear_port, 0x6A9D60, "NiDX8Renderer::SetStencilClear",
    fn(this, stencil: u32) -> (),
    compare_eax = false,
    regions = |set| set.add(this, R::SIZE),
    rust = set_stencil_clear);

port_thiscall!(get_flags_port, 0x6A9D70, "NiDX8Renderer::GetFlags",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| set.add(this, R::SIZE),
    rust = get_flags);

port_thiscall!(set_render_target_port, 0x6ABD20, "NiDX8Renderer::SetRenderTarget",
    fn(this, texture: usize) -> u8,
    compare_eax = true,
    regions = |set| set_render_target_regions(set, this),
    rust = set_render_target);

port_thiscall!(swap_buffers_port, 0x6ABE40, "NiDX8Renderer::SwapBuffers",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| swap_buffers_regions(set, this),
    rust = swap_buffers);

port_thiscall!(clear_buffer_port, 0x6ABE60, "NiDX8Renderer::ClearBuffer",
    fn(this, rect: usize, flags: u8) -> (),
    compare_eax = false,
    regions = |set| clear_buffer_regions(set, this),
    rust = clear_buffer);

port_thiscall!(get_left_right_swap_port, 0x6ABFB0, "NiDX8Renderer::GetLeftRightSwap",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| set.add(this, R::SIZE),
    rust = get_left_right_swap);

port_thiscall!(get_max_fog_value_port, 0x6AC310, "NiDX8Renderer::GetMaxFogValue",
    fn(this) -> f32,
    compare_eax = true,
    regions = |set| set.add(rd(this, R::renderState), RS::SIZE),
    rust = get_max_fog_value);

port_thiscall!(base_get_max_fog_value_port, 0x6F5670, "NiRenderer::GetMaxFogValue",
    fn(this) -> f32,
    compare_eax = true,
    regions = |set| set.add(this, 4),
    rust = base_get_max_fog_value);

port_thiscall!(base_set_left_right_swap_port, 0x6F5650, "NiRenderer::SetLeftRightSwap",
    fn(this, swap: u8) -> u8,
    compare_eax = false,
    regions = |set| set.add(this, 4),
    rust = base_set_left_right_swap);

port_thiscall!(precache_texture_port, 0x6AC370, "NiDX8Renderer::PrecacheTexture",
    fn(this, texture: usize, a3: u32, a4: u32) -> u32,
    compare_eax = false,
    regions = |set| set.add(this, R::SIZE),
    rust = precache_texture);

port_thiscall!(base_batch_render_shape_port, 0x6F53E0, "NiRenderer::BatchRenderShape",
    fn(this, geometry_data: usize, skin: usize, transform: usize, bound: usize) -> u32,
    compare_eax = false,
    regions = |set| set.add(this, 4),
    rust = base_batch_render_shape);

port_thiscall!(base_batch_render_strips_port, 0x6F53F0, "NiRenderer::BatchRenderStrips",
    fn(this, geometry_data: usize, skin: usize, transform: usize, bound: usize) -> u32,
    compare_eax = false,
    regions = |set| set.add(this, 4),
    rust = base_batch_render_strips);

port_thiscall!(set_left_right_swap_port, 0x6ABFD0, "NiDX8Renderer::SetLeftRightSwap",
    fn(this, swap: u8) -> u8,
    compare_eax = true,
    regions = |set| set.add(rd(this, R::renderState), RS::SIZE),
    rust = set_left_right_swap);

port_thiscall!(set_use_w_buffer_port, 0x6ABFF0, "NiDX8Renderer::SetUseWBuffer",
    fn(this, use_w_buffer: u8) -> u8,
    compare_eax = true,
    regions = |set| {
        set.add(this, R::SIZE);
        set.add(rd(this, R::renderState), RS::SIZE)
    },
    rust = set_use_w_buffer);

port_thiscall!(get_backbuffer_width_port, 0x6AC030, "NiDX8Renderer::GetBackbufferWidth",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| set.add(this, R::SIZE),
    rust = get_backbuffer_width);

port_thiscall!(get_backbuffer_height_port, 0x6AC040, "NiDX8Renderer::GetBackbufferHeight",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| set.add(this, R::SIZE),
    rust = get_backbuffer_height);

port_thiscall!(set_max_fog_value_port, 0x6AC320, "NiDX8Renderer::SetMaxFogValue",
    fn(this, value: f32) -> (),
    compare_eax = false,
    regions = |set| set.add(rd(this, R::renderState), RS::SIZE),
    rust = set_max_fog_value);

port_thiscall!(begin_paint_port, 0x6AC520, "NiDX8Renderer::BeginPaint",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| begin_paint_regions(set, this),
    rust = begin_paint);

port_thiscall!(end_paint_port, 0x6AC590, "NiDX8Renderer::EndPaint",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| end_paint_regions(set, this),
    rust = end_paint);

port_thiscall!(purge_material_port, 0x6AC5C0, "NiDX8Renderer::PurgeMaterial",
    fn(this, material: usize) -> (),
    compare_eax = false,
    regions = |set| purge_material_regions(set, this, material),
    rust = purge_material);

port_thiscall!(purge_effect_port, 0x6AC5D0, "NiDX8Renderer::PurgeEffect",
    fn(this, effect: usize) -> (),
    compare_eax = false,
    regions = |set| purge_effect_regions(set, this, effect),
    rust = purge_effect);

port_thiscall!(set_camera_data_port, 0x6AC620, "NiDX8Renderer::SetCameraData",
    fn(this, location: usize, direction: usize, up: usize, right: usize, frustum: usize, port: usize) -> (),
    compare_eax = false,
    regions = |set| set_camera_data_regions(set, this),
    rust = set_camera_data);

port_thiscall!(set_model_transform_port, 0x6AC9C0, "NiDX8Renderer::SetModelTransform",
    fn(this, transform: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(this, R::SIZE),
    rust = set_model_transform);

port_thiscall!(set_bone_transform_port, 0x6ACB10, "NiDX8Renderer::SetBoneTransform",
    fn(this, transform: usize, bone_index: u32) -> (),
    compare_eax = false,
    regions = |set| set.add(this, R::SIZE),
    rust = set_bone_transform);

port_thiscall!(set_skinned_model_transforms_port, 0x6ACBE0, "NiDX8Renderer::SetSkinnedModelTransforms",
    fn(this, skin: usize, partition: usize, transform: usize, bound: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(this, R::SIZE),
    rust = set_skinned_model_transforms);

port_thiscall!(begin_batch_port, 0x6AE030, "NiDX8Renderer::BeginBatch",
    fn(this, property_state: usize, effect_state: usize) -> (),
    compare_eax = false,
    regions = |set| begin_batch_regions(set, this, property_state, effect_state),
    rust = begin_batch);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &init_port::STATE,
    &set_depth_clear_port::STATE,
    &set_bg_color_port::STATE,
    &get_bg_color_port::STATE,
    &set_stencil_clear_port::STATE,
    &get_flags_port::STATE,
    &set_render_target_port::STATE,
    &swap_buffers_port::STATE,
    &clear_buffer_port::STATE,
    &get_left_right_swap_port::STATE,
    &set_left_right_swap_port::STATE,
    &get_max_fog_value_port::STATE,
    &base_get_max_fog_value_port::STATE,
    &base_set_left_right_swap_port::STATE,
    &precache_texture_port::STATE,
    &base_batch_render_shape_port::STATE,
    &base_batch_render_strips_port::STATE,
    &set_use_w_buffer_port::STATE,
    &get_backbuffer_width_port::STATE,
    &get_backbuffer_height_port::STATE,
    &set_max_fog_value_port::STATE,
    &begin_paint_port::STATE,
    &end_paint_port::STATE,
    &purge_material_port::STATE,
    &purge_effect_port::STATE,
    &set_camera_data_port::STATE,
    &set_model_transform_port::STATE,
    &set_bone_transform_port::STATE,
    &set_skinned_model_transforms_port::STATE,
    &begin_batch_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A renderer of zeros, on the heap.
    fn renderer() -> Vec<u8> {
        vec![0u8; R::SIZE]
    }

    fn floats(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    unsafe fn matrix(this: usize, offset: usize) -> [[f32; 4]; 4] {
        rd(this, offset)
    }

    // A device that records the calls that the functions here make.
    thread_local! {
        static CALLS: RefCell<Vec<(u32, Vec<u32>)>> = const { RefCell::new(Vec::new()) };
        static COOPERATIVE_LEVEL: RefCell<u32> = const { RefCell::new(0) };
    }

    fn note(slot: u32, args: &[u32]) -> u32 {
        CALLS.with(|calls| calls.borrow_mut().push((slot, args.to_vec())));
        0
    }
    unsafe extern "system" fn fake_test_cooperative_level(_this: usize) -> u32 {
        note(3, &[]);
        COOPERATIVE_LEVEL.with(|level| *level.borrow())
    }
    unsafe extern "system" fn fake_present(_this: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
        note(15, &[a, b, c, d])
    }
    unsafe extern "system" fn fake_set_render_target(_this: usize, a: u32, b: u32) -> u32 {
        note(31, &[a, b])
    }
    unsafe extern "system" fn fake_begin_scene(_this: usize) -> u32 {
        note(34, &[])
    }
    unsafe extern "system" fn fake_end_scene(_this: usize) -> u32 {
        note(35, &[])
    }
    unsafe extern "system" fn fake_clear(_this: usize, count: u32, rects: u32, flags: u32, color: u32, z: u32, stencil: u32) -> u32 {
        let rect: [u32; 4] = rd(rects as usize, 0);
        note(36, &[count, rect[0], rect[1], rect[2], rect[3], flags, color, z, stencil])
    }
    unsafe extern "system" fn fake_set_transform(_this: usize, state: u32, matrix: u32) -> u32 {
        note(37, &[state, matrix])
    }
    unsafe extern "system" fn fake_set_viewport(_this: usize, viewport: u32) -> u32 {
        let v: [u32; 6] = rd(viewport as usize, 0);
        note(40, &v)
    }

    /// The vtable and the device object. The device object is one pointer to the vtable.
    struct FakeDevice {
        _vtable: Box<[usize; 48]>,
        object: Box<usize>,
    }

    impl FakeDevice {
        fn new() -> FakeDevice {
            let mut vtable = Box::new([0usize; 48]);
            vtable[3] = fake_test_cooperative_level as *const () as usize;
            vtable[15] = fake_present as *const () as usize;
            vtable[31] = fake_set_render_target as *const () as usize;
            vtable[34] = fake_begin_scene as *const () as usize;
            vtable[35] = fake_end_scene as *const () as usize;
            vtable[36] = fake_clear as *const () as usize;
            vtable[37] = fake_set_transform as *const () as usize;
            vtable[40] = fake_set_viewport as *const () as usize;
            let object = Box::new(vtable.as_ptr() as usize);
            CALLS.with(|calls| calls.borrow_mut().clear());
            COOPERATIVE_LEVEL.with(|level| *level.borrow_mut() = 0);
            FakeDevice { _vtable: vtable, object }
        }
        fn address(&self) -> usize {
            &*self.object as *const usize as usize
        }
        fn calls(&self) -> Vec<(u32, Vec<u32>)> {
            CALLS.with(|calls| calls.borrow_mut().drain(..).collect())
        }
    }

    #[test]
    fn world_matrix_is_the_scaled_rotation_transposed() {
        let mut this = renderer();
        // Rotation rows (1,2,3), (4,5,6), (7,8,9). Translation (10,20,30). Scale 2.
        let transform = floats(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 20.0, 30.0, 2.0]);
        let (r, t) = (this.as_mut_ptr() as usize, transform.as_ptr() as usize);
        unsafe {
            write_world_matrix(r, t);
            assert_eq!(
                matrix(r, R::currentWorldMatrix),
                [[2.0, 8.0, 14.0, 0.0], [4.0, 10.0, 16.0, 0.0], [6.0, 12.0, 18.0, 0.0], [10.0, 20.0, 30.0, 1.0]]
            );
        }
    }

    #[test]
    fn bone_transform_goes_to_the_world_matrix_of_the_bone() {
        let device = FakeDevice::new();
        let mut this = renderer();
        let transform = floats(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 5.0, 6.0, 7.0, 3.0]);
        let (r, t) = (this.as_mut_ptr() as usize, transform.as_ptr() as usize);
        unsafe {
            wr(r, R::d3dDevice, device.address());
            set_bone_transform(r, t, 2);
            assert_eq!(matrix(r, R::currentWorldMatrix)[1], [0.0, 3.0, 0.0, 0.0]);
            assert_eq!(matrix(r, R::currentWorldMatrix)[3], [5.0, 6.0, 7.0, 1.0]);
        }
        assert_eq!(device.calls(), vec![(37, vec![258, (r + R::currentWorldMatrix) as u32])]);
    }

    #[test]
    fn view_matrix_has_the_camera_axes_in_columns() {
        let mut this = renderer();
        let location = floats(&[1.0, 2.0, 3.0]);
        let right = floats(&[1.0, 0.0, 0.0]);
        let up = floats(&[0.0, 0.0, 1.0]);
        let direction = floats(&[0.0, 1.0, 0.0]);
        let r = this.as_mut_ptr() as usize;
        unsafe {
            write_view_matrix(r, location.as_ptr() as usize, direction.as_ptr() as usize, up.as_ptr() as usize, right.as_ptr() as usize);
            assert_eq!(
                matrix(r, R::d3dViewMat),
                [[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 1.0, 0.0, 0.0], [-1.0, -3.0, -2.0, 1.0]]
            );
        }
    }

    #[test]
    fn projection_from_a_frustum() {
        let mut this = renderer();
        // left -1, right 3, top 2, bottom -2, near 1, far 5
        let frustum = floats(&[-1.0, 3.0, 2.0, -2.0, 1.0, 5.0]);
        let (r, f) = (this.as_mut_ptr() as usize, frustum.as_ptr() as usize);
        unsafe {
            write_projection(r, f, 0);
            // 1/width = 0.25, 1/height = 0.25, 1/depth = 0.25
            assert_eq!(
                matrix(r, R::d3dProjMat),
                [[0.5, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0], [-0.5, -0.0, 1.25, 1.0], [0.0, 0.0, -1.25, 0.0]]
            );
            // Minus zero, from the sign change of 0.25 * (2 + -2).
            assert_eq!(rd::<u32>(r, element(R::d3dProjMat, 3, 2)), 0x8000_0000);
            assert_eq!(rd::<f32>(r, R::frustumNear), 1.0);
            assert_eq!(rd::<f32>(r, R::frustumDistance), 4.0);
            assert_eq!(rd::<f32>(r, element(R::screenPolyProj, 3, 3)), 1.25);
            assert_eq!(rd::<f32>(r, element(R::screenPolyProj, 4, 3)), -1.25);

            write_projection(r, f, 1);
            assert_eq!(matrix(r, R::d3dProjMat)[0], [-0.5, 0.0, 0.0, 0.0]);
            assert_eq!(matrix(r, R::d3dProjMat)[2], [0.5, -0.0, 1.25, 1.0]);
        }
    }

    #[test]
    fn camera_viewport_in_pixels() {
        let target = [800u32, 600];
        // left 0.25, right 0.75, top 1, bottom 0.5
        let port = floats(&[0.25, 0.75, 1.0, 0.5]);
        unsafe {
            let viewport = camera_viewport(target.as_ptr() as usize, port.as_ptr() as usize);
            assert_eq!(viewport, [200, 0, 400, 300, 0, 0x3F80_0000]);
        }
    }

    #[test]
    fn clear_rect_in_pixels() {
        // left 0.25, right 0.75, top 0.75, bottom 0.25
        let rect = floats(&[0.25, 0.75, 0.75, 0.25]);
        unsafe {
            assert_eq!(clear_rect(800, 600, rect.as_ptr() as usize), [200, 150, 600, 450]);
        }
    }

    #[test]
    fn clear_buffer_clears_what_the_target_has() {
        let device = FakeDevice::new();
        let mut this = renderer();
        let mut target = [0u32; RT::SIZE / 4];
        target[0] = 640;
        target[1] = 480;
        target[RT::d3dSurfaceDepthStencil / 4] = 0x1234;
        let (r, t) = (this.as_mut_ptr() as usize, target.as_mut_ptr() as usize);
        unsafe {
            wr(r, R::d3dDevice, device.address());
            wr(r, R::currentRenderTarget, t);
            wr::<u32>(r, R::backgroundColour, 0xFF10_2030);
            wr::<f32>(r, R::depthClearValue, 1.0);
            wr::<u32>(r, R::stencilClearValue, 9);
            wr::<u32>(r, R::d3dViewport_MaxZ, ONE);

            // The call asks for colour, stencil and depth. The device has no stencil (bit 7 of the
            // flags).
            clear_buffer(r, 0, 7);
            let calls = device.calls();
            assert_eq!(calls[0], (3, vec![]));
            assert_eq!(calls[1], (40, vec![0, 0, 640, 480, 0, ONE]));
            assert_eq!(calls[2], (36, vec![1, 0, 0, 640, 480, 3, 0xFF10_2030, ONE, 9]));

            wr::<u32>(r, R::capabilityFlags, 0x80);
            clear_buffer(r, 0, 7);
            assert_eq!(device.calls()[2].1[5], 7);

            // There is no depth surface, so the port clears only the colour.
            wr::<u32>(t, RT::d3dSurfaceDepthStencil, 0);
            clear_buffer(r, 0, 7);
            assert_eq!(device.calls()[2].1[5], 1);

            // The device is lost, so no device call follows the test.
            COOPERATIVE_LEVEL.with(|level| *level.borrow_mut() = D3DERR_DEVICELOST);
            clear_buffer(r, 0, 7);
            assert_eq!(device.calls(), vec![(3, vec![])]);
        }
    }

    #[test]
    fn end_paint_and_swap_buffers() {
        let device = FakeDevice::new();
        let mut this = renderer();
        let mut target = [0u32; RT::SIZE / 4];
        let (r, t) = (this.as_mut_ptr() as usize, target.as_mut_ptr() as usize);
        unsafe {
            wr(r, R::d3dDevice, device.address());
            wr(r, R::currentRenderTarget, t);
            end_paint(r);
            assert_eq!(device.calls(), vec![]);
            wr::<u8>(r, R::bSceneOpen, 1);
            end_paint(r);
            assert_eq!(device.calls(), vec![(35, vec![])]);
            assert_eq!(rd::<u8>(r, R::bSceneOpen), 0);

            swap_buffers(r);
            assert_eq!(device.calls(), vec![(15, vec![0, 0, 0, 0])]);
            wr::<u32>(t, RT::d3dTexture, 0x1234);
            swap_buffers(r);
            assert_eq!(device.calls(), vec![]);
        }
    }

    #[test]
    fn set_render_target_to_the_back_buffer() {
        let device = FakeDevice::new();
        let mut this = renderer();
        let r = this.as_mut_ptr() as usize;
        unsafe {
            wr(r, R::d3dDevice, device.address());
            wr::<u32>(r, R::backbufferRenderTarget_d3dSurfaceBackBuffer, 0x111);
            wr::<u32>(r, R::backbufferRenderTarget_d3dSurfaceDepthStencil, 0x222);
            wr::<u8>(r, R::bSceneOpen, 1);
            assert_eq!(set_render_target(r, 0), 0);
            assert_eq!(device.calls(), vec![]);
            wr::<u8>(r, R::bSceneOpen, 0);
            assert_eq!(set_render_target(r, 0), 1);
            assert_eq!(device.calls(), vec![(31, vec![0x111, 0x222])]);
            assert_eq!(rd::<usize>(r, R::currentRenderTarget), r + R::backbufferRenderTarget);
        }
    }

    #[test]
    fn background_colour_round_trip() {
        let mut this = renderer();
        let r = this.as_mut_ptr() as usize;
        let colour = floats(&[1.0, 0.5, 0.0]);
        let mut out = [0f32; 3];
        unsafe {
            assert_eq!(set_bg_color(r, colour.as_ptr() as usize), 1);
            // 0.5 * 255 = 127.5, and the conversion truncates it to 127.
            assert_eq!(rd::<u32>(r, R::backgroundColour), 0xFFFF_7F00);
            assert_eq!(get_bg_color(r, out.as_mut_ptr() as usize), out.as_ptr() as usize);
            assert_eq!(out, [255.0 * f32::from_bits(INVERSE_255), 127.0 * f32::from_bits(INVERSE_255), 0.0]);
        }
    }

    #[test]
    fn render_state_accessors() {
        let mut this = renderer();
        let mut state = vec![0u8; RS::SIZE];
        let (r, s) = (this.as_mut_ptr() as usize, state.as_mut_ptr() as usize);
        unsafe {
            wr(r, R::renderState, s);
            assert_eq!(get_left_right_swap(r), 0);
            assert_eq!(set_left_right_swap(r, 0x40), 1);
            assert_eq!(rd::<u32>(s, RS::bLeftRightSwap), 1);
            assert_eq!(get_left_right_swap(r), 1);

            set_max_fog_value(r, 4.0);
            assert_eq!(rd::<f32>(s, RS::maxFogValue), 4.0);
            assert_eq!(rd::<f32>(s, RS::inverseMaxFogValue), -0.75);
            for value in [0.0, -0.0, -3.0] {
                set_max_fog_value(r, value);
                assert_eq!(rd::<u32>(s, RS::maxFogValue), MIN_MAX_FOG);
            }
            set_max_fog_value(r, f32::NAN);
            assert!(rd::<f32>(s, RS::maxFogValue).is_nan());

            wr::<u32>(r, R::capabilityFlags, 0x1234_5678);
            assert_eq!(get_flags(r), 0x1234_5678);
            set_depth_clear(r, 0.5f32.to_bits());
            set_stencil_clear(r, 3);
            assert_eq!(rd::<f32>(r, R::depthClearValue), 0.5);
            assert_eq!(rd::<u32>(r, R::stencilClearValue), 3);

            let target = [320u32, 200];
            wr(r, R::currentRenderTarget, target.as_ptr() as usize);
            assert_eq!((get_backbuffer_width(r), get_backbuffer_height(r)), (320, 200));
        }
    }

    #[test]
    fn begin_batch_moves_the_references() {
        let mut this = renderer();
        let r = this.as_mut_ptr() as usize;
        // vtable, reference count
        let (mut a, mut b, mut e) = ([0u32, 5], [0u32, 1], [0u32, 2]);
        let (pa, pb, pe) = (a.as_mut_ptr() as usize, b.as_mut_ptr() as usize, e.as_mut_ptr() as usize);
        unsafe {
            begin_batch(r, pa, pe);
            assert_eq!((rd::<u32>(pa, 4), rd::<u32>(pe, 4)), (6, 3));
            begin_batch(r, pa, pe);
            assert_eq!((rd::<u32>(pa, 4), rd::<u32>(pe, 4)), (6, 3));
            begin_batch(r, pb, 0);
            assert_eq!((rd::<u32>(pa, 4), rd::<u32>(pb, 4), rd::<u32>(pe, 4)), (5, 2, 2));
            assert_eq!(rd::<usize>(r, R::propertyStatePtr), pb);
            assert_eq!(rd::<usize>(r, R::effectStatePtr), 0);

            // The regions. The shadow test cannot compare a release that deletes.
            let mut set = Regions::new();
            begin_batch_regions(&mut set, r, pa, 0);
            assert_eq!(set.as_slice(), &[(r, R::SIZE), (pb, 8), (pa, 8)]);
        }
    }

    #[test]
    fn init_sets_the_start_values() {
        let mut this = vec![0xEEu8; R::SIZE];
        let r = this.as_mut_ptr() as usize;
        let empty = [0u8; 4];
        let x_axis = floats(&[1.0, 0.0, 0.0]);
        let y_axis = floats(&[0.0, 1.0, 0.0]);
        unsafe {
            zero_words(r, R::defaultTexturePixelData, 4);
            init_from(r, empty.as_ptr() as usize, x_axis.as_ptr() as usize, y_axis.as_ptr() as usize);
            let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0f32]];
            assert_eq!(matrix(r, R::matrixD3DIdentity), identity);
            assert_eq!(matrix(r, R::screenPolyView), identity);
            assert_eq!(
                matrix(r, R::screenPolyProj),
                [[2.0, 0.0, 0.0, 0.0], [0.0, 2.0, 0.0, 0.0], [-1.0, -1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0]]
            );
            assert_eq!(rd::<[f32; 3]>(r, R::cameraRight), [1.0, 0.0, 0.0]);
            assert_eq!(rd::<[f32; 3]>(r, R::modelCameraUp), [0.0, 1.0, 0.0]);
            assert_eq!(rd::<[u32; 6]>(r, R::d3dViewport), [0, 0, 1, 1, 0, ONE]);
            assert_eq!(rd::<u32>(r, R::backgroundColour), 0xFF80_8080);
            assert_eq!(rd::<f32>(r, R::frustumNear), 0.1);
            // The port writes one byte of each description and no more.
            assert_eq!(rd::<[u8; 2]>(r, R::driverDesc), [0, 0xEE]);
            assert_eq!(rd::<[u8; 2]>(r, DESCRIPTION_2), [0, 0xEE]);
            // The port does not write the adapter, the hash maps and the world matrix.
            assert_eq!(rd::<u32>(r, R::maybe_currentAdapter), 0xEEEE_EEEE);
            assert_eq!(rd::<u32>(r, R::hashGeomBuffers), 0xEEEE_EEEE);
            assert_eq!(rd::<u32>(r, R::currentWorldMatrix), 0xEEEE_EEEE);
            assert_eq!(rd::<u32>(r, R::tempGeomBuffer + 0x3C), 0);
        }
    }

    #[test]
    fn each_light_list_item_is_one_block() {
        // Three list items in one block. Their links are not in the order of their addresses.
        let mut items = [0u32; 9];
        let base = items.as_mut_ptr() as usize;
        let mut manager = [0u8; LM::SIZE];
        let m = manager.as_mut_ptr() as usize;
        unsafe {
            wr(base + 24, LIST_ITEM_NEXT, base);
            wr(base, LIST_ITEM_NEXT, base + 12);
            wr::<u32>(m, LM::deviceLights_count, 3);
            wr(m, LM::deviceLights_head, base + 24);
            let mut set = Regions::new();
            add_light_list_items(&mut set, m);
            assert_eq!(set.as_slice(), &[(base + 24, 12), (base, 12), (base + 12, 12)]);
            // The count of the list is the limit.
            wr::<u32>(m, LM::deviceLights_count, 2);
            let mut set = Regions::new();
            add_light_list_items(&mut set, m);
            assert_eq!(set.as_slice(), &[(base + 24, 12), (base, 12)]);
        }
    }
}
