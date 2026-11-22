//! interface: mge-xe-core 1
// A fixture of the replay gate (W525), not a mod: the colour of the stages tinted red for a
// draw that adds and green for any other. Each function of the library is called; the time,
// the texture read and the other fields have no effect on the picture.

const mod_adds_tint = vec4f(1.0, 0.8, 0.8, 1.0);
const mod_other_tint = vec4f(0.8, 1.0, 0.8, 1.0);

fn particle_color(p: ParticlePixel) -> vec4f {
    let still = 0.0 * (particle_time() + p.distance + p.pixel.x + p.vertex_color.a + particle_texture(p.texcoords).a);
    if particle_adds() {
        return p.color * mod_adds_tint + vec4f(still);
    }
    return p.color * mod_other_tint + vec4f(still);
}
