//! The items of a frame: their run, the steps of the plan, the world's start and end,
//! the parts the extension draws (the replaced classes, the held draws, the post).

use crate::*;

impl Player {
    /// Does the items of the frame that are not done yet. See `Item`.
    pub(crate) fn run_items(&mut self) {
        let items = std::mem::take(&mut self.items);
        if !self.drawing() {
            return;
        }
        let clock = ticks::Clock::now();
        let inside_before = sum_except(&self.renderer.counts.times, TIME_ITEMS);
        self.run_items_inner(items);
        let inside_after = sum_except(&self.renderer.counts.times, TIME_ITEMS);
        self.renderer.counts.times[TIME_ITEMS] += clock.elapsed().saturating_sub(inside_after - inside_before);
    }

    pub(crate) fn run_items_inner(&mut self, items: Vec<Item>) {
        let mut items = items;
        self.plane_ahead = self.parameters.as_ref().and_then(|p| p.class_plane).filter(|(class, _)| items.iter().any(|item| matches!(item, Item::Replace(c, ..) if c == class))).map(|(_, plane)| plane);
        if self.renderer.order_opaque_first {
            Self::order_opaque_first(&mut items);
        }
        // The origin of the instances follows the camera of the world of the frame.
        let world_origin = items.iter().find_map(|item| match item {
            Item::WorldStart(facts) => Some(facts.origin),
            _ => None,
        });
        if let Some(origin) = world_origin {
            self.retained.rebase(origin, &self.renderer.lights_known);
        }
        self.renderer.retained_upload(&mut self.retained);
        self.renderer.upload_lights(&self.frame_lights);
        self.renderer.upload_bones(&self.frame_bones);
        // Room for the copies that the parts of the frame make (clipped draws, replays).
        let copies = 3 * self.renderer.record_count + 256;
        self.renderer.upload_instances(&self.frame_instances, copies);
        self.renderer.upload_records(copies);
        let draws = items.iter().filter(|item| matches!(item, Item::Draw(_))).count();
        // The retained instances: a view of them in the world and in each replay.
        self.renderer.reserve_indirect(self.frame_args.len() + draws + 64 + 8 * self.retained.most_args());
        // The resources of the extension for the frame.
        let (values, texture) = self.parameters.as_ref().map_or(([[0.0f32; 4]; 32], None), |p| (p.values, p.texture.clone()));
        self.renderer.queue.write_buffer(&self.renderer.frame_uniform, 0, bytemuck::cast_slice(&values));
        let texture = texture.unwrap_or_else(|| self.renderer.white.clone());
        let renderer = &mut self.renderer;
        let clock = ticks::Clock::now();
        // The view and the projection of the scene: those of the start of the world, as the
        // lights of the extension have them.
        let scene = items.iter().find_map(|item| if let Item::WorldStart(facts) = item { Some((facts.view, facts.projection)) } else { None });
        renderer.clusters.build(&renderer.device, &renderer.queue, &self.frame_lights, scene, self.frame_size, &mut renderer.counts);
        renderer.counts.times[TIME_CLUSTERS] += clock.elapsed();
        renderer.frame_group = Rc::new(frame_group(&renderer.device, &renderer.frame_layout, &renderer.frame_uniform, &renderer.frame_sampler, &texture, &renderer.clusters, &renderer.light_buffer, &renderer.bone_buffer, &renderer.record_buffer, &renderer.light_index_buffer, &renderer.scene_buffer, &renderer.retained_light_buffer));
        for item in items {
            if !matches!(item, Item::Draw(_)) {
                self.end_run();
                self.forget_bound();
                if self.renderer.runs_dump {
                    let kind = match &item {
                        Item::Target(_) => "target",
                        Item::Clear(..) => "clear",
                        Item::Viewport(_) => "viewport",
                        Item::Screen(_) => "screen",
                        Item::WorldStart(_) => "world-start",
                        Item::WorldEnd(_) => "world-end",
                        Item::SortedBlends(_) => "sorted-blends",
                        Item::HeldHere(..) => "held",
                        Item::Replace(..) => "replace",
                        Item::Post(_) => "post",
                        Item::Draw(_) => unreachable!(),
                    };
                    println!("run frame {} barrier {kind}", self.renderer.counts.frames);
                }
            }
            match item {
                Item::Target(target) => {
                    self.pass_target = target;
                    self.open_pass(None, None, None);
                }
                Item::Clear(color, depth, stencil) => self.open_pass(color, depth, stencil),
                Item::Viewport(viewport) => {
                    self.viewport = Some(viewport);
                    self.apply_viewport();
                }
                Item::Draw(draw) => self.issue(&draw),
                Item::Screen(screen) => {
                    let Some(pass) = self.pass.as_mut() else { continue };
                    pass.set_pipeline(&screen.pipeline);
                    pass.set_bind_group(0, &screen.group, &[]);
                    pass.set_vertex_buffer(0, screen.buffer.slice(..));
                    pass.draw(0..screen.vertices, 0..1);
                    self.renderer.counts.screen_draws += 1;
                }
                Item::WorldStart(facts) => self.start_world(&facts),
                Item::WorldEnd(facts) => self.end_world(&facts),
                Item::SortedBlends(generation) => self.draw_retained_sorted(generation),
                Item::HeldHere(held, facts) => self.give_held_here(*held, &facts),
                Item::Replace(class, facts, drawn) => self.replace_class(class, &facts, drawn),
                Item::Post(facts) => self.post_process(&facts),
            }
        }
    }

    /// Within each run of draws between other items, the opaque draws of the world first (a
    /// world draw that does not blend, writes depth with a less or less-or-equal test, has no
    /// stencil and writes all colours), in the order of their pipeline, textures and
    /// geometry; then the other draws in the order of the game. The depth test orders the
    /// opaque draws among themselves. Where a blended draw that writes depth came before an
    /// opaque draw behind it, the blend now has that draw under it instead of the background.
    pub(crate) fn order_opaque_first(items: &mut Vec<Item>) {
        let opaque = |draw: &Draw| {
            draw.world
                && draw.key.blend[0] == 0
                && draw.key.depth[0] != 0
                && draw.key.depth[1] != 0
                && matches!(draw.key.depth[2], 2 | 4)
                && draw.key.stencil[0] == 0
                && draw.key.color_write & 7 == 7
        };
        let order = |draw: &Draw| {
            use std::hash::{Hash, Hasher};
            let mut hasher = rustc_hash::FxHasher::default();
            draw.key.hash(&mut hasher);
            (hasher.finish(), draw.stencil_reference, draw.texture_handles, Rc::as_ptr(&draw.vertices) as usize, draw.base_vertex, draw.range.0)
        };
        let mut start = 0;
        while start < items.len() {
            if !matches!(items[start], Item::Draw(_)) {
                start += 1;
                continue;
            }
            let mut end = start;
            while end < items.len() && matches!(items[end], Item::Draw(_)) {
                end += 1;
            }
            let run: Vec<Rc<Draw>> = items[start..end].iter().map(|item| if let Item::Draw(d) = item { d.clone() } else { unreachable!() }).collect();
            let mut first: Vec<Rc<Draw>> = run.iter().filter(|d| opaque(d)).cloned().collect();
            if first.len() > 1 {
                first.sort_by_cached_key(|d| order(d));
                let rest = run.iter().filter(|d| !opaque(d)).cloned();
                let ordered: Vec<Item> = first.into_iter().chain(rest).map(Item::Draw).collect();
                items.splice(start..end, ordered);
            }
            start = end;
        }
    }

    /// Does a step of the plan of the items (`plan::Plan`).
    pub(crate) fn plan_step(&mut self, step: plan::Step) {
        match step {
            plan::Step::StartWorld => {
                if self.parameters.is_none() {
                    self.ask_frame();
                }
                let facts = self.world_facts(true);
                self.items.push(Item::WorldStart(Box::new(facts)));
            }
            plan::Step::ReleaseHeld => self.items.append(&mut self.held_draws),
            plan::Step::SortedBlends(generation) => self.items.push(Item::SortedBlends(generation)),
            plan::Step::EndWorld => {
                let facts = self.world_facts(false);
                self.items.push(Item::WorldEnd(Box::new(facts)));
            }
            plan::Step::Post => self.post_item(),
        }
    }

    /// The post-processing item, one time in a frame, after the world began.
    pub(crate) fn post_item(&mut self) {
        if self.posted || !self.plan.world_begun || !self.drawing() || !self.parameters.as_ref().is_some_and(|p| p.post) {
            return;
        }
        self.posted = true;
        let mut facts = self.world_facts(false);
        if let Some((view, projection)) = self.world_matrices {
            facts.view = view;
            facts.projection = projection;
        }
        self.items.push(Item::Post(Box::new(facts)));
    }

    /// The facts of the world scene as the stream has them now. With `first`, the sun and
    /// the ambient light come from the shape record in progress (the first draw of the
    /// world), or from the last one for a world scene without a draw; without it, they are
    /// those of that draw.
    pub(crate) fn world_facts(&mut self, first: bool) -> WorldFacts {
        let shape = &self.stream.shape;
        if first {
            // The first light of the draw that is a sun or another directional light: type 3,
            // or type 4 for the light that the plugin names as the sun.
            let sun = shape.lights.chunks_exact(STATE_LIGHT_SIZE).map(|light| &light[4..]).find(|l| matches!(word(l, 0), 3 | 4)).map(|l| {
                let diffuse: [f32; 3] = floats(l, 4);
                let light_ambient: [f32; 3] = floats(l, 9 * 4);
                // The game does not give a direction of length 1.
                let direction: [f32; 3] = floats(l, 16 * 4);
                let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
                (direction.map(|v| v / length), diffuse, light_ambient)
            });
            let ambient = color(known(shape.ambient, 0));
            if switches().replay_debug {
                println!("world start: lights {} sun {:?} ambient {:?}", shape.lights.len() / STATE_LIGHT_SIZE, sun, ambient);
            }
            self.world_sun = sun;
            self.world_ambient = [ambient[0], ambient[1], ambient[2]];
        }
        // The fog and the fill mode as the device of the game has them now: the fog states
        // of the last draw with fog, and the fill mode of the last draw (D3DFILL_WIREFRAME
        // is 2).
        let fog = self.stream.fog;
        WorldFacts {
            target_size: self.size_of_target(),
            origin: self.stream.origin,
            view: self.stream.view,
            projection: self.stream.projection,
            interior: self.stream.interior,
            wireframe: self.stream.shape.fill == 2,
            fog_color: color(fog[0]),
            fog_start: f32::from_bits(fog[1]),
            fog_end: f32::from_bits(fog[2]),
            sun: self.world_sun,
            ambient: self.world_ambient,
        }
    }

    /// Asks the extension for its answers for the frame. See `Extension::frame`.
    pub(crate) fn ask_frame(&mut self) {
        let Some(mut extension) = self.extension.take() else { return };
        let facts = self.world_facts(true);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let scene = facts.view(&device, &queue, &commands, format);
        let clock = ticks::Clock::now();
        let parameters = extension.frame(&scene);
        let clusters = &mut self.renderer.clusters;
        clusters.on = clusters.forced.unwrap_or(parameters.clustered_lights);
        self.parameters = Some(parameters);
        self.renderer.counts.times[TIME_EXT_FRAME] += clock.elapsed();
        self.extension = Some(extension);
    }

    /// Gives the extension its call at the start of the world. See `Extension::before_world`.
    pub(crate) fn start_world(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let scene = facts.view(&device, &queue, &commands, format);
        let clear_depth = match self.pass.as_mut() {
            Some(pass) => {
                let world = std::mem::take(&mut self.world_draws);
                let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world, instances: &self.frame_instances, retained: &mut self.retained, args: &mut self.frame_args, retained_on: self.retained_on };
                let clock = ticks::Clock::now();
                pass.push_debug_group("extension: before the world");
                let clear_depth = extension.before_world(&scene, pass, &mut replayer);
                pass.pop_debug_group();
                self.renderer.counts.times[TIME_EXT_BEFORE] += clock.elapsed();
                self.world_draws = world;
                clear_depth
            }
            None => false,
        };
        let blend = self.parameters.as_ref().is_some_and(|p| p.blend);
        self.extension = Some(extension);
        if clear_depth && blend {
            self.keep_before();
        }
        if clear_depth {
            // The depth of the draws of the extension, for its post-processing and for the
            // draws that it makes with the held shapes.
            if self.parameters.as_ref().is_some_and(|p| p.post || p.hold || p.watched != 0) {
                self.before_depth_kept = self.copy_depth(2).is_some();
            }
            // The draws of the game start with a depth target of their own.
            self.open_pass(None, Some(1.0), None);
        }
        self.draw_retained_main(facts);
    }

    /// Gives the extension its call at the end of the world scene. See `Extension::after_world`.
    pub(crate) fn end_world(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world, instances: &self.frame_instances, retained: &mut self.retained, args: &mut self.frame_args, retained_on: self.retained_on };
            let clock = ticks::Clock::now();
            pass.push_debug_group("extension: after the world");
            extension.after_world(&scene, pass, &mut replayer);
            pass.pop_debug_group();
            self.renderer.counts.times[TIME_EXT_AFTER] += clock.elapsed();
            self.world_draws = world;
        }
        // The depth of the world, for the post-processing: the game clears its depth before
        // its later scenes.
        let mut world_depth_now = false;
        if self.parameters.as_ref().is_some_and(|p| p.post) {
            world_depth_now = self.copy_depth(1).is_some();
            self.world_depth_kept = world_depth_now;
            self.open_pass(None, None, None);
        }
        // The blend: a pass on the picture only, so that a shader can read the depth. A copy
        // of the picture when the extension reads it.
        let picture = if self.before_kept && extension.reads(ReadingCall::Blend).picture { self.copy_picture() } else { None };
        if let (true, Some(before), Some(color), Some(depth), Some(encoder)) = (self.before_kept, self.before.as_ref(), self.color.clone(), self.depth.clone(), self.encoder.as_mut()) {
            if let Some(pass) = self.pass.as_mut() {
                flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
            }
            drop(self.pass.take());
            scene.picture = picture.as_ref();
            let before = before.create_view(&Default::default());
            let depth = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blend"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            let clock = ticks::Clock::now();
            extension.blend(&scene, &mut pass, &before, &depth);
            drop(pass);
            self.renderer.counts.times[TIME_EXT_AFTER] += clock.elapsed();
            self.open_pass(None, None, None);
        }
        self.extension = Some(extension);
        // The blend pass has no depth target: the copy of the world depth is the depth now.
        self.give_held(facts, world_depth_now);
    }

    /// Gives the extension the shapes that it took in the frame. See `Extension::draw_held`.
    /// `world_depth_now`: the copy of the world depth is the depth as it is now.
    pub(crate) fn give_held(&mut self, facts: &WorldFacts, world_depth_now: bool) {
        if self.taken.is_empty() && !self.extension.as_ref().is_some_and(|extension| extension.wants_held()) {
            return;
        }
        let Some(mut extension) = self.extension.take() else { return };
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::Held(&self.taken)), world_depth_now);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        // The lights for the extension, binned for the view that it draws with.
        let renderer = &mut self.renderer;
        renderer.extension_lights.build(&renderer.device, &renderer.queue, &self.frame_lights, facts.view, facts.projection, self.frame_size, &mut renderer.counts);
        let own = &self.renderer.extension_lights;
        let lights = own.built.then(|| (own.lights.clone(), own.uniform.clone(), own.table.clone(), own.indexes.clone(), own.origin));
        scene.lights = lights.as_ref().map(|(lights, clusters, table, indexes, origin)| FrameLights { lights, clusters, table, indexes, origin: *origin });
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world, instances: &self.frame_instances, retained: &mut self.retained, args: &mut self.frame_args, retained_on: self.retained_on };
            let clock = ticks::Clock::now();
            pass.push_debug_group("extension: held draws");
            extension.draw_held(&scene, pass, &self.taken, &mut replayer);
            pass.pop_debug_group();
            self.renderer.counts.times[TIME_EXT_CLASS] += clock.elapsed();
            self.world_draws = world;
        }
        self.extension = Some(extension);
    }

    /// Gives the extension a held shape in its place. See `Extension::draw_held_in_place`.
    pub(crate) fn give_held_here(&mut self, held: HeldDraw, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::HeldInPlace(&held)), false);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        let own = &self.renderer.extension_lights;
        let lights = own.built.then(|| (own.lights.clone(), own.uniform.clone(), own.table.clone(), own.indexes.clone(), own.origin));
        scene.lights = lights.as_ref().map(|(lights, clusters, table, indexes, origin)| FrameLights { lights, clusters, table, indexes, origin: *origin });
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world, instances: &self.frame_instances, retained: &mut self.retained, args: &mut self.frame_args, retained_on: self.retained_on };
            let clock = ticks::Clock::now();
            pass.push_debug_group("extension: a held draw in its place");
            extension.draw_held_in_place(&scene, pass, core::slice::from_ref(&held), &mut replayer);
            pass.pop_debug_group();
            self.renderer.counts.times[TIME_EXT_CLASS] += clock.elapsed();
            self.world_draws = world;
        }
        self.extension = Some(extension);
    }

    /// Keeps the picture of the frame as it is now, for `Extension::blend`.
    pub(crate) fn keep_before(&mut self) {
        let (Some(source), Some(encoder)) = (self.back_buffer.as_ref(), self.encoder.as_mut()) else { return };
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC) || !self.depth.as_ref().is_some_and(|depth| depth.texture().usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)) {
            return;
        }
        let size = source.size();
        if self.before.as_ref().is_none_or(|before| before.size() != size) {
            self.before = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST));
        }
        // A copy is not a part of a pass.
        if let Some(pass) = self.pass.as_mut() {
            flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
        }
        drop(self.pass.take());
        encoder.copy_texture_to_texture(source.as_image_copy(), self.before.as_ref().unwrap().as_image_copy(), size);
        self.renderer.counts.picture_copies += 1;
        self.before_kept = true;
    }

    /// Copies what a call of the extension reads (`Extension::reads`): the picture of the
    /// frame and the depth of the game, outside of the pass. With `world_depth_now`, the copy
    /// of the world depth is the depth as it is now, and is given for it. The pass is open
    /// again after a copy.
    pub(crate) fn copy_for(&mut self, reads: Reads, world_depth_now: bool) -> (Option<wgpu::TextureView>, Option<wgpu::TextureView>) {
        let picture = if reads.picture { self.copy_picture() } else { None };
        let depth = match (reads.depth, world_depth_now) {
            (false, _) => None,
            (true, true) => self.world_depth_view(),
            (true, false) => self.copy_depth(0),
        };
        if self.pass.is_none() {
            self.open_pass(None, None, None);
        }
        (picture, depth)
    }

    /// The copy of the depth at the end of the world scene, when it was kept in this frame.
    pub(crate) fn world_depth_view(&self) -> Option<wgpu::TextureView> {
        self.world_depth
            .as_ref()
            .filter(|_| self.world_depth_kept)
            .map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }))
    }

    /// Copies the picture of the frame, outside of the pass. The pass stays closed.
    pub(crate) fn copy_picture(&mut self) -> Option<wgpu::TextureView> {
        let (Some(source), Some(encoder)) = (self.back_buffer.as_ref(), self.encoder.as_mut()) else { return None };
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let size = source.size();
        if self.picture_copy.as_ref().is_none_or(|copy| copy.size() != size) {
            self.picture_copy = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST));
        }
        if let Some(pass) = self.pass.as_mut() {
            flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
        }
        drop(self.pass.take());
        encoder.copy_texture_to_texture(source.as_image_copy(), self.picture_copy.as_ref().unwrap().as_image_copy(), size);
        self.renderer.counts.picture_copies += 1;
        self.picture_copy.as_ref().map(|copy| copy.create_view(&Default::default()))
    }

    /// Copies the depth as it is now, outside of the pass: into the copy of the moment (0),
    /// the copy of the world (1, for `Extension::post`) or the copy of the draws of
    /// `Extension::before_world` (2). The caller opens the pass again.
    pub(crate) fn copy_depth(&mut self, which: u32) -> Option<wgpu::TextureView> {
        let (Some(source), Some(encoder)) = (self.depth.as_ref().map(|depth| depth.texture().clone()), self.encoder.as_mut()) else { return None };
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let size = source.size();
        let copy = match which {
            1 => &mut self.world_depth,
            2 => &mut self.before_depth,
            _ => &mut self.depth_copy,
        };
        if copy.as_ref().is_none_or(|copy| copy.size() != size) {
            *copy = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST));
        }
        if let Some(pass) = self.pass.as_mut() {
            flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
        }
        drop(self.pass.take());
        encoder.copy_texture_to_texture(source.as_image_copy(), copy.as_ref().unwrap().as_image_copy(), size);
        self.renderer.counts.depth_copies += 1;
        copy.as_ref().map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }))
    }

    /// Gives the extension its post-processing call. See `Extension::post`.
    pub(crate) fn post_process(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        // The depth of the end of the world scene, when it was kept; else a copy of that of now.
        let mut reads = extension.reads(ReadingCall::Post);
        let world_depth = self.world_depth_view().filter(|_| reads.depth);
        reads.depth &= world_depth.is_none();
        let (picture, depth_now) = self.copy_for(reads, false);
        let depth = world_depth.or(depth_now);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        if let (Some(color), Some(encoder)) = (self.color.clone(), self.encoder.as_mut()) {
            if let Some(pass) = self.pass.as_mut() {
                flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
            }
            drop(self.pass.take());
            let clock = ticks::Clock::now();
            encoder.push_debug_group("extension: post-processing");
            extension.post(&scene, encoder, &color);
            encoder.pop_debug_group();
            self.renderer.counts.times[TIME_EXT_POST] += clock.elapsed();
            // The pictures that the extension wants: one without the interface of the game
            // is the picture as it is now.
            for request in extension.pictures() {
                match self.back_buffer.as_ref().filter(|source| !request.with_ui && source.usage().contains(wgpu::TextureUsages::COPY_SRC)) {
                    Some(source) => {
                        let (buffer, row) = copy_picture(&device, encoder, source, source.width(), source.height());
                        self.early_pictures.push((buffer, row, source.width(), source.height(), request.path));
                    }
                    None => self.shots.push(request.path),
                }
            }
        }
        self.extension = Some(extension);
        self.open_pass(None, None, None);
    }

    /// Gives the extension its call in place of the draws of a class. See
    /// `Extension::draw_class`. `drawn` is the number of world draws before the call. With
    /// a plane for the class (`FrameParameters::class_plane`): the world draws that blend,
    /// write no depth and come after the call go into the picture first, on the far side of
    /// the plane (those before the call are there already, on that side only); after the
    /// call each of them that came before it goes on the side of the camera, over the call's
    /// drawing.
    pub(crate) fn replace_class(&mut self, class: u32, facts: &WorldFacts, drawn: usize) {
        let Some(mut extension) = self.extension.take() else { return };
        let plane = self.parameters.as_ref().and_then(|p| p.class_plane).filter(|(c, _)| *c == class).map(|(_, plane)| plane);
        let drawn = drawn.min(self.world_draws.len());
        if let Some(plane) = plane {
            self.replay_blended(drawn.., plane.map(|v| -v), facts.target_size);
            self.plane_ahead = None;
            self.split = Some(plane);
        }
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::Class(class)), false);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world, instances: &self.frame_instances, retained: &mut self.retained, args: &mut self.frame_args, retained_on: self.retained_on };
            let clock = ticks::Clock::now();
            pass.push_debug_group("extension: a class of draws");
            extension.draw_class(class, &scene, pass, &mut replayer);
            pass.pop_debug_group();
            self.renderer.counts.times[TIME_EXT_CLASS] += clock.elapsed();
            self.world_draws = world;
        }
        if let Some(plane) = plane {
            self.replay_blended(..drawn, plane, facts.target_size);
        }
        self.extension = Some(extension);
    }

    /// Draws the world draws in `range` that blend and write no depth again, into the pass,
    /// only where they are on the positive side of `plane`.
    pub(crate) fn replay_blended(&mut self, range: impl std::slice::SliceIndex<[Rc<Draw>], Output = [Rc<Draw>]>, plane: [f64; 4], target_size: (u32, u32)) {
        let Some(pass) = self.pass.as_mut() else { return };
        let world = std::mem::take(&mut self.world_draws);
        let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world[range], instances: &self.frame_instances, retained: &mut self.retained, args: &mut self.frame_args, retained_on: self.retained_on };
        replayer.replay(
            pass,
            &ReplayView {
                world: None,
                view: None,
                projection: None,
                target_size,
                flip_cull: false,
                clip_plane: Some(plane),
                opaque_only: false,
                translucent_only: true,
                force_depth_test: false,
                classes: ReplayView::WORLD,
            },
        );
        self.world_draws = world;
    }
}
