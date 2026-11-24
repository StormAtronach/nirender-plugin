//! interface: mge-xe-core 1
// A fixture of the replay gate (W528 item 5), not a mod. shadow_esm has a sharper edge (1.5 times
// the exponent). It reads the library.

fn shadow_esm(dz: f32) -> f32 {
    let esm = shadow_esm_values();
    return 1.0 - clamp(exp(1.5 * esm.x * dz + esm.y), 0.0, 1.0);
}
