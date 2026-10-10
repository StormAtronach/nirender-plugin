//! `NiDX8RenderState`: the translation of a `NiPropertyState` into D3D render states, behind
//! a cache that drops a state the device already has.

use crate::engine;
use crate::gen::layouts::{NiAlphaProperty, NiDX8RenderState as RS, NiFogProperty, NiMaterialProperty, NiPropertyState, NiStencilProperty, NiZBufferProperty};
use crate::mem::{rd, wr};
use crate::ports::material;
use crate::x87;

// D3DRENDERSTATETYPE values the functions use.
const D3DRS_ZENABLE: u32 = 7;
const D3DRS_FILLMODE: u32 = 8;
const D3DRS_SHADEMODE: u32 = 9;
const D3DRS_ZWRITEENABLE: u32 = 14;
const D3DRS_ALPHATESTENABLE: u32 = 15;
const D3DRS_SRCBLEND: u32 = 19;
const D3DRS_DESTBLEND: u32 = 20;
const D3DRS_CULLMODE: u32 = 22;
const D3DRS_ZFUNC: u32 = 23;
const D3DRS_ALPHAREF: u32 = 24;
const D3DRS_ALPHAFUNC: u32 = 25;
const D3DRS_DITHERENABLE: u32 = 26;
const D3DRS_ALPHABLENDENABLE: u32 = 27;
const D3DRS_FOGENABLE: u32 = 28;
const D3DRS_SPECULARENABLE: u32 = 29;
const D3DRS_FOGCOLOR: u32 = 34;
const D3DRS_FOGTABLEMODE: u32 = 35;
const D3DRS_FOGSTART: u32 = 36;
const D3DRS_FOGEND: u32 = 37;
const D3DRS_RANGEFOGENABLE: u32 = 48;
const D3DRS_STENCILENABLE: u32 = 52;
const D3DRS_STENCILFAIL: u32 = 53;
const D3DRS_STENCILZFAIL: u32 = 54;
const D3DRS_STENCILPASS: u32 = 55;
const D3DRS_STENCILFUNC: u32 = 56;
const D3DRS_STENCILREF: u32 = 57;
const D3DRS_STENCILMASK: u32 = 58;
const D3DRS_FOGVERTEXMODE: u32 = 140;
const D3DRS_MULTISAMPLEANTIALIAS: u32 = 161;

const D3DCMP_ALWAYS: u32 = 8;
const D3DFOG_LINEAR: u32 = 3;

/// `NiProperty::flags`, the same place in every property class.
const PROPERTY_FLAGS: usize = NiAlphaProperty::flags;

#[inline(always)]
unsafe fn current(this: usize, state: u32) -> u32 {
    rd(this, RS::renderStateSettings + state as usize * 8)
}

#[inline(always)]
unsafe fn set_current(this: usize, state: u32, value: u32) {
    wr(this, RS::renderStateSettings + state as usize * 8, value)
}

#[inline(always)]
unsafe fn to_device(this: usize, state: u32, value: u32) {
    engine::device_set_render_state(rd(this, RS::device), state, value);
}

/// The pattern the game code has inline in most places: write the cache and the device when
/// the value is new. The previous value is not kept.
#[inline(always)]
unsafe fn set_if_changed(this: usize, state: u32, value: u32) {
    if current(this, state) != value {
        set_current(this, state, value);
        to_device(this, state, value);
    }
}

/// 0x65EEB0 `NiRenderStateSetting::SetValue`
pub unsafe fn setting_set_value(setting: usize, value: u32, save: u8) {
    if save != 0 {
        wr::<u32>(setting, 4, rd::<u32>(setting, 0));
    }
    wr(setting, 0, value);
}

/// 0x65ED60 `NiRenderState::SetRenderState`
pub unsafe fn set_render_state(this: usize, state: u32, value: u32, save: u8) {
    let now = current(this, state);
    if now != value {
        if save != 0 {
            wr(this, RS::renderStateSettings + state as usize * 8 + 4, now);
        }
        set_current(this, state, value);
        to_device(this, state, value);
    }
}

/// 0x65EDE0 `NiDX8RenderState::setTextureStageStateWithSave`
pub unsafe fn set_texture_stage_state_with_save(this: usize, stage: u32, state: u32, value: u32, save: u8) {
    let setting = this + RS::maybe_textureStageStateSettings + (state.wrapping_add(stage.wrapping_mul(32))) as usize * 8;
    let now: u32 = rd(setting, 0);
    if now != value {
        if save != 0 {
            wr(setting, 4, now);
        }
        wr(setting, 0, value);
        engine::device_set_texture_stage_state(rd(this, RS::device), stage, state, value);
    }
}

/// 0x6B8410 `NiDX8RenderState::setMultisampling`
pub unsafe fn set_multisampling(this: usize, enable: u8) {
    set_if_changed(this, D3DRS_MULTISAMPLEANTIALIAS, (enable != 0) as u32);
}

/// 0x6B8440 `NiDX8RenderState::setZBufferEnable`
pub unsafe fn set_z_buffer_enable(this: usize, use_w_buffer: u8) {
    let mut value = 1; // D3DZB_TRUE
    if use_w_buffer != 0 && (rd::<u32>(this, RS::flags) >> 4) & 1 != 0 {
        value = 2; // D3DZB_USEW
    }
    set_if_changed(this, D3DRS_ZENABLE, value);
}

/// 0x6B85F0 `NiDX8RenderState::nextFrame`. The game code leaves 0 in EAX.
pub unsafe fn next_frame(this: usize) -> u32 {
    wr::<u32>(this, RS::currentFogProperty, 0);
    wr::<u32>(this, RS::currentAlphaProperty, 0);
    0
}

/// 0x6B8610 `NiDX8RenderState::resetTextureStateStates`
pub unsafe fn reset_texture_stage_states(this: usize) {
    for stage in 0u32..8 {
        for state in 0u32..32 {
            let setting = this + RS::maybe_textureStageStateSettings + (stage * 32 + state) as usize * 8;
            let value = if state == 11 {
                // D3DTSS_TEXCOORDINDEX: each stage reads its own coordinate set.
                stage
            } else if (state == 1 || state == 4) && stage != 0 {
                // D3DTSS_COLOROP and ALPHAOP: D3DTOP_DISABLE past the first stage.
                1
            } else {
                rd::<u32>(engine::TEXTURE_STAGE_STATE_INITIAL, state as usize * 4)
            };
            wr(setting, 4, value);
            wr(setting, 0, value);
        }
    }
}

/// 0x6B8480 `NiDX8RenderState::updateAlphaStates`
pub unsafe fn update_alpha_states(this: usize, alpha_property: usize) {
    if alpha_property == rd::<usize>(this, RS::currentAlphaProperty) && rd::<u8>(this, RS::changed) == 0 {
        return;
    }
    wr(this, RS::currentAlphaProperty, alpha_property);
    let flags = || rd::<u16>(alpha_property, PROPERTY_FLAGS) as u32;

    if flags() & 1 != 0 {
        set_if_changed(this, D3DRS_ALPHABLENDENABLE, 1);
        let source: u32 = rd(this, RS::alphaMapping + ((flags() >> 1) & 0xF) as usize * 4);
        set_if_changed(this, D3DRS_SRCBLEND, source);
        let dest: u32 = rd(this, RS::alphaMapping + ((flags() >> 5) & 0xF) as usize * 4);
        set_if_changed(this, D3DRS_DESTBLEND, dest);
    } else {
        set_if_changed(this, D3DRS_ALPHABLENDENABLE, 0);
    }

    if (flags() >> 9) & 1 != 0 {
        set_if_changed(this, D3DRS_ALPHATESTENABLE, 1);
        let func: u32 = rd(this, RS::alphaTestMapping + ((flags() >> 10) & 7) as usize * 4);
        set_if_changed(this, D3DRS_ALPHAFUNC, func);
        let reference = rd::<u8>(alpha_property, NiAlphaProperty::alphaTestRef) as u32;
        set_if_changed(this, D3DRS_ALPHAREF, reference);
    } else {
        set_render_state(this, D3DRS_ALPHATESTENABLE, 0, 0);
    }
}

/// 0x6B8A00 `NiDX8RenderState::updateFogStates`
pub unsafe fn update_fog_states(this: usize, fog_property: usize) {
    if fog_property == rd::<usize>(this, RS::currentFogProperty) && rd::<u8>(this, RS::changed) == 0 {
        return;
    }
    wr(this, RS::currentFogProperty, fog_property);
    if (rd::<u8>(fog_property, PROPERTY_FLAGS) >> 1) & 1 == 0 {
        set_render_state(this, D3DRS_FOGENABLE, 0, 0);
        return;
    }
    set_if_changed(this, D3DRS_FOGENABLE, 1);

    let (start, end) = x87::fog_range(
        rd(fog_property, NiFogProperty::density),
        rd(this, RS::frustumDistance),
        rd(this, RS::frustumFar),
        rd(this, RS::inverseMaxFogValue),
    );
    set_if_changed(this, D3DRS_FOGSTART, start);
    set_if_changed(this, D3DRS_FOGEND, end);

    let w_fog = rd::<u8>(this, RS::flags) & 1 != 0;
    let (table_mode, vertex_mode, range) = match (rd::<u8>(fog_property, PROPERTY_FLAGS) >> 3) & 3 {
        0 => (D3DFOG_LINEAR, 0, 0),
        1 => (0, D3DFOG_LINEAR, 1),
        _ => {
            // The game code switches fog off again here, after the states above.
            set_render_state(this, D3DRS_FOGENABLE, 0, 0);
            return;
        }
    };
    if w_fog {
        set_if_changed(this, D3DRS_FOGTABLEMODE, table_mode);
        set_if_changed(this, D3DRS_FOGVERTEXMODE, vertex_mode);
    }
    set_if_changed(this, D3DRS_RANGEFOGENABLE, range);
}

/// 0x6B8C10 `NiDX8RenderState::setStencilStates`. Also sets the cull mode, which comes from
/// the draw mode of the stencil property.
pub unsafe fn set_stencil_states(this: usize, stencil: usize) {
    let enabled = rd::<u8>(stencil, NiStencilProperty::enabled);
    if enabled != 0 {
        let action = |index: u32| rd::<u32>(this, RS::stencilActionMapping + index as usize * 4);
        let func: u32 = rd(this, RS::stencilFuncMapping + rd::<u32>(stencil, NiStencilProperty::testFunc) as usize * 4);
        set_if_changed(this, D3DRS_STENCILFUNC, func);
        set_if_changed(this, D3DRS_STENCILREF, rd(stencil, NiStencilProperty::reference));
        set_if_changed(this, D3DRS_STENCILMASK, rd(stencil, NiStencilProperty::mask));
        set_if_changed(this, D3DRS_STENCILFAIL, action(rd(stencil, NiStencilProperty::failAction)));
        set_if_changed(this, D3DRS_STENCILZFAIL, action(rd(stencil, NiStencilProperty::zFailAction)));
        set_if_changed(this, D3DRS_STENCILPASS, action(rd(stencil, NiStencilProperty::passAction)));
    }
    set_if_changed(this, D3DRS_STENCILENABLE, enabled as u32);

    let draw_mode: u32 = rd(stencil, NiStencilProperty::drawMode);
    let swap: u32 = rd(this, RS::bLeftRightSwap);
    let index = swap.wrapping_add(draw_mode.wrapping_mul(2));
    let cull: u32 = rd(this, RS::cullModeMapping + index as usize * 4);
    set_if_changed(this, D3DRS_CULLMODE, cull);
}

/// 0x6B82A0 `NiDX8RenderState::updateD3DState`: every render state of one property state.
pub unsafe fn update_d3d_state(this: usize, property_state: usize) {
    let flags_of = |property: usize| rd::<u8>(property, PROPERTY_FLAGS) as u32;

    let wireframe = flags_of(rd(property_state, NiPropertyState::propWireframe)) & 1;
    set_render_state(this, D3DRS_FILLMODE, (wireframe ^ 1) | 2, 0);

    let smooth = flags_of(engine::property_state_global_shade_property(property_state)) & 1;
    set_render_state(this, D3DRS_SHADEMODE, smooth + 1, 0);

    update_alpha_states(this, rd(property_state, NiPropertyState::propAlpha));

    let z_buffer: usize = rd(property_state, NiPropertyState::propZBuffer);
    set_if_changed(this, D3DRS_ZWRITEENABLE, (flags_of(z_buffer) >> 1) & 1);
    let mut z_func: u32 = rd(this, RS::zFuncMapping + rd::<u32>(z_buffer, NiZBufferProperty::testFunction) as usize * 4);
    if flags_of(z_buffer) & 1 == 0 {
        z_func = D3DCMP_ALWAYS;
    }
    set_render_state(this, D3DRS_ZFUNC, z_func, 0);

    update_fog_states(this, rd(property_state, NiPropertyState::propFog));
    let fog: usize = rd(property_state, NiPropertyState::propFog);
    set_if_changed(this, D3DRS_FOGCOLOR, rd(fog, NiFogProperty::color));

    let dither = flags_of(engine::property_state_global_dither_property(property_state)) & 1;
    set_if_changed(this, D3DRS_DITHERENABLE, dither);
    let specular = flags_of(engine::property_state_global_specular_property(property_state)) & 1;
    set_if_changed(this, D3DRS_SPECULARENABLE, specular);

    set_stencil_states(this, rd(property_state, NiPropertyState::propStencil));

    let material_property = engine::property_state_update_current_material_revision(property_state);
    let manager = this + RS::materialManager;
    material::set_current_material(manager, material_property);
    engine::material_manager_update_to_device(manager, rd(this, RS::device));
    wr::<u8>(this, RS::changed, 0);
}

/// The memory `updateD3DState` may write besides the render state: the material, which gets
/// its revision moved on, and the entry the material manager keeps for it.
unsafe fn update_d3d_state_regions(set: &mut crate::shadow::Regions, this: usize, property_state: usize) {
    set.add(this, RS::SIZE);
    let material_property: usize = rd(property_state, NiPropertyState::propMaterial);
    set.add(material_property, NiMaterialProperty::SIZE);
    let entry: usize = rd(material_property, NiMaterialProperty::rendererData);
    if entry == 0 {
        // The entry is made in this call, at a different address in each of the two runs.
        set.skip();
    }
    set.add(entry, material::ENTRY_SIZE);
}

port_thiscall!(set_render_state_port, 0x65ED60, "NiRenderState::SetRenderState",
    fn(this, state: u32, value: u32, save: u8) -> (),
    compare_eax = false,
    regions = |set| set.add(this, RS::SIZE),
    rust = set_render_state);

port_thiscall!(set_texture_stage_state_with_save_port, 0x65EDE0, "NiDX8RenderState::setTextureStageStateWithSave",
    fn(this, stage: u32, state: u32, value: u32, save: u8) -> (),
    compare_eax = false,
    regions = |set| set.add(this, RS::SIZE),
    rust = set_texture_stage_state_with_save);

port_thiscall!(setting_set_value_port, 0x65EEB0, "NiRenderStateSetting::SetValue",
    fn(this, value: u32, save: u8) -> (),
    compare_eax = false,
    regions = |set| set.add(this, 8),
    rust = setting_set_value);

port_thiscall!(update_d3d_state_port, 0x6B82A0, "NiDX8RenderState::updateD3DState",
    fn(this, property_state: usize) -> (),
    compare_eax = false,
    regions = |set| update_d3d_state_regions(set, this, property_state),
    rust = update_d3d_state);

port_thiscall!(set_multisampling_port, 0x6B8410, "NiDX8RenderState::setMultisampling",
    fn(this, enable: u8) -> (),
    compare_eax = false,
    regions = |set| set.add(this, RS::SIZE),
    rust = set_multisampling);

port_thiscall!(set_z_buffer_enable_port, 0x6B8440, "NiDX8RenderState::setZBufferEnable",
    fn(this, use_w_buffer: u8) -> (),
    compare_eax = false,
    regions = |set| set.add(this, RS::SIZE),
    rust = set_z_buffer_enable);

port_thiscall!(update_alpha_states_port, 0x6B8480, "NiDX8RenderState::updateAlphaStates",
    fn(this, alpha_property: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(this, RS::SIZE),
    rust = update_alpha_states);

port_thiscall!(next_frame_port, 0x6B85F0, "NiDX8RenderState::nextFrame",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| set.add(this, RS::SIZE),
    rust = next_frame);

port_thiscall!(reset_texture_stage_states_port, 0x6B8610, "NiDX8RenderState::resetTextureStateStates",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.add(this, RS::SIZE),
    rust = reset_texture_stage_states);

port_thiscall!(update_fog_states_port, 0x6B8A00, "NiDX8RenderState::updateFogStates",
    fn(this, fog_property: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(this, RS::SIZE),
    rust = update_fog_states);

port_thiscall!(set_stencil_states_port, 0x6B8C10, "NiDX8RenderState::setStencilStates",
    fn(this, stencil: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(this, RS::SIZE),
    rust = set_stencil_states);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &set_render_state_port::STATE,
    &set_texture_stage_state_with_save_port::STATE,
    &setting_set_value_port::STATE,
    &update_d3d_state_port::STATE,
    &set_multisampling_port::STATE,
    &set_z_buffer_enable_port::STATE,
    &update_alpha_states_port::STATE,
    &next_frame_port::STATE,
    &reset_texture_stage_states_port::STATE,
    &update_fog_states_port::STATE,
    &set_stencil_states_port::STATE,
];
