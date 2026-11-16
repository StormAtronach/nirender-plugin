//! interface: mge-xe-core 1
// A fixture of the replay gate (W528 item 5), not a mod: the colour of the vertex times the
// clouds' texture (the built-in CloudsPS), tinted red.

const mod_tint = vec4f(1.0, 0.8, 0.8, 1.0);

fn clouds_color(p: CloudPixel) -> vec4f {
    return p.color * cloud_texture(p.texcoords) * mod_tint;
}
