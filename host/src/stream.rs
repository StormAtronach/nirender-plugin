//! The stream of the records: the facts of a shape as the records carry them (`ShapeFacts`),
//! and the state of the stream between the records (`Stream`).

use super::*;

/// The facts of the shape record in progress, or of the last one: what is the same for each
/// pass of its draw.
pub(crate) struct ShapeFacts {
    /// The geometry handle of the record.
    pub(crate) geometry: u32,
    pub(crate) fvf: u32,
    pub(crate) stride: u32,
    pub(crate) world: [f32; 16],
    /// The world matrix is relative to the origin of the scene.
    pub(crate) relative: bool,
    /// The `D3DMATERIAL8`: diffuse, ambient, specular, emissive with the alpha, then power.
    pub(crate) material: [f32; 17],
    /// `D3DRS_AMBIENT` of the draw.
    pub(crate) ambient: u32,
    /// The fill mode of the material (D3DFILLMODE: 2 lines, 3 solid).
    pub(crate) fill: u32,
    /// The lights of the draw from their records: the key, then the `D3DLIGHT8`,
    /// `STATE_LIGHT_SIZE` bytes each.
    pub(crate) lights: Vec<u8>,
}

/// What the stream said before a draw.
pub(crate) struct Stream {
    pub(crate) view: [f32; 16],
    pub(crate) projection: [f32; 16],
    pub(crate) viewport: Option<([f32; 4], [f32; 2])>,
    pub(crate) back_buffer: bool,
    pub(crate) origin: Option<[f64; 3]>,
    /// The player is in an interior cell, from the scene packet.
    pub(crate) interior: bool,
    /// The game hides the sky (the sky root is culled), from the scene packet: the world
    /// scene has no sky draw.
    pub(crate) sky_hidden: bool,
    /// The fog distances of the extension for the draws of the world, or `None`.
    pub(crate) fog_range: Option<(f32, f32)>,
    /// The class of the object of the draws that follow: MGE_LINK_CLASS_* of its packet.
    pub(crate) object_class: u32,
    /// The sky values of the extension for the draw in progress, or `None`.
    pub(crate) sky: Option<[[f32; 4]; 8]>,
    pub(crate) shape: ShapeFacts,
    /// The fog of the shape record in progress (flag 32): the colour, the start, the end
    /// and the range fog flag, as the render state of the game gets them from the fog
    /// property of the draw. `None` for a draw without fog.
    pub(crate) shape_fog: Option<[u32; 4]>,
    /// The fog of the last shape record that had one: the fog states of the device of the
    /// game stay as the last draw with fog set them.
    pub(crate) fog: [u32; 4],
    /// `D3DRS_AMBIENTMATERIALSOURCE` as the last shape record with a value for it set it:
    /// the light manager of the game leaves the state of the draw before for a material
    /// without one (see `material::Derived::sources`). [I] No engine function was read for
    /// this, and the default sources of a new stream do not agree with it.
    pub(crate) ambient_source: u32,
    /// The bones of the skinned shape in progress in the bones of the frame: the first and
    /// the number, or `None` for a draw that is not skinned.
    pub(crate) shape_bones: Option<(u32, u32)>,
    /// The world bound of the shape record in progress (flag 0x400): the centre and the
    /// radius, for the sort of a blended draw among the blends of the table.
    pub(crate) shape_bound: Option<[f32; 4]>,
    /// The shape record in progress is a draw of the engine's main sorted queue (flag
    /// 0x800).
    pub(crate) shape_queued: bool,
    /// The head of the stream says that its shape records carry the flag 0x800 (bit 1 of its
    /// fourth word). Without it (an older capture), a blend is taken as sorted by its
    /// material (`engine::alpha::sorted_by_engine`).
    pub(crate) queue_marked: bool,
}

impl Stream {
    /// The data of one pass of the shape in progress for the shader.
    pub(crate) fn draw_data(&self, derived: &material::Derived, pass: &Pass, counts: &mut Counts, frame_lights: &mut Vec<LightData>) -> DrawData {
        let shape = &self.shape;
        let mut data = DrawData::zeroed();
        let fvf = shape.fvf;
        // The world matrix is relative to the camera when the scene has an origin and the
        // record says so. Then the camera is at 0.
        let relative = self.origin.filter(|_| shape.relative);
        data.world[0] = shape.world;
        data.view = self.view;
        if relative.is_some() {
            data.view[12] = 0.0;
            data.view[13] = 0.0;
            data.view[14] = 0.0;
        }
        data.projection = self.projection;
        if let Some((rectangle, _)) = self.viewport {
            data.pixel = [1.0 / rectangle[2].max(1.0), -1.0 / rectangle[3].max(1.0), 0.0, 0.0];
        }

        let material = &shape.material;
        data.material_diffuse = material[0..4].try_into().unwrap();
        data.material_ambient = material[4..8].try_into().unwrap();
        data.material_emissive = material[12..16].try_into().unwrap();
        data.scene_ambient = color(known(shape.ambient, 0));
        // The fog of the draw: the colour, the start, the end, the range fog flag.
        if let (true, Some(fog)) = (derived.fog, self.shape_fog) {
            data.fog_color = color(fog[0]);
            data.fog = [f32::from_bits(fog[1]), f32::from_bits(fog[2]), 1.0, (fog[3] != 0) as u32 as f32];
            // The fog of the extension is for the world only: a perspective projection.
            if let (Some((start, end)), true) = (self.fog_range, self.projection[11] == 1.0) {
                data.fog[0] = start;
                data.fog[1] = end;
                // The fog of the extension is on the distance from the camera, as the fog
                // of what the extension draws: the two do not change with the direction
                // of the camera.
                data.fog[3] = 1.0;
            }
        }
        // D3DRS_TEXTUREFACTOR: white, the value of a new texture stage of the game. No stage
        // of a derived state has the factor as an argument or as its blend.
        data.texture_factor = [1.0; 4];
        if let Some(sky) = self.sky {
            data.extension = sky;
            if pass.stages[0].texture == 0 {
                // The sky dome has no texture.
                data.counts[3] |= 4;
            } else if self.object_class == 6 && pass.blend[2] == 6 {
                // MGE_LINK_CLASS_SKY_MOON_SHADOW with D3DBLEND_INVSRCALPHA: the disc that keeps
                // the stars out of a moon.
                data.counts[3] |= 8;
            }
        }

        let has_color = fvf & 0x40 != 0;
        data.flags = [derived.lighting as u32, (fvf & 0x10 != 0) as u32, has_color as u32, 0];
        let alpha_test = pass.alpha_test.unwrap_or(derived.alpha_test);
        // D3DMCS_COLOR1 (1) is the default of the diffuse source and D3DMCS_MATERIAL (0) of
        // the emissive source.
        data.more = [
            if has_color { derived.sources[0].unwrap_or(1) } else { 0 },
            if has_color { derived.sources[1].unwrap_or(0) } else { 0 },
            if alpha_test[0] != 0 { alpha_test[1] } else { 0 },
            alpha_test[2],
        ];

        let lights = shape.lights.len() / STATE_LIGHT_SIZE;
        counts.most_lights = counts.most_lights.max(lights as u32);
        data.light_range = [frame_lights.len() as u32, lights as u32, 0, flat_shading_bit(derived)];
        data.counts[0] = lights as u32;
        if let Some((first, _)) = self.shape_bones {
            // A skinned shape: the vertex blend reads four bones of the palette of the frame.
            data.flags[3] = 4;
            data.light_range[2] = first;
        }
        for light in shape.lights.chunks_exact(STATE_LIGHT_SIZE) {
            // The `D3DLIGHT8` after the key.
            frame_lights.push(light_data(&light[4..], relative));
        }

        let mut stages = 0;
        while stages < 8 {
            let stage = &pass.stages[stages];
            if stage.color[0] == 1 {
                break;
            }
            if stages < STAGES {
                // The coordinate index has the way to generate it in its high word (1 to 3
                // from a vector of camera space, as Direct3D; 5 to 7 from the same vector of
                // the world, for the effects of the retained table). The transform flags are
                // the number of parts, and 256 for a projected coordinate.
                let (coordinate, transform) = (stage.coordinate, stage.transform);
                if coordinate >> 16 != 0 || transform != 0 {
                    counts.coordinate_work += 1;
                }
                // The result argument of a stage is D3DTA_CURRENT (1).
                data.stages[stages] = StageData {
                    color: [stage.color[0], stage.color[1], stage.color[2], 1],
                    alpha: [stage.alpha[0], stage.alpha[1], stage.alpha[2], 0],
                    coordinate: [coordinate >> 16 & 7, coordinate & 3, transform & 7, (transform & 256 != 0) as u32],
                    bump: [stage.bump[0], stage.bump[1], stage.bump[2], stage.bump[3]],
                    luminance: [stage.bump[4], stage.bump[5], 0.0, 0.0],
                    slots: [0; 4],
                };
                data.texture_transforms[stages] = stage.texture_transform;
            }
            stages += 1;
        }
        if stages > STAGES {
            counts.more_stages += 1;
        }
        data.counts[1] = stages.min(STAGES) as u32;
        data.counts[2] = if has_color { self.ambient_source } else { 0 };
        data
    }
}
