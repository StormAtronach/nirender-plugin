//! A draw of the game into the pass: its pipeline, its groups and its data (`draw`), and
//! the issue of a draw that the items hold (`issue`).

use crate::*;

impl Player {
    /// Puts a draw into the pass.
    pub(crate) fn issue(&mut self, draw: &Draw) {
        let renderer = &mut self.renderer;
        let clock = ticks::Clock::now();
        // A world draw that blends and writes no depth (not the sky), with the call of a
        // class with a plane in the frame: on the far side before the call, on the side of
        // the camera after it. The call draws the other side again. A draw that writes depth
        // needs no split: the depth test of the call keeps it in front where it is.
        let side = match (self.split, self.plane_ahead) {
            (Some(plane), _) => Some(plane),
            (None, Some(plane)) => Some(plane.map(|v| -v)),
            (None, None) => None,
        };
        let split = draw.world && draw.translucent() && ReplayView::WORLD & (1 << draw.class.min(31)) != 0;
        // The instances of the frame come after the segment of the retained instances.
        let mut instances = (renderer.retained_instances as u32 + draw.instances.0, draw.instances.1);
        if let Some(plane) = side.filter(|_| split) {
            // A copy of the record with the plane, and instances that name it.
            let mut data = draw.data;
            data.clip_plane = plane_of_draw(plane, draw.origin);
            let (start, n) = (draw.instances.0 as usize, draw.instances.1 as usize);
            match renderer.copy_of_draw(&data, &self.frame_instances[start..start + n]) {
                Some(first) => instances = (first, draw.instances.1),
                None => renderer.counts.no_record += 1,
            }
        }
        renderer.counts.times[TIME_DRAW_UPLOAD] += clock.elapsed();
        let clock = ticks::Clock::now();
        let Some(pipeline) = renderer.pipeline(draw.key) else {
            renderer.counts.no_layout += 1;
            return;
        };
        renderer.counts.times[TIME_DRAW_GROUPS] += clock.elapsed();
        self.begin_pass();
        let renderer = &mut self.renderer;
        let clock = ticks::Clock::now();
        let Some(pass) = self.pass.as_mut() else { return };
        // A change of the state ends the run that is open.
        let same = self.pass_pipeline.as_ref().is_some_and(|bound| Rc::ptr_eq(bound, &pipeline))
            && self.pass_stencil == Some(draw.stencil_reference)
            && self.pass_vertices.as_ref().is_some_and(|bound| Rc::ptr_eq(bound, &draw.vertices))
            && self.pass_indexes.as_ref().is_some_and(|bound| Rc::ptr_eq(bound, &draw.indexes))
            && self.bound.is(0, Some(&renderer.frame_group));
        if !same {
            flush_run(pass, renderer, &mut self.run_args, &mut self.frame_args);
        }
        let mut new_run = false;
        if !self.pass_pipeline.as_ref().is_some_and(|bound| Rc::ptr_eq(bound, &pipeline)) {
            pass.set_pipeline(&pipeline);
            self.pass_pipeline = Some(pipeline.clone());
            new_run = true;
        }
        if self.pass_stencil != Some(draw.stencil_reference) {
            pass.set_stencil_reference(draw.stencil_reference);
            self.pass_stencil = Some(draw.stencil_reference);
            new_run = true;
        }
        renderer.counts.pipeline_runs += new_run as u32;
        // `MGE_XE_RUNS_DUMP=1`: a line for each draw, for a study of the runs offline.
        if renderer.runs_dump {
            println!(
                "run frame {} pipeline {:?} stencil {} world {} class {} scene {} blend {:?} depth {:?} textures {:?} samplers {:?} vertices {:?} indexes {:?} range {:?} instances {:?}",
                renderer.counts.frames,
                Rc::as_ptr(&pipeline),
                draw.stencil_reference,
                draw.world as u8,
                draw.class,
                self.plan.scene_kind,
                draw.key.blend,
                draw.key.depth,
                draw.texture_handles,
                draw.data.stages.map(|stage| stage.slots[1]),
                Rc::as_ptr(&draw.vertices),
                Rc::as_ptr(&draw.indexes),
                draw.range,
                instances
            );
        }
        self.bound.set(pass, 0, &renderer.frame_group);
        if !self.pass_arrays {
            let group = renderer.array_group_for(self.pass_target);
            pass.set_bind_group(1, &*group, &[]);
            crate::gpu::note_array_bind(crate::gpu::BindOrigin::Game);
            self.pass_arrays = true;
        }
        if !self.pass_vertices.as_ref().is_some_and(|bound| Rc::ptr_eq(bound, &draw.vertices)) {
            pass.set_vertex_buffer(0, draw.vertices.slice(..));
            self.pass_vertices = Some(draw.vertices.clone());
        }
        if !self.pass_shared {
            // The defaults and the instances of the frame: the same buffers for every draw
            // of the pass. A draw takes its instances by the first instance of the draw.
            pass.set_vertex_buffer(1, renderer.defaults.slice(..));
            pass.set_vertex_buffer(2, renderer.instance_buffer.slice(..));
            self.pass_shared = true;
        }
        let (first, count) = draw.range;
        if !self.pass_indexes.as_ref().is_some_and(|bound| Rc::ptr_eq(bound, &draw.indexes)) {
            pass.set_index_buffer(draw.indexes.slice(..), wgpu::IndexFormat::Uint16);
            self.pass_indexes = Some(draw.indexes.clone());
        }
        if renderer.indirect && self.frame_args.len() + self.run_args.len() < renderer.indirect_capacity {
            // The draw joins the run; the run becomes one indirect call at its end.
            self.run_args.push(IndirectArgs { index_count: count, instance_count: instances.1, first_index: first, base_vertex: draw.base_vertex, first_instance: instances.0 });
        } else {
            if renderer.indirect {
                renderer.counts.indirect_over += 1;
            }
            flush_run(pass, renderer, &mut self.run_args, &mut self.frame_args);
            pass.draw_indexed(first..first + count, draw.base_vertex, instances.0..instances.0 + instances.1);
        }
        renderer.counts.times[TIME_DRAW_PASS] += clock.elapsed();
        renderer.counts.draws += 1;
    }

    /// The end of the sky's walk in the world camera's Click ([`EVENT_WORLD_WALK`]): the world
    /// of the frame starts here, from the engine's walk, not from a draw.
    pub(crate) fn world_walk(&mut self) {
        let in_back_buffer = self.stream.back_buffer && self.target.is_none();
        for step in self.plan.world_walk(self.extension.is_some(), in_back_buffer) {
            self.plan_step(step);
        }
    }

    /// The end of the flush of the engine's main sorted queue ([`EVENT_QUEUE_FLUSHED`]): the
    /// sorted blends of the frame go here.
    pub(crate) fn queue_flushed(&mut self) {
        let in_back_buffer = self.stream.back_buffer && self.target.is_none();
        for step in self.plan.queue_flushed(in_back_buffer) {
            self.plan_step(step);
        }
    }

    /// The target of a scene from its record ([`SCENE_BACK_BUFFER`]): the renderer's target at
    /// `BeginPaint`. The back buffer ends a texture target of the calls; a texture that this
    /// program did not make takes the draws nowhere. A change of the calls' target is counted.
    pub(crate) fn scene_target(&mut self, back_buffer: bool) {
        self.renderer.counts.scene_targets += 1;
        let from_calls = self.stream.back_buffer && self.target.is_none();
        if from_calls != back_buffer {
            self.renderer.counts.scene_target_changed += 1;
        }
        if back_buffer {
            self.stream.back_buffer = true;
            if self.target.take().is_some() && self.drawing() {
                self.items.push(Item::Target(None));
            }
        } else if self.target.is_none() {
            self.stream.back_buffer = false;
        }
    }

    /// Collects one draw of a pass of the shape in progress: `topology` is the primitive
    /// type of Direct3D (2 a line list, 4 a triangle list, 5 a triangle strip), over
    /// `indexes` entries of the index list of the geometry from `first_index`.
    /// Engine-rules R21 (the skill wgpu-renderer-dev).
    pub(crate) fn draw(&mut self, derived: &material::Derived, pass: &Pass, topology: u32, first_index: u32, indexes: u32) {
        if !self.stream.back_buffer {
            self.renderer.counts.other_target += 1;
            return;
        }
        let in_back_buffer = self.target.is_none();
        if in_back_buffer && self.plan.scene_kind == scene::SCENE_FIRST_PERSON {
            self.first_person_drawn = true;
        }
        // The first draw of the world scene (kind 1) into the back buffer: the answers of
        // the extension for the frame.
        if self.extension.is_some() && self.parameters.is_none() && in_back_buffer && self.plan.scene_kind == scene::SCENE_WORLD {
            self.ask_frame();
        }
        // The world of the game starts with the first draw of its world scene into the
        // back buffer that is not a part of the sky and comes after the sky: the classes 1
        // to 7 are the sky and 8 is the glare of the sun. The world scene is one
        // `NiCamera::Click`: the engine draws a geometry without alpha blending as it comes
        // to it in the scene graph (`NiAlphaAccumulator::RegisterObject` 0x429B90 keeps
        // the blended ones for the end), so a draw can come before the sky (the lines of
        // the cell borders); it is not the world. With the sky hidden (the scene packet:
        // a plain interior cell, or ToggleSky) there is no sky draw: the world starts at
        // the first draw.
        let sky = scene::sky_class(self.stream.object_class);
        let (steps, hold) = self.plan.world_draw(self.extension.is_some(), in_back_buffer, sky, self.stream.sky_hidden, self.stream.world_walk);
        for step in steps {
            self.plan_step(step);
        }
        // A draw of a class that the extension takes: the extension draws at the first one.
        let class = self.stream.object_class;
        let (watched, shade, fog_range, sky) = self.parameters.as_ref().map_or((0, false, None, None), |p| (p.watched, p.shade, p.fog_range, p.sky));
        if in_back_buffer && class < 32 && watched & (1 << class) != 0 {
            if self.replaced & (1 << class) == 0 {
                self.replaced |= 1 << class;
                self.renderer.counts.class_calls += 1;
                let facts = self.world_facts(false);
                self.items.push(Item::Replace(class, Box::new(facts), self.world_draws.len()));
            }
            return;
        }
        let renderer = &mut self.renderer;
        let shape = &self.stream.shape;
        let stencil = derived.stencil;
        let key = PipelineKey {
            fvf: shape.fvf,
            stride: shape.stride,
            topology,
            blend: pass.blend,
            depth: [(derived.depth[0] != 0) as u32, (derived.depth[1] != 0) as u32, derived.depth[2]],
            cull: derived.cull,
            fill: derived.fill,
            // The back buffer of the game has no alpha (X8R8G8B8): a draw into it writes
            // none, and the alpha there stays 1, as a read of such a target gives.
            color_write: if in_back_buffer { pass.color_write & 7 } else { pass.color_write },
            // On, fail, depth fail, pass, function, mask, write mask.
            stencil: if stencil[0] == 0 { [0; 7] } else { [1, stencil[4], stencil[5], stencil[6], stencil[1], stencil[3], !0] },
            // The samples of the targets that the draw goes into, known now: the pass that
            // draws it opens later, when the items run.
            samples: if in_back_buffer { renderer.samples } else { 1 },
        };
        let stencil_reference = stencil[2];
        if self.stream.shape.geometry == crate::dump_wanted() {
            println!("key dump record path: fvf {} stride {} topology {} blend {:?} depth {:?} cull {} fill {} color_write {} stencil {:?} reference {} textures {:?} samplers {:?} stages {} passes {} range {}+{} base {}", key.fvf, key.stride, key.topology, key.blend, key.depth, key.cull, key.fill, key.color_write, key.stencil, stencil_reference, pass.stages.iter().map(|s| s.texture).collect::<Vec<_>>(), pass.stages.iter().map(|s| s.sampler).collect::<Vec<_>>(), pass.stages.len(), derived.passes.len(), first_index, indexes, self.stream.shape.geometry);
        }
        // The buffers of the geometry record of the shape.
        let Some((vertex_buffer, index_buffer, base_vertex, geometry_first)) = renderer.geometries.get(&shape.geometry).filter(|g| first_index + indexes <= g.entries).map(|g| (g.vertex.clone(), g.index.clone(), g.base_vertex, g.first_index)) else {
            renderer.counts.geometry_short += 1;
            return;
        };
        renderer.counts.geometry_draws += 1;
        let clock = ticks::Clock::now();
        // The answers of the extension: the light for each pixel and the fog distances for
        // the world in the back buffer (not for the sky, which is before the world, and not
        // for a texture that the game draws into), the sky for a sky draw in the back buffer
        // (MGE_LINK_CLASS_SKY_DOME is 1 and MGE_LINK_CLASS_SKY_OTHER is 7).
        // The views of the world: the world camera's scenes and the first-person scene, by the
        // kind that the plugin gives, not by the projection. The matrices of the post chain
        // come from the world camera only.
        let kind = self.plan.scene_kind;
        let world = self.plan.world_begun && in_back_buffer && (scene::world_camera(kind) || kind == scene::SCENE_FIRST_PERSON);
        if world && self.stream.projection[11] != 1.0 {
            renderer.counts.orthographic_world_draws += 1;
        }
        if world && scene::world_camera(kind) {
            self.world_matrices = Some((self.stream.view, self.stream.projection));
        }
        self.stream.fog_range = fog_range.filter(|_| world);
        self.stream.sky = sky.filter(|_| in_back_buffer && (1..=7).contains(&self.stream.object_class));
        let mut draw_data = self.stream.draw_data(derived, pass, &mut renderer.counts, &mut self.frame_lights);
        if let Some(uv) = renderer.geometries.get(&self.stream.shape.geometry).and_then(|g| g.uv) {
            crate::uv_animation::compose(&mut draw_data, &uv);
        }
        crate::draw_dump("record path", self.stream.shape.geometry, &draw_data);
        if self.stream.shape.geometry == crate::dump_wanted() {
            let (first, count) = (draw_data.light_range[0] as usize, draw_data.light_range[1] as usize);
            crate::light_dump("record path", &self.frame_lights[first..first + count]);
        }
        let mut shaded = false;
        if shade && world {
            if let Some(extension) = self.extension.as_mut() {
                let (first, count) = (draw_data.light_range[0] as usize, draw_data.light_range[1] as usize);
                let facts = DrawFacts {
                    class,
                    blend_enable: key.blend[0] != 0,
                    source_blend: key.blend[1],
                    destination_blend: key.blend[2],
                    depth_write: key.depth[1] != 0,
                    specular_power: shape.material[16],
                    emissive: shape.material[12..16].try_into().unwrap(),
                };
                extension.shade(&mut draw_data, &mut self.frame_lights[first..first + count], &facts);
                crate::draw_dump("record path, shaded", self.stream.shape.geometry, &draw_data);
                shaded = true;
            }
        }
        // The lights of a draw have keys: they are candidates for the clusters of the frame,
        // for the extension (`FrameLights`), and the draw that the extension shades can take
        // its light from them.
        let masked = shaded && renderer.clusters.on && !renderer.clusters.world;
        let extension_lights = self.parameters.as_ref().is_some_and(|p| p.lights) && world;
        let first = draw_data.light_range[0] as usize;
        let relative = shape.relative && self.stream.origin.is_some();
        // An emissive-only draw has no light (`Derived::no_lights`): none in its mask.
        let noted = if derived.no_lights { 0 } else { shape.lights.len() / STATE_LIGHT_SIZE };
        for (i, light) in shape.lights.chunks_exact(STATE_LIGHT_SIZE).take(noted).enumerate() {
            if masked {
                if let Some(slot) = renderer.clusters.note(word(light, 0), (first + i) as u32, relative, self.stream.origin) {
                    draw_data.light_mask[slot as usize / 128][slot as usize / 32 % 4] |= 1 << (slot % 32);
                }
            }
            if extension_lights {
                renderer.extension_lights.note(word(light, 0), (first + i) as u32, relative, self.stream.origin);
            }
        }
        if masked {
            draw_data.light_range[3] |= 1;
        }
        // The lights of the world (`MGE_XE_LIGHTS=world`): a lit draw of the world, relative to
        // the camera, takes the point lights of its pixel's cluster in place of its own.
        if shaded && world && relative && renderer.clusters.world && !derived.no_lights {
            draw_data.light_range[3] |= crate::WORLD_LIGHTS_BIT;
        }
        renderer.counts.times[TIME_DRAW_DATA] += clock.elapsed();
        let clock = ticks::Clock::now();
        let mut handles = [0u32; STAGES];
        let mut sampler_values = [[0u32; 5]; STAGES];
        for stage in 0..STAGES {
            let handle = pass.stages[stage].texture;
            let used = stage < draw_data.counts[1] as usize && handle != 0;
            if used && !renderer.textures.contains_key(&handle) {
                renderer.counts.missing_textures += 1;
            } else if used {
                handles[stage] = handle;
                sampler_values[stage] = pass.stages[stage].sampler;
            }
        }
        // The textures and the samplers of the draw in the arrays of the frame: the record
        // names their slots.
        for stage in 0..STAGES {
            let texture = renderer.frame_texture_slot(handles[stage]);
            let sampler = renderer.frame_sampler_slot(sampler_values[stage]);
            draw_data.stages[stage].slots = [texture, sampler, 0, 0];
        }
        renderer.counts.times[TIME_DRAW_GROUPS] += clock.elapsed();
        if self.target.is_some() {
            renderer.counts.target_draws += 1;
        }
        // The record of the draw, and the instance that names it.
        let Some(record) = renderer.record(&draw_data) else { return };
        let instance = self.frame_instances.len() as u32;
        self.frame_instances.push(Instance { world: IDENTITY, record: [record, 0, 0, 0] });
        let draw = Rc::new(Draw {
            class,
            // The world scene of the game is 1, with the sky at its start; its sorted draws
            // come in a scene of their own (2), or with the water plane in the scene of the
            // water (3). The water plane of the game (MGE_LINK_CLASS_WATER_PLANE, 10) is not
            // a thing of the world.
            world: in_back_buffer && scene::world_camera(self.plan.scene_kind) && class != scene::CLASS_WATER_PLANE,
            key,
            stencil_reference,
            data: draw_data,
            origin: self.stream.origin.filter(|_| shape.relative),
            vertices: vertex_buffer,
            indexes: index_buffer,
            texture_handles: handles,
            range: (geometry_first + first_index, indexes),
            base_vertex,
            instances: (instance, 1),
        });
        if draw.world {
            self.world_draws.push(draw.clone());
        }
        if hold {
            self.held_draws.push(Item::Draw(draw));
        } else if draw.world && self.plan.world_begun && self.target.is_none() && (if self.stream.queue_marked { self.stream.shape_queued } else { engine::alpha::sorted_by_engine(class, &derived.blends, derived.no_sorter) }) {
            // A draw of the flush of the engine's main sorted queue (the plugin marks it): it
            // waits for the sorted blends of the table, and the table's blends go between the
            // engine's draws by depth (`draw_retained_sorted`). A blend that the engine draws
            // during the walk (no sorter, a sort adjust node in the modes off and sub-sort)
            // stays in place. An older capture has no mark: there a blend of the material
            // waits. The centre of its bound is the key; a record without one (an older
            // capture) gives the translation of the draw.
            let centre = match self.stream.shape_bound {
                Some(b) => [b[0], b[1], b[2]],
                None => {
                    let (w, o) = (&draw.data.world[0], draw.origin.unwrap_or([0.0; 3]));
                    [(w[12] as f64 + o[0]) as f32, (w[13] as f64 + o[1]) as f32, (w[14] as f64 + o[2]) as f32]
                }
            };
            let sequence = self.frame_sorted.len() as u32;
            self.frame_sorted.push((centre, sequence, crate::Waiting::Draw(draw), self.plan.defer()));
        } else {
            self.items.push(Item::Draw(draw));
        }
    }
}
