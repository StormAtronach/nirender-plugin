//! The material of a draw, from the `MATERIAL` record of the plugin (step R2 of the retained
//! pipeline): the facts of the properties of the shape, and the derivation of the state of
//! the draw from them, as the fixed-function pipeline of the engine makes it. The helper
//! draws a shape with the derived state (`Pass` in lib.rs).
//!
//! The rules come from the ports of `NiDX8TexturePass::setCTPipelineState`,
//! `NiDX8ConfigurableTexturePipeline::initAlphaStateTemplates`, `NiDX8RenderState` and
//! `NiDX8LightManager::setState` in the plugin.

/// The words of a record. See `material_stream` of the plugin for the layout.
pub const WORDS: usize = 91;
/// The words of a record that has the static values of its texture effects after the 91
/// (13 for each: the environment map at 91, the fog map at 104). A shorter record has none.
pub const WORDS_WITH_EFFECTS: usize = 117;

pub const BLEND: u32 = 1;
pub const ALPHA_TEST: u32 = 2;
pub const DEPTH_TEST: u32 = 4;
pub const DEPTH_WRITE: u32 = 8;
pub const WIREFRAME: u32 = 16;
pub const SMOOTH: u32 = 32;
pub const SPECULAR: u32 = 128;
pub const FOG: u32 = 256;
pub const STENCIL: u32 = 512;
pub const ENVIRONMENT_MAP: u32 = 1024;
pub const FOG_MAP: u32 = 2048;
pub const PROJECTED_LIGHTS: u32 = 4096;
pub const PROJECTED_SHADOWS: u32 = 8192;
/// The shape has a fog property of its own (not the cell's): words 78 and 79 are its colour
/// and density, 54 and 55 its start and end.
/// Engine-rules R11 (the skill wgpu-renderer-dev).
pub const OWN_FOG: u32 = 16384;
/// The alpha property has the no-sorter bit: the blend draws at once in the order of the
/// scene, out of the engine's sort.
pub const NO_SORTER: u32 = 32768;

/// The maps of a texturing property, in the order of its array.
pub const BASE: usize = 0;
pub const DARK: usize = 1;
pub const DETAIL: usize = 2;
pub const GLOSS: usize = 3;
pub const GLOW: usize = 4;
pub const BUMP: usize = 5;
pub const DECAL0: usize = 6;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Map {
    pub texture: u32,
    pub clamp: u32,
    pub filter: u32,
    pub set: u32,
    pub present: bool,
}

/// A record, parsed.
#[derive(Clone, Debug)]
pub struct Material {
    pub flags: u32,
    pub source_blend: u32,
    pub destination_blend: u32,
    pub alpha_function: u32,
    pub alpha_reference: u32,
    pub apply_mode: u32,
    pub decals: u32,
    pub maps: [Map; 8],
    pub bump: [f32; 6],
    /// Ambient, diffuse, specular, emissive (3 each), shininess, alpha.
    pub material: [f32; 14],
    pub source_mode: u32,
    pub lighting_mode: u32,
    pub depth_function: u32,
    pub stencil: [u32; 7],
    pub fog_mode: u32,
    /// The colour (a D3DCOLOR), the start and the end of the shape's own fog.
    pub own_fog: Option<(u32, f32, f32)>,
    pub environment: Map,
    pub environment_coordinates: u32,
    pub fog_map: Map,
    pub fog_map_coordinates: u32,
    pub projected_lights: u32,
    pub projected_shadows: u32,
    /// The values of the effects that do not change with the camera, when the record has
    /// them: the table makes the stage of a retained draw from them.
    pub environment_effect: Option<Effect>,
    pub fog_map_effect: Option<Effect>,
}

/// The values of a texture effect that do not change with the camera: the 3x3 matrix (row
/// by row) and the translation of its world projection, and the transform flags of its
/// stage. The stage for any camera is the matrix applied to a vector of the world (the
/// position, the reflection or the normal, by the coordinate mode) plus the translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effect {
    pub matrix: [f32; 9],
    pub translation: [f32; 3],
    pub flags: u32,
}

impl Material {
    pub fn parse(data: &[u8]) -> Option<Material> {
        if data.len() < WORDS * 4 {
            return None;
        }
        let w = |i: usize| u32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]);
        let f = |i: usize| f32::from_bits(w(i));
        let map = |at: usize| Map { texture: w(at), clamp: w(at + 1), filter: w(at + 2), set: w(at + 3), present: w(at + 4) != 0 };
        let effect = |at: usize, present: bool| -> Option<Effect> {
            (present && data.len() >= WORDS_WITH_EFFECTS * 4).then(|| Effect { matrix: core::array::from_fn(|k| f(at + k)), translation: core::array::from_fn(|k| f(at + 9 + k)), flags: w(at + 12) })
        };
        Some(Material {
            flags: w(1),
            source_blend: w(2),
            destination_blend: w(3),
            alpha_function: w(4),
            alpha_reference: w(5),
            apply_mode: w(6),
            decals: w(7),
            maps: core::array::from_fn(|i| map(8 + i * 5)),
            bump: core::array::from_fn(|k| f(48 + k)),
            material: core::array::from_fn(|k| f(54 + k)),
            source_mode: w(68),
            lighting_mode: w(69),
            depth_function: w(70),
            stencil: core::array::from_fn(|k| w(71 + k)),
            fog_mode: w(80),
            own_fog: (w(1) & OWN_FOG != 0).then(|| (w(78), f(54), f(55))),
            environment: Map { texture: w(81), clamp: w(82), filter: w(83), set: 1, present: w(1) & ENVIRONMENT_MAP != 0 },
            environment_coordinates: w(84),
            fog_map: Map { texture: w(85), clamp: w(86), filter: w(87), set: 1, present: w(1) & FOG_MAP != 0 },
            fog_map_coordinates: w(88),
            projected_lights: w(89),
            projected_shadows: w(90),
            environment_effect: effect(91, w(1) & ENVIRONMENT_MAP != 0),
            fog_map_effect: effect(104, w(1) & FOG_MAP != 0),
        })
    }
}

/// One stage of the derived state: the texture, the colour and alpha operations
/// (operation, argument 1, argument 2) and the texture coordinate set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage {
    pub texture: u32,
    pub color: [u32; 3],
    pub alpha: [u32; 3],
    pub set: u32,
    /// The map the stage came from, for the report: 0 to 7 the maps, 8 the environment
    /// map, 9 the fog map, 10 the stage with no texture.
    pub source: u32,
    /// The clamp mode and the filter mode of the map (`NiTexturingProperty` values).
    pub clamp: u32,
    pub filter: u32,
}

/// The sampler states of a map: address U, address V, the magnify, minify and mip filters,
/// as Direct3D has them. The clamp modes are clamp-clamp, clamp-wrap, wrap-clamp, wrap-wrap;
/// the filters nearest, bilinear, trilinear, nearest with nearest mip, nearest with linear
/// mip, bilinear with nearest mip.
pub fn sampler_states(clamp: u32, filter: u32) -> [u32; 5] {
    let (u, v) = match clamp {
        0 => (3, 3),
        1 => (3, 1),
        2 => (1, 3),
        _ => (1, 1),
    };
    let f = match filter {
        0 => [1, 1, 0],
        1 => [2, 2, 0],
        2 => [2, 2, 2],
        3 => [1, 1, 1],
        4 => [1, 1, 2],
        _ => [2, 2, 1],
    };
    [u, v, f[0], f[1], f[2]]
}

/// The state of a draw as the pipeline would set it for a material.
#[derive(Clone, Debug, Default)]
pub struct Derived {
    /// The device passes, each with its stages.
    pub passes: Vec<Vec<Stage>>,
    /// For each pass: blend on, source, destination (Direct3D values).
    pub blends: Vec<[u32; 3]>,
    /// The alpha test of the first pass (on, function, reference); the passes after it have
    /// the test of their template (`template_alpha_test`), in `alpha_tests`.
    pub alpha_test: [u32; 3],
    pub alpha_tests: Vec<[u32; 3]>,
    /// Depth test on, depth write on, function.
    pub depth: [u32; 3],
    pub cull: u32,
    pub fill: u32,
    pub shade: u32,
    /// Enable, function, reference, mask, fail, z-fail, pass.
    pub stencil: [u32; 7],
    pub lighting: bool,
    /// Diffuse, emissive and ambient material sources; `None` when the engine leaves the
    /// state of the draw before.
    pub sources: [Option<u32>; 3],
    /// The `D3DMATERIAL8`: diffuse, ambient, specular, emissive with the alpha, then power.
    pub material: [f32; 17],
    pub fog: bool,
    /// The fog is by the distance from the eye (RANGE_SQ), not by the depth: the engine
    /// sets `D3DRS_RANGEFOGENABLE` for it.
    pub range_fog: bool,
    /// The blend draws at once in the order of the scene, out of the sort (the no-sorter
    /// bit of the alpha property): the table draws it with the opaque.
    /// Engine-rules R2 (the skill wgpu-renderer-dev).
    pub no_sorter: bool,
    /// The shape's own fog, when its fog property is not the cell's: the colour, the start
    /// and the end. The table keeps them in the slot; the scene's fog is for the others.
    pub own_fog: Option<(u32, f32, f32)>,
    /// The record has no effects that this model does not make (projected lights or
    /// shadows); a shape with them is drawn without them.
    pub modelled: bool,
}

// The first-stage table (pipeline + 0x108) and the later-stage table (+ 0x1F8) of the blend
// slots, from `BLEND_TEMPLATES`: colour operation, argument 1, argument 2, alpha operation,
// argument 1, argument 2. `SIGNED` is the bump operation of the device (BUMPENVMAPLUMINANCE
// here, 0x17). Slots 3 and 5 of the first table are not written by the engine (zeros).
const SIGNED: u32 = 0x17;
const FIRST: [[u32; 6]; 9] = [
    [2, 2, 0, 2, 2, 0],
    [4, 2, 0, 4, 2, 0],
    [4, 2, 0, 4, 2, 0],
    [0, 0, 0, 0, 0, 0],
    [4, 2, 0, 4, 2, 0],
    [0, 0, 0, 0, 0, 0],
    [4, 2, 0, 2, 2, 0],
    [0, 0, 0, 0, 0, 0],
    [SIGNED, 2, 1, 1, 2, 1],
];
const LATER: [[u32; 6]; 10] = [
    [7, 2, 1, 1, 2, 1],
    [0xD, 2, 1, 1, 2, 1],
    [4, 2, 1, 4, 2, 1],
    [5, 2, 1, 1, 2, 1],
    [2, 2, 1, 2, 2, 1],
    [2, 2, 1, 3, 2, 1],
    [4, 2, 1, 2, 2, 1],
    [4, 2, 1, 3, 2, 1],
    [SIGNED, 2, 1, 2, 2, 1],
    [4, 2, 1, 4, 2, 1],
];

/// The blend of a pass whose first stage has the slot: from `adjustAlphaTemplate`. Slots 0
/// and 8 add (ONE, ONE); 1 and 6 blend by the alpha (SRCALPHA, INVSRCALPHA); 2 and 4
/// multiply (ZERO, SRCCOLOR); the others do not blend.
fn template_blend(slot: u32) -> [u32; 3] {
    match slot {
        0 | 8 => [1, 2, 2],
        1 | 6 => [1, 5, 6],
        2 | 4 => [1, 1, 3],
        _ => [0, 0, 0],
    }
}

/// The alpha test of a pass after the first, whose first stage has the slot: the pass takes
/// the alpha state of its template (`NiDX8TexturePass::setBlendingState` gives the template
/// to `updateAlphaStates` for each pass after the first), not that of the material. Only
/// template 4 tests: "greater than" 128 (`adjustAlphaTemplate`).
fn template_alpha_test(slot: u32) -> [u32; 3] {
    match slot {
        4 => [1, compare_function(4), 0x80],
        _ => [0, 0, 0],
    }
}

/// `NiAlphaProperty` blend modes to `D3DBLEND`, as `NiDX8RenderState::updateAlphaStates`
/// reads them: the mode is a field of 4 bits that indexes the table of 11 blend values
/// without a bound, and the table of the alpha test functions follows it in the render
/// state (`alphaMapping` at 0xE4, `alphaTestMapping` at 0x110). A mode of 11 to 15 thus
/// gives an alpha test function as the blend value; the test level has a mesh with mode 13.
pub fn blend_mode(value: u32) -> u32 {
    [2, 1, 3, 4, 9, 10, 5, 6, 7, 8, 11, 8, 2, 3, 4, 5][(value & 0xF) as usize]
}

/// The test functions of the properties to `D3DCMP`.
pub fn compare_function(value: u32) -> u32 {
    [8, 2, 3, 4, 5, 6, 7, 1].get(value as usize).copied().unwrap_or(8)
}

/// `NiStencilProperty` actions to `D3DSTENCILOP`.
fn stencil_action(value: u32) -> u32 {
    [1, 2, 3, 4, 5, 6].get(value as usize).copied().unwrap_or(1)
}

/// The texture coordinate set of a map: its set when the geometry has it, else the last.
fn coordinate_set(set: u32, sets: u32) -> u32 {
    let last = sets.saturating_sub(1);
    if set < sets {
        set
    } else {
        last
    }
}

/// The ops of a stage: the first-stage table for the first stage of a pass, else the
/// later-stage table. `apply` is 1 when the apply mode is not "replace": a first stage with
/// 0 takes the texture only. `has_texture` false gives the vertex colour only.
fn ops(slot: u32, first: bool, apply: bool, has_texture: bool) -> ([u32; 3], [u32; 3]) {
    if first {
        let t = FIRST.get(slot as usize).copied().unwrap_or([4, 2, 0, 4, 2, 0]);
        if !apply {
            return ([2, 2, t[2]], [2, 2, t[5]]);
        }
        if !has_texture {
            return ([3, t[1], t[2]], [3, t[4], t[5]]);
        }
        ([t[0], t[1], t[2]], [t[3], t[4], t[5]])
    } else {
        let t = LATER.get(slot as usize).copied().unwrap_or([4, 2, 1, 4, 2, 1]);
        ([t[0], t[1], t[2]], [t[3], t[4], t[5]])
    }
}

/// Derives the state of a draw for a material with a geometry of `sets` texture sets.
pub fn derive(m: &Material, sets: u32) -> Derived {
    let mut d = Derived { modelled: m.flags & (PROJECTED_LIGHTS | PROJECTED_SHADOWS) == 0, ..Default::default() };
    let apply = m.apply_mode != 0;

    // The render states.
    d.alpha_test = if m.flags & ALPHA_TEST != 0 { [1, compare_function(m.alpha_function), m.alpha_reference] } else { [0, 0, 0] };
    d.depth = [1, (m.flags & DEPTH_WRITE != 0) as u32, if m.flags & DEPTH_TEST != 0 { compare_function(m.depth_function) } else { 8 }];
    // The cull mode from the draw mode of the stencil property: both or CCW draw with the
    // CW faces culled (2), CW with CCW culled (3), both sides none (1).
    d.cull = [2, 2, 3, 1][(m.stencil[6] & 3) as usize];
    d.fill = if m.flags & WIREFRAME != 0 { 2 } else { 3 };
    d.shade = if m.flags & SMOOTH != 0 { 2 } else { 1 };
    // The test function of the stencil property counts from NEVER to ALWAYS, as `D3DCMP`
    // does; the alpha and z-buffer properties count the other way.
    d.stencil = if m.flags & STENCIL != 0 {
        [1, (m.stencil[0] + 1).min(8), m.stencil[1], m.stencil[2], stencil_action(m.stencil[3]), stencil_action(m.stencil[4]), stencil_action(m.stencil[5])]
    } else {
        [0; 7]
    };
    d.fog = m.flags & FOG != 0 && m.fog_mode < 2;
    d.range_fog = d.fog && m.fog_mode == 1;
    d.no_sorter = m.flags & NO_SORTER != 0;
    d.own_fog = m.own_fog;
    // The colours come with each draw (the shape record): `d3d_material`.

    // The lighting and the material sources (`NiDX8LightManager::setState`).
    let base = m.maps[BASE];
    if base.present && base.texture != 0 && m.apply_mode == 0 {
        d.lighting = false;
    } else if m.lighting_mode == 0 {
        if m.source_mode != 0 && m.source_mode != 2 {
            d.lighting = false;
        } else {
            d.lighting = true;
            d.sources = [None, Some(0), None];
        }
    } else {
        d.lighting = true;
        d.sources = match m.source_mode {
            0 => [Some(0), Some(0), Some(0)],
            1 => [Some(0), Some(1), Some(0)],
            2 => [Some(1), Some(0), Some(1)],
            _ => [None, None, None],
        };
    }

    // The stages. A pass has at most 8 stages.
    let stage = |map: usize, slot: u32, first: bool| -> Stage {
        let mp = m.maps[map];
        let (color, alpha) = ops(slot, first, apply, mp.texture != 0);
        Stage { texture: mp.texture, color, alpha, set: coordinate_set(mp.set, sets), source: map as u32, clamp: mp.clamp, filter: mp.filter }
    };
    let mut pass: Vec<Stage> = Vec::new();
    let mut pass_slot = 2u32;
    let has = |i: usize| m.maps[i].present && m.maps[i].texture != 0;
    let multi = has(DARK) || has(DETAIL) || has(GLOW) || m.decals != 0 || (m.environment.present && (has(GLOSS) || has(BUMP)));
    let effects = m.environment.present || m.fog_map.present;
    if !multi && !effects {
        // The short path: the base stage, or the stage with no texture.
        if has(BASE) {
            pass.push(stage(BASE, 2, true));
        } else {
            let (color, alpha) = ops(2, true, true, false);
            pass.push(Stage { texture: 0, color, alpha, set: 0, source: 10, clamp: 3, filter: 1 });
        }
        d.passes.push(pass);
        d.blends.push(if m.flags & BLEND != 0 { [1, blend_mode(m.source_blend), blend_mode(m.destination_blend)] } else { [0, 0, 0] });
        d.alpha_tests.push(d.alpha_test);
        return d;
    }
    // The base stage with the detail stage, then the dark stage.
    if has(BASE) {
        pass.push(stage(BASE, 2, true));
        if has(DETAIL) {
            pass.push(stage(DETAIL, 3, false));
        }
    } else if !has(DARK) {
        let (color, alpha) = ops(2, true, true, false);
        pass.push(Stage { texture: 0, color, alpha, set: 0, source: 10, clamp: 3, filter: 1 });
    }
    if has(DARK) {
        let first = pass.is_empty();
        pass.push(stage(DARK, 2, first));
    }
    let mut passes: Vec<(Vec<Stage>, u32)> = Vec::new();
    // Each decal in a pass of its own: `setCTPipelineState` (0x6B41B0) first acquires the
    // sub-pass for a pixel shader (0x6B3EE0), which moves to a new sub-pass when the one
    // in progress already has its pixel shader, as it has after its base stage; only then
    // does it take a sub-pass with capacity for the stage (0x6B3B30). The device replay
    // shows the second draw with the decal and its blend.
    for i in 0..m.decals as usize {
        let map = DECAL0 + i;
        if map < 8 && has(map) {
            passes.push((core::mem::take(&mut pass), pass_slot));
            pass_slot = 1;
            pass.push(stage(map, 1, true));
        }
    }
    // The environment map with the bump and gloss maps.
    if m.environment.present {
        let needed = 1 + has(BUMP) as u32 + has(GLOSS) as u32;
        // A new pass takes the alpha template of the blend slot of its first stage
        // (`template_for` of `setCTPipelineState`): 8 for the bump stage, else 0 for the
        // stage of the environment map, which adds. The enchanted items of the game have a
        // gloss map (a texture of one colour), so their glow is always a pass of its own.
        if has(GLOSS) || pass.len() as u32 + needed > 8 {
            passes.push((core::mem::take(&mut pass), pass_slot));
            pass_slot = if has(BUMP) { 8 } else { 0 };
        }
        if has(BUMP) {
            pass.push(stage(BUMP, 8, pass.is_empty()));
        }
        // The stage of the environment map has blend slot 0 (it adds), and its generated
        // texture coordinates have 0 in the low word of the index.
        let (mut color, alpha) = ops(0, pass.is_empty(), true, true);
        // Seen in the test level, not read in the code: with fog and a bump map, or with fog
        // and no base texture, the stage multiplies the environment map by the diffuse
        // colour and adds the current colour (MULTIPLYADD); else it adds the map.
        if !pass.is_empty() && m.flags & FOG != 0 && (has(BUMP) || !has(BASE)) {
            color = [25, 2, 0];
        }
        pass.push(Stage { texture: m.environment.texture, color, alpha, set: 0, source: 8, clamp: m.environment.clamp, filter: m.environment.filter });
        if has(GLOSS) {
            pass.push(stage(GLOSS, 2, false));
        }
    }
    // The glow stage, in the pass when it has room.
    if has(GLOW) {
        if pass.len() >= 8 {
            passes.push((core::mem::take(&mut pass), pass_slot));
            pass_slot = 0;
        }
        pass.push(stage(GLOW, 0, pass.is_empty()));
    }
    if m.fog_map.present {
        if pass.len() >= 8 {
            passes.push((core::mem::take(&mut pass), pass_slot));
            pass_slot = 2;
        }
        let (color, alpha) = ops(2, pass.is_empty(), false, true);
        pass.push(Stage { texture: m.fog_map.texture, color, alpha, set: 1, source: 9, clamp: m.fog_map.clamp, filter: m.fog_map.filter });
    }
    if !pass.is_empty() {
        passes.push((pass, pass_slot));
    }
    for (index, (stages, slot)) in passes.into_iter().enumerate() {
        d.passes.push(stages);
        d.blends.push(if index == 0 {
            if m.flags & BLEND != 0 { [1, blend_mode(m.source_blend), blend_mode(m.destination_blend)] } else { [0, 0, 0] }
        } else {
            template_blend(slot)
        });
        d.alpha_tests.push(if index == 0 { d.alpha_test } else { template_alpha_test(slot) });
    }
    d
}

/// The `D3DMATERIAL8` of the colours of a material (ambient, diffuse, specular, emissive,
/// shininess, alpha, as the records have them): diffuse, ambient, specular, emissive with the
/// alpha, then the power.
pub fn d3d_material(values: &[f32; 14]) -> [f32; 17] {
    let mut material = [0.0f32; 17];
    for (i, from) in [3usize, 0, 6, 9].into_iter().enumerate() {
        material[i * 4] = values[from];
        material[i * 4 + 1] = values[from + 1];
        material[i * 4 + 2] = values[from + 2];
        material[i * 4 + 3] = values[13];
    }
    material[16] = values[12];
    material
}
