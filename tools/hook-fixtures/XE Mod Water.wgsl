//! interface: mge-xe-core 1
// A fixture of the replay gate (W528 item 5), not a mod. Each of the three hooks reads the
// library: calmer waves (half the slope of the built-in), and a colour of the water from the
// reflection, the picture behind the water and the sun, tinted red.

const mod_tint = vec3f(1.0, 0.8, 0.8);

fn water_normal(texcoords: vec4f, distance: f32, world_xy: vec2f) -> vec3f {
    let t = 0.4 * water_time();
    let far = wave_at(vec3f(texcoords.xy, t)).rg;
    var close = wave_at(vec3f(texcoords.zw, t)).rg;
    close = ripples_on(close, texcoords.zw, world_xy);
    let n = 2.0 * mix(close, far, clamp(distance / 8000.0, 0.0, 1.0)) - 1.0;
    return normalize(vec3f(0.5 * n, 1.0));
}

fn water_color(p: WaterPixel) -> vec4f {
    let fog = fog_water(p.eye_vec, p.distance);
    let normal = water_normal(p.texcoords, p.distance, p.world.xy);
    let offset = 0.01 * normal.xy;
    let reflected = mix(fog.rgb, reflection_at(p.reflect_uv - offset, p.reflect_w, 1.0), fog.a);
    let depth = max(ground_distance_at(p.uv) - p.clip_w, 0.0);
    let below = mix(deep_color(fog, vec3f(0.0)), picture_at(p.uv + offset), transmission(vec3f(0.5), depth));
    let fresnel = 0.02 + 0.98 * pow(1.0 - clamp(abs(p.eye_vec.z), 0.0, 1.0), 5.0);
    let color = mix(below, reflected, fresnel) + sun_glint(p.eye_vec, normal, fog.a);
    return vec4f(color * mod_tint, 1.0);
}

fn underwater_color(p: WaterPixel) -> vec4f {
    return vec4f(mix(picture_at(p.uv), water_fog_color(), 0.5) * mod_tint, 1.0);
}
