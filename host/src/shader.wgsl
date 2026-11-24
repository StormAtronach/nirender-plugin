// The binding arrays are an extension of WGSL that the shader must enable before any
// other item.
enable wgpu_binding_array;

// The work of the fixed-function pipeline of Direct3D 8 for one draw of the game: transform,
// lighting for each vertex, the texture stages, the alpha test and the fog. One shader does
// all draws: each choice of a draw is a value in `Draw`.
//
// A matrix has the 16 floats of Direct3D in their order. Direct3D multiplies a row vector
// with the matrix. Here the same floats are the columns, so `matrix * vector` is that product.

// The debug view of this module: 0 in normal play. The module of a view with code in the world
// shader is made from this text with the view's id on this line (`world_variant_source` in
// request.rs), so the code of the view is not in the pipelines of normal play.
const DEBUG_VIEW: u32 = 0u;
// The ids of the views with code in the world shader (`debug_lines::view_id`).
const VIEW_LIGHTS: u32 = 1u;
const VIEW_SHADOW_LAYERS: u32 = 2u;
const VIEW_FOG: u32 = 6u;
// The colour of the sky and the clouds in the fog view: they take no fog of the draws.
const FOG_VIEW_SKY = vec3f(0.1, 0.2, 0.6);

struct Light {
    diffuse: vec4f,
    ambient: vec4f,
    // w: the type of Direct3D. 1 point, 2 spot, 3 directional.
    position: vec4f,
    // w: the range.
    direction: vec4f,
    // The three attenuation values. w: the falloff of a spot light.
    attenuation: vec4f,
    // A spot light: the cosine of half of the inner angle and of half of the outer angle.
    // z: the radius of the light of the game. w: 1 over the cut-off of the extension's fade.
    cone: vec4f,
}

struct Stage {
    // Colour: operation, argument 1, argument 2, argument 0.
    color: vec4u,
    // Alpha: operation, argument 1, argument 2.
    alpha: vec4u,
    // The texture coordinate: how Direct3D makes it (0 from the vertex, 1 the normal, 2 the
    // position, 3 the reflection vector, each in camera space), the coordinate set of the
    // vertex, the number of parts that the texture transform gives (0 for no transform), and
    // 1 when the last of them divides the others.
    coordinate: vec4u,
    // A bump stage: the matrix that turns its texel into a move of the coordinate of the
    // next stage (m00, m01, m10, m11).
    bump: vec4f,
    // A bump stage with luminance: the scale and the offset of the luminance.
    luminance: vec4f,
    // The texture and the sampler of the stage among the arrays of the frame (with the
    // arrays on: see `array_shader`), and the entry of its texture effect (0 for none). The w
    // of stage 0 of a record: the distance to the draw's overflow record (`stage_record`).
    slots: vec4u,
}

struct Draw {
    world: array<mat4x4f, 4>,
    view: mat4x4f,
    projection: mat4x4f,
    material_diffuse: vec4f,
    material_ambient: vec4f,
    material_emissive: vec4f,
    scene_ambient: vec4f,
    fog_color: vec4f,
    // start, end, 1 when fog is on, 1 when the fog is on the distance from the eye and not
    // on the depth.
    fog: vec4f,
    texture_factor: vec4f,
    // lighting on, the vertex has a normal, the vertex has a colour, the number of weights.
    flags: vec4u,
    // diffuse source, emissive source, alpha test function (0 is off), alpha reference.
    more: vec4u,
    // The number of lights, the number of stages, the ambient source. w: bit 1, the light is
    // shaded by the functions of the extension (`extension_shade`, `extension_finish`);
    // bit 4, the draw is the sky dome and the extension gives its shape and its colour; bit
    // 8, the extension gives the colour and the draw keeps its alpha (the shadow of a moon);
    // bit 128, the extension makes the light for each pixel: the vertex stage makes no light
    // and the colour is not kept between 0 and 1 between the stages; bit 1024, this crate's
    // CLOUDS_HOOK_BIT: the clouds with the extension's colour; bit 2048, this crate's
    // PARTICLE_HOOK_BIT: a particle whose colour after the stages goes through
    // `extension_particle_color`; bit 16384, this crate's CORNER_LIGHT_BIT: a particle takes its
    // lights at its corners (`extension_corner_shade` in the vertex stage); bit 32768, this
    // crate's NO_LIGHTS_BIT: a draw lit for each pixel with no lights; bit 2 and the bits from
    // 16 (but 128, 1024, 2048, 16384 and 32768) are for the extension.
    counts: vec4u,
    // The lights of the draw in `lights`: the first and the number.
    light_range: vec4u,
    // The lights of the draw among the clustered lights of the frame, a bit for each slot:
    // an entry of `cluster_indexes` is the slot in its high half and the index in `lights`
    // in its low half, and a pixel takes the light only when the draw has its bit.
    light_mask: array<vec4u, 2>,
    // The first RECORD_STAGES stages; the later stages are in the overflow record
    // (`stage_record`).
    stages: array<Stage, RECORD_STAGES>,
    // The texture transform of each of these stages.
    texture_transforms: array<mat4x4f, RECORD_STAGES>,
    // 1 / width and -1 / height of the viewport.
    pixel: vec4f,
    // The values of the extension for its sky functions.
    extension: array<vec4f, 8>,
    // A plane in the space of the world of the draw: a pixel on its negative side is not
    // drawn. All 0: no plane. For a draw that the renderer makes again in a reflection.
    clip_plane: vec4f,
}

// The values of the extension for the frame, and its texture: see `FrameParameters`.
struct Frame {
    values: array<vec4f, 32>,
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var extension_texture: texture_2d<f32>;
@group(0) @binding(2) var extension_sampler: sampler;

// The clusters of the lights of the frame: a grid of tiles of the screen by slices of
// depth, each with the lights whose sphere touches it, as indexes into `lights`.
struct Clusters {
    // Tiles across, tiles down, slices; w is 1 when the clusters are on.
    grid: vec4u,
    // The tiles for each pixel, across and down.
    screen: vec4f,
    // The slice of a depth z is scale * log(z) + bias; z is at least the third value.
    depth: vec4f,
    // The lights of the world (w of `grid` 3): the translation from the space of the retained
    // set to the space of a draw of the stream.
    offset: vec4f,
    // The lights of the world: the origin of the retained set, the translation to the space of
    // a draw in the world's own space (bit 128 of `light_range.w`).
    origin: vec4f,
}
@group(0) @binding(3) var<uniform> clusters: Clusters;
@group(0) @binding(4) var<storage, read> cluster_table: array<vec2u>;
@group(0) @binding(5) var<storage, read> cluster_indexes: array<u32>;
// The lights of all draws of the frame.
@group(0) @binding(6) var<storage, read> lights: array<Light>;
// The bones of all skinned draws of the frame.
@group(0) @binding(7) var<storage, read> palette: array<mat4x4f>;
// The bones of the skinned instances of the table (`retained.rs`), in the space of the set:
// a retained skinned draw reads them from the first bone of its instance.
@group(0) @binding(12) var<storage, read> retained_palette: array<mat4x4f>;
// The bone influences of the shapes that a packed vertex cannot hold (`influences.rs`): a vertex
// of such a shape names its entries.
struct Influence {
    bone: u32,
    weight: f32,
}
@group(0) @binding(13) var<storage, read> influences: array<Influence>;
// The retained instances (`retained.rs`): the lights of a draw with bit 8 of
// `light_range.w` are `lights_retained[light_indexes[i]]`; a draw with bit 4 takes the
// values of its view from `scene` (bound with a dynamic offset: one scene for each view).
@group(0) @binding(9) var<storage, read> light_indexes: array<u32>;
struct Scene {
    view: mat4x4f,
    projection: mat4x4f,
    fog_color: vec4f,
    fog: vec4f,
    scene_ambient: vec4f,
    pixel: vec4f,
    clip_plane: vec4f,
    // The sun of the view for the extension's shade (`extension[5]` and `[7]` of a draw the
    // extension shaded), and the bits of `counts.w` that the extension set for the frame
    // for each kind of draw (`slot_kind`: the blend form times 2, plus 1 with a depth write).
    extension5: vec4f,
    extension7: vec4f,
    counts: array<vec4u, 2>,
    // The origin of the retained set in the world: the positions of its draws are relative
    // to it.
    origin: vec4f,
    // The factor the extension's shade applies to an ambient light: a retained draw's own
    // ambient (its instance's lights record) times this is the scaled value.
    ambient_scale: vec4f,
    // x: 1 when the view takes the point lights of the world (`world_lights_here`).
    world_lights: vec4u,
}
@group(0) @binding(10) var<uniform> scene: Scene;
// The texture matrix of each texture effect of the table, one for each effect, whatever number
// of shapes it lights: a stage with `slots.z` takes the entry `slots.z - 1` (`retained.rs`).
@group(0) @binding(14) var<storage, read> effect_entries: array<mat4x4f>;
@group(0) @binding(11) var<storage, read> lights_retained: array<Light>;

// The facts of a stage key (W530). This text declares them as `const`, with FIXED false: each
// function below reads the fact from the draw's record. It is the general shader and the vertex
// module of every pipeline. The fragment module of a stage pipeline is made from this text with
// each `const` of the block as an `override` (`stage_shaders::stage_facts_as_overrides`), and the
// pipeline gives FIXED true and its key's facts and stage codes (`stage_shaders::StageKey::constants`).
// No other part of the text changes, so every stage key has the same module. The compiler drops the
// branch that FIXED cannot take only after it inlines the callees. So on Direct3D 12 each stage
// pipeline costs about 10 ms more to make in DXC than with a text written for its key (W584,
// option 3).
// STAGE FACTS BEGIN: one declaration on each line between the marks.
const FIXED: bool = false;
// The bits of SHADE_BITS of the record's `counts.w`.
const FIXED_SHADE: u32 = 0u;
// `flags.x` (the lighting) and `flags.y` (the vertex has a normal).
const FIXED_LIT: u32 = 0u;
const FIXED_NORMAL: u32 = 0u;
// Bits 4 (the scene's values), 16 (flat-shaded) and 64 (adds to the picture) of `light_range.w`.
const FIXED_SCENE: bool = false;
const FIXED_FLAT: bool = false;
const FIXED_ADDS: bool = false;
// The draw has a plane of its own.
const FIXED_PLANE: bool = false;
// The alpha test's function, 0 for none (`more.z`).
const FIXED_ALPHA_TEST: u32 = 0u;
// The number of texture stages (`counts.y`), and the codes of each stage (W530 step 2): in the
// bytes of FIXED_COLOR_<i> from the lowest, the colour operation and its arguments 1, 2 and 0
// (`stage.color`); in the bytes of FIXED_ALPHA_<i>, the alpha operation and its arguments 1 and
// 2 (`stage.alpha`).
const FIXED_COUNT: u32 = 0u;
const FIXED_COLOR_0: u32 = 0u;
const FIXED_COLOR_1: u32 = 0u;
const FIXED_COLOR_2: u32 = 0u;
const FIXED_COLOR_3: u32 = 0u;
const FIXED_COLOR_4: u32 = 0u;
const FIXED_COLOR_5: u32 = 0u;
const FIXED_COLOR_6: u32 = 0u;
const FIXED_COLOR_7: u32 = 0u;
const FIXED_ALPHA_0: u32 = 0u;
const FIXED_ALPHA_1: u32 = 0u;
const FIXED_ALPHA_2: u32 = 0u;
const FIXED_ALPHA_3: u32 = 0u;
const FIXED_ALPHA_4: u32 = 0u;
const FIXED_ALPHA_5: u32 = 0u;
const FIXED_ALPHA_6: u32 = 0u;
const FIXED_ALPHA_7: u32 = 0u;
// STAGE FACTS END
// The bits of `counts.w` that a stage key holds (`stage_shaders::SHADE_BITS`), and the sky's bits
// among them, which the scene's part never has (`stage_shaders::SKY_BITS`).
const SHADE_BITS: u32 = 52927u;
const SKY_BITS: u32 = 12u;
// The bits of `light_range.w` that a stage key holds: 4, 16 and 64.
const LIGHT_FACT_BITS: u32 = 84u;

fn from_scene() -> bool {
    if FIXED { return FIXED_SCENE; }
    return (draws[record].light_range.w & 4u) != 0u;
}
fn draw_flat() -> bool {
    if FIXED { return FIXED_FLAT; }
    return (draws[record].light_range.w & 16u) != 0u;
}
fn draw_adds() -> bool {
    if FIXED { return FIXED_ADDS; }
    return (draws[record].light_range.w & 64u) != 0u;
}
fn draw_lit() -> u32 {
    if FIXED { return FIXED_LIT; }
    return draws[record].flags.x;
}
fn draw_normal() -> u32 {
    if FIXED { return FIXED_NORMAL; }
    return draws[record].flags.y;
}
fn draw_alpha_test() -> u32 {
    if FIXED { return FIXED_ALPHA_TEST; }
    return draws[record].more.z;
}
fn stage_count() -> u32 {
    if FIXED { return FIXED_COUNT; }
    return draws[record].counts.y;
}
// The four bytes of a word of the stage codes, from the lowest.
fn stage_codes(word: u32) -> vec4u {
    return vec4u(word & 255u, (word >> 8u) & 255u, (word >> 16u) & 255u, word >> 24u);
}
// The stages of a draw in its record. A draw of more stages has an overflow record with the
// later stages from its stage 0, at the distance in `slots.w` of the record's stage 0 from the
// record (0 when the draw has none).
const RECORD_STAGES: u32 = 6u;

// The record that holds stage `index` of the draw.
fn stage_record(index: u32) -> u32 {
    if index < RECORD_STAGES {
        return record;
    }
    return record + draws[record].stages[0].slots.w;
}

// The place of stage `index` in the stages of its record (`stage_record`).
fn stage_place(index: u32) -> u32 {
    if index < RECORD_STAGES {
        return index;
    }
    return index - RECORD_STAGES;
}

// The colour operation and its arguments 1, 2 and 0 of stage `index` (`stage.color`).
fn stage_color(index: u32) -> vec4u {
    if FIXED {
        switch index {
            case 0u: { return stage_codes(FIXED_COLOR_0); }
            case 1u: { return stage_codes(FIXED_COLOR_1); }
            case 2u: { return stage_codes(FIXED_COLOR_2); }
            case 3u: { return stage_codes(FIXED_COLOR_3); }
            case 4u: { return stage_codes(FIXED_COLOR_4); }
            case 5u: { return stage_codes(FIXED_COLOR_5); }
            case 6u: { return stage_codes(FIXED_COLOR_6); }
            default: { return stage_codes(FIXED_COLOR_7); }
        }
    }
    return draws[stage_record(index)].stages[stage_place(index)].color;
}
// The alpha operation and its arguments 1 and 2 of stage `index` (`stage.alpha`).
fn stage_alpha(index: u32) -> vec3u {
    if FIXED {
        switch index {
            case 0u: { return stage_codes(FIXED_ALPHA_0).xyz; }
            case 1u: { return stage_codes(FIXED_ALPHA_1).xyz; }
            case 2u: { return stage_codes(FIXED_ALPHA_2).xyz; }
            case 3u: { return stage_codes(FIXED_ALPHA_3).xyz; }
            case 4u: { return stage_codes(FIXED_ALPHA_4).xyz; }
            case 5u: { return stage_codes(FIXED_ALPHA_5).xyz; }
            case 6u: { return stage_codes(FIXED_ALPHA_6).xyz; }
            default: { return stage_codes(FIXED_ALPHA_7).xyz; }
        }
    }
    return draws[stage_record(index)].stages[stage_place(index)].alpha.xyz;
}
// Whether the plane `plane` of the draw can clip: a key without a plane has none.
fn draw_has_plane(plane: vec4f) -> bool {
    if FIXED && !FIXED_PLANE { return false; }
    return any(plane != vec4f(0.0));
}
// The record's `light_range.w` with the key's facts in their bits, for the extension's
// `draw.light_range.w` (`retained::rewrite_extension_text`).
fn draw_light_range_w() -> u32 {
    let word = draws[record].light_range.w;
    if FIXED {
        return (word & ~LIGHT_FACT_BITS) | select(0u, 4u, FIXED_SCENE) | select(0u, 16u, FIXED_FLAT) | select(0u, 64u, FIXED_ADDS);
    }
    return word;
}
fn draw_view() -> mat4x4f {
    if from_scene() { return scene.view; }
    return draws[record].view;
}
fn draw_projection() -> mat4x4f {
    if from_scene() { return scene.projection; }
    return draws[record].projection;
}
fn draw_pixel() -> vec4f {
    if from_scene() { return scene.pixel; }
    return draws[record].pixel;
}
fn draw_scene_ambient() -> vec4f {
    // A retained draw lights with the ambient of its own instance, scaled as the extension
    // scaled the scene's. The engine sums the ambient lights of the shape's own effect state
    // only when `useAmbientDiffuse` is set (`updateLights` 0x6BB6C0); `setState` (0x6BB200)
    // can turn the lighting off or give a black ambient. The plugin's sum
    // (`ambient_of_state`) skips a light that is disabled or has a dimmer below 0.01. The
    // helper's material applies the two other cases: an emissive-only draw gets a black
    // ambient (`Derived::no_lights`), and a draw with the lighting off takes no light.
    if from_scene() { return draws[record].scene_ambient * scene.ambient_scale; }
    return draws[record].scene_ambient;
}
fn draw_fog_color() -> vec4f {
    if from_scene() && draws[record].fog.z != 2.0 { return scene.fog_color; }
    return draws[record].fog_color;
}
// The fog of the scene applies to a retained draw only when its material has fog (z); the
// fog is by the distance (w) when the scene's is, or the material's (RANGE_SQ).
// A retained shape with a fog property of its own (z 2) keeps its colour and range.
fn draw_fog() -> vec4f {
    if from_scene() {
        let own = draws[record].fog;
        if own.z == 2.0 { return vec4f(own.xy, 1.0, own.w); }
        return vec4f(scene.fog.xy, own.z, max(scene.fog.w, own.w));
    }
    return draws[record].fog;
}
// The colour of a pixel in the fog (`clear` 1 is no fog). A draw that adds to the picture
// (bit 64 of `light_range.w`, `adds_to_picture`) gets darker and takes no fog colour, as
// MGE XE's Direct3D 9 FFE does; any other draw goes toward the fog colour.
fn fogged(color: vec3f, clear: f32) -> vec3f {
    if draw_adds() { return color * clear; }
    return mix(draw_fog_color().rgb, color, clear);
}
// The fog view (VIEW_FOG): the part of the fog that the draw applied, 1 - `clear`, as grey, with
// the draw's own alpha. A draw that adds to the picture writes black, so it adds nothing.
fn fog_view(clear: f32, alpha: f32) -> vec4f {
    if draw_adds() { return vec4f(0.0, 0.0, 0.0, alpha); }
    return vec4f(vec3f(1.0 - clear), alpha);
}
// A draw of the sky in the fog view (SKY_DRAW_BIT, 256 of `light_range.w`): the sky's colour
// with the draw's own alpha. A sky draw that adds (the sun glare) writes black, so it adds nothing.
fn fog_view_sky(alpha: f32) -> vec4f {
    if draw_adds() { return vec4f(0.0, 0.0, 0.0, alpha); }
    return vec4f(FOG_VIEW_SKY, alpha);
}
fn draw_clip_plane() -> vec4f {
    if from_scene() { return scene.clip_plane; }
    return draws[record].clip_plane;
}
// The bits of the extension's shade for the kind of draw (`light_range.z`, the index of
// `slot_kind`), as the extension sets them for a draw of the stream.
fn draw_counts_w() -> u32 {
    var own = draws[record].counts.w;
    if FIXED {
        own = (own & ~SHADE_BITS) | FIXED_SHADE;
    }
    if from_scene() {
        let k = draws[record].light_range.z;
        var kind = scene.counts[k >> 2u][k & 3u];
        if FIXED {
            kind = kind & ~SKY_BITS;
        }
        return own | kind;
    }
    return own;
}
fn draw_extension(i: u32) -> vec4f {
    if from_scene() {
        if i == 5u { return scene.extension5; }
        if i == 7u { return scene.extension7; }
        return vec4f(0.0);
    }
    return draws[record].extension[i];
}
fn light_at(at: u32) -> Light {
    if (draws[record].light_range.w & 8u) != 0u { return lights_retained[light_indexes[at]]; }
    return lights[at];
}
fn light_of(i: u32) -> Light {
    return light_at(draws[record].light_range.x + i);
}

// The lights of the cluster of a pixel: the first entry in `cluster_indexes` and the
// number, for a pixel of the picture at a depth of the view. None when the clusters are
// off or the draw cannot use them (`draws[record].light_range.w`).
fn cluster_lights(pixel: vec2f, view_z: f32) -> vec2u {
    if clusters.grid.w == 0u || (draws[record].light_range.w & 1u) == 0u {
        return vec2u(0u, 0u);
    }
    return cluster_at(pixel, view_z);
}
fn cluster_at(pixel: vec2f, view_z: f32) -> vec2u {
    let tile = vec2u(clamp(vec2i(pixel * clusters.screen.xy), vec2i(0), vec2i(clusters.grid.xy) - 1));
    let slice = u32(clamp(log(max(view_z, clusters.depth.z)) * clusters.depth.x + clusters.depth.y, 0.0, f32(clusters.grid.z) - 1.0));
    return cluster_table[(slice * clusters.grid.y + tile.y) * clusters.grid.x + tile.x];
}

// The lights of the world (`MGE_XE_LIGHTS=world`, `clusters.grid.w` 3): a lit draw of the world
// (bit 32 of `light_range.w`) takes the point lights of the world in the cluster of its pixel
// in place of its own point lights. A retained draw takes them only in a view whose scene says
// so (the main view): a reflection or a shadow map keeps the lights of the draw.
fn world_lights_here() -> bool {
    let bits = draws[record].light_range.w;
    if clusters.grid.w != 3u || (bits & 32u) == 0u {
        return false;
    }
    return (bits & 8u) == 0u || scene.world_lights.x != 0u;
}
// The entries of the cluster of a pixel among the lights of the world.
fn world_cluster(pixel: vec2f, view_z: f32) -> vec2u {
    return cluster_at(pixel, view_z);
}
// A light of the world, an entry of `cluster_indexes`, in the space of the draw: the positions
// are those of the retained set; a draw of the stream is relative to the camera, or in the
// world's own space with bit 128 (the shadow casters' scene).
fn world_light(at: u32) -> Light {
    var light = lights_retained[cluster_indexes[at]];
    let bits = draws[record].light_range.w;
    if (bits & 8u) == 0u {
        let to_draw = select(clusters.offset.xyz, clusters.origin.xyz, (bits & 128u) != 0u);
        light.position = vec4f(light.position.xyz + to_draw, light.position.w);
    }
    return light;
}

// The records of the draws of the frame, and the index of the record of this draw: each
// stage takes it first from the instance of the draw, and reads the fields of the record
// where it needs them (`draws[record].field`). A copy of the whole record into a private
// variable made the GPU three times slower (2026-10-16).
@group(0) @binding(8) var<storage, read> draws: array<Draw>;
var<private> record: u32;
// The two gradients of the first stage's coordinate on the screen, made in `fs` before any
// branch: `particle_texture` reads at the level of the stage's own read with them.
var<private> first_stage_dx: vec2f;
var<private> first_stage_dy: vec2f;
// The textures and the samplers of the frame in binding arrays: a stage names its slots
// in its record (x the texture, y the sampler; the slot 0 is white and plain).
@group(1) @binding(0) var textures: binding_array<texture_2d<f32>>;
@group(1) @binding(1) var samplers: binding_array<sampler>;

struct VertexIn {
    @location(0) position: vec3f,
    @location(1) normal: vec3f,
    // A D3DCOLOR: the bytes are blue, green, red, alpha.
    @location(2) color: vec4f,
    @location(3) uv0: vec2f,
    @location(4) uv1: vec2f,
    @location(5) uv2: vec2f,
    @location(6) uv3: vec2f,
    // The weights of the vertex blend. A vertex has 0 to 3 of them; the shader reads only
    // the ones that the draw has.
    @location(7) weights: vec3f,
    // A skinned draw: the four bones of the vertex, indexes into its part of the palette.
    @location(8) bones: vec4u,
    // The world matrix of the instance, by columns, for a draw with instances.
    @location(9) instance0: vec4f,
    @location(10) instance1: vec4f,
    @location(11) instance2: vec4f,
    @location(12) instance3: vec4f,
    // The record of the draw in `draws`, and for a retained skinned instance the first of its
    // bones in `retained_palette`.
    @location(13) record: vec2u,
}

// The values of a vertex and of a pixel, as the stages of the shader and the extensions
// (`extension_shade`) read them. The shader sends them from the vertex stage to the fragment
// stage packed, in a `StageOut`.
struct VertexOut {
    // The position in the clip space; in the fragment stage, the position of the pixel.
    clip: vec4f,
    color: vec4f,
    // The texture coordinate of each stage, before the divide of a projected one.
    coordinate0: vec4f,
    coordinate1: vec4f,
    coordinate2: vec4f,
    coordinate3: vec4f,
    coordinate4: vec4f,
    coordinate5: vec4f,
    coordinate6: vec4f,
    coordinate7: vec4f,
    fog: f32,
    // For the light of a pixel: the position and the normal in the space of the lights, and
    // the colour of the vertex.
    world_position: vec3f,
    world_normal: vec3f,
    vertex_color: vec4f,
    record: u32,
    // Flat shading (bit 16 of `light_range.w`, a material with NiShadeProperty off): the
    // lit colour and the normal of the first vertex of the triangle, as Direct3D 8 drew it
    // (D3DSHADE_FLAT). The same values as `color` and `world_normal`, not blended.
    color_flat: vec4f,
    world_normal_flat: vec3f,
    // A particle lit at its corners (CORNER_LIGHT_BIT): the light its corners take, for the
    // lights view; 0 for any other draw.
    corner_light: vec3f,
}

// A `VertexOut` between the stages. Two pairs of values share a location, so that the
// coordinates of all the stages fit in the locations of the downlevel profile (15):
// - the fog depth is the w of the world position (both interpolated with the perspective);
// - the record is the w of the flat normal (both flat, of the first vertex). It is a float:
//   a float holds each whole number below 2^24 exactly, and the records of one binding are
//   fewer (`RECORD_SIZE` in data.rs). A record cast bit for bit would be a subnormal float,
//   which a driver can make 0.
struct StageOut {
    @builtin(position) clip: vec4f,
    @location(0) color: vec4f,
    @location(1) coordinate0: vec4f,
    @location(2) coordinate1: vec4f,
    @location(3) coordinate2: vec4f,
    @location(4) coordinate3: vec4f,
    @location(5) coordinate4: vec4f,
    @location(6) coordinate5: vec4f,
    @location(7) world_position_fog: vec4f,
    @location(8) world_normal: vec3f,
    @location(9) vertex_color: vec4f,
    @location(10) @interpolate(flat, first) color_flat: vec4f,
    @location(11) @interpolate(flat, first) world_normal_flat_record: vec4f,
    @location(12) corner_light: vec3f,
    @location(13) coordinate6: vec4f,
    @location(14) coordinate7: vec4f,
}

fn stage_out(v: VertexOut) -> StageOut {
    var s: StageOut;
    s.clip = v.clip;
    s.color = v.color;
    s.coordinate0 = v.coordinate0;
    s.coordinate1 = v.coordinate1;
    s.coordinate2 = v.coordinate2;
    s.coordinate3 = v.coordinate3;
    s.coordinate4 = v.coordinate4;
    s.coordinate5 = v.coordinate5;
    s.coordinate6 = v.coordinate6;
    s.coordinate7 = v.coordinate7;
    s.world_position_fog = vec4f(v.world_position, v.fog);
    s.world_normal = v.world_normal;
    s.vertex_color = v.vertex_color;
    s.color_flat = v.color_flat;
    s.world_normal_flat_record = vec4f(v.world_normal_flat, f32(v.record));
    s.corner_light = v.corner_light;
    return s;
}

fn vertex_out(s: StageOut) -> VertexOut {
    var v: VertexOut;
    v.clip = s.clip;
    v.color = s.color;
    v.coordinate0 = s.coordinate0;
    v.coordinate1 = s.coordinate1;
    v.coordinate2 = s.coordinate2;
    v.coordinate3 = s.coordinate3;
    v.coordinate4 = s.coordinate4;
    v.coordinate5 = s.coordinate5;
    v.coordinate6 = s.coordinate6;
    v.coordinate7 = s.coordinate7;
    v.fog = s.world_position_fog.w;
    v.world_position = s.world_position_fog.xyz;
    v.world_normal = s.world_normal;
    v.vertex_color = s.vertex_color;
    v.record = u32(s.world_normal_flat_record.w);
    v.color_flat = s.color_flat;
    v.world_normal_flat = s.world_normal_flat_record.xyz;
    v.corner_light = s.corner_light;
    return v;
}

// The colour of a particle's corner after its lights (`extension_corner_shade` of the
// extension), and the light that the corner takes.
struct CornerShade {
    color: vec4f,
    light: vec3f,
}

// A pixel of the clouds for `extension_clouds_color` (CLOUDS_HOOK_BIT, 1024): the coordinate
// of the first texture stage, the colour of the vertex, the direction from the camera (length
// 1, in the space of the world) and the screen pixel.
struct CloudPixel {
    texcoords: vec2f,
    color: vec4f,
    direction: vec3f,
    pixel: vec2f,
}

// A pixel of a particle for `extension_particle_color` (PARTICLE_HOOK_BIT, 2048): the colour
// after the texture stages, the particle's own colour and alpha from the game, the coordinate
// of the first texture stage, the screen pixel, and the depth along the view in units of the
// world.
struct ParticlePixel {
    color: vec4f,
    vertex_color: vec4f,
    texcoords: vec2f,
    pixel: vec2f,
    distance: f32,
}

// The texture coordinate of a stage, as the fixed-function pipeline makes it.
fn stage_coordinate(index: u32, uvs: array<vec2f, 4>, view_position: vec3f, view_normal: vec3f, world_position: vec3f, world_normal: vec3f) -> vec4f {
    let holder = stage_record(index);
    let place = stage_place(index);
    let how = draws[holder].stages[place].coordinate;
    var sets = uvs;
    var value: vec4f;
    switch how.x {
        case 1u: { value = vec4f(view_normal, 1.0); }
        case 2u: { value = vec4f(view_position, 1.0); }
        case 3u: {
            let from_eye = normalize(view_position);
            value = vec4f(from_eye - 2.0 * dot(view_normal, from_eye) * view_normal, 1.0);
        }
        // The same vectors in the world, for the texture effects of the retained table: the
        // engine's callbacks turn the vector of camera space into one of the world before
        // the effect's matrix, and the table keeps only the matrix. The view matrix has no
        // scale: its transpose turns a vector of the view into one of the world. The position
        // is relative to the origin of the table, and so is the translation of its entry.
        case 5u: { value = vec4f(normalize(world_normal), 1.0); }
        case 6u: { value = vec4f(world_position, 1.0); }
        case 7u: {
            let from_eye = normalize(view_position);
            let reflected = from_eye - 2.0 * dot(view_normal, from_eye) * view_normal;
            let v = draw_view();
            value = vec4f(transpose(mat3x3f(v[0].xyz, v[1].xyz, v[2].xyz)) * reflected, 1.0);
        }
        // A coordinate set with two parts: Direct3D puts 1 into the third part, so that the
        // third row of a texture transform moves the coordinate.
        default: { value = vec4f(sets[how.y & 3u], 1.0, 0.0); }
    }
    if how.z != 0u {
        let entry = draws[holder].stages[place].slots.z;
        if entry != 0u {
            value = effect_entries[entry - 1u] * value;
        } else {
            value = draws[holder].texture_transforms[place] * value;
        }
    }
    return value;
}

// A bone of a skinned draw: of the table's palette for a retained draw, else of the frame's.
fn skin_bone(index: u32) -> mat4x4f {
    if from_scene() {
        return retained_palette[index];
    }
    return palette[index];
}

@vertex
fn vs(in: VertexIn) -> StageOut {
    record = in.record.x;
    var local = vec4f(in.position, 1.0);
    if (draw_counts_w() & 4u) != 0u {
        local = vec4f(extension_sky_position(in.position), 1.0);
    }
    var world_position: vec4f;
    var world_normal: vec3f;
    let weights = draws[record].flags.w;
    if weights == 0u {
        var world0 = draws[record].world[0];
        if (draws[record].light_range.w & 2u) != 0u {
            world0 = mat4x4f(in.instance0, in.instance1, in.instance2, in.instance3);
        }
        world_position = world0 * local;
        world_normal = (world0 * vec4f(in.normal, 0.0)).xyz;
    } else if weights == 4u && !from_scene() {
        // A skinned draw: four bones of the palette of the frame, the fourth weight 1 less
        // the sum of the three, as the vertex blend of Direct3D.
        let first = draws[record].light_range.z;
        var last = 1.0;
        world_position = vec4f(0.0);
        world_normal = vec3f(0.0);
        for (var i = 0u; i < 3u; i++) {
            let weight = in.weights[i];
            last -= weight;
            let bone = palette[first + in.bones[i]];
            world_position += weight * (bone * local);
            world_normal += weight * (bone * vec4f(in.normal, 0.0)).xyz;
        }
        let bone = palette[first + in.bones[3]];
        world_position += last * (bone * local);
        world_normal += last * (bone * vec4f(in.normal, 0.0)).xyz;
    } else if weights == 5u {
        // A shape with a list: a shape that the game skins in software, or one with a vertex of
        // more than 4 influences. Each influence of the vertex with its weight, as
        // `NiSkinInstance::Deform` sums them. The first two weights of the vertex are its first
        // entry and its count. A normal takes the rotation of a bone without its scale, as
        // there. For a shape in software, the plugin's last bone has no rotation: it only gives
        // the place of the shape to the part of the weights that the sum lacks of 1.
        let first = select(draws[record].light_range.z, in.record.y, from_scene());
        let at = u32(in.weights.x);
        world_position = vec4f(0.0);
        world_normal = vec3f(0.0);
        for (var i = at; i < at + u32(in.weights.y); i++) {
            let bone = skin_bone(first + influences[i].bone);
            let weight = influences[i].weight;
            world_position += weight * (bone * local);
            let scale = length(bone[0].xyz);
            if scale > 0.0 {
                world_normal += weight * (bone * vec4f(in.normal, 0.0)).xyz / scale;
            }
        }
    } else if weights == 4u {
        // A retained skinned draw: the same blend with the bones of its instance in the
        // table's palette, from the first bone its instance carries.
        let first = in.record.y;
        var last = 1.0;
        world_position = vec4f(0.0);
        world_normal = vec3f(0.0);
        for (var i = 0u; i < 3u; i++) {
            let weight = in.weights[i];
            last -= weight;
            let bone = retained_palette[first + in.bones[i]];
            world_position += weight * (bone * local);
            world_normal += weight * (bone * vec4f(in.normal, 0.0)).xyz;
        }
        let bone = retained_palette[first + in.bones[3]];
        world_position += last * (bone * local);
        world_normal += last * (bone * vec4f(in.normal, 0.0)).xyz;
    } else {
        // The vertex blend of Direct3D: the last weight is 1 less the sum of the others.
        var last = 1.0;
        world_position = vec4f(0.0);
        world_normal = vec3f(0.0);
        for (var i = 0u; i < weights; i++) {
            let weight = in.weights[i];
            last -= weight;
            world_position += weight * (draws[record].world[i] * local);
            world_normal += weight * (draws[record].world[i] * vec4f(in.normal, 0.0)).xyz;
        }
        world_position += last * (draws[record].world[weights] * local);
        world_normal += last * (draws[record].world[weights] * vec4f(in.normal, 0.0)).xyz;
    }
    let view_position = draw_view() * vec4f(world_position.xyz, 1.0);

    var out: VertexOut;
    out.record = in.record.x;
    out.clip = draw_projection() * view_position;
    // Direct3D 8 has the centre of a pixel at a whole number: the picture is half a pixel to
    // the right and down from where newer APIs put it.
    out.clip = vec4f(out.clip.xy + draw_pixel().xy * out.clip.w, out.clip.zw);
    let uvs = array<vec2f, 4>(in.uv0, in.uv1, in.uv2, in.uv3);
    let view_normal = normalize((draw_view() * vec4f(world_normal, 0.0)).xyz + vec3f(0.0, 0.0, 1e-20));
    out.coordinate0 = stage_coordinate(0u, uvs, view_position.xyz, view_normal, world_position.xyz, world_normal);
    out.coordinate1 = stage_coordinate(1u, uvs, view_position.xyz, view_normal, world_position.xyz, world_normal);
    out.coordinate2 = stage_coordinate(2u, uvs, view_position.xyz, view_normal, world_position.xyz, world_normal);
    out.coordinate3 = stage_coordinate(3u, uvs, view_position.xyz, view_normal, world_position.xyz, world_normal);
    out.coordinate4 = stage_coordinate(4u, uvs, view_position.xyz, view_normal, world_position.xyz, world_normal);
    out.coordinate5 = stage_coordinate(5u, uvs, view_position.xyz, view_normal, world_position.xyz, world_normal);
    out.coordinate6 = stage_coordinate(6u, uvs, view_position.xyz, view_normal, world_position.xyz, world_normal);
    out.coordinate7 = stage_coordinate(7u, uvs, view_position.xyz, view_normal, world_position.xyz, world_normal);

    var vertex_color = vec4f(1.0);
    if draws[record].flags.z != 0u {
        vertex_color = in.color.bgra;
    }
    out.world_position = world_position.xyz;
    out.world_normal = world_normal;
    out.vertex_color = vertex_color;
    out.world_normal_flat = world_normal;
    if (draw_counts_w() & 1036u) != 0u {
        // For the sky (the dome 4, the moon's shadow 8, the clouds with the extension's colour
        // 1024): the direction from the camera, in the space of the world.
        let v = draw_view();
        out.world_position = transpose(mat3x3f(v[0].xyz, v[1].xyz, v[2].xyz)) * view_position.xyz;
    }
    if (draw_counts_w() & 4u) != 0u {
        // The dome is behind all other things: just in front of the far plane (0 when the
        // depth is reversed).
        out.clip.z = select(0.999999, 0.000001, DEPTH_REVERSED) * out.clip.w;
    }
    // With bit 128 the extension makes the light for each pixel, and this stage makes none.
    // A draw that the extension only finishes (bit 1 alone: its fog, its shadow) keeps the
    // light of this stage.
    if draw_lit() == 0u || (draw_counts_w() & 128u) != 0u {
        out.color = vertex_color;
    } else {
        var diffuse = draws[record].material_diffuse;
        if draws[record].more.x == 1u {
            diffuse = vertex_color;
        }
        var emissive = draws[record].material_emissive.rgb;
        if draws[record].more.y == 1u {
            emissive = vertex_color.rgb;
        }
        var ambient = draws[record].material_ambient.rgb;
        if draws[record].counts.z == 1u {
            ambient = vertex_color.rgb;
        }
        // A lit draw without normals: the fixed function takes a zero normal, so the lights
        // give it their ambient light and no diffuse light.
        var normal = vec3f(0.0);
        if draw_normal() != 0u {
            normal = normalize(world_normal);
        }
        var sum = emissive + ambient * draw_scene_ambient().rgb;
        for (var i = 0u; i < draws[record].light_range.y; i++) {
            let light = light_of(i);
            var direction: vec3f;
            var strength = 1.0;
            if light.position.w == 3.0 {
                direction = -normalize(light.direction.xyz);
            } else {
                let to_light = light.position.xyz - world_position.xyz;
                let distance = length(to_light);
                direction = to_light / max(distance, 0.0001);
                if distance > light.direction.w {
                    strength = 0.0;
                } else {
                    let a = light.attenuation;
                    strength = 1.0 / max(a.x + a.y * distance + a.z * distance * distance, 0.0001);
                }
                // The cone of a spot light (`spot_cone` of spot.rs).
                strength *= spot_cone(light, direction);
            }
            sum += strength * (ambient * light.ambient.rgb + diffuse.rgb * light.diffuse.rgb * max(dot(normal, direction), 0.0));
        }
        out.color = vec4f(clamp(sum, vec3f(0.0), vec3f(1.0)), diffuse.a);
    }
    // A particle lit at its corners (CORNER_LIGHT_BIT, set only with the light for each
    // pixel): the extension lights the corner here, and the fragment stage takes the colour.
    out.corner_light = vec3f(0.0);
    if (draw_counts_w() & 16384u) != 0u {
        let corner = extension_corner_shade(world_position.xyz, world_normal, vertex_color, out.color);
        out.color = corner.color;
        out.corner_light = corner.light;
    }
    // A draw lit for each pixel with no lights (NO_LIGHTS_BIT) that is not flat-shaded: its
    // light has no part that changes over a triangle, so the extension makes its colour here
    // (`extension_no_lights_shade`), and the fragment stage takes it. Both bits are facts of
    // the stage key. A flat-shaded one is lit in the fragment stage.
    if (draw_counts_w() & 32768u) != 0u && !draw_flat() {
        out.color = extension_no_lights_shade(vertex_color, out.color);
    }

    out.color_flat = out.color;
    // The fog of the game is table fog: linear on the depth, for each pixel. The fragment
    // shader makes the fog from this depth, so that a large triangle is right.
    out.fog = abs(view_position.z);
    if draw_fog().w != 0.0 {
        out.fog = length(view_position.xyz);
    }
    return stage_out(out);
}

// An argument of a stage: D3DTA_*.
fn argument(code: u32, diffuse: vec4f, current: vec4f, texel: vec4f) -> vec4f {
    var value: vec4f;
    switch code & 15u {
        case 0u: { value = diffuse; }
        case 1u: { value = current; }
        case 2u: { value = texel; }
        case 3u: { value = draws[record].texture_factor; }
        default: { value = vec4f(1.0); }
    }
    if (code & 16u) != 0u {
        value = vec4f(1.0) - value;
    }
    if (code & 32u) != 0u {
        value = value.aaaa;
    }
    return value;
}

// An operation of a stage: D3DTOP_*.
fn operation(op: u32, a1: vec4f, a2: vec4f, a0: vec4f, diffuse: vec4f, current: vec4f, texel: vec4f) -> vec4f {
    switch op {
        case 2u: { return a1; }
        case 3u: { return a2; }
        case 4u: { return a1 * a2; }
        case 5u: { return a1 * a2 * 2.0; }
        case 6u: { return a1 * a2 * 4.0; }
        case 7u: { return a1 + a2; }
        case 8u: { return a1 + a2 - 0.5; }
        case 9u: { return (a1 + a2 - 0.5) * 2.0; }
        case 10u: { return a1 - a2; }
        case 11u: { return a1 + a2 - a1 * a2; }
        case 12u: { return mix(a2, a1, diffuse.a); }
        case 13u: { return mix(a2, a1, texel.a); }
        case 14u: { return mix(a2, a1, draws[record].texture_factor.a); }
        case 16u: { return mix(a2, a1, current.a); }
        // A bump stage gives what the stage before it gave.
        case 22u, 23u: { return current; }
        case 24u: {
            let d = dot(a1.rgb - 0.5, a2.rgb - 0.5) * 4.0;
            return vec4f(d);
        }
        case 25u: { return a0 + a1 * a2; }
        case 26u: { return mix(a2, a1, a0); }
        default: { return a1; }
    }
}

fn compare(function: u32, value: f32, reference: f32) -> bool {
    switch function {
        case 1u: { return false; }
        case 2u: { return value < reference; }
        case 3u: { return value == reference; }
        case 4u: { return value <= reference; }
        case 5u: { return value > reference; }
        case 6u: { return value != reference; }
        case 7u: { return value >= reference; }
        default: { return true; }
    }
}

// The two parts of a texture coordinate that the texture read uses. A projected coordinate
// is divided by the last part that its texture transform gives.
fn final_coordinate(index: u32, value: vec4f) -> vec2f {
    let how = draws[stage_record(index)].stages[stage_place(index)].coordinate;
    var divisor = 1.0;
    if how.w != 0u && how.z >= 2u {
        divisor = value[min(how.z, 4u) - 1u];
    }
    return value.xy / divisor;
}

// The first texture of the draw with its sampler, at level 0: for `extension_clouds_color`,
// which may read it in a branch and at moved coordinates.
fn cloud_texture(uv: vec2f) -> vec4f {
    return textureSampleLevel(textures[draws[record].stages[0].slots.x], samplers[draws[record].stages[0].slots.y], uv, 0.0);
}

// The first texture of the draw with its sampler, at the level of the first stage's own read:
// for `extension_particle_color`, which may read it in a branch and at moved coordinates.
fn particle_texture(uv: vec2f) -> vec4f {
    return textureSampleGrad(textures[draws[record].stages[0].slots.x], samplers[draws[record].stages[0].slots.y], uv, first_stage_dx, first_stage_dy);
}

// The particle's draw adds to the picture: its fade goes to black, not to alpha 0.
fn particle_adds() -> bool {
    return draw_adds();
}

// D3DTOP_BUMPENVMAP is 22 and D3DTOP_BUMPENVMAPLUMINANCE is 23. The move that the bump stage
// `index` gives to the coordinate of the next stage, from its texel.
fn bump_move(index: u32, texel: vec4f) -> vec2f {
    let op = stage_color(index).x;
    if index >= stage_count() || (op != 22u && op != 23u) {
        return vec2f(0.0);
    }
    let stage = draws[stage_record(index)].stages[stage_place(index)];
    return vec2f(texel.r * stage.bump.x + texel.g * stage.bump.z, texel.r * stage.bump.y + texel.g * stage.bump.w);
}

// With luminance (23), a bump stage also scales the colour of the texel of the next stage: the
// blue channel of the bump map, by the scale and the offset of the material, kept between 0
// and 1, as `bumpmapLumiStage` of MGE XE's `XE FixedFuncEmu.fx`. The running engine looks the
// same in the case seen: a DXT5 map on the water of the test level. In a format without the
// channel, the blue channel reads 1 (`d3d::gpu_pixels`), so the scale and the offset still apply.
fn bump_light(index: u32, bump_texel: vec4f, texel: vec4f) -> vec4f {
    if index >= stage_count() || stage_color(index).x != 23u {
        return texel;
    }
    let stage = draws[stage_record(index)].stages[stage_place(index)];
    let light =clamp(bump_texel.b * stage.luminance.x + stage.luminance.y, 0.0, 1.0);
    return vec4f(texel.rgb * light, texel.a);
}

// The texel of stage `index` (1 to 7) for a stage pipeline: white at and past the key's count,
// where no stage reads it, so that the compiler drops the read. The general shader keeps the
// read.
fn stage_texel(index: u32, texel: vec4f) -> vec4f {
    if FIXED && index >= FIXED_COUNT {
        return vec4f(1.0);
    }
    return texel;
}

// One texture stage: its colour and alpha operations (`color`, `alpha`: the codes of
// `stage_color` and `stage_alpha`) on its texel, the diffuse colour and the current colour; the
// result is the new current colour. Both paths of `fs` call it: the loop of the general shader
// and the written steps of a stage pipeline.
fn stage_step(color: vec4u, alpha: vec3u, texel: vec4f, diffuse: vec4f, current: vec4f) -> vec4f {
    let rgb = operation(color.x, argument(color.y, diffuse, current, texel),
        argument(color.z, diffuse, current, texel), argument(color.w, diffuse, current, texel), diffuse, current, texel);
    var a = current;
    // A bump stage (22 and 23) moves the coordinate of the next stage, and its output is the
    // current colour and alpha. The engine's table has SELECTARG1 for its alpha, and plain
    // DXVK follows the table. The per-pixel path of MGE XE keeps the shape's alpha
    // (`ffeshader.cpp`), and so does the running engine on Direct3D 8 in the cases seen: the
    // particles of the test level and the quads of a mod. Not read in a driver.
    if alpha.x != 1u && color.x != 22u && color.x != 23u {
        a = operation(alpha.x, argument(alpha.y, diffuse, current, texel),
            argument(alpha.z, diffuse, current, texel), vec4f(0.0), diffuse, current, texel);
    }
    var next = vec4f(rgb.rgb, a.a);
    // The fixed-function pipeline keeps a colour between 0 and 1. A draw with the light
    // for each pixel has no such limit.
    if (draw_counts_w() & 128u) == 0u {
        next = clamp(next, vec4f(0.0), vec4f(1.0));
    }
    return next;
}

@fragment
fn fs(stage_in: StageOut) -> @location(0) vec4f {
    let in = vertex_out(stage_in);
    record = in.record;
    var inp = in;
    if draw_flat() {
        inp.color = inp.color_flat;
        inp.world_normal = inp.world_normal_flat;
    }
    // A texture read must not be in a branch, so each stage reads its texture first. The
    // gradients of the first stage's coordinate are made here for the same reason.
    first_stage_dx = dpdx(final_coordinate(0u, inp.coordinate0));
    first_stage_dy = dpdy(final_coordinate(0u, inp.coordinate0));
    var texels: array<vec4f, 8>;
    // A bump stage moves the coordinate of the stage after it.
    texels[0] = textureSample(textures[draws[record].stages[0].slots.x], samplers[draws[record].stages[0].slots.y], final_coordinate(0u, inp.coordinate0));
    texels[1] = stage_texel(1u, bump_light(0u, texels[0], textureSample(textures[draws[record].stages[1].slots.x], samplers[draws[record].stages[1].slots.y], final_coordinate(1u, inp.coordinate1) + bump_move(0u, texels[0]))));
    texels[2] = stage_texel(2u, bump_light(1u, texels[1], textureSample(textures[draws[record].stages[2].slots.x], samplers[draws[record].stages[2].slots.y], final_coordinate(2u, inp.coordinate2) + bump_move(1u, texels[1]))));
    texels[3] = stage_texel(3u, bump_light(2u, texels[2], textureSample(textures[draws[record].stages[3].slots.x], samplers[draws[record].stages[3].slots.y], final_coordinate(3u, inp.coordinate3) + bump_move(2u, texels[2]))));
    texels[4] = stage_texel(4u, bump_light(3u, texels[3], textureSample(textures[draws[record].stages[4].slots.x], samplers[draws[record].stages[4].slots.y], final_coordinate(4u, inp.coordinate4) + bump_move(3u, texels[3]))));
    texels[5] = stage_texel(5u, bump_light(4u, texels[4], textureSample(textures[draws[record].stages[5].slots.x], samplers[draws[record].stages[5].slots.y], final_coordinate(5u, inp.coordinate5) + bump_move(4u, texels[4]))));
    // Stages 6 and 7 are the first two of the overflow record. A draw without one reads its own
    // stage 0 there, and no stage uses that texel.
    texels[6] = stage_texel(6u, bump_light(5u, texels[5], textureSample(textures[draws[stage_record(6u)].stages[0].slots.x], samplers[draws[stage_record(6u)].stages[0].slots.y], final_coordinate(6u, inp.coordinate6) + bump_move(5u, texels[5]))));
    texels[7] = stage_texel(7u, bump_light(6u, texels[6], textureSample(textures[draws[stage_record(7u)].stages[1].slots.x], samplers[draws[stage_record(7u)].stages[1].slots.y], final_coordinate(7u, inp.coordinate7) + bump_move(6u, texels[6]))));

    if (draw_counts_w() & 4u) != 0u {
        if DEBUG_VIEW == VIEW_FOG {
            return vec4f(FOG_VIEW_SKY, 1.0);
        }
        return vec4f(extension_sky_color(normalize(inp.world_position), inp.clip.xy), 1.0);
    }
    if (draw_counts_w() & 1024u) != 0u {
        // The clouds with the extension's colour (CLOUDS_HOOK_BIT): its colour and alpha are
        // final, as a pixel shader's on the fixed-function state; the alpha test of the draw
        // still acts on them.
        let cloud = extension_clouds_color(CloudPixel(final_coordinate(0u, inp.coordinate0), inp.vertex_color, normalize(inp.world_position), inp.clip.xy));
        if draw_alpha_test() != 0u && !compare(draw_alpha_test(), floor(cloud.a * 255.0 + 0.5), f32(draws[record].more.w)) {
            discard;
        }
        if DEBUG_VIEW == VIEW_FOG {
            return fog_view_sky(cloud.a);
        }
        return cloud;
    }
    // A sky draw has a direction in `world_position`, and is above the water in any case.
    let clip_plane = draw_clip_plane();
    if (draw_counts_w() & 12u) == 0u && draw_has_plane(clip_plane) && dot(inp.world_position, clip_plane.xyz) + clip_plane.w < 0.0 {
        discard;
    }
    let shaded = (draw_counts_w() & 1u) != 0u;
    var diffuse = inp.color;
    if shaded {
        diffuse = extension_shade(inp);
    }
    var current = diffuse;
    if FIXED {
        // A stage pipeline: one step for each stage of its key, with the key's codes (W530 step 2).
        // The general shader keeps its loop: written steps there were slower (W279).
        if FIXED_COUNT > 0u { current = stage_step(stage_color(0u), stage_alpha(0u), texels[0], diffuse, current); }
        if FIXED_COUNT > 1u { current = stage_step(stage_color(1u), stage_alpha(1u), texels[1], diffuse, current); }
        if FIXED_COUNT > 2u { current = stage_step(stage_color(2u), stage_alpha(2u), texels[2], diffuse, current); }
        if FIXED_COUNT > 3u { current = stage_step(stage_color(3u), stage_alpha(3u), texels[3], diffuse, current); }
        if FIXED_COUNT > 4u { current = stage_step(stage_color(4u), stage_alpha(4u), texels[4], diffuse, current); }
        if FIXED_COUNT > 5u { current = stage_step(stage_color(5u), stage_alpha(5u), texels[5], diffuse, current); }
        if FIXED_COUNT > 6u { current = stage_step(stage_color(6u), stage_alpha(6u), texels[6], diffuse, current); }
        if FIXED_COUNT > 7u { current = stage_step(stage_color(7u), stage_alpha(7u), texels[7], diffuse, current); }
    } else {
        for (var i = 0u; i < stage_count(); i++) {
            current = stage_step(stage_color(i), stage_alpha(i), texels[i], diffuse, current);
        }
    }

    if (draw_counts_w() & 2048u) != 0u {
        // A particle with the extension's colour (PARTICLE_HOOK_BIT): the function takes the
        // result of the stages, and its result takes the stages' limit under their condition.
        let distance = abs((draw_view() * vec4f(inp.world_position, 1.0)).z);
        current = extension_particle_color(ParticlePixel(current, inp.vertex_color, final_coordinate(0u, inp.coordinate0), inp.clip.xy, distance));
        if stage_count() != 0u && (draw_counts_w() & 128u) == 0u {
            current = clamp(current, vec4f(0.0), vec4f(1.0));
        }
    }
    if draw_alpha_test() != 0u && !compare(draw_alpha_test(), floor(current.a * 255.0 + 0.5), f32(draws[record].more.w)) {
        discard;
    }
    var clear = 1.0;
    let fog = draw_fog();
    if fog.z != 0.0 {
        clear = clamp((fog.y - inp.fog) / max(fog.y - fog.x, 0.0001), 0.0, 1.0);
    }
    if DEBUG_VIEW == VIEW_FOG {
        // A draw of the sky (SKY_DRAW_BIT) has the sky's colour. A draw with a stencil test
        // that can fail (STENCIL_TEST_BIT, 512: the dark overlay of the stencil shadows) only
        // darkens what is under it: it writes nothing, so the fog of that pixel stays.
        if (draw_light_range_w() & 256u) != 0u {
            return fog_view_sky(current.a);
        }
        if (draw_light_range_w() & 512u) != 0u {
            discard;
        }
        return fog_view(clear, current.a);
    }
    if shaded {
        return extension_finish(current.rgb, current.a, clear);
    }
    if (draw_counts_w() & 8u) != 0u {
        return vec4f(extension_sky_color(normalize(inp.world_position), inp.clip.xy), current.a);
    }
    return vec4f(fogged(current.rgb, clear), current.a);
}
