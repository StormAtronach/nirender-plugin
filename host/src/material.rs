//! The material of a draw, from the `MATERIAL` record of the plugin: the facts of the properties of the shape, and the derivation of the state of
//! the draw from them, as the ports of the engine's pipeline make it, where a rule names its
//! function. The helper draws a shape with the derived state (`Pass` in lib.rs).
//!
//! Each rule names the engine function it comes from (read in IDA, 2026-10-23, guess audit
//! item 4): `NiDX8TexturePass::setCTPipelineState` 0x6B41B0 and `applyStageBlendOps`
//! 0x6B56B0 (the passes and the stages), `NiDX8ConfigurableTexturePipeline::
//! initAlphaStateTemplates` 0x6B90E0 (the stage tables and the pass templates),
//! `NiDX8RenderState::ctor` 0x6B7D70 (the tables of the blend, test, stencil and cull values),
//! `updateD3DState` 0x6B82A0, `setStencilStates` 0x6B8C10 and `updateFogStates` 0x6B8A00 (the
//! render states), `NiDX8LightManager::setState` 0x6BB200 (the lighting and the material
//! sources). One rule is not the engine's: the environment map's combiner of the Morrowind
//! Code Patch, which the plugin finds in the game (`derive`). One rule is \[I\]: the short
//! path's test (`derive`).

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
/// as Direct3D has them. The clamp modes are clamp-clamp, clamp-wrap, wrap-clamp, wrap-wrap
/// (`setStageAddressingMode` 0x6B9E20 reads the pairs at 0x7DDBC8 that
/// `initAlphaStateTemplates` writes); the filters nearest, bilinear, trilinear, nearest with
/// nearest mip, nearest with linear mip, bilinear with nearest mip (`setTextureFiltering`
/// 0x6C1FA0 reads the tables at 0x7DDB80 that `initFilteringAndAddressCaps` 0x6BA840 writes).
/// For a texture without mip levels the engine takes the filters without a mip filter; a
/// sampler of one level gives the same picture.
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
    /// The lighting is on with no light and a black ambient: the emissive-only mode with the
    /// sources "ignore" and "ambient and diffuse" (`useAmbientDiffuse` 0 of
    /// `NiDX8LightManager::setState` 0x6BB200; `updateLights` 0x6BB6C0 then enables no light
    /// and sets D3DRS_AMBIENT black). Only the emissive colour shows.
    pub no_lights: bool,
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
// slots, as `initAlphaStateTemplates` 0x6B90E0 writes them and `applyStageBlendOps` 0x6B56B0
// reads them: colour operation, argument 1, argument 2, alpha operation, argument 1, argument
// 2. `SIGNED` is the bump operation of the device: BUMPENVMAPLUMINANCE (0x17) when the device
// has it, else BUMPENVMAP (0x16), else 0; here 0x17. Slots 3, 5 and 7 of the first table are
// not written by the engine (zeros).
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

/// The blend of a pass whose first stage has the slot: the alpha template of the slot
/// (pipeline + 0x2E8, the ten `NiAlphaProperty` of `initAlphaStateTemplates` 0x6B90E0).
/// Slots 0 and 8 add (ONE, ONE); 1 and 6 blend by the alpha (SRCALPHA, INVSRCALPHA); 2 and 4
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
/// template 4 tests: "greater than" 128 (`initAlphaStateTemplates` 0x6B90E0).
fn template_alpha_test(slot: u32) -> [u32; 3] {
    match slot {
        4 => [1, compare_function(4), 0x80],
        _ => [0, 0, 0],
    }
}

/// `NiAlphaProperty` blend modes to `D3DBLEND`, as `NiDX8RenderState::updateAlphaStates`
/// 0x6B8480 reads them: the mode is a field of 4 bits that indexes the table of 11 blend values
/// without a bound, and the table of the alpha test functions follows it in the render
/// state (`alphaMapping` at 0xE4, `alphaTestMapping` at 0x110; `NiDX8RenderState::ctor`
/// 0x6B7D70 writes both). A mode of 11 to 15 thus gives an alpha test function as the blend
/// value.
pub fn blend_mode(value: u32) -> u32 {
    [2, 1, 3, 4, 9, 10, 5, 6, 7, 8, 11, 8, 2, 3, 4, 5][(value & 0xF) as usize]
}

/// The test functions of the alpha and z-buffer properties to `D3DCMP`: `alphaTestMapping`
/// (0x110) and `zFuncMapping` (0x1D4) of `NiDX8RenderState::ctor` 0x6B7D70, the same values.
pub fn compare_function(value: u32) -> u32 {
    [8, 2, 3, 4, 5, 6, 7, 1].get(value as usize).copied().unwrap_or(8)
}

/// `NiStencilProperty` actions to `D3DSTENCILOP`: `stencilActionMapping` (0x178) of
/// `NiDX8RenderState::ctor` 0x6B7D70, which `setStencilStates` 0x6B8C10 reads.
fn stencil_action(value: u32) -> u32 {
    [1, 2, 3, 4, 5, 6].get(value as usize).copied().unwrap_or(1)
}

/// The texture coordinate set of a map: its set when the geometry has it, else the last
/// (`setCTPipelineState` 0x6B41B0, for each map but the base, which always takes set 0).
fn coordinate_set(set: u32, sets: u32) -> u32 {
    let last = sets.saturating_sub(1);
    if set < sets {
        set
    } else {
        last
    }
}

/// The ops of a stage, as `applyStageBlendOps` 0x6B56B0 sets them: the first-stage table for
/// the first stage of a pass, else the later-stage table. `apply` is 1 when the apply mode is
/// not "replace": a first stage with 0 takes the texture only. `has_texture` false gives the
/// vertex colour only.
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
/// `envmap_lit`: the game's environment map stages of the reflection vector take the lit
/// colour (the stream header's `HEADER_ENVMAP_LIT`); `None` for an older capture without the
/// fact.
pub fn derive(m: &Material, sets: u32, envmap_lit: Option<bool>) -> Derived {
    let mut d = Derived { modelled: m.flags & (PROJECTED_LIGHTS | PROJECTED_SHADOWS) == 0, ..Default::default() };
    let apply = m.apply_mode != 0;

    // The render states (`NiDX8RenderState::updateD3DState` 0x6B82A0): the depth write is
    // flag 2 of the z-buffer property, its function ALWAYS without the test flag; the fill is
    // WIREFRAME (2) with the wireframe flag, else SOLID (3); the shade GOURAUD (2) with the
    // shade flag, else FLAT (1).
    d.alpha_test = if m.flags & ALPHA_TEST != 0 { [1, compare_function(m.alpha_function), m.alpha_reference] } else { [0, 0, 0] };
    d.depth = [1, (m.flags & DEPTH_WRITE != 0) as u32, if m.flags & DEPTH_TEST != 0 { compare_function(m.depth_function) } else { 8 }];
    // The cull mode from the draw mode of the stencil property, with the stencil test on or
    // off (`setStencilStates` 0x6B8C10: `cullModeMapping` at 0x190, indexed by twice the mode
    // plus `bLeftRightSwap`, which the engine leaves 0): both or CCW draw with the CW faces
    // culled (2), CW with CCW culled (3), both sides none (1).
    d.cull = [2, 2, 3, 1][(m.stencil[6] & 3) as usize];
    d.fill = if m.flags & WIREFRAME != 0 { 2 } else { 3 };
    d.shade = if m.flags & SMOOTH != 0 { 2 } else { 1 };
    // The test function of the stencil property counts from NEVER to ALWAYS, as `D3DCMP`
    // does (`stencilFuncMapping` at 0x158); the alpha and z-buffer properties count the
    // other way.
    d.stencil = if m.flags & STENCIL != 0 {
        [1, (m.stencil[0] + 1).min(8), m.stencil[1], m.stencil[2], stencil_action(m.stencil[3]), stencil_action(m.stencil[4]), stencil_action(m.stencil[5])]
    } else {
        [0; 7]
    };
    // As `NiDX8RenderState::updateFogStates` (0x6B8A00): the fog function 0 is linear fog,
    // 1 is range fog, any other value has no fog.
    d.fog = m.flags & FOG != 0 && m.fog_mode < 2;
    d.range_fog = d.fog && m.fog_mode == 1;
    d.no_sorter = m.flags & NO_SORTER != 0;
    d.own_fog = m.own_fog;
    // The colours come with each draw (the shape record): `d3d_material`.

    // The lighting and the material sources (`NiDX8LightManager::setState` 0x6BB200). A base
    // texture with the apply mode "replace" turns the lighting off. The full lighting mode
    // sets the three sources by the source mode. The emissive-only mode with the source
    // modes "ignore" and "ambient and diffuse" leaves the sources as the draw before left
    // them (`None`), keeps the lighting on and clears `useAmbientDiffuse`: `updateLights`
    // (0x6BB6C0) enables no light and sets a black ambient, so only the emissive colour
    // shows (`no_lights`). With "emissive" it turns the lighting off.
    let base = m.maps[BASE];
    if base.present && base.texture != 0 && m.apply_mode == 0 {
        d.lighting = false;
    } else if m.lighting_mode == 0 {
        if m.source_mode != 0 && m.source_mode != 2 {
            d.lighting = false;
        } else {
            d.lighting = true;
            d.no_lights = true;
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

    // The stages. A pass has at most 8 stages. The base stage takes coordinate set 0,
    // whatever set its map names (`setCTPipelineState` 0x6B41B0).
    let stage = |map: usize, slot: u32, first: bool| -> Stage {
        let mp = m.maps[map];
        let (color, alpha) = ops(slot, first, apply, mp.texture != 0);
        let set = if map == BASE { 0 } else { coordinate_set(mp.set, sets) };
        Stage { texture: mp.texture, color, alpha, set, source: map as u32, clamp: mp.clamp, filter: mp.filter }
    };
    let mut pass: Vec<Stage> = Vec::new();
    let mut pass_slot = 2u32;
    let has = |i: usize| m.maps[i].present && m.maps[i].texture != 0;
    // [I] The engine takes the short path when the shape has no texture effect and no map
    // after the base (`NiTexturingProperty::IsMultiTexture` 0x6E1D40 counts a map without a
    // texture too). This test takes it also for the maps that give no stage here (a map
    // without a texture; a gloss or bump map without an environment map), taken to give the
    // same stages; the long path is not read for them.
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
    // does it take a sub-pass with capacity for the stage (0x6B3B30).
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
        // The stage of the environment map has blend slot 0 (it adds). As a first stage it
        // takes the texture alone (SELECTARG1: `configureStageFromTextureProp` 0x6B5A20
        // clears its apply flag); after another stage it adds the texture to the current
        // colour (ADD). Its coordinates come from the effect's mode (the plugin's
        // `effect_stage_words`): 0 and 1 the camera-space position, 2 and 3 the camera-space
        // reflection vector of set 0 (0x30000), 4 the camera-space normal.
        let first = pass.is_empty();
        let (mut color, alpha) = ops(0, first, true, true);
        let reflection = matches!(m.environment_coordinates, 2 | 3);
        match envmap_lit {
            // The Morrowind Code Patch's hook on the coordinate index (0x6C1BA4 to 0x736F80 in
            // the patched game) scales the map by the lit colour when the index is the
            // camera-space reflection vector of set 0 and nothing else (it compares the whole
            // index with 0x30000): SELECTARG1 becomes MODULATE(texture, diffuse) and ADD
            // becomes MULTIPLYADD(current + texture * diffuse); argument 0 is the current
            // colour (`stream.rs`). A projected map (mode 0 or 1) and a diffuse cube map
            // (mode 4) keep the engine's combiner.
            Some(true) if reflection => color = if first { [4, 2, 0] } else { [25, 2, 0] },
            Some(true) => {}
            Some(false) => {}
            // An older capture without the fact: the rule seen in the test level before the
            // hook was read (fog with a bump map, or fog without a base texture).
            None => {
                if !first && m.flags & FOG != 0 && (has(BUMP) || !has(BASE)) {
                    color = [25, 2, 0];
                }
            }
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
    // The stage of the fog map has blend slot 1 (the pass constructor writes it into the
    // stage, `applyStageBlendOps` 0x6B56B0 reads it): after another stage it blends the
    // texture by its alpha; as the first stage it takes the texture alone (0x6B55F7 clears
    // the stage's apply flag).
    if m.fog_map.present {
        if pass.len() >= 8 {
            passes.push((core::mem::take(&mut pass), pass_slot));
            pass_slot = 1;
        }
        let (color, alpha) = ops(1, pass.is_empty(), false, true);
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
/// alpha, then the power (`NiDX8MaterialManager::Entry::Update` 0x6C2240).
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

#[cfg(test)]
mod fog_map_tests {
    use super::*;

    /// A material record with a fog map, and a base map when `base` is set.
    fn material(base: bool) -> Material {
        let mut words = [0u32; WORDS];
        words[1] = FOG_MAP;
        words[6] = 2;
        if base {
            words[8] = 0x100;
            words[12] = 1;
        }
        words[85] = 0x200;
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        Material::parse(&bytes).unwrap()
    }

    #[test]
    fn the_fog_map_after_a_stage_blends_by_its_alpha() {
        let d = derive(&material(true), 1, None);
        let fog = d.passes[0].iter().find(|s| s.source == 9).unwrap();
        // Slot 1 of the later-stage table: BLENDTEXTUREALPHA (0xD) of the texture and the
        // current colour.
        assert_eq!(fog.color, [0xD, 2, 1]);
    }

    #[test]
    fn without_a_base_map_the_fog_map_follows_the_stage_with_no_texture() {
        let d = derive(&material(false), 1, None);
        assert_eq!(d.passes[0][0].source, 10);
        let fog = d.passes[0].iter().find(|s| s.source == 9).unwrap();
        assert_eq!(fog.color, [0xD, 2, 1]);
    }
}

#[cfg(test)]
mod engine_rule_tests {
    use super::*;

    /// A material record with a base map (texture 1, the coordinate set `base_set`) and an
    /// environment map (texture 5) of a sphere map (the reflection vector); with `gloss` also
    /// a gloss map (texture 3), which puts the environment map first in a pass of its own.
    fn material(gloss: bool, base_set: u32) -> Material {
        material_of_mode(gloss, base_set, 2)
    }

    /// The same with the effect's coordinate mode `mode` (0 and 1 projected, 2 a sphere map,
    /// 3 a specular cube map, 4 a diffuse cube map).
    fn material_of_mode(gloss: bool, base_set: u32, mode: u32) -> Material {
        let mut words = [0u32; WORDS];
        words[84] = mode;
        words[1] = ENVIRONMENT_MAP;
        words[6] = 2;
        words[8] = 1;
        words[11] = base_set;
        words[12] = 1;
        if gloss {
            words[8 + GLOSS * 5] = 3;
            words[12 + GLOSS * 5] = 1;
        }
        words[81] = 5;
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        Material::parse(&bytes).unwrap()
    }

    fn environment_stage(d: &Derived) -> Stage {
        *d.passes.iter().flatten().find(|s| s.source == 8).unwrap()
    }

    #[test]
    fn the_code_patch_scales_an_environment_map_by_the_lit_colour() {
        // After the base stage: ADD, or MULTIPLYADD with the Code Patch's hook.
        assert_eq!(environment_stage(&derive(&material(false, 0), 1, Some(false))).color, [7, 2, 1]);
        assert_eq!(environment_stage(&derive(&material(false, 0), 1, Some(true))).color, [25, 2, 0]);
        // First in a pass of its own (with a gloss map): SELECTARG1 of the texture, or
        // MODULATE by the diffuse colour.
        assert_eq!(environment_stage(&derive(&material(true, 0), 1, Some(false))).color, [2, 2, 0]);
        assert_eq!(environment_stage(&derive(&material(true, 0), 1, Some(true))).color, [4, 2, 0]);
        // An older capture without the fact keeps the rule seen before (no fog here: ADD).
        assert_eq!(environment_stage(&derive(&material(false, 0), 1, None)).color, [7, 2, 1]);
    }

    #[test]
    fn the_code_patch_leaves_other_coordinates_alone() {
        // The hook compares the whole coordinate index with the camera-space reflection
        // vector of set 0: a projected map (modes 0 and 1) and a diffuse cube map (mode 4)
        // keep ADD and SELECTARG1 (the rainbow particles of the test level's exhibits 107 and
        // 108 drew black when every map took MULTIPLYADD by their black vertex colour).
        for mode in [0, 1, 4] {
            assert_eq!(environment_stage(&derive(&material_of_mode(false, 0, mode), 1, Some(true))).color, [7, 2, 1], "mode {mode}");
            assert_eq!(environment_stage(&derive(&material_of_mode(true, 0, mode), 1, Some(true))).color, [2, 2, 0], "mode {mode}");
        }
        assert_eq!(environment_stage(&derive(&material_of_mode(false, 0, 3), 1, Some(true))).color, [25, 2, 0]);
    }

    #[test]
    fn the_emissive_only_mode_takes_no_light_and_a_black_ambient() {
        // `NiDX8LightManager::setState` 0x6BB200 and `updateLights` 0x6BB6C0: lighting mode 0
        // (emissive) with the sources "ignore" (0) and "ambient and diffuse" (2) clears
        // `useAmbientDiffuse`: no light, a black ambient. With the source "emissive" (1) the
        // lighting is off. The full mode (1) takes the lights.
        let of = |lighting: u32, source: u32| {
            let mut words = [0u32; WORDS];
            words[68] = source;
            words[69] = lighting;
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            derive(&Material::parse(&bytes).unwrap(), 1, Some(false))
        };
        for source in [0, 2] {
            let d = of(0, source);
            assert!(d.lighting && d.no_lights, "source {source}");
        }
        assert!(!of(0, 1).lighting);
        for source in [0, 1, 2] {
            let d = of(1, source);
            assert!(d.lighting && !d.no_lights, "source {source}");
        }
    }

    #[test]
    fn the_base_stage_takes_coordinate_set_0() {
        let d = derive(&material(false, 2), 3, Some(false));
        assert_eq!(d.passes[0][0].source, BASE as u32);
        assert_eq!(d.passes[0][0].set, 0);
    }
}
