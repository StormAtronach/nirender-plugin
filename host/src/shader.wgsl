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
    // The three attenuation values.
    attenuation: vec4f,
}

struct Stage {
    // Colour: operation, argument 1, argument 2, argument 0.
    color: vec4u,
    // Alpha: operation, argument 1, argument 2. w: the texture coordinate set.
    alpha: vec4u,
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
    // start, end, 1 when fog is on.
    fog: vec4f,
    texture_factor: vec4f,
    // lighting on, the vertex has a normal, the vertex has a colour, the number of weights.
    flags: vec4u,
    // diffuse source, emissive source, alpha test function (0 is off), alpha reference.
    more: vec4u,
    // The number of lights, the number of stages, the ambient source.
    counts: vec4u,
    lights: array<Light, 8>,
    stages: array<Stage, 6>,
}

@group(0) @binding(0) var<uniform> draw: Draw;
@group(1) @binding(0) var texture0: texture_2d<f32>;
@group(1) @binding(1) var texture1: texture_2d<f32>;
@group(1) @binding(2) var texture2: texture_2d<f32>;
@group(1) @binding(3) var texture3: texture_2d<f32>;
@group(1) @binding(4) var texture4: texture_2d<f32>;
@group(1) @binding(5) var texture5: texture_2d<f32>;
@group(2) @binding(0) var sampler0: sampler;
@group(2) @binding(1) var sampler1: sampler;
@group(2) @binding(2) var sampler2: sampler;
@group(2) @binding(3) var sampler3: sampler;
@group(2) @binding(4) var sampler4: sampler;
@group(2) @binding(5) var sampler5: sampler;

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
}

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) color: vec4f,
    @location(1) uv0: vec2f,
    @location(2) uv1: vec2f,
    @location(3) uv2: vec2f,
    @location(4) uv3: vec2f,
    @location(5) fog: f32,
}

@vertex
fn vs(in: VertexIn) -> VertexOut {
    let local = vec4f(in.position, 1.0);
    var world_position: vec4f;
    var world_normal: vec3f;
    let weights = draw.flags.w;
    if weights == 0u {
        world_position = draw.world[0] * local;
        world_normal = (draw.world[0] * vec4f(in.normal, 0.0)).xyz;
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
    out.uv0 = in.uv0;
    out.uv1 = in.uv1;
    out.uv2 = in.uv2;
    out.uv3 = in.uv3;

    var vertex_color = vec4f(1.0);
    if draw.flags.z != 0u {
        vertex_color = in.color.bgra;
    }
    if draw.flags.x == 0u {
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
        var normal = vec3f(0.0, 0.0, 1.0);
        if draw.flags.y != 0u {
            normal = normalize(world_normal);
        }
        var sum = emissive + ambient * draw.scene_ambient.rgb;
        for (var i = 0u; i < draw.counts.x; i++) {
            let light = draw.lights[i];
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
            }
            sum += strength * (ambient * light.ambient.rgb + diffuse.rgb * light.diffuse.rgb * max(dot(normal, direction), 0.0));
        }
        out.color = vec4f(clamp(sum, vec3f(0.0), vec3f(1.0)), diffuse.a);
    }

    // The fog of the game is linear on the distance from the eye.
    out.fog = 1.0;
    if draw.fog.z != 0.0 {
        out.fog = clamp((draw.fog.y - length(view_position.xyz)) / max(draw.fog.y - draw.fog.x, 0.0001), 0.0, 1.0);
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

@fragment
fn fs(in: VertexOut) -> @location(0) vec4f {
    var uvs = array<vec2f, 4>(in.uv0, in.uv1, in.uv2, in.uv3);
    // A texture read must not be in a branch, so each stage reads its texture first.
    var texels: array<vec4f, 6>;
    texels[0] = textureSample(texture0, sampler0, uvs[draw.stages[0].alpha.w & 3u]);
    texels[1] = textureSample(texture1, sampler1, uvs[draw.stages[1].alpha.w & 3u]);
    texels[2] = textureSample(texture2, sampler2, uvs[draw.stages[2].alpha.w & 3u]);
    texels[3] = textureSample(texture3, sampler3, uvs[draw.stages[3].alpha.w & 3u]);
    texels[4] = textureSample(texture4, sampler4, uvs[draw.stages[4].alpha.w & 3u]);
    texels[5] = textureSample(texture5, sampler5, uvs[draw.stages[5].alpha.w & 3u]);

    let diffuse = in.color;
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
        current = clamp(vec4f(color.rgb, alpha.a), vec4f(0.0), vec4f(1.0));
    }

    if draw.more.z != 0u && !compare(draw.more.z, floor(current.a * 255.0 + 0.5), f32(draw.more.w)) {
        discard;
    }
    return vec4f(mix(draw.fog_color.rgb, current.rgb, in.fog), current.a);
}
