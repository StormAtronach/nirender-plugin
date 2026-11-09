//! The cone of a spot light in WGSL: the one text of the cone for each shader that lights with
//! the lights of the frame. The world shader takes it ([`crate::world_shader_source`]), and an
//! extension's shader of its own takes [`WGSL`]. The shader that takes it has a struct `Light`
//! with the fields of [`crate::LightData`].

/// `spot_cone(light, direction)`: the factor of a light for a place in `direction` (from the
/// place to the light), as the fixed function of Direct3D 8 has it. All of the light inside the
/// inner angle (`cone.x`, the cosine of the half angle), none outside the outer angle (`cone.y`),
/// and a curve between them (the exponent `attenuation.w`). 1 for each light that is not a spot
/// light (`position.w` 2).
pub const WGSL: &str = "
fn spot_cone(light: Light, direction: vec3f) -> f32 {
    if light.position.w != 2.0 {
        return 1.0;
    }
    let rho = dot(normalize(light.direction.xyz), -direction);
    if rho <= light.cone.y {
        return 0.0;
    }
    if rho < light.cone.x {
        return pow((rho - light.cone.y) / max(light.cone.x - light.cone.y, 0.0001), light.attenuation.w);
    }
    return 1.0;
}
";
