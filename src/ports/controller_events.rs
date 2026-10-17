//! The time-varying writers of a shape's material or geometry data that no other event sees
//! (entry 13; the owner, 2026-10-17: events for all the time-varying controllers). The
//! controller inventory (`controller-inventory-2026-10-17.md`) sorts the controllers by what
//! their `Update` writes:
//! - a transform (keyframe, keyframe manager, look-at, path, roll, MWSE's copy transform): the
//!   update pass then calls `NiGeometry::UpdateWorldData` (`state_update`);
//! - the app-cull bit (visibility): `NiAVObject::SetAppCulled` (`state_update`);
//! - a light's colours: the table walk notes the lights of every effect state once a frame;
//! - the vertices at the display (geometry morpher): only from the shape's own chain, which
//!   makes the shape live;
//! - a property or the geometry data: the four ports here. A controller of these kinds can sit
//!   in the chain of another object than its target (Inscribed Enchantments keeps the flip
//!   controllers of a worn item on its clock node `IE_anim`), where the live classification of
//!   a shape does not look; the water code calls a flip controller's `Update` directly.
//! The vanilla enchant effect swaps its texture each frame with no controller
//! (`WorldController::updateEnchantedItemEffect`): the fifth port.
//!
//! Each port compares a fingerprint of the target before and after the engine's function and
//! marks the target only when it changed (a flip controller of 12 frames a second changes its
//! texture in one update of five at 60 frames a second). The hooks read the target the engine
//! just wrote and nothing below it (`instance_stream::note_source`).

use crate::gen::layouts::{NiGeometryData as GD, NiTexturingProperty as NTP, NiTexturingProperty_Map as MAP, NiTimeController as TC};
use crate::mem::rd;

/// The calls and the marks of each port: flip, alpha, material colour, UV, enchant effect.
static mut COUNTS: [[u64; 2]; 5] = [[0; 2]; 5];

unsafe fn count(port: usize, marked: bool) {
    let counts = &mut *core::ptr::addr_of_mut!(COUNTS);
    counts[port][0] += 1;
    if marked {
        counts[port][1] += 1;
    }
}

/// The ports' line of the report.
pub unsafe fn report() -> String {
    let c = *core::ptr::addr_of!(COUNTS);
    format!(
        "controller events (entry 13): flip {} calls, {} marks; alpha {}, {}; material colour {}, {}; UV {}, {}; enchant effect {}, {}",
        c[0][0], c[0][1], c[1][0], c[1][1], c[2][0], c[2][1], c[3][0], c[3][1], c[4][0], c[4][1]
    )
}

/// `NiGeometry::spModelData`.
const GEOMETRY_MODEL_DATA: usize = 0x98;
/// The values of `NiMaterialProperty`: ambient, diffuse, specular and emissive (three floats
/// each), shininess, alpha, and the revision (+0x1C to +0x58).
const MATERIAL_VALUES: core::ops::Range<usize> = 0x1C..0x58;
/// `WorldController::enchantedItemTextureEffect` and `NiTextureEffect::sourceTexture`.
const WORLD_CONTROLLER_ENCHANT_EFFECT: usize = 0x36C;
const TEXTURE_EFFECT_SOURCE: usize = 0x108;

/// A fingerprint of a texturing property's maps: the array and each map's texture.
unsafe fn texturing_print(property: usize) -> u64 {
    if property == 0 {
        return 0;
    }
    let maps: usize = rd(property, NTP::maps_storage);
    let end: u32 = rd(property, NTP::maps_endIndex);
    let mut print = (maps as u64) << 32 | end as u64;
    if maps != 0 {
        for i in 0..end.min(16) as usize {
            let map: usize = rd(maps, i * 4);
            let texture: usize = if map != 0 { rd(map, MAP::texture) } else { 0 };
            print = (print ^ texture as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    print
}

/// A fingerprint of a material property's colours, shininess and alpha.
unsafe fn material_print(property: usize) -> u64 {
    if property == 0 {
        return 0;
    }
    MATERIAL_VALUES.step_by(4).fold(0xcbf2_9ce4_8422_2325u64, |h, at| (h ^ rd::<u32>(property, at) as u64).wrapping_mul(0x100_0000_01b3))
}

/// The revision of a shape's geometry data (a UV controller increments it).
unsafe fn data_print(shape: usize) -> u64 {
    if shape == 0 {
        return 0;
    }
    let data: usize = rd(shape, GEOMETRY_MODEL_DATA);
    if data == 0 {
        0
    } else {
        (data as u64) << 16 | rd::<u16>(data, GD::revisionID) as u64
    }
}

port_thiscall!(flip_update_port, 0x715A50, "NiFlipController::Update",
    fn(this, time: u32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = flip_update);

/// A flip controller swaps a map's texture of its target texturing property.
pub unsafe fn flip_update(this: usize, time: u32) {
    crate::shadow::touch(&flip_update_port::STATE);
    let target: usize = rd(this, TC::target);
    let before = texturing_print(target);
    flip_update_port::engine(this, time);
    let marked = target != 0 && texturing_print(target) != before;
    count(0, marked);
    if marked {
        crate::instance_stream::note_source(target);
    }
}

port_thiscall!(alpha_update_port, 0x7130A0, "NiAlphaController::Update",
    fn(this, time: u32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = alpha_update);

/// An alpha controller writes the alpha of its target material property.
pub unsafe fn alpha_update(this: usize, time: u32) {
    crate::shadow::touch(&alpha_update_port::STATE);
    let target: usize = rd(this, TC::target);
    let before = material_print(target);
    alpha_update_port::engine(this, time);
    let marked = target != 0 && material_print(target) != before;
    count(1, marked);
    if marked {
        crate::instance_stream::note_source(target);
    }
}

port_thiscall!(material_color_update_port, 0x71A6E0, "NiMaterialColorController::Update",
    fn(this, time: u32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = material_color_update);

/// A material colour controller writes one colour of its target material property.
pub unsafe fn material_color_update(this: usize, time: u32) {
    crate::shadow::touch(&material_color_update_port::STATE);
    let target: usize = rd(this, TC::target);
    let before = material_print(target);
    material_color_update_port::engine(this, time);
    let marked = target != 0 && material_print(target) != before;
    count(2, marked);
    if marked {
        crate::instance_stream::note_source(target);
    }
}

port_thiscall!(uv_update_port, 0x7220E0, "NiUVController::Update",
    fn(this, time: u32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = uv_update);

/// A UV controller rewrites the texture coordinates of its target shape's data.
pub unsafe fn uv_update(this: usize, time: u32) {
    crate::shadow::touch(&uv_update_port::STATE);
    let target: usize = rd(this, TC::target);
    let before = data_print(target);
    uv_update_port::engine(this, time);
    let marked = target != 0 && data_print(target) != before;
    count(3, marked);
    if marked {
        crate::instance_stream::note_source(target);
    }
}

port_thiscall!(enchant_effect_update_port, 0x410960, "WorldController::updateEnchantedItemEffect",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = enchant_effect_update);

/// The vanilla enchant effect: one texture effect shared by every enchanted item, its texture
/// swapped to the next of 32 caustic frames as the time goes.
pub unsafe fn enchant_effect_update(this: usize) {
    crate::shadow::touch(&enchant_effect_update_port::STATE);
    let effect: usize = rd(this, WORLD_CONTROLLER_ENCHANT_EFFECT);
    let before: usize = if effect != 0 { rd(effect, TEXTURE_EFFECT_SOURCE) } else { 0 };
    enchant_effect_update_port::engine(this);
    let effect: usize = rd(this, WORLD_CONTROLLER_ENCHANT_EFFECT);
    let marked = effect != 0 && rd::<usize>(effect, TEXTURE_EFFECT_SOURCE) != before;
    count(4, marked);
    if marked {
        crate::instance_stream::note_source(effect);
    }
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &flip_update_port::STATE,
    &alpha_update_port::STATE,
    &material_color_update_port::STATE,
    &uv_update_port::STATE,
    &enchant_effect_update_port::STATE,
];
