//! The two exact keys of a lit draw's light, decided from the draw's record of each frame (the
//! record path, `player::draws`). A draw whose colour cannot take light drops its lights
//! ([`colour_takes_no_light`]); a draw with no lights takes the bit [`crate::NO_LIGHTS_BIT`]
//! of `counts.w` ([`has_no_lights`]), a fact of its stage key, so that its fragment shader has
//! no light loop, no lookup of the clusters and no lights of the world. Neither changes the
//! picture. Both are only for a draw that the extension lights for each pixel.
//!
//! A shader key that removes invisible work is off while a debug view of its input is on: in
//! a frame with the lights view ([`crate::FrameParameters::lights_view`]) a draw keeps the
//! lights its colour cannot take, so that the view shows where they reach. The zero key stays:
//! no lights is the truth.
//!
//! The switches `NIRENDER_BLIND_LIGHT_KEY=0` and `NIRENDER_ZERO_LIGHT_KEY=0` turn the keys
//! off. The draws of the retained table that the GPU makes (`retained`, by kind) do not take
//! them.
//!
//! The corner light of a particle ([`lights_at_corners`], [`crate::CORNER_LIGHT_BIT`]) comes
//! after the two keys: blind first, then zero, then the corners, so a draw takes at most one
//! of zero and corner. It is a look the user chooses (the setting `per_pixel_particles` off,
//! [`crate::FrameParameters::particle_corners`]), not a key that removes invisible work: it
//! stays while a debug view is on.

use crate::data::DrawData;

/// The material way of the lit colour cannot take light: the diffuse and the ambient colours
/// come from the material (not from the vertex colour: `more.x` and `counts.z` are not 1), and
/// both are black, each part exactly 0. The lit colour is then the emissive colour (or the
/// vertex colour as the emissive source) whatever the lights are: the light of each light is
/// multiplied by the black diffuse and ambient colours, and the scene's ambient light by the
/// black ambient colour. A value of 1/255 is not black. Only a draw that the extension lights
/// for each pixel ([`lit_for_each_pixel`]) takes it.
pub(crate) fn colour_takes_no_light(data: &DrawData) -> bool {
    let diffuse_from_material = data.more[0] != 1;
    let ambient_from_material = data.counts[2] != 1;
    lit_for_each_pixel(data)
        && diffuse_from_material
        && ambient_from_material
        && black(data.material_diffuse)
        && black(data.material_ambient)
}

/// The draw is lit, and the extension makes its light for each pixel (bit 128 of `counts.w`):
/// the vertex stage makes none.
fn lit_for_each_pixel(data: &DrawData) -> bool {
    data.flags[0] != 0 && data.counts[3] & 128 != 0
}

/// The red, green and blue parts are 0 exactly; the alpha does not count.
fn black(colour: [f32; 4]) -> bool {
    colour[..3].iter().all(|&part| part == 0.0)
}

/// A draw lit for each pixel with no lights: none of its own, no lights from the clusters of the
/// frame (bit 1 of `light_range.w`) and none of the world ([`crate::WORLD_LIGHTS_BIT`]).
pub(crate) fn has_no_lights(data: &DrawData) -> bool {
    let from_elsewhere = data.light_range[3] & (1 | crate::WORLD_LIGHTS_BIT) != 0;
    lit_for_each_pixel(data) && data.light_range[1] == 0 && !from_elsewhere
}

/// A particle draw lit for each pixel that has lights the vertex stage can take: its own lights
/// (with clusters on, a corner takes every light of the draw: the vertex stage cannot read the
/// clusters). Not with the lights of the world ([`crate::WORLD_LIGHTS_BIT`]): only a pixel's
/// cluster has them. Not a draw with no lights: the zero key takes it.
pub(crate) fn lights_at_corners(data: &DrawData, particles: bool) -> bool {
    let world = data.light_range[3] & crate::WORLD_LIGHTS_BIT != 0;
    particles && lit_for_each_pixel(data) && !world && !has_no_lights(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lit draw of the material way with a black diffuse and ambient colour and 8 lights:
    /// the particle draw of the NIF test level's 080 (W304b, `MGE_XE_DRAW_DUMP` of copy D6).
    fn blind() -> DrawData {
        let mut data = DrawData::zeroed();
        data.flags = [1, 1, 1, 0];
        data.counts = [0, 2, 0, 0x293];
        data.light_range = [42, 8, 0, 0];
        data
    }

    use bytemuck::Zeroable;

    #[test]
    fn a_black_material_takes_no_light() {
        assert!(colour_takes_no_light(&blind()));
        // The emissive colour and the alphas do not count.
        let mut emissive = blind();
        emissive.material_emissive = [1.0, 0.5, 0.0, 1.0];
        emissive.material_diffuse[3] = 0.15;
        emissive.material_ambient[3] = 1.0;
        assert!(colour_takes_no_light(&emissive));
        // The vertex colour as the emissive source (`more.y` 1): the lit colour is the vertex colour.
        let mut vertex_emissive = blind();
        vertex_emissive.more[1] = 1;
        assert!(colour_takes_no_light(&vertex_emissive));
    }

    #[test]
    fn a_colour_that_takes_light_keeps_its_lights() {
        let cases: [(&str, fn(&mut DrawData)); 8] = [
            ("diffuse from the vertex colour", |d| d.more[0] = 1),
            ("ambient from the vertex colour", |d| d.counts[2] = 1),
            ("a diffuse of 1/255", |d| d.material_diffuse[0] = 1.0 / 255.0),
            ("a blue diffuse", |d| d.material_diffuse[2] = 1.0),
            ("an ambient of 1/255", |d| d.material_ambient[1] = 1.0 / 255.0),
            ("a white ambient", |d| d.material_ambient = [1.0; 4]),
            ("unlit", |d| d.flags[0] = 0),
            ("lit for each vertex", |d| d.counts[3] &= !128),
        ];
        for (name, change) in cases {
            let mut data = blind();
            change(&mut data);
            assert!(!colour_takes_no_light(&data), "{name}");
        }
    }

    #[test]
    fn the_key_follows_the_record_of_each_frame() {
        // A material controller turns the diffuse from black to white between two frames: the
        // record of the second frame keeps its lights.
        let frame_1 = blind();
        let mut frame_2 = blind();
        frame_2.material_diffuse = [1.0, 1.0, 1.0, 1.0];
        assert!(colour_takes_no_light(&frame_1));
        assert!(!colour_takes_no_light(&frame_2));
    }

    #[test]
    fn a_particle_with_lights_takes_them_at_its_corners() {
        let particle = blind();
        assert!(lights_at_corners(&particle, true));
        assert!(!lights_at_corners(&particle, false), "not the quads of a particle system");
        let cases: [(&str, fn(&mut DrawData)); 4] = [
            ("no lights: the zero key takes it", |d| d.light_range[1] = 0),
            ("the lights of the world", |d| d.light_range[3] |= crate::WORLD_LIGHTS_BIT),
            ("unlit", |d| d.flags[0] = 0),
            ("lit for each vertex", |d| d.counts[3] &= !128),
        ];
        for (name, change) in cases {
            let mut other = particle;
            change(&mut other);
            assert!(!lights_at_corners(&other, true), "{name}");
        }
        // With clusters on, a corner takes every light of the draw.
        let mut clustered = particle;
        clustered.light_range[3] |= 1;
        assert!(lights_at_corners(&clustered, true));
    }

    #[test]
    fn no_lights_means_none_of_any_source() {
        let mut data = blind();
        data.light_range[1] = 0;
        assert!(has_no_lights(&data));
        let cases: [(&str, fn(&mut DrawData)); 5] = [
            ("one light", |d| d.light_range[1] = 1),
            ("the clusters", |d| d.light_range[3] |= 1),
            ("the lights of the world", |d| d.light_range[3] |= crate::WORLD_LIGHTS_BIT),
            ("unlit", |d| d.flags[0] = 0),
            ("lit for each vertex", |d| d.counts[3] &= !128),
        ];
        for (name, change) in cases {
            let mut other = data;
            change(&mut other);
            assert!(!has_no_lights(&other), "{name}");
        }
    }
}
