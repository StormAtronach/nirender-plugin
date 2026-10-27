//! The records of the stream into the player: `feed` and what it calls for a draw of
//! the game (the shape, the instance and the held-shape records, the screen draws).

use crate::*;

/// The generation of the reply memory that a `READ_PIXELS` record names: 0 in a record of a
/// plugin before the third word.
pub(crate) fn reply_generation(data: &[u8]) -> u32 {
    if data.len() >= 12 {
        word(data, 8)
    } else {
        0
    }
}

impl Player {
    /// Does one record. Returns true for the record that ends a frame.
    /// Engine-rules R19, R22 (the skill wgpu-renderer-dev).
    pub(crate) fn feed(&mut self, tag: u32, data: &[u8]) -> bool {
        // A record shorter than its tag needs (the fixed part, and the variable parts that
        // its counts name) is refused: not read past its end, counted, the first of each tag
        // in the log. The reads below stay within `needed`; the checks of the readers on
        // their own counts stay as a second line.
        let needed = tag::needed(tag, data);
        if data.len() < needed {
            self.refuse(tag, data.len(), needed);
            return false;
        }
        let move_check = self.move_check.take();
        match tag {
            tag::HEADER => {
                self.size = (word(data, 0), word(data, 4));
                let abilities = if data.len() >= 16 { word(data, 12) } else { 0 };
                self.stream.queue_marked = abilities & HEADER_QUEUE_MARKED != 0;
                self.stream.world_walk = abilities & HEADER_WORLD_WALK != 0;
                self.stream.scene_target = abilities & HEADER_SCENE_TARGET != 0;
                self.stream.queue_flush = abilities & HEADER_QUEUE_FLUSH != 0;
                self.retained.walk.on = abilities & HEADER_WALK_PLACES != 0;
                self.stream.scene_wireframe = abilities & HEADER_SCENE_WIREFRAME != 0;
                self.stream.envmap_lit = (abilities & HEADER_ENVMAP_KNOWN != 0).then_some(abilities & HEADER_ENVMAP_LIT != 0);
            }
            tag::TEXTURE => self.renderer.texture(data),
            tag::TEXTURE_UPDATE => self.renderer.texture_update(data),
            tag::SHOT => self.shots.push(String::from_utf8_lossy(data).into_owned()),
            tag::MARK => self.mark(&String::from_utf8_lossy(data)),
            tag::OVERLAY => self.overlay_record = Some(data.to_vec()),
            tag::GAMMA_RAMP => {
                self.gamma_ramp = gamma::read_ramp(data).filter(|ramp| !gamma::is_identity(ramp));
                self.gamma_new = true;
                match self.gamma_ramp.as_ref() {
                    Some(ramp) => println!("gamma ramp: a look-up table for the window; red 64 -> {}, 128 -> {}, 192 -> {} (of 65535)", ramp[64], ramp[128], ramp[192]),
                    None => println!("gamma ramp: the identity; the window takes the plain copy"),
                }
            }
            tag::EXTENSION if data.len() >= 4 => {
                if let Some(extension) = self.extension.as_mut() {
                    extension.data(word(data, 0), &data[4..]);
                }
            }
            tag::DRAW_UP if self.drawing() && data.len() >= 20 => self.draw_up(data),
            tag::READ_PIXELS => {
                // The game waits for the answer. Without the target, the answer has no size.
                let answer = self.read_pixels(word(data, 4));
                if let Some(reply) = self.reply.as_mut() {
                    let generation = reply_generation(data);
                    match answer {
                        Some((width, height, pixels)) => reply(word(data, 0), generation, width, height, &pixels),
                        None => reply(word(data, 0), generation, 0, 0, &[]),
                    }
                }
            }
            tag::COPY_TILE => self.copy_tile(word(data, 0), word(data, 4), word(data, 8)),
            tag::GEOMETRY if data.len() >= 32 => {
                let handle = word(data, 0);
                let known = self.renderer.geometries.contains_key(&handle);
                let had_uv = self.renderer.geometries.get(&handle).is_some_and(|g| g.uv.is_some());
                let in_place = self.renderer.geometry(data);
                // New vertices in place hold the engine's coordinates: no step on them, until
                // the `GEOMETRY_UV` record of their state.
                if in_place && had_uv {
                    self.geometry_uv(handle, None);
                }
                if known && !in_place {
                    self.geometry_changed(handle);
                }
            }
            tag::GEOMETRY_UV => self.geometry_uv_record(data),
            tag::MORPH_TARGETS => self.renderer.morph_targets(data),
            tag::MORPH_WEIGHTS => self.renderer.morph_weights(data),
            tag::GEOMETRY_GONE => {
                let handle = word(data, 0);
                if self.renderer.geometry_gone(handle, word(data, 4)) {
                    self.gone_geometries.insert(handle);
                }
            }
            tag::TEXTURE_GONE => self.renderer.texture_gone(word(data, 0)),
            tag::MATERIAL_GONE => {
                self.gone_materials.push(word(data, 0));
                self.gone_materials_new = true;
            }
            tag::MATERIAL if data.len() >= 4 => {
                if let Some(parsed) = material::Material::parse(&data[4..]) {
                    let handle = word(data, 0);
                    // A record with a handle that is known has the same content: a hold of a
                    // capture file sends it again.
                    self.renderer.materials.entry(handle).or_insert(parsed);
                    // The plugin sent it again after a `MATERIAL_GONE`: it stays.
                    if !self.gone_materials.is_empty() {
                        self.gone_materials.retain(|&gone| gone != handle);
                    }
                    self.renderer.counts.material_records += 1;
                }
            }
            tag::SCENE_BEGIN => {
                self.stream.origin = (word(data, 20) & 1 != 0).then(|| [double(data, 24), double(data, 32), double(data, 40)]);
                self.stream.interior = word(data, 20) & 2 != 0;
                self.stream.sky_hidden = word(data, 20) & 4 != 0;
                self.stream.wireframe = word(data, 20) & SCENE_WIREFRAME != 0;
                if self.stream.scene_target {
                    self.scene_target(word(data, 20) & SCENE_BACK_BUFFER != 0);
                }
                for step in self.plan.scene_begin(word(data, 4)) {
                    self.plan_step(step);
                }
            }
            tag::SCENE_END => {
                // Flag 1: the engine's sorted queue stays open after the scene. An older
                // capture has no word.
                let queue_open = data.len() >= 4 && word(data, 0) & 1 != 0;
                let steps = self.plan.scene_end(self.extension.is_some(), self.target.is_none(), self.drawing(), queue_open, self.stream.queue_flush);
                for step in steps {
                    self.plan_step(step);
                }
            }
            tag::FRAME_EVENT => {
                if word(data, 0) == EVENT_SET_TARGET {
                    self.wants_texture = word(data, 4) != 0;
                }
                if word(data, 0) == EVENT_WORLD_WALK && self.stream.world_walk {
                    self.world_walk();
                }
                if word(data, 0) == EVENT_QUEUE_FLUSHED && self.stream.queue_flush {
                    self.queue_flushed();
                }
            }
            // The kind and the class of the object of the shape records that follow.
            tag::OBJECT if data.len() >= 12 => {
                self.stream.object_class = word(data, 4);
                self.stream.shape_fog = None;
                self.stream.shape_bones = None;
            }
            tag::SHAPE if data.len() >= 144 => {
                // Flag 0x200: the twin of the instance draw before it.
                if self.retained_on && word(data, 8) & 0x200 != 0 {
                    self.renderer.counts.shape_twins += 1;
                } else {
                    self.shape(data);
                }
            }
            tag::INSTANCE if data.len() >= 176 => self.instance(data, move_check),
            tag::INSTANCE_MOVE => self.instance_move(data),
            tag::INSTANCE_LIGHTS if data.len() >= 8 => {
                let (id, count) = (word(data, 0), word(data, 4) as usize);
                if data.len() >= 8 + count * 4 {
                    let keys: Vec<u32> = (0..count).map(|i| word(data, 8 + i * 4)).collect();
                    // The ambient light of the instance's own effect state follows the keys
                    // (an older plugin sends none: the instance keeps the scene's).
                    let ambient = (data.len() >= 12 + count * 4).then(|| word(data, 8 + count * 4));
                    self.retained.set_lights(id, &keys, ambient, &self.renderer.lights_known);
                }
            }
            tag::INSTANCE_STATE if data.len() >= 8 => self.retained.set_visible(word(data, 0), word(data, 4) != 0),
            tag::INSTANCE_BONES if data.len() >= 24 => {
                let bound = core::array::from_fn(|i| float(data, 8 + i * 4));
                self.retained.set_bones(word(data, 0), word(data, 4), bound, &data[24..]);
            }
            tag::RETAINED_SCENE if data.len() >= 8 => {
                let flags = word(data, 4);
                let fog = (flags & 32 != 0 && data.len() >= 24).then(|| core::array::from_fn(|i| word(data, 8 + i * 4)));
                self.retained_values = Some((word(data, 0), fog));
                // Flag 64: the sun of the frame, the 26 words of the weather's sky light as
                // the engine gives them to the device, after the fog.
                let at = if fog.is_some() { 24 } else { 8 };
                self.retained_sun = (flags & 64 != 0 && data.len() >= at + 104).then(|| data[at..at + 104].try_into().unwrap());
            }
            tag::INSTANCE_DRAW if data.len() >= 12 => {
                if self.retained_on {
                    self.instance_draw(data);
                }
            }
            tag::INSTANCE_PARTICLES if data.len() >= 36 => {
                if self.retained_on {
                    let count = word(data, 4) as usize;
                    let particles = &data[36..(36 + count * 20).min(data.len())];
                    let floats3 = |at: usize| -> [f32; 3] { core::array::from_fn(|i| f32::from_bits(word(data, at + i * 4))) };
                    self.renderer.particles(word(data, 0), f32::from_bits(word(data, 8)), floats3(12), floats3(24), particles);
                    self.renderer.counts.instance_particles += 1;
                }
            }
            tag::INSTANCE_QUEUED if data.len() >= 4 => {
                if self.retained_on {
                    self.instance_queued(word(data, 0));
                }
            }
            tag::INSTANCE_WALK if data.len() >= 4 => {
                if self.retained_on && self.retained.walk.on {
                    self.walk_place(word(data, 0));
                }
            }
            tag::INSTANCE_DELETE if data.len() >= 4 => {
                self.instance_take(word(data, 0));
                self.retained_remove(word(data, 0));
            }
            tag::LIGHT if data.len() >= 108 => {
                // The engine keeps the radius of a point light in its specular colour
                // (`game_dynamicLightTest` 0x4D3039). The device values carry that colour times
                // the dimmer, so a dim or flickering light's reach would follow its dimmer; the
                // word after them is the radius itself. It goes into the first specular word,
                // which `light_data` reads as the radius (no shading reads the specular of a
                // light). An older record without the word keeps the scaled value.
                let mut light: [u8; 104] = data[4..108].try_into().unwrap();
                if data.len() >= 112 {
                    light[20..24].copy_from_slice(&data[108..112]);
                }
                self.renderer.lights_known.insert(word(data, 0), light);
                self.renderer.counts.light_records += 1;
                self.retained.light_changed(word(data, 0), &light);
            }
            tag::LIGHT_GONE if data.len() >= 4 => {
                // The engine gives the key to the next light it makes: that light comes as a
                // new record.
                self.renderer.lights_known.remove(&word(data, 0));
                self.renderer.counts.lights_gone += 1;
                self.retained.light_gone(word(data, 0));
            }
            tag::WORLD_LIGHTS if data.len() >= 4 => {
                let count = (word(data, 0) as usize).min((data.len() - 4) / 4);
                let keys: Vec<u32> = (0..count).map(|i| word(data, 4 + i * 4)).collect();
                self.retained.set_world_lights(&keys, &self.renderer.lights_known);
            }
            tag::FRAME_END => {
                self.renderer.counts.frames += 1;
                return true;
            }
            tag::CALL => {
                let arguments = &data[16..];
                match word(data, 0) {
                    CALL_VIEWPORT => {
                        let viewport = &arguments[word(data, 12) as usize * 4..];
                        if viewport.len() >= 24 {
                            let rectangle = [word(viewport, 0) as f32, word(viewport, 4) as f32, word(viewport, 8) as f32, word(viewport, 12) as f32];
                            let viewport = (rectangle, [float(viewport, 16), float(viewport, 20)]);
                            self.stream.viewport = Some(viewport);
                            if self.stream.back_buffer {
                                self.items.push(Item::Viewport(viewport));
                            }
                        }
                    }
                    CALL_CREATE_TEXTURE => {
                        // Width, height, levels, usage, format, pool. D3DUSAGE_RENDERTARGET is 1.
                        let handle = word(data, 8);
                        if handle != 0 && word(arguments, 12) & 1 != 0 {
                            self.renderer.render_texture(handle, word(arguments, 0), word(arguments, 4));
                        } else if handle != 0 {
                            self.renderer.empty_texture(handle, word(arguments, 0), word(arguments, 4), word(arguments, 8), word(arguments, 16));
                        }
                    }
                    CALL_TEXTURE_LEVEL => {
                        // Texture, level; the handle that the call gave is the surface.
                        if word(data, 8) != 0 && word(arguments, 4) == 0 {
                            self.renderer.surfaces.insert(word(data, 8), word(arguments, 0));
                        }
                    }
                    CALL_RENDER_TARGET => {
                        // The colour surface and the depth surface. A surface of a texture
                        // that this program made is that texture; each other one is the
                        // back buffer.
                        let texture = self.renderer.surfaces.get(&word(arguments, 0)).copied().filter(|handle| self.renderer.targets.contains_key(handle));
                        // A rendered texture that is not known here: its draws go nowhere.
                        self.stream.back_buffer = texture.is_some() || !self.wants_texture;
                        if texture != self.target {
                            self.target = texture;
                            if self.drawing() {
                                self.items.push(Item::Target(texture));
                            }
                        }
                    }
                    CALL_COPY_PIXELS => {
                        // Source surface, rectangles, their number, destination surface,
                        // points. With pixels, the source is an image surface: the game read
                        // a rendered texture, changed the pixels, and now writes them back.
                        let pixels = &arguments[(word(data, 12) as usize * 4).min(arguments.len())..];
                        // The record names the texture, or the surface is one that a call
                        // of the stream gave.
                        let named = (pixels.len() >= 20 && word(pixels, 16) != 0).then(|| word(pixels, 16));
                        let texture = named.or_else(|| self.renderer.surfaces.get(&word(arguments, 12)).copied());
                        if let (Some(texture), true) = (texture, pixels.len() >= 20) {
                            // The draws into the texture come first: a write of a texture is
                            // done before the commands that are not sent yet.
                            self.flush(|player| player.renderer.target_pixels(texture, word(pixels, 0), word(pixels, 4), &pixels[20..]));
                        }
                    }
                    CALL_CLEAR if self.drawing() && self.stream.back_buffer => {
                        // Rectangles, their address, flags, colour, depth, stencil.
                        let flags = word(arguments, 8);
                        let clear_color = (flags & 1 != 0).then(|| {
                            // The alpha of the back buffer is 1: see the colour write of a draw.
                            let c = color(word(arguments, 12));
                            wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 }
                        });
                        // The game's value, in the convention of the depth (`depth::clear`).
                        let clear_depth = (flags & 2 != 0).then(|| depth::clear(float(arguments, 16)));
                        let clear_stencil = (flags & 4 != 0).then(|| word(arguments, 20));
                        if clear_depth.is_some() {
                            for step in self.plan.depth_clear() {
                                if matches!(step, plan::Step::SortedBlends(_)) {
                                    self.renderer.counts.blends_before_clear += 1;
                                }
                                self.plan_step(step);
                            }
                        }
                        self.items.push(Item::Clear(clear_color, clear_depth, clear_stencil));
                    }
                    _ => {}
                }
            }
            // The view and the projection of the camera (D3DTS_VIEW is 2 and
            // D3DTS_PROJECTION is 3). The draws of the game come as shape records and as
            // draws with their vertices, each with its own state; a command of another
            // kind, in an older capture, does nothing.
            tag::COMMAND => match word(data, 0) {
                COMMAND_TRANSFORM if data.len() >= 16 + 64 => match word(data, 4) {
                    2 => self.stream.view = floats(data, 16),
                    3 => self.stream.projection = floats(data, 16),
                    _ => {}
                },
                _ => {}
            },
            _ if !tag::is_known(tag) => {
                let counts = &mut self.renderer.counts;
                if counts.unknown_records == 0 {
                    counts.unknown_tag = tag;
                    println!("record of the tag {tag}, which this build does not know: skipped (the plugin is another build)");
                }
                counts.unknown_records += 1;
            }
            _ => {}
        }
        false
    }

    /// Refuses a record shorter than its tag needs: counted, and the first of each tag in the
    /// log with the tag, the length and the length needed.
    fn refuse(&mut self, tag: u32, length: usize, needed: usize) {
        let counts = &mut self.renderer.counts;
        counts.short_records += 1;
        let bit = 1u64 << tag.min(63);
        if counts.refused_tags & bit == 0 {
            counts.refused_tags |= bit;
            println!("record of the tag {tag} ({}): {length} bytes, {needed} needed: refused (the first such record of this tag; the count is in the report)", tag::name(tag));
        }
    }

    /// A draw with its vertices in the record. This program does the vertex format of the
    /// films of the game: a position on the screen, a colour and one texture coordinate
    /// (D3DFVF_XYZRHW | D3DFVF_DIFFUSE | D3DFVF_TEX1, 28 bytes).
    pub(crate) fn draw_up(&mut self, data: &[u8]) {
        let (texture, vertex_format, stride, kind, primitives) = (word(data, 0), word(data, 4), word(data, 8) as usize, word(data, 12), word(data, 16) as usize);
        let vertices = &data[20..];
        if vertex_format != 0x144 || stride != 28 || !self.stream.back_buffer || !(4..=6).contains(&kind) {
            self.renderer.counts.screen_draws_lost += 1;
            return;
        }
        self.film_draws += 1;
        if self.film_draws >= self.film_shot_at {
            if let Some(path) = self.film_shot.take() {
                self.shots.push(path);
            }
        }
        let count = vertices.len() / stride;
        let size = self.size_of_target();
        let (width, height) = (size.0 as f32, size.1 as f32);
        // The centre of a pixel is at a whole number in Direct3D 8, and at a half here.
        let vertex = |i: usize| -> [f32; 10] {
            let v = &vertices[i * stride..];
            let c = color(word(v, 16));
            [(float(v, 0) + 0.5) / width * 2.0 - 1.0, 1.0 - (float(v, 4) + 0.5) / height * 2.0, float(v, 8).clamp(0.0, 1.0), 1.0, c[0], c[1], c[2], c[3], float(v, 20), float(v, 24)]
        };
        // A triangle list from a list, a strip or a fan.
        let mut list: Vec<f32> = Vec::new();
        for n in 0..primitives {
            let corners = match kind {
                4 => [n * 3, n * 3 + 1, n * 3 + 2],
                5 if n % 2 == 0 => [n, n + 1, n + 2],
                5 => [n + 1, n, n + 2],
                _ => [0, n + 1, n + 2],
            };
            if corners.iter().all(|i| *i < count) {
                for i in corners {
                    list.extend_from_slice(&vertex(i));
                }
            }
        }
        if list.is_empty() {
            return;
        }
        let renderer = &mut self.renderer;
        // The samples of the targets of the draw: the pass that draws it opens later.
        let samples = if self.target.is_none() { renderer.samples } else { 1 };
        let pipeline = renderer.screen_pipeline(samples);
        let clock = ticks::Clock::now();
        let buffer = renderer.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("screen draw"), contents: bytemuck::cast_slice(&list), usage: wgpu::BufferUsages::VERTEX });
        let view = renderer.textures.get(&texture).unwrap_or(&renderer.white).clone();
        let group = crate::gpu::frame_group(&renderer.device, &wgpu::BindGroupDescriptor {
            label: Some("screen draw"),
            layout: renderer.screen_layout.as_ref().unwrap(),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(renderer.screen_sampler.as_ref().unwrap()) },
            ],
        });
        renderer.counts.times[TIME_BUFFERS] += clock.elapsed();
        self.items.push(Item::Screen(ScreenDraw { pipeline, group, buffer, vertices: (list.len() / 10) as u32 }));
    }

    /// The size of the target that the game draws into.
    pub(crate) fn size_of_target(&self) -> (u32, u32) {
        self.target.and_then(|handle| self.renderer.targets.get(&handle)).map_or(self.frame_size, |texture| texture.size)
    }

    /// A shape record: the draws of the object from its geometry and material records, one
    /// for each pass of the state derived from its material.
    pub(crate) fn shape(&mut self, data: &[u8]) {
        let (geometry, material, flags, light_count, ambient) = (word(data, 0), word(data, 4), word(data, 8), word(data, 12) as usize, word(data, 16));
        self.renderer.counts.shape_records += 1;
        let colours: [f32; 14] = core::array::from_fn(|i| float(data, 84 + i * 4));
        let bone_count = word(data, 140) as usize;
        let lights_at = 20 + 64 + 56 + 4;
        // The stages of the texture effects (flag 16): the environment map, the fog map.
        let effects_at = lights_at + light_count * 4;
        let has_effects = flags & 16 != 0;
        // The fog of the draw (flag 32): colour, start, end, range fog.
        let fog_at = effects_at + if has_effects { 2 * EFFECT_STAGE_WORDS * 4 } else { 0 };
        let has_fog = flags & 32 != 0;
        let bones_at = fog_at + if has_fog { 16 } else { 0 };
        if data.len() < bones_at + bone_count * 64 {
            self.renderer.counts.shape_missing += 1;
            return;
        }
        let effects: Option<[[u32; EFFECT_STAGE_WORDS]; 2]> = has_effects.then(|| core::array::from_fn(|e| core::array::from_fn(|i| word(data, effects_at + (e * EFFECT_STAGE_WORDS + i) * 4))));
        self.stream.shape_fog = has_fog.then(|| core::array::from_fn(|i| word(data, fog_at + i * 4)));
        if let Some(fog) = self.stream.shape_fog {
            self.stream.fog = fog;
        }
        // The lights of the draw from their records: the key, then the D3DLIGHT8.
        let lights = &mut self.stream.shape.lights;
        lights.clear();
        for i in 0..light_count {
            let key = word(data, lights_at + i * 4);
            match self.renderer.lights_known.get(&key) {
                Some(values) => {
                    lights.extend_from_slice(&key.to_le_bytes());
                    lights.extend_from_slice(values);
                }
                None => self.renderer.counts.light_missing += 1,
            }
        }
        self.stream.shape.ambient = ambient;
        // The bones of a skinned shape go to the palette of the frame.
        self.stream.shape_bones = (bone_count != 0).then(|| {
            let first = self.frame_bones.len() as u32;
            for b in 0..bone_count {
                self.frame_bones.push(floats(data, bones_at + b * 64));
            }
            self.renderer.counts.shape_skinned += 1;
            (first, bone_count as u32)
        });
        // A particle system: its geometry for the frame comes from the record itself.
        if flags & 8 != 0 {
            let tail = bones_at + bone_count * 64;
            let count = if data.len() >= tail + 32 { word(data, tail) as usize } else { 0 };
            let at = tail + 32;
            if count == 0 || data.len() < at + count * 20 {
                self.renderer.counts.shape_missing += 1;
                return;
            }
            let axes: [f32; 6] = core::array::from_fn(|i| float(data, tail + 8 + i * 4));
            self.renderer.particles(geometry, float(data, tail + 4), [axes[0], axes[1], axes[2]], [axes[3], axes[4], axes[5]], &data[at..at + count * 20]);
        }
        // The world bound of the shape (flag 0x400): the last 16 bytes of the record.
        self.stream.shape_bound = (flags & 0x400 != 0 && data.len() >= 16).then(|| core::array::from_fn(|i| float(data, data.len() - 16 + i * 4)));
        self.stream.shape_queued = flags & 0x800 != 0;
        self.draw_shape(geometry, material, flags, core::array::from_fn(|i| float(data, 20 + i * 4)), &colours, effects.as_ref());
    }

    /// A geometry sent again under its handle (a morpher or a UV controller changed its
    /// vertices or its texture coordinates: a new revision of the same data object): the
    /// slots of the instances that draw it are made again, with the new ranges of the
    /// buffers. The instances come from a scan of the records: a frame changes few
    /// geometries.
    /// Engine-rules R6 (the skill wgpu-renderer-dev).
    pub(crate) fn geometry_changed(&mut self, handle: u32) {
        let changed = self.instances_of(handle);
        for (id, r) in changed {
            self.retained_insert(id, &r);
            self.retained.counts.refreshed += 1;
        }
    }

    /// The geometries that went in the last frame: an instance of the table that still names
    /// one leaves the table before the ranges of the geometry go back to the pools, because
    /// its slot holds those ranges. The plugin sends an instance's delete before the delete of
    /// its geometry, so this finds an instance only when a shape got other data while it lived
    /// (`counts.geometry_orphans`). A handle is the serial of one geometry data, so a later
    /// data never comes under it: the plugin sends the instance again with the handle of its
    /// new data (at its next check of the handles). One scan of the records for each frame
    /// with such deletes.
    pub(crate) fn orphans_of_gone_geometry(&mut self) {
        if self.gone_geometries.is_empty() {
            return;
        }
        let gone = std::mem::take(&mut self.gone_geometries);
        let orphans: Vec<u32> = gone.iter().flat_map(|&handle| self.instances_of(handle).into_iter().map(|(id, _)| id)).collect();
        for id in orphans {
            self.retained_remove(id);
            self.renderer.counts.geometry_orphans += 1;
        }
    }

    /// The materials of `MATERIAL_GONE` records go, with their derived states, when no
    /// instance names them: the slots of the table read their material at each frame. One
    /// scan of the instances in a frame after a new record, and in every 64th frame while some
    /// wait (an instance gets its handles noted again within 64 frames).
    pub(crate) fn drop_gone_materials(&mut self) {
        if self.gone_materials.is_empty() || !(std::mem::take(&mut self.gone_materials_new) || self.renderer.counts.frames % 64 == 0) {
            return;
        }
        let named: FxHashSet<u32> = self.instances.values().map(|r| r.material).collect();
        let (waiting, going): (Vec<u32>, Vec<u32>) = std::mem::take(&mut self.gone_materials).into_iter().partition(|handle| named.contains(handle));
        self.gone_materials = waiting;
        if going.is_empty() {
            return;
        }
        let going: FxHashSet<u32> = going.into_iter().collect();
        for handle in &going {
            if self.renderer.materials.remove(handle).is_some() {
                self.renderer.counts.materials_gone += 1;
            }
        }
        self.renderer.derived.retain(|(material, _), _| !going.contains(material));
    }

    /// An instance into `instances` and its id under its geometry in `by_geometry`.
    fn instance_put(&mut self, id: u32, retained: Retained) {
        if let Some(old) = self.instances.insert(id, retained) {
            if old.geometry == retained.geometry {
                return;
            }
            self.by_geometry_remove(old.geometry, id);
        }
        self.by_geometry.entry(retained.geometry).or_default().push(id);
    }

    /// An instance out of `instances` and `by_geometry`.
    fn instance_take(&mut self, id: u32) {
        if let Some(old) = self.instances.remove(&id) {
            self.by_geometry_remove(old.geometry, id);
        }
    }

    fn by_geometry_remove(&mut self, geometry: u32, id: u32) {
        if let Some(ids) = self.by_geometry.get_mut(&geometry) {
            if let Some(at) = ids.iter().position(|&other| other == id) {
                ids.swap_remove(at);
            }
            if ids.is_empty() {
                self.by_geometry.remove(&geometry);
            }
        }
    }

    /// The instances of the table whose geometry is `handle`.
    fn instances_of(&self, handle: u32) -> Vec<(u32, Retained)> {
        let Some(ids) = self.by_geometry.get(&handle) else { return Vec::new() };
        ids.iter().filter_map(|id| self.instances.get(id).map(|r| (*id, *r))).filter(|(_, r)| r.flags & INSTANCE_BLOCK != 0).collect()
    }

    /// An instance record: into the table, by its id.
    /// `move_check`: the id and the values of an `INSTANCE_MOVE` record just before.
    pub(crate) fn instance(&mut self, data: &[u8], move_check: Option<(u32, Retained)>) {
        let retained = Retained {
            class: word(data, 4),
            flags: word(data, 8),
            geometry: word(data, 12),
            material: word(data, 16),
            rotation: core::array::from_fn(|i| float(data, 20 + i * 4)),
            exact: core::array::from_fn(|i| double(data, 68 + i * 8)),
            stored: core::array::from_fn(|i| float(data, 92 + i * 4)),
            colours: core::array::from_fn(|i| float(data, 104 + i * 4)),
            bound: core::array::from_fn(|i| float(data, 160 + i * 4)),
        };
        let id = word(data, 0);
        if let Some((_, moved)) = move_check.filter(|(moved_id, _)| *moved_id == id) {
            self.renderer.counts.moves_checked += 1;
            if moved != retained {
                self.renderer.counts.moves_different += 1;
            }
        }
        self.instance_put(id, retained);
        self.renderer.counts.instance_records += 1;
        if retained.flags & INSTANCE_BLOCK != 0 {
            if !self.retained.update_place(id, &retained) {
                self.retained_insert(id, &retained);
            }
        } else {
            self.retained_remove(id);
        }
    }

    /// A move of a known instance ([`tag::INSTANCE_MOVE`]): the new rotation, exact and stored
    /// translations and bound over its last record, moved in place as a full record would be.
    pub(crate) fn instance_move(&mut self, data: &[u8]) {
        let id = word(data, 0);
        let Some(mut retained) = self.instances.get(&id).copied() else {
            self.renderer.counts.instance_missing += 1;
            return;
        };
        retained.rotation = core::array::from_fn(|i| float(data, 4 + i * 4));
        retained.exact = core::array::from_fn(|i| double(data, 52 + i * 8));
        retained.stored = core::array::from_fn(|i| float(data, 76 + i * 4));
        retained.bound = core::array::from_fn(|i| float(data, 88 + i * 4));
        self.instance_put(id, retained);
        self.renderer.counts.instance_moves += 1;
        self.move_check = Some((id, retained));
        if retained.flags & INSTANCE_BLOCK != 0 && !self.retained.update_place(id, &retained) {
            self.retained_insert(id, &retained);
        }
    }

    /// A state of the UV controller of a geometry ([`tag::GEOMETRY_UV`]).
    pub(crate) fn geometry_uv_record(&mut self, data: &[u8]) {
        let (handle, unique, set_flags) = (word(data, 0), word(data, 4), word(data, 8));
        let state: [f32; 4] = core::array::from_fn(|i| float(data, 12 + i * 4));
        let Some(geometry) = self.renderer.geometries.get(&handle).filter(|g| g.unique == Some(unique)) else {
            self.renderer.counts.uv_missing += 1;
            return;
        };
        let set = set_flags & 0xFFFF;
        let uv = if set_flags >> 16 & 1 != 0 {
            self.renderer.counts.uv_bases += 1;
            crate::uv_animation::UvAnimation { set, base: state, now: state }
        } else if let Some(uv) = geometry.uv.filter(|uv| uv.set == set) {
            self.renderer.counts.uv_steps += 1;
            crate::uv_animation::UvAnimation { now: state, ..uv }
        } else {
            self.renderer.counts.uv_missing += 1;
            return;
        };
        self.geometry_uv(handle, Some(uv));
    }

    /// Sets the UV state of a geometry, and the texture transforms of the table's slots of
    /// its instances.
    pub(crate) fn geometry_uv(&mut self, handle: u32, uv: Option<crate::uv_animation::UvAnimation>) {
        if let Some(geometry) = self.renderer.geometries.get_mut(&handle) {
            geometry.uv = uv;
        }
        if let Some(ids) = self.by_geometry.get(&handle) {
            for &id in ids {
                self.retained.set_uv(id, uv.as_ref());
            }
        }
    }

    /// The place of a sorted blend of the table in the flush of the engine's queue: it waits
    /// with the engine's queued draws and keeps its place among them
    /// (`draw_retained_sorted`). Its key for the blends without a place is the centre of its
    /// bound. The place counts in a scene of the world camera in the back buffer, also
    /// before the world began: in a view where the table holds every shape of the world the
    /// engine draws nothing before its queue flushes, and the world begins at the scene's
    /// end (`Plan::scene_end`), before the sorted blends of the scene.
    pub(crate) fn instance_queued(&mut self, id: u32) {
        let Some(centre) = self.retained.centre_of(id) else {
            self.renderer.counts.queued_unknown += 1;
            return;
        };
        let kind = self.plan.scene_kind;
        let world_camera = kind == engine::scene::SCENE_WORLD || engine::scene::later_world_scene(kind);
        if !world_camera || self.target.is_some() {
            self.renderer.counts.queued_outside += 1;
            return;
        }
        let sequence = self.frame_sorted.len() as u32;
        let generation = self.plan.defer();
        if generation != 0 {
            // Risk 34: a place after the frame's first sorted blends: the kept list was taken.
            self.renderer.counts.queued_late_places += 1;
        }
        self.frame_sorted.push((centre, sequence, crate::Waiting::Table(id), generation));
    }

    /// The place of a blend of the table in the engine's walk ([`tag::INSTANCE_WALK`]): its
    /// slots draw here, among the draws of the engine's path, in the walk's order. Places
    /// with no other item between them draw as one run. A place counts in a scene of the world
    /// camera in the back buffer after the world began: the table's main view draws at the
    /// start of the world.
    /// Engine-rules R2, R3, R22 (the skill wgpu-renderer-dev).
    pub(crate) fn walk_place(&mut self, id: u32) {
        let world_view = self.plan.world_begun && scene::world_camera(self.plan.scene_kind) && self.stream.back_buffer && self.target.is_none();
        if !world_view {
            self.renderer.counts.walk_outside += 1;
            return;
        }
        let adjacent = matches!(self.items.last(), Some(Item::WalkPlaces(_)));
        match self.retained.walk.place(id, self.retained.centre_of(id).is_some(), adjacent) {
            engine::walk::Place::Unknown => self.renderer.counts.walk_unknown += 1,
            engine::walk::Place::Joined => self.renderer.counts.walk_places += 1,
            engine::walk::Place::New(run) => {
                self.renderer.counts.walk_places += 1;
                self.items.push(Item::WalkPlaces(run));
            }
        }
    }

    /// A draw of an instance: the draws of the shape from the instance, the ambient light
    /// and the lights of the record. The translation of the world matrix is the exact one
    /// of the instance relative to the origin of the scene, as the plugin makes it for a
    /// shape record.
    pub(crate) fn instance_draw(&mut self, data: &[u8]) {
        let (id, ambient, flags) = (word(data, 0), word(data, 4), word(data, 8));
        // The stages of the texture effects (flag 16), as the shape record has them: the
        // engine's, for the camera of this draw.
        let has_effects = flags & 16 != 0;
        let fog_at = if has_effects { 12 + 2 * EFFECT_STAGE_WORDS * 4 } else { 12 };
        let has_fog = flags & 32 != 0;
        let count_at = if has_fog { fog_at + 16 } else { fog_at };
        if data.len() < count_at + 4 {
            self.renderer.counts.shape_missing += 1;
            return;
        }
        let light_count = word(data, count_at) as usize;
        let lights_at = count_at + 4;
        if data.len() < lights_at + light_count * 4 {
            self.renderer.counts.shape_missing += 1;
            return;
        }
        let Some(retained) = self.instances.get(&id).copied() else {
            self.renderer.counts.instance_missing += 1;
            return;
        };
        self.renderer.counts.instance_draws += 1;
        self.stream.object_class = retained.class;
        // A draw of the flush of the engine's sorted queue (0x800), as the shape record has it,
        // with the world bound of the instance as the key of its sort (the shape record's
        // bound words).
        self.stream.shape_queued = flags & 0x800 != 0;
        self.stream.shape_bound = Some(retained.bound);
        let effects: Option<[[u32; EFFECT_STAGE_WORDS]; 2]> = has_effects.then(|| core::array::from_fn(|e| core::array::from_fn(|i| word(data, 12 + (e * EFFECT_STAGE_WORDS + i) * 4))));
        self.stream.shape_fog = has_fog.then(|| core::array::from_fn(|i| word(data, fog_at + i * 4)));
        if let Some(fog) = self.stream.shape_fog {
            self.stream.fog = fog;
        }
        self.stream.shape_bones = None;
        let lights = &mut self.stream.shape.lights;
        lights.clear();
        for i in 0..light_count {
            let key = word(data, lights_at + i * 4);
            match self.renderer.lights_known.get(&key) {
                Some(values) => {
                    lights.extend_from_slice(&key.to_le_bytes());
                    lights.extend_from_slice(values);
                }
                None => self.renderer.counts.light_missing += 1,
            }
        }
        self.stream.shape.ambient = ambient;
        // A skinned instance (flag 4): the palette of its bones follows the lights, and its
        // world is the identity, as the shape record of a skinned draw has it.
        let skinned = flags & 4 != 0;
        if skinned {
            let bones_at = lights_at + light_count * 4;
            let bone_count = if data.len() >= bones_at + 4 { word(data, bones_at) as usize } else { 0 };
            if bone_count == 0 || data.len() < bones_at + 4 + bone_count * 64 {
                self.renderer.counts.shape_missing += 1;
                return;
            }
            let first = self.frame_bones.len() as u32;
            for b in 0..bone_count {
                self.frame_bones.push(floats(data, bones_at + 4 + b * 64));
            }
            self.renderer.counts.shape_skinned += 1;
            self.stream.shape_bones = Some((first, bone_count as u32));
        }
        let mut world = IDENTITY;
        if !skinned {
            world[..12].copy_from_slice(&retained.rotation);
            // The translation as the shape record of the draw would have it: the exact one
            // less the origin of the scene for a relative draw, the stored one of the engine
            // else.
            let relative = retained.flags & 1 != 0;
            for i in 0..3 {
                world[12 + i] = match (relative, self.stream.origin) {
                    (true, Some(origin)) => (retained.exact[i] - origin[i]) as f32,
                    (true, None) => retained.exact[i] as f32,
                    (false, _) => retained.stored[i],
                };
            }
            world[15] = 1.0;
        }
        self.draw_shape(retained.geometry, retained.material, retained.flags | (flags & 32), world, &retained.colours, effects.as_ref());
        self.stream.shape_bones = None;
    }

    /// The draws of a shape from its geometry and material records: one for each pass of
    /// the state derived from its material. `world` is the world matrix of the draw, with
    /// the flag 1 when it is relative to the origin of the scene.
    pub(crate) fn draw_shape(&mut self, geometry: u32, material: u32, flags: u32, world: [f32; 16], colours: &[f32; 14], effects: Option<&[[u32; EFFECT_STAGE_WORDS]; 2]>) {
        let Some((fvf, stride, entries, strips)) = self.renderer.geometries.get(&geometry).map(|g| (g.fvf, g.stride, g.entries, g.strips.clone())) else {
            self.renderer.counts.shape_missing += 1;
            return;
        };
        let sets = (fvf >> 8) & 0xF;
        let Some(m) = self.renderer.materials.get(&material).cloned() else {
            self.renderer.counts.shape_missing += 1;
            return;
        };
        self.renderer.counts.shape_flat += (m.flags & material::SMOOTH == 0) as u32;
        self.renderer.counts.shape_specular += (m.flags & material::SPECULAR != 0) as u32;
        let derived = match self.renderer.derived.get(&(material, sets)) {
            Some(d) => d.clone(),
            None => {
                let d = Rc::new(material::derive(&m, sets, self.stream.envmap_lit));
                self.renderer.derived.insert((material, sets), d.clone());
                d
            }
        };
        let shape = &mut self.stream.shape;
        shape.geometry = geometry;
        shape.fvf = fvf;
        shape.stride = stride;
        shape.world = world;
        shape.relative = flags & 1 != 0;
        shape.material = material::d3d_material(colours);
        shape.fill = derived.fill;
        if let Some(source) = derived.sources[2] {
            self.stream.ambient_source = source;
        }
        if self.hold_shape(&m, &derived, flags, effects, entries, &strips) {
            self.stream.shape_bones = None;
            return;
        }
        for pass in 0..derived.passes.len() {
            let state = Pass::new(&derived, pass, &m, &self.renderer.textures, effects);
            // The flag 128: the draw goes into the picture of the character with the Code
            // Patch, which draws a pass that blends in a way of its own. The alpha of the
            // material is the last of its colours.
            let (state, second) = if flags & 128 != 0 { state.code_patch_picture(colours[13]) } else { (state, None) };
            // The primitive type of Direct3D, the first index and the number of indexes.
            // The flag 64: lines, as a line list of the index entries of the geometry.
            if flags & 64 != 0 {
                for state in core::iter::once(&state).chain(&second) {
                    self.draw(&derived, state, 2, 0, entries / 2 * 2);
                }
            } else if strips.is_empty() {
                for state in core::iter::once(&state).chain(&second) {
                    self.draw(&derived, state, 4, 0, entries / 3 * 3);
                }
            } else {
                // The Code Patch makes its draws at each draw call: for each strip.
                let mut start = 0u32;
                for &length in &strips {
                    for state in core::iter::once(&state).chain(&second) {
                        self.draw(&derived, state, 5, start, (length as u32).max(2));
                    }
                    start += length as u32;
                }
            }
            self.renderer.counts.shape_draws += 1;
        }
        self.stream.shape_bones = None;
    }

    /// Gives the shape in progress to the extension when it takes it: see [`Extension::hold`].
    /// True when the extension took it; then none of its passes is drawn.
    pub(crate) fn hold_shape(&mut self, m: &material::Material, derived: &material::Derived, flags: u32, effects: Option<&[[u32; EFFECT_STAGE_WORDS]; 2]>, entries: u32, strips: &[u16]) -> bool {
        let class = self.stream.object_class;
        // Flag 8 is a particle system, 64 lines, 128 a draw into the picture of the
        // character. Classes 1 to 8 are the sky and the glare of the sun, 10 the water plane.
        let world = self.extension.is_some()
            && self.stream.back_buffer
            && self.target.is_none()
            && scene::world_camera(self.plan.scene_kind)
            && !scene::sky_class(class)
            && class != scene::CLASS_WATER_PLANE
            && flags & (8 | 64 | 128) == 0
            && self.stream.shape_bones.is_none()
            && !derived.passes.is_empty();
        if !world {
            return false;
        }
        // The answers of the frame, as `draw` asks for them at the first draw of the world
        // scene.
        if self.parameters.is_none() && self.plan.scene_kind == scene::SCENE_WORLD {
            self.ask_frame();
        }
        let Some((true, fog_range)) = self.parameters.as_ref().map(|p| (p.hold, p.fog_range)) else { return false };
        let shape = &self.stream.shape;
        let Some(layout) = VertexLayout::of(shape.fvf, shape.stride) else { return false };
        let frame = self.renderer.geometry_frame;
        let Some((vertices, indexes, base_vertex, geometry_first)) = self.renderer.geometries.get_mut(&shape.geometry).filter(|g| entries <= g.entries).map(|g| {
            g.drawn_in = frame;
            (g.vertex.clone(), g.index.clone(), g.base_vertex, g.first_index)
        }) else {
            return false;
        };
        let pass = Pass::new(derived, 0, m, &self.renderer.textures, effects);
        let facts = DrawFacts {
            class,
            blend_enable: pass.blend[0] != 0,
            source_blend: pass.blend[1],
            destination_blend: pass.blend[2],
            depth_write: derived.depth[1] != 0,
            specular_power: shape.material[16],
            emissive: shape.material[12..16].try_into().unwrap(),
        };
        if !self.extension.as_mut().is_some_and(|extension| extension.hold(&facts)) {
            return false;
        }
        let (topology, parts) = if strips.is_empty() {
            (wgpu::PrimitiveTopology::TriangleList, vec![(geometry_first, entries / 3 * 3)])
        } else {
            let mut start = geometry_first;
            let parts = strips
                .iter()
                .map(|&length| {
                    let part = (start, (length as u32).max(2));
                    start += length as u32;
                    part
                })
                .collect();
            (wgpu::PrimitiveTopology::TriangleStrip, parts)
        };
        let maps = self.renderer.held_maps(m, layout.coordinates.len() as u32);
        self.stream.fog_range = fog_range;
        self.stream.sky = None;
        let mut lights = Vec::new();
        let mut data = self.stream.draw_data(derived, &pass, &mut self.renderer.counts, &mut lights);
        if let Some(uv) = self.renderer.geometries.get(&self.stream.shape.geometry).and_then(|g| g.uv) {
            crate::uv_animation::compose(&mut data, &uv);
        }
        let origin = self.stream.origin.filter(|_| self.stream.shape.relative);
        let held = HeldDraw {
            facts,
            vertices,
            base_vertex,
            indexes,
            layout,
            topology,
            parts,
            data,
            origin,
            lights,
            maps,
            depth: [(derived.depth[0] != 0) as u32, (derived.depth[1] != 0) as u32, derived.depth[2]],
            cull: derived.cull,
            stencil: derived.stencil,
        };
        if self.extension.as_ref().is_some_and(|extension| extension.held_in_place(&held)) {
            let facts = self.world_facts(false);
            self.items.push(Item::HeldHere(Box::new(held), Box::new(facts)));
        } else {
            self.taken.push(held);
        }
        self.renderer.counts.held += 1;
        true
    }
}
