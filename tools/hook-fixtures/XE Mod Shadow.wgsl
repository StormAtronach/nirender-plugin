//! interface: mge-xe-core 1
// A fixture of the replay gate (W528 item 5), not a mod. shadow_delta is the built-in text;
// shadow_esm has a sharper edge (1.5 times the exponent). Both read the library.

fn shadow_delta(s0: vec3f, s1: vec3f) -> f32 {
    let rcp = shadow_texel();
    let margin = vec3f(1.0 - 8.0 * rcp, 1.0 - 8.0 * rcp, 1.0);
    if all(abs(s0) < margin) {
        let uv = vec2f(0.5 + 0.5 * s0.x, 0.5 - 0.5 * s0.y);
        return shadow_map_depth(0u, uv) / shadow_esm_scale() - s0.z;
    }
    if all(abs(s1) < margin) {
        let uv = vec2f(0.5 + 0.5 * s1.x, 0.5 - 0.5 * s1.y);
        return shadow_map_depth(1u, uv) / shadow_esm_scale() - s1.z;
    }
    return 1e-6;
}

fn shadow_esm(dz: f32) -> f32 {
    let esm = shadow_esm_values();
    return 1.0 - clamp(exp(1.5 * esm.x * dz + esm.y), 0.0, 1.0);
}
