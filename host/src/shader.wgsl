// The work of the fixed-function pipeline of Direct3D 8 for one draw of the game: transform,
// lighting for each vertex, the texture stages, the alpha test and the fog. One shader does
// all draws: each choice of a draw is a value in `Draw`.
//
// A matrix has the 16 floats of Direct3D in their order. Direct3D multiplies a row vector
// with the matrix. Here the same floats are the columns, so `matrix * vector` is that product.

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
    // and the colour is not kept between 0 and 1 between the stages; bit 2 and the
    // bits from 16 (but 128) are for the extension.
    counts: vec4u,
    // The lights of the draw in `lights`: the first and the number.
    light_range: vec4u,
    // The lights of the draw among the clustered lights of the frame, a bit for each slot:
    // an entry of `cluster_indexes` is the slot in its high half and the index in `lights`
    // in its low half, and a pixel takes the light only when the draw has its bit.
    light_mask: array<vec4u, 2>,
    stages: array<Stage, 6>,
    // The texture transform of each stage.
    texture_transforms: array<mat4x4f, 6>,
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
}
@group(0) @binding(3) var<uniform> clusters: Clusters;
@group(0) @binding(4) var<storage, read> cluster_table: array<vec2u>;
@group(0) @binding(5) var<storage, read> cluster_indexes: array<u32>;
// The lights of all draws of the frame.
@group(0) @binding(6) var<storage, read> lights: array<Light>;
// The bones of all skinned draws of the frame.
@group(0) @binding(7) var<storage, read> palette: array<mat4x4f>;

// The lights of the cluster of a pixel: the first entry in `cluster_indexes` and the
// number, for a pixel of the picture at a depth of the view. None when the clusters are
// off or the draw cannot use them (`draw.light_range.w`).
fn cluster_lights(pixel: vec2f, view_z: f32) -> vec2u {
    if clusters.grid.w == 0u || draw.light_range.w == 0u {
        return vec2u(0u, 0u);
    }
    let tile = vec2u(clamp(vec2i(pixel * clusters.screen.xy), vec2i(0), vec2i(clusters.grid.xy) - 1));
    let slice = u32(clamp(log(max(view_z, clusters.depth.z)) * clusters.depth.x + clusters.depth.y, 0.0, f32(clusters.grid.z) - 1.0));
    return cluster_table[(slice * clusters.grid.y + tile.y) * clusters.grid.x + tile.x];
}

@group(3) @binding(0) var<uniform> draw: Draw;
@group(2) @binding(0) var texture0: texture_2d<f32>;
@group(2) @binding(1) var texture1: texture_2d<f32>;
@group(2) @binding(2) var texture2: texture_2d<f32>;
@group(2) @binding(3) var texture3: texture_2d<f32>;
@group(2) @binding(4) var texture4: texture_2d<f32>;
@group(2) @binding(5) var texture5: texture_2d<f32>;
@group(1) @binding(0) var sampler0: sampler;
@group(1) @binding(1) var sampler1: sampler;
@group(1) @binding(2) var sampler2: sampler;
@group(1) @binding(3) var sampler3: sampler;
@group(1) @binding(4) var sampler4: sampler;
@group(1) @binding(5) var sampler5: sampler;

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
}

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) color: vec4f,
    // The texture coordinate of each stage, before the divide of a projected one.
    @location(1) coordinate0: vec4f,
    @location(2) coordinate1: vec4f,
    @location(3) coordinate2: vec4f,
    @location(4) coordinate3: vec4f,
    @location(5) coordinate4: vec4f,
    @location(6) coordinate5: vec4f,
    @location(7) fog: f32,
    // For the light of a pixel: the position and the normal in the space of the lights, and
    // the colour of the vertex.
    @location(8) world_position: vec3f,
    @location(9) world_normal: vec3f,
    @location(10) vertex_color: vec4f,
}

// The texture coordinate of a stage, as the fixed-function pipeline makes it.
fn stage_coordinate(index: u32, uvs: array<vec2f, 4>, view_position: vec3f, view_normal: vec3f) -> vec4f {
    let how = draw.stages[index].coordinate;
    var sets = uvs;
    var value: vec4f;
    switch how.x {
        case 1u: { value = vec4f(view_normal, 1.0); }
        case 2u: { value = vec4f(view_position, 1.0); }
        case 3u: {
            let from_eye = normalize(view_position);
            value = vec4f(from_eye - 2.0 * dot(view_normal, from_eye) * view_normal, 1.0);
        }
        // A coordinate set with two parts: Direct3D puts 1 into the third part, so that the
        // third row of a texture transform moves the coordinate.
        default: { value = vec4f(sets[how.y & 3u], 1.0, 0.0); }
    }
    if how.z != 0u {
        value = draw.texture_transforms[index] * value;
    }
    return value;
}

@vertex
fn vs(in: VertexIn) -> VertexOut {
    var local = vec4f(in.position, 1.0);
    if (draw.counts.w & 4u) != 0u {
        local = vec4f(extension_sky_position(in.position), 1.0);
    }
    var world_position: vec4f;
    var world_normal: vec3f;
    let weights = draw.flags.w;
    if weights == 0u {
        var world0 = draw.world[0];
        if (draw.light_range.w & 2u) != 0u {
            world0 = mat4x4f(in.instance0, in.instance1, in.instance2, in.instance3);
        }
        world_position = world0 * local;
        world_normal = (world0 * vec4f(in.normal, 0.0)).xyz;
    } else if weights == 4u {
        // A skinned draw: four bones of the palette of the frame, the fourth weight 1 less
        // the sum of the three, as the vertex blend of Direct3D.
        let first = draw.light_range.z;
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
    } else {
        // The vertex blend of Direct3D: the last weight is 1 less the sum of the others.
        var last = 1.0;
        world_position = vec4f(0.0);
        world_normal = vec3f(0.0);
        for (var i = 0u; i < weights; i++) {
            let weight = in.weights[i];
            last -= weight;
            world_position += weight * (draw.world[i] * local);
            world_normal += weight * (draw.world[i] * vec4f(in.normal, 0.0)).xyz;
        }
        world_position += last * (draw.world[weights] * local);
        world_normal += last * (draw.world[weights] * vec4f(in.normal, 0.0)).xyz;
    }
    let view_position = draw.view * vec4f(world_position.xyz, 1.0);

    var out: VertexOut;
    out.clip = draw.projection * view_position;
    // Direct3D 8 has the centre of a pixel at a whole number: the picture is half a pixel to
    // the right and down from where newer APIs put it.
    out.clip = vec4f(out.clip.xy + draw.pixel.xy * out.clip.w, out.clip.zw);
    let uvs = array<vec2f, 4>(in.uv0, in.uv1, in.uv2, in.uv3);
    let view_normal = normalize((draw.view * vec4f(world_normal, 0.0)).xyz + vec3f(0.0, 0.0, 1e-20));
    out.coordinate0 = stage_coordinate(0u, uvs, view_position.xyz, view_normal);
    out.coordinate1 = stage_coordinate(1u, uvs, view_position.xyz, view_normal);
    out.coordinate2 = stage_coordinate(2u, uvs, view_position.xyz, view_normal);
    out.coordinate3 = stage_coordinate(3u, uvs, view_position.xyz, view_normal);
    out.coordinate4 = stage_coordinate(4u, uvs, view_position.xyz, view_normal);
    out.coordinate5 = stage_coordinate(5u, uvs, view_position.xyz, view_normal);

    var vertex_color = vec4f(1.0);
    if draw.flags.z != 0u {
        vertex_color = in.color.bgra;
    }
    out.world_position = world_position.xyz;
    out.world_normal = world_normal;
    out.vertex_color = vertex_color;
    if (draw.counts.w & 12u) != 0u {
        // For the sky: the direction from the camera, in the space of the world.
        out.world_position = transpose(mat3x3f(draw.view[0].xyz, draw.view[1].xyz, draw.view[2].xyz)) * view_position.xyz;
    }
    if (draw.counts.w & 4u) != 0u {
        // The dome is behind all other things.
        out.clip.z = 0.999999 * out.clip.w;
    }
    // With bit 128 the extension makes the light for each pixel, and this stage makes none.
    // A draw that the extension only finishes (bit 1 alone: its fog, its shadow) keeps the
    // light of this stage.
    if draw.flags.x == 0u || (draw.counts.w & 128u) != 0u {
        out.color = vertex_color;
    } else {
        var diffuse = draw.material_diffuse;
        if draw.more.x == 1u {
            diffuse = vertex_color;
        }
        var emissive = draw.material_emissive.rgb;
        if draw.more.y == 1u {
            emissive = vertex_color.rgb;
        }
        var ambient = draw.material_ambient.rgb;
        if draw.counts.z == 1u {
            ambient = vertex_color.rgb;
        }
        // A lit draw without normals: the fixed function takes a zero normal, so the lights
        // give it their ambient light and no diffuse light.
        var normal = vec3f(0.0);
        if draw.flags.y != 0u {
            normal = normalize(world_normal);
        }
        var sum = emissive + ambient * draw.scene_ambient.rgb;
        for (var i = 0u; i < draw.light_range.y; i++) {
            let light = lights[draw.light_range.x + i];
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
                if light.position.w == 2.0 {
                    // The cone of a spot light: all of the light inside the inner angle, none
                    // outside the outer angle, and a curve between them.
                    let rho = dot(normalize(light.direction.xyz), -direction);
                    if rho <= light.cone.y {
                        strength = 0.0;
                    } else if rho < light.cone.x {
                        strength *= pow((rho - light.cone.y) / max(light.cone.x - light.cone.y, 0.0001), light.attenuation.w);
                    }
                }
            }
            sum += strength * (ambient * light.ambient.rgb + diffuse.rgb * light.diffuse.rgb * max(dot(normal, direction), 0.0));
        }
        out.color = vec4f(clamp(sum, vec3f(0.0), vec3f(1.0)), diffuse.a);
    }

    // The fog of the game is table fog: linear on the depth, for each pixel. The fragment
    // shader makes the fog from this depth, so that a large triangle is right.
    out.fog = abs(view_position.z);
    if draw.fog.w != 0.0 {
        out.fog = length(view_position.xyz);
    }
    return out;
}

// An argument of a stage: D3DTA_*.
fn argument(code: u32, diffuse: vec4f, current: vec4f, texel: vec4f) -> vec4f {
    var value: vec4f;
    switch code & 15u {
        case 0u: { value = diffuse; }
        case 1u: { value = current; }
        case 2u: { value = texel; }
        case 3u: { value = draw.texture_factor; }
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
        case 14u: { return mix(a2, a1, draw.texture_factor.a); }
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
    let how = draw.stages[index].coordinate;
    var divisor = 1.0;
    if how.w != 0u && how.z >= 2u {
        divisor = value[min(how.z, 4u) - 1u];
    }
    return value.xy / divisor;
}

// D3DTOP_BUMPENVMAP is 22 and D3DTOP_BUMPENVMAPLUMINANCE is 23. The move that the bump stage
// `index` gives to the coordinate of the next stage, from its texel.
fn bump_move(index: u32, texel: vec4f) -> vec2f {
    let stage = draw.stages[index];
    if index >= draw.counts.y || (stage.color.x != 22u && stage.color.x != 23u) {
        return vec2f(0.0);
    }
    return vec2f(texel.r * stage.bump.x + texel.g * stage.bump.z, texel.r * stage.bump.y + texel.g * stage.bump.w);
}

// With luminance, a bump stage also makes the texel of the next stage darker, by the
// luminance channel of the bump map. The bump maps of the game have formats without such a
// channel, and then Direct3D takes a luminance of 1: the texel stays as it is. A format with
// luminance is not in this program.
fn bump_light(index: u32, bump_texel: vec4f, texel: vec4f) -> vec4f {
    return texel;
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4f {
    // A texture read must not be in a branch, so each stage reads its texture first.
    var texels: array<vec4f, 6>;
    // A bump stage moves the coordinate of the stage after it.
    texels[0] = textureSample(texture0, sampler0, final_coordinate(0u, in.coordinate0));
    texels[1] = bump_light(0u, texels[0], textureSample(texture1, sampler1, final_coordinate(1u, in.coordinate1) + bump_move(0u, texels[0])));
    texels[2] = bump_light(1u, texels[1], textureSample(texture2, sampler2, final_coordinate(2u, in.coordinate2) + bump_move(1u, texels[1])));
    texels[3] = bump_light(2u, texels[2], textureSample(texture3, sampler3, final_coordinate(3u, in.coordinate3) + bump_move(2u, texels[2])));
    texels[4] = bump_light(3u, texels[3], textureSample(texture4, sampler4, final_coordinate(4u, in.coordinate4) + bump_move(3u, texels[3])));
    texels[5] = bump_light(4u, texels[4], textureSample(texture5, sampler5, final_coordinate(5u, in.coordinate5) + bump_move(4u, texels[4])));

    if (draw.counts.w & 4u) != 0u {
        return vec4f(extension_sky_color(normalize(in.world_position), in.clip.xy), 1.0);
    }
    // A sky draw has a direction in `world_position`, and is above the water in any case.
    if (draw.counts.w & 12u) == 0u && any(draw.clip_plane != vec4f(0.0)) && dot(in.world_position, draw.clip_plane.xyz) + draw.clip_plane.w < 0.0 {
        discard;
    }
    let shaded = (draw.counts.w & 1u) != 0u;
    var diffuse = in.color;
    if shaded {
        diffuse = extension_shade(in);
    }
    var current = diffuse;
    for (var i = 0u; i < draw.counts.y; i++) {
        let stage = draw.stages[i];
        let texel = texels[i];
        let color = operation(stage.color.x, argument(stage.color.y, diffuse, current, texel),
            argument(stage.color.z, diffuse, current, texel), argument(stage.color.w, diffuse, current, texel), diffuse, current, texel);
        var alpha = current;
        if stage.alpha.x != 1u {
            alpha = operation(stage.alpha.x, argument(stage.alpha.y, diffuse, current, texel),
                argument(stage.alpha.z, diffuse, current, texel), vec4f(0.0), diffuse, current, texel);
        }
        current = vec4f(color.rgb, alpha.a);
        // The fixed-function pipeline keeps a colour between 0 and 1. A draw with the light
        // for each pixel has no such limit.
        if (draw.counts.w & 128u) == 0u {
            current = clamp(current, vec4f(0.0), vec4f(1.0));
        }
    }

    if draw.more.z != 0u && !compare(draw.more.z, floor(current.a * 255.0 + 0.5), f32(draw.more.w)) {
        discard;
    }
    var clear = 1.0;
    if draw.fog.z != 0.0 {
        clear = clamp((draw.fog.y - in.fog) / max(draw.fog.y - draw.fog.x, 0.0001), 0.0, 1.0);
    }
    if shaded {
        return extension_finish(current.rgb, current.a, clear);
    }
    if (draw.counts.w & 8u) != 0u {
        return vec4f(extension_sky_color(normalize(in.world_position), in.clip.xy), current.a);
    }
    return vec4f(mix(draw.fog_color.rgb, current.rgb, clear), current.a);
}
