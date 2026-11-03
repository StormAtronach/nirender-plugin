//! The items of a frame: their run, the steps of the plan, the world's start and end,
//! the parts the extension draws (the replaced classes, the held draws, the post).

use crate::*;
use crate::stamps::{Stamp, StampKind, Stamps};

/// The view groups of the main view that draw the table. Each name is that of the group's GPU span
/// and of its CPU time (`view_times`): both cover the same items. The sorted blends' items also
/// draw the engine's blends that the sorted list took in (`merged_blends`).
const MAIN_VIEW_OPAQUE: &str = "the table's opaque slots";
const MAIN_VIEW_WALK: &str = "table: blends in the engine's walk";
const MAIN_VIEW_SORTED: &str = "sorted blends (the engine's and the table's)";

impl Player {
    /// Does the items of the frame that are not done yet. See [`Item`].
    pub(crate) fn run_items(&mut self) {
        let items = std::mem::take(&mut self.items);
        if !self.drawing() {
            return;
        }
        let clock = ticks::Clock::now();
        let inside_before = sum_except(&self.renderer.counts.times, TIME_ITEMS);
        self.renderer.flush_particles();
        self.renderer.flush_morphs();
        self.run_items_inner(items);
        let inside_after = sum_except(&self.renderer.counts.times, TIME_ITEMS);
        self.renderer.counts.times[TIME_ITEMS] += clock.elapsed().saturating_sub(inside_after - inside_before);
    }

    pub(crate) fn run_items_inner(&mut self, items: Vec<Item>) {
        self.plane_ahead = self.parameters.as_ref().and_then(|p| p.class_plane).filter(|(class, _)| items.iter().any(|item| matches!(item, Item::Replace(c, ..) if c == class))).map(|(_, plane)| plane);
        // The origin of the instances follows the camera of the world of the frame.
        let world_origin = items.iter().find_map(|item| match item {
            Item::WorldStart(facts) => Some(facts.origin),
            _ => None,
        });
        if let Some(origin) = world_origin {
            self.retained.rebase(origin, &self.renderer.lights_known);
        }
        // The texture effects under a turning billboard, for the camera of the world.
        if let Some(facts) = items.iter().find_map(|item| if let Item::WorldStart(facts) = item { Some(facts) } else { None }) {
            let location = facts.eye();
            let began = std::time::Instant::now();
            self.retained.turn_effects(&crate::effect_turn::Camera::of_view(&facts.view, location));
            self.retained.counts.turn_seconds += began.elapsed().as_secs_f64();
        }
        let began = std::time::Instant::now();
        let segment = self.renderer.retained_records;
        self.renderer.retained_upload(&mut self.retained);
        // A new shape can make the segment grow in any frame. The uploads below write the frame's
        // records and instances again, at the new place.
        Instance::follow_segment(&mut self.frame_instances, segment, self.renderer.retained_records);
        self.retained.counts.upload_seconds += began.elapsed().as_secs_f64();
        self.renderer.upload_lights(&self.frame_lights);
        self.renderer.upload_bones(&self.frame_bones);
        // The small group and the copies of its draws, in the records and instances of the
        // frame before their upload.
        let small = self.mark_small_draws(&items);
        // Room for the copies that the parts of the frame make (clipped draws, replays).
        let copies = 3 * self.renderer.record_count + 256;
        let generations = (self.renderer.record_generation, self.renderer.instance_generation);
        let frame = self.renderer.counts.frames;
        if let Some(path) = self.growth_shot_next.take().filter(|_| world_origin.is_some()) {
            self.shots.push(path);
        }
        if self.growth_test.0 != 0 && world_origin.is_some() && frame % self.growth_test.0 == 0 {
            self.renderer.force_record_growth = true;
        }
        self.renderer.upload_instances(&self.frame_instances, copies);
        self.renderer.upload_records(copies);
        let instances = self.retained.counts.instances;
        if (self.renderer.record_generation, self.renderer.instance_generation) != generations && instances != 0 {
            // A buffer that grew in these uploads is new and zeroed: the table's segment goes
            // into it now, or every draw of the table reads zeros in this frame (audit U4-03).
            // The upload before them sized the segment; this one writes the whole of it.
            self.renderer.retained_upload(&mut self.retained);
            println!("buffers: a buffer grew at frame {frame} with {instances} instances in the table");
            if let Some(prefix) = self.growth_test.1.clone() {
                self.shots.push(format!("{prefix}-{frame:06}-growth.png"));
                self.growth_shot_next = Some(format!("{prefix}-{frame:06}-after.png"));
            }
        }
        let draws = items.iter().filter(|item| matches!(item, Item::Draw(_))).count();
        // The retained instances: a view of them in the world and in each replay.
        self.renderer.reserve_indirect(self.frame_args.len() + draws + 64 + 8 * self.retained.most_args());
        // The resources of the extension for the frame.
        let (values, texture) = self.parameters.as_ref().map_or(([[0.0f32; 4]; 32], None), |p| (p.values, p.texture.clone()));
        self.renderer.queue.write_buffer(&self.renderer.frame_uniform, 0, bytemuck::cast_slice(&values));
        let texture = texture.unwrap_or_else(|| self.renderer.white.clone());
        let clock = ticks::Clock::now();
        // The lights of the world (`MGE_XE_LIGHTS=world`): the table's lights shaded for the
        // main view before the clusters, which bin them; `draw_retained_main` takes the shade.
        let world_start = items.iter().find_map(|item| if let Item::WorldStart(facts) = item { Some(facts) } else { None });
        self.table_shaded = None;
        let world = match world_start.filter(|_| self.renderer.clusters.world) {
            Some(facts) => {
                let shaded = self.shade_table_main(&facts.view);
                self.renderer.upload_retained_lights(&shaded.0);
                self.shade_frame = Some(shaded.1);
                self.table_shaded = Some(shaded);
                Some((self.retained.world_light_slots(), self.retained.set_to_scene(facts.origin), facts.origin))
            }
            None => None,
        };
        let renderer = &mut self.renderer;
        // The view and the projection of the scene: those of the start of the world, as the
        // lights of the extension have them.
        let scene = world_start.map(|facts| (facts.view, facts.projection));
        match (world, self.table_shaded.as_ref()) {
            (Some((slots, to_scene, camera)), Some((lights, _))) => {
                renderer.clusters.build_world(&renderer.device, &renderer.queue, lights, &slots, scene, camera, to_scene, self.frame_size, &mut renderer.counts);
            }
            _ => {
                renderer.clusters.build(&renderer.device, &renderer.queue, &self.frame_lights, scene, self.frame_size, &mut renderer.counts);
            }
        }
        renderer.counts.times[TIME_CLUSTERS] += clock.elapsed();
        renderer.frame_group = Rc::new(frame_group(&renderer.device, &renderer.frame_layout, &renderer.frame_uniform, &renderer.frame_sampler, &texture, &renderer.clusters, &renderer.light_buffer, &renderer.bone_buffer, &renderer.record_buffer, &renderer.light_index_buffer, &renderer.scene_buffer, &renderer.retained_light_buffer, &renderer.retained_bone_buffer, &renderer.influences.buffer, &renderer.effect_entry_buffer));
        // The GPU time of the frame by part (`MGE_XE_GPU_TIME=1`): one span for each group of
        // items of one kind in one phase of the frame. The world's start and end, the post
        // chain and the first person's depth have spans of their own inside.
        let (mut world_started, mut world_ended, mut posted) = (false, false, false);
        let mut group: Option<(&'static str, Stamp)> = None;
        for (index, item) in items.into_iter().enumerate() {
            let label = match &item {
                Item::Draw(_) | Item::Screen(_) if posted => Some("game: the interface"),
                Item::Draw(_) | Item::Screen(_) if world_ended => Some("game: after the world (first person)"),
                Item::Draw(_) | Item::Screen(_) if world_started => Some("game: draws in the world"),
                Item::Draw(_) | Item::Screen(_) => Some("game: before the world (sky)"),
                Item::WalkPlaces(_) => Some(MAIN_VIEW_WALK),
                Item::ViewPlaces(group, _) | Item::ViewSorted(group) => Some(self.view_group_name(*group)),
                Item::SortedBlends(_) => Some(MAIN_VIEW_SORTED),
                Item::Replace(..) => Some("a class of draws (extension: water)"),
                Item::HeldHere(..) => Some("a held draw in place (extension)"),
                Item::Target(_) | Item::Clear(..) | Item::Viewport(_) => group.as_ref().map(|(name, _)| *name),
                Item::WorldStart(_) | Item::WorldEnd(_) | Item::Post(_) | Item::FirstPersonDepth(_) => None,
            };
            if group.as_ref().map(|(name, _)| *name) != label {
                if let Some((_, at)) = group.take() {
                    self.part_end(at);
                }
                if let Some(name) = label {
                    let at = self.part_begin(name);
                    group = Some((name, at));
                }
            }
            match &item {
                Item::WorldStart(_) => {
                    world_started = true;
                    self.phase_world = true;
                }
                Item::WorldEnd(_) => world_ended = true,
                Item::Post(_) => {
                    posted = true;
                    self.phase_posted = true;
                }
                _ => {}
            }
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
                        Item::WalkPlaces(_) => "walk-places",
                        Item::ViewPlaces(..) => "view-places",
                        Item::ViewSorted(_) => "view-sorted",
                        Item::HeldHere(..) => "held",
                        Item::Replace(..) => "replace",
                        Item::Post(_) => "post",
                        Item::FirstPersonDepth(_) => "first-person-depth",
                        Item::Draw(_) => unreachable!(),
                    };
                    println!("run frame {} barrier {kind}", self.renderer.counts.frames);
                }
            }
            match item {
                Item::Target(target) => {
                    self.pass_target = target;
                    self.open_pass(crate::gpu::PassReason::Target, None, None, None);
                }
                Item::Clear(color, depth, stencil) => self.open_pass(crate::gpu::PassReason::Clear, color, depth, stencil),
                Item::Viewport(viewport) => {
                    self.viewport = Some(viewport);
                    self.apply_viewport();
                }
                Item::Draw(draw) => {
                    self.small_copy = small.get(index).copied().flatten();
                    self.issue(&draw);
                    self.small_copy = None;
                }
                Item::Screen(screen) => {
                    self.begin_pass();
                    let Some(pass) = self.pass.as_mut() else { continue };
                    pass.set_pipeline(&screen.pipeline);
                    pass.set_bind_group(0, &screen.group, &[]);
                    pass.set_vertex_buffer(0, screen.buffer.slice(..));
                    pass.draw(0..screen.vertices, 0..1);
                    self.renderer.counts.screen_draws += 1;
                }
                Item::WorldStart(facts) => self.start_world(&facts),
                Item::WorldEnd(facts) => self.end_world(&facts),
                Item::SortedBlends(generation) => {
                    self.timed(MAIN_VIEW_SORTED, 0, |player| player.draw_retained_sorted(generation));
                }
                Item::WalkPlaces(run) => self.timed(MAIN_VIEW_WALK, 0, |player| player.draw_walk_run(run)),
                Item::ViewPlaces(group, run) => {
                    let ids = self.views.take_run(group, run);
                    self.timed(self.view_group_name(group), ids.len() as u64, |player| player.draw_view_run(group, &ids));
                }
                Item::ViewSorted(group) => {
                    let ids = self.views.take_sorted(group);
                    self.timed(self.view_group_name(group), ids.len() as u64, |player| player.draw_view_run(group, &ids));
                }
                Item::HeldHere(held, facts) => self.give_held_here(core::slice::from_ref(&*held), &facts),
                Item::Replace(class, facts, drawn) => self.replace_class(class, &facts, drawn),
                Item::Post(facts) => self.post_process(&facts),
                Item::FirstPersonDepth(projection) => self.merge_first_person_depth(&projection),
            }
        }
        if let Some((_, at)) = group.take() {
            self.part_end(at);
        }
        self.small_arrays = None;
    }

    /// Marks the game's draws among `items` that come before the world or after the post
    /// chain into the back buffer, and makes the small group of their textures
    /// (`small_arrays`): no draw of the table comes in those parts of the frame, and their
    /// textures are few, so their passes need not bind the whole arrays. Each marked draw
    /// gets a copy of its record with its slots in the small group, and copies of its
    /// instances that name it, in the records and instances of the frame before their one
    /// upload: the first instance of the copy, for each item. Nothing with
    /// `MGE_XE_PASS_GROUPS=0`, or in tier C (no arrays: each draw has its own group).
    pub(crate) fn mark_small_draws(&mut self, items: &[Item]) -> Vec<Option<u32>> {
        self.small_arrays = None;
        if !switches().pass_groups || self.renderer.tier == crate::request::TextureTier::C {
            return Vec::new();
        }
        let (mut world, mut posted, mut target) = (self.phase_world, self.phase_posted, self.pass_target);
        let mut slots = Vec::new();
        let marks: Vec<bool> = items
            .iter()
            .map(|item| match item {
                Item::WorldStart(_) => {
                    world = true;
                    false
                }
                Item::Post(_) => {
                    posted = true;
                    false
                }
                Item::Target(next) => {
                    target = *next;
                    false
                }
                Item::Draw(draw) if (!world || posted) && target.is_none() => {
                    slots.extend(draw.data.stages.iter().map(|stage| stage.slots[0]));
                    true
                }
                _ => false,
            })
            .collect();
        if !marks.iter().any(|&mark| mark) {
            return Vec::new();
        }
        let (group, remap) = self.renderer.small_array_group(&slots);
        let copies = items
            .iter()
            .zip(&marks)
            .map(|(item, &mark)| match item {
                Item::Draw(draw) if mark => self.small_copy_of(draw, &remap),
                _ => None,
            })
            .collect();
        self.small_arrays = Some((group, remap));
        copies
    }

    /// The copy of a draw for the small group (`mark_small_draws`): its first instance, or
    /// [`None`] when the records are full (the draw then binds the whole arrays).
    fn small_copy_of(&mut self, draw: &Draw, remap: &FxHashMap<u32, u32>) -> Option<u32> {
        let mut data = draw.data;
        for stage in data.stages.iter_mut() {
            stage.slots[0] = remap.get(&stage.slots[0]).copied().unwrap_or(0);
        }
        let record = self.renderer.record(&data)?;
        let (start, n) = (draw.instances.0 as usize, draw.instances.1 as usize);
        let first = self.frame_instances.len() as u32;
        for index in start..start + n {
            let world = self.frame_instances[index].world;
            self.frame_instances.push(Instance { world, record: [record, 0, 0, 0] });
        }
        Some(first)
    }

    /// Does a step of the plan of the items ([`plan::Plan`]).
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
                self.world_end_facts = Some(facts);
                self.items.push(Item::WorldEnd(Box::new(facts)));
            }
            plan::Step::MissingClasses => self.replace_missing_classes(),
            plan::Step::Post => self.post_item(),
            plan::Step::FirstPersonEnd => {
                if self.first_person_drawn && self.parameters.as_ref().is_some_and(|p| p.post) {
                    self.items.push(Item::FirstPersonDepth(self.stream.projection));
                }
            }
        }
    }

    /// The call of each class that the extension takes and that no draw of the world camera's
    /// scenes brought in this frame, with the camera of the world's end: the engine culls its
    /// water plane when the plane is out of the view (high above the sea), and the water of
    /// the extension must still draw. The Direct3D 9 path of MGE XE does the same at the end
    /// of the scene after the world (`renderStageWater` when `waterDrawn` is false).
    pub(crate) fn replace_missing_classes(&mut self) {
        let missing = self.takes & !self.replaced;
        let Some(facts) = self.world_end_facts.filter(|_| missing != 0 && self.drawing()) else { return };
        for class in (0..32).filter(|class| missing & (1 << class) != 0) {
            self.replaced |= 1 << class;
            self.renderer.counts.class_calls_without_draw += 1;
            self.items.push(Item::Replace(class, Box::new(facts), self.world_draws.len()));
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
    /// the ambient light come from the frame's `RETAINED_SCENE` record; in an older capture
    /// without it, from the shape record in progress (the first draw of the world), or from
    /// the last one for a world scene without a draw. Without `first`, they are those of the
    /// world's start.
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
            let mut ambient = color(known(shape.ambient, 0));
            let mut sun = sun;
            // With the table on, the engine draws no shape of the world, so the shape record
            // above can be the last one it sent, with the sun and the ambient of that moment.
            // The frame's `RETAINED_SCENE` record gives the ambient of this frame.
            if let Some((scene_ambient, _)) = self.retained_values {
                ambient = color(known(scene_ambient, 0));
            }
            // The sun: the weather's sky light as the engine gives it to the device, sent by
            // the plugin in the scene record (flag 64). Do not guess it from the light records.
            // With the record, a frame without the flag has no sun (the sky light off or too
            // dim): a draw's directional light can be a mod's. Without the record (an older
            // plugin) the sun stays that of the engine's last world draw.
            if self.retained_values.is_some() {
                if self.world_sun_logged < 4 {
                    self.world_sun_logged += 1;
                    println!("world sun: the scene record {} the sun", if self.retained_sun.is_some() { "carries" } else { "lacks" });
                }
                sun = self.retained_sun.as_ref().map(|l| {
                    let l = &l[..];
                    let diffuse: [f32; 3] = floats(l, 4);
                    let light_ambient: [f32; 3] = floats(l, 9 * 4);
                    let direction: [f32; 3] = floats(l, 16 * 4);
                    let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
                    (direction.map(|v| v / length), diffuse, light_ambient)
                });
            }
            if switches().replay_debug {
                println!("world start: lights {} sun {:?} ambient {:?}", shape.lights.len() / STATE_LIGHT_SIZE, sun, ambient);
            }
            self.world_sun = sun;
            self.world_ambient = [ambient[0], ambient[1], ambient[2]];
        }
        // The fill mode: the scene record's flag, from the game's own wireframe property; only
        // in an older capture the last draw's (D3DFILL_WIREFRAME is 2), which a mod's mesh with
        // a wireframe property can give.
        let (fog_on, fog) = world_fog(self.retained_values, self.fog_off_color, self.stream.fog);
        WorldFacts {
            target_size: self.size_of_target(),
            frame: self.renderer.counts.frames as u64,
            samples: self.renderer.samples,
            depth_format: self.renderer.depth_format,
            origin: self.stream.origin,
            view: self.stream.view,
            projection: self.stream.projection,
            interior: self.stream.interior,
            wireframe: if self.stream.scene_wireframe { self.stream.wireframe } else { self.stream.shape.fill == 2 },
            fog_on,
            fog_color: color(fog[0]),
            fog_start: f32::from_bits(fog[1]),
            fog_end: f32::from_bits(fog[2]),
            sun: self.world_sun,
            ambient: self.world_ambient,
            // The extension's calls are the main view's: the steps of the plan run in it only.
            view_key: ViewKey::MAIN,
        }
    }

    /// Asks the extension for its answers for the frame. See [`Extension::frame`].
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
        clusters.world = switches().world_lights || parameters.world_lights;
        self.parameters = Some(parameters);
        self.takes = self.parameters.as_ref().map_or(0, |p| p.watched & (1 << scene::CLASS_WATER_PLANE));
        self.shadow_reach = self.parameters.as_ref().map_or(0.0, |p| p.shadow_reach);
        self.renderer.counts.times[TIME_EXT_FRAME] += clock.elapsed();
        self.extension = Some(extension);
    }

    /// Gives the extension its call at the start of the world. See [`Extension::before_world`].
    pub(crate) fn start_world(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let scene = facts.view(&device, &queue, &commands, format);
        let part = self.part_begin("before the world (extension: terrain, distant statics, glows)");
        self.begin_pass();
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
        self.part_end(part);
        let blend = self.parameters.as_ref().is_some_and(|p| p.blend);
        self.extension = Some(extension);
        if clear_depth && blend {
            self.keep_before();
        }
        if clear_depth {
            // The depth of the draws of the extension, for its post-processing and for the
            // draws that it makes with the held shapes.
            if self.parameters.as_ref().is_some_and(|p| p.post || p.hold || p.watched != 0) {
                self.before_depth_kept = self.copy_depth(2, "depth: before the world").is_some();
            }
            // The draws of the game start with a depth target of their own; with one depth
            // buffer they test against the extension's draws, in a pass that keeps the depth.
            let clear = (!depth::one_buffer()).then(|| depth::clear(1.0));
            self.open_pass(crate::gpu::PassReason::WorldStart, None, clear, None);
        }
        let part = self.part_begin(MAIN_VIEW_OPAQUE);
        self.timed(MAIN_VIEW_OPAQUE, 0, |player| player.draw_retained_main(facts));
        self.part_end(part);
    }

    /// Does `draw`, the item of one view group, and adds its CPU time to the group's
    /// (`view_times`). `places`: the places that the item draws.
    fn timed(&mut self, name: &'static str, places: u64, draw: impl FnOnce(&mut Self)) {
        let clock = ticks::Clock::now();
        draw(self);
        self.renderer.counts.view_times.add(name, clock.elapsed(), places);
    }

    /// The name of a view of its own of the frame, for its GPU span and its CPU time: its order
    /// among the frame's views of their own, the size of its target, its kind and its camera.
    /// Two views with the same order, size, kind and camera in different frames share a name. The name is made
    /// once for each view, when its facts are known.
    pub(crate) fn view_group_name(&self, group: usize) -> &'static str {
        let Some(g) = self.views.group(group) else { return stamps::name(&format!("table: view of its own {}", group + 1)) };
        if let Some(name) = g.name.get() {
            return name;
        }
        let camera = if g.world_camera { "the world camera" } else { "another camera" };
        let kind = g.kind.name();
        let Some(facts) = g.facts else { return stamps::name(&format!("table: view of its own {} ({kind}, {camera})", group + 1)) };
        let name = stamps::name(&format!("table: view of its own {} ({}x{}, {kind}, {camera})", group + 1, facts.target_size.0, facts.target_size.1));
        g.name.set(Some(name));
        name
    }

    /// Puts what a late sorted run wrote into the copy of the world depth: the depth after
    /// the run where it differs from the depth before it, the nearer of the two
    /// ([`late_depth`]). The held draws at the end of the world scene drew before the run, so
    /// their depth is in both copies and does not enter. A held draw at its place in the
    /// sorted queue is in the run: the depth that it writes enters (a water surface that asks
    /// for the write, or any surface with a dry space in the frame).
    pub(crate) fn merge_late_depth(&mut self) {
        if self.copy_depth(4, "depth: after late sorted blends").is_none() {
            self.open_pass(crate::gpu::PassReason::AfterCopy, None, None, None);
            return;
        }
        let device = self.renderer.device.clone();
        let format = self.renderer.depth_format;
        let late = self.late_depth.get_or_insert_with(|| late_depth::LateDepth::new(&device, format));
        let depth_only = wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() };
        if let (Some(before), Some(after), Some(world), Some(encoder)) = (self.late_before.as_ref(), self.late_after.as_ref(), self.world_depth.as_ref(), self.encoder.as_mut()) {
            let at = stamp(&self.renderer.stamps, encoder, StampKind::Outside, "depth: late sorted blends into the world depth");
            late.merge(&device, encoder, &before.create_view(&depth_only), &after.create_view(&depth_only), &world.create_view(&Default::default()));
            stamp_end(&self.renderer.stamps, encoder, at);
        }
        self.open_pass(crate::gpu::PassReason::AfterCopy, None, None, None);
    }

    /// Gives the extension its call at the end of the world scene. See [`Extension::after_world`].
    pub(crate) fn end_world(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        let part = self.part_begin("after the world (extension: grass)");
        self.begin_pass();
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
        self.part_end(part);
        // The depth of the world, for the post-processing: the game clears its depth before
        // its later scenes.
        let mut world_depth_now = false;
        if self.parameters.as_ref().is_some_and(|p| p.post) {
            world_depth_now = self.copy_depth(1, "depth: the world, for the post chain").is_some();
            self.world_depth_kept = world_depth_now;
            self.world_depth_projection = Some(facts.projection);
            self.open_pass(crate::gpu::PassReason::AfterCopy, None, None, None);
        }
        // The blend: a pass on the picture only, so that a shader can read the depth. A copy
        // of the picture when the extension reads it. A multisampled depth goes to the shader
        // resolved: the copy of the world depth when it was made now, else a resolve now.
        let picture = if self.before_kept && extension.reads(ReadingCall::Blend).picture { self.copy_picture("picture: the blend") } else { None };
        let resolved = if self.before_kept && self.renderer.samples > 1 { if world_depth_now { self.world_depth_view() } else { self.copy_depth(0, "depth: the blend") } } else { None };
        if self.before_kept && self.before.is_some() && self.color.is_some() && self.depth.is_some() && self.encoder.is_some() {
            self.close_pass();
        }
        if let (true, Some(before), Some(color), Some(depth), Some(encoder)) = (self.before_kept, self.before.as_ref(), self.color.clone(), self.depth.clone(), self.encoder.as_mut()) {
            let at =stamp(&self.renderer.stamps, encoder, StampKind::Part, "the blend pass (extension)");
            scene.picture = picture.as_ref();
            let before = before.create_view(&Default::default());
            let depth = resolved.unwrap_or_else(|| depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
            crate::gpu::note_other_pass();
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blend"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color,
                        depth_slice: None,
                        resolve_target: self.back_view.as_deref(),
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
            stamp_end(&self.renderer.stamps, encoder, at);
            self.renderer.counts.times[TIME_EXT_AFTER] += clock.elapsed();
            self.open_pass(crate::gpu::PassReason::AfterExtension, None, None, None);
        }
        self.extension = Some(extension);
        // The blend pass has no depth target: the copy of the world depth is the depth now.
        self.give_held(facts, world_depth_now);
    }

    /// Gives the extension the shapes that it took in the frame. See [`Extension::draw_held`].
    /// `world_depth_now`: the copy of the world depth is the depth as it is now.
    pub(crate) fn give_held(&mut self, facts: &WorldFacts, world_depth_now: bool) {
        if self.taken.is_empty() && !self.extension.as_ref().is_some_and(|extension| extension.wants_held()) {
            return;
        }
        let Some(mut extension) = self.extension.take() else { return };
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::Held(&self.taken)), world_depth_now, ("picture: held draws", "depth: held draws"));
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        self.build_extension_lights(facts);
        let own = &self.renderer.extension_lights;
        let lights = own.built.then(|| (own.lights.clone(), own.uniform.clone(), own.table.clone(), own.indexes.clone(), own.origin));
        scene.lights = lights.as_ref().map(|(lights, clusters, table, indexes, origin)| FrameLights { lights, clusters, table, indexes, origin: *origin });
        let part = self.part_begin("held draws (extension: caustics, water volumes, far water)");
        self.begin_pass();
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
        self.part_end(part);
        self.extension = Some(extension);
    }

    /// Gives the extension held shapes in their place, from one copy of the picture. See
    /// [`Extension::draw_held_in_place`].
    pub(crate) fn give_held_here(&mut self, held: &[HeldDraw], facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        // Inside the sorted blends this is no item of its own: the run of draws before it ends here.
        self.end_run();
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::HeldInPlace(held)), false, ("picture: a held draw in place", "depth: a held draw in place"));
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        self.build_extension_lights(facts);
        let own = &self.renderer.extension_lights;
        let lights = own.built.then(|| (own.lights.clone(), own.uniform.clone(), own.table.clone(), own.indexes.clone(), own.origin));
        scene.lights = lights.as_ref().map(|(lights, clusters, table, indexes, origin)| FrameLights { lights, clusters, table, indexes, origin: *origin });
        self.begin_pass();
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world, instances: &self.frame_instances, retained: &mut self.retained, args: &mut self.frame_args, retained_on: self.retained_on };
            let clock = ticks::Clock::now();
            pass.push_debug_group("extension: a held draw in its place");
            extension.draw_held_in_place(&scene, pass, held, &mut replayer);
            pass.pop_debug_group();
            self.renderer.counts.times[TIME_EXT_CLASS] += clock.elapsed();
            self.world_draws = world;
        }
        self.forget_bound();
        self.extension = Some(extension);
    }

    /// The lights for the extension, binned for the view that it draws with, once in a frame:
    /// a held draw in place can come before the held draws, or be the only one.
    fn build_extension_lights(&mut self, facts: &WorldFacts) {
        let renderer = &mut self.renderer;
        if !renderer.extension_lights.built {
            renderer.extension_lights.build(&renderer.device, &renderer.queue, &self.frame_lights, facts.view, facts.projection, self.frame_size, &mut renderer.counts);
        }
    }

    /// Keeps the picture of the frame as it is now, for [`Extension::blend`].
    pub(crate) fn keep_before(&mut self) {
        let Some(source) = self.back_buffer.clone().filter(|_| self.encoder.is_some()) else { return };
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC) || !self.depth.as_ref().is_some_and(|depth| depth.texture().usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)) {
            return;
        }
        let size = source.size();
        if self.before.as_ref().is_none_or(|before| before.size() != size) {
            self.before = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST));
        }
        // A copy is not a part of a pass.
        self.close_pass();
        let Some(encoder) = self.encoder.as_mut() else { return };
        let at = stamp(&self.renderer.stamps, encoder, StampKind::Outside, "picture kept for the blend");
        encoder.copy_texture_to_texture(source.as_image_copy(), self.before.as_ref().unwrap().as_image_copy(), size);
        stamp_end(&self.renderer.stamps, encoder, at);
        self.renderer.counts.picture_copies += 1;
        self.before_kept = true;
    }

    /// Copies what a call of the extension reads ([`Extension::reads`]): the picture of the
    /// frame and the depth of the game, outside of the pass. With `world_depth_now`, the copy
    /// of the world depth is the depth as it is now, and is given for it. The pass is open
    /// again after a copy.
    /// `names`: the names of the picture copy and of the depth copy for the GPU time.
    pub(crate) fn copy_for(&mut self, reads: Reads, world_depth_now: bool, names: (&'static str, &'static str)) -> (Option<wgpu::TextureView>, Option<wgpu::TextureView>) {
        let picture = if reads.picture { self.copy_picture(names.0) } else { None };
        let depth = match (reads.depth, world_depth_now) {
            (false, _) => None,
            (true, true) => self.world_depth_view(),
            (true, false) => self.copy_depth(0, names.1),
        };
        if !self.pass_wanted() {
            self.open_pass(crate::gpu::PassReason::AfterCopy, None, None, None);
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

    /// Copies the picture of the frame, outside of the pass. The pass stays closed. `name`:
    /// the copy's name for the GPU time.
    pub(crate) fn copy_picture(&mut self, name: &'static str) -> Option<wgpu::TextureView> {
        let source = self.back_buffer.clone().filter(|_| self.encoder.is_some())?;
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let size = source.size();
        if self.picture_copy.as_ref().is_none_or(|copy| copy.size() != size) {
            self.picture_copy = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST));
        }
        self.close_pass();
        let encoder = self.encoder.as_mut()?;
        let at =stamp(&self.renderer.stamps, encoder, StampKind::Outside, name);
        encoder.copy_texture_to_texture(source.as_image_copy(), self.picture_copy.as_ref().unwrap().as_image_copy(), size);
        stamp_end(&self.renderer.stamps, encoder, at);
        self.renderer.counts.picture_copies += 1;
        self.picture_copy.as_ref().map(|copy| copy.create_view(&Default::default()))
    }

    /// Copies the depth as it is now, outside of the pass: into the copy of the moment (0),
    /// the copy of the world (1, for [`Extension::post`]), the copy of the draws of
    /// [`Extension::before_world`] (2), or the depth before (3) and after (4) a late sorted
    /// run ([`late_depth`]). The caller opens the pass again. `name`: the copy's name for the
    /// GPU time.
    pub(crate) fn copy_depth(&mut self, which: u32, name: &'static str) -> Option<wgpu::TextureView> {
        let source = self.depth.as_ref().map(|depth| depth.texture().clone())?;
        // A multisampled depth cannot be copied: the resolve pass writes the nearest sample
        // of each pixel (`multisample.rs`).
        let resolve = source.sample_count() > 1;
        if !resolve && !source.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        self.close_pass();
        let encoder = self.encoder.as_mut()?;
        let size = source.size();
        let copy = match which {
            1 => &mut self.world_depth,
            2 => &mut self.before_depth,
            3 => &mut self.late_before,
            4 => &mut self.late_after,
            _ => &mut self.depth_copy,
        };
        if copy.as_ref().is_none_or(|copy| copy.size() != size) {
            // A target too: the resolve writes it, and the depth of the first person goes into
            // the copy of the world (`merge_first_person_depth`).
            let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST;
            *copy = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), usage));
        }
        if resolve && self.renderer.multisample.is_none() {
            return None;
        }
        let at = stamp(&self.renderer.stamps, encoder, StampKind::Outside, name);
        if resolve {
            let multisample = self.renderer.multisample.as_ref()?;
            let samples = source.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
            multisample.resolve_depth(&self.renderer.device, encoder, &samples, &copy.as_ref().unwrap().create_view(&Default::default()));
        } else {
            encoder.copy_texture_to_texture(source.as_image_copy(), copy.as_ref().unwrap().as_image_copy(), size);
        }
        stamp_end(&self.renderer.stamps, encoder, at);
        self.renderer.counts.depth_copies += 1;
        copy.as_ref().map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }))
    }

    /// Puts the depth of the first-person scene into the copy of the world depth that the
    /// post-processing reads, as the nearer of the two in each pixel: the game draws the
    /// arms after a clear of the depth, so the copy of the world has the depth behind them.
    /// The Direct3D 9 path of MGE XE merges the depth of its later scenes the same way
    /// (`renderStage2`, the nearest of the two). `projection` is that of the first person.
    pub(crate) fn merge_first_person_depth(&mut self, projection: &[f32; 16]) {
        let Some(world_projection) = self.world_depth_projection.filter(|_| self.world_depth_kept) else { return };
        let Some(arms) = self.copy_depth(0, "depth: first person") else {
            self.open_pass(crate::gpu::PassReason::AfterCopy, None, None, None);
            return;
        };
        if let (Some(target), Some(encoder)) = (self.world_depth.as_ref(), self.encoder.as_mut()) {
            let merge = self.renderer.depth_merge.get_or_insert_with(|| depth_merge::DepthMerge::new(&self.renderer.device, self.renderer.depth_format));
            let at = stamp(&self.renderer.stamps, encoder, StampKind::Outside, "first person depth merged into the world's");
            merge.merge(&self.renderer.device, encoder, &arms, &target.create_view(&Default::default()), projection, &world_projection);
            stamp_end(&self.renderer.stamps, encoder, at);
            self.renderer.counts.depth_copies += 1;
        }
        self.open_pass(crate::gpu::PassReason::AfterCopy, None, None, None);
    }

    /// Gives the extension its post-processing call. See [`Extension::post`].
    pub(crate) fn post_process(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        // The depth of the end of the world scene, when it was kept; else a copy of that of now.
        let mut reads = extension.reads(ReadingCall::Post);
        let world_depth = self.world_depth_view().filter(|_| reads.depth);
        reads.depth &= world_depth.is_none();
        let (picture, depth_now) = self.copy_for(reads, false, ("picture: the post chain", "depth: the post chain"));
        let depth = world_depth.or(depth_now);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        // The extension writes the picture with one sample: the back buffer, which the pass
        // resolved into, with more than one sample; `color` is it with one.
        let picture_target = self.back_view.clone().or_else(|| self.color.clone());
        if picture_target.is_some() && self.encoder.is_some() {
            self.close_pass();
        }
        if let (Some(color), Some(encoder)) = (picture_target, self.encoder.as_mut()) {
            let clock = ticks::Clock::now();
            let at = stamp(&self.renderer.stamps, encoder, StampKind::Part, "the post chain (extension)");
            encoder.push_debug_group("extension: post-processing");
            extension.post(&scene, encoder, &color);
            encoder.pop_debug_group();
            stamp_end(&self.renderer.stamps, encoder, at);
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
        // The draws after the post-processing (the interface of the game) go into the
        // multisampled target: it gets the picture of the back buffer first.
        self.load_color();
        self.open_pass(crate::gpu::PassReason::AfterExtension, None, None, None);
    }

    /// Gives the extension its call in place of the draws of a class. See
    /// [`Extension::draw_class`]. `drawn` is the number of world draws before the call. With
    /// a plane for the class ([`FrameParameters::class_plane`]): the world draws that blend,
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
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::Class(class)), false, ("picture: a class of draws", "depth: a class of draws"));
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let commands = self.renderer.commands.clone();
        let mut scene = facts.view(&device, &queue, &commands, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        self.begin_pass();
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
        self.begin_pass();
        let Some(pass) = self.pass.as_mut() else { return };
        let world = std::mem::take(&mut self.world_draws);
        let samples = self.pass_samples;
        let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world[range], instances: &self.frame_instances, retained: &mut self.retained, args: &mut self.frame_args, retained_on: self.retained_on };
        replayer.replay(
            pass,
            &ReplayView {
                world: None,
                view: None,
                projection: None,
                target_size,
                samples,
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

/// Whether the fog of the world is on, and its words: the colour, the start, the end, the range
/// flag. The scene record has the game's own fog property, which the plugin reads each frame: a
/// record without fog means that the property is off, with its colour kept and no distances.
/// Only without a record (an older capture), the fog of the last draw with fog.
fn world_fog(record: Option<(u32, Option<[u32; 4]>)>, off_color: u32, last_draw: [u32; 4]) -> (bool, [u32; 4]) {
    match record {
        Some((_, Some(fog))) => (true, fog),
        Some((_, None)) => (false, [off_color, 0, 0, 0]),
        None => (true, last_draw),
    }
}

/// The start of a named span of the frame's GPU work in the encoder, with no pass open
/// (`stamps::Stamps`).
fn stamp(stamps: &Stamps, encoder: &mut wgpu::CommandEncoder, kind: StampKind, name: &'static str) -> Stamp {
    stamps.begin(encoder, name, kind)
}

/// The end of a span that [`stamp`] started.
fn stamp_end(stamps: &Stamps, encoder: &mut wgpu::CommandEncoder, at: Stamp) {
    stamps.end(encoder, at);
}

impl Player {
    /// Starts a part of the frame for the GPU time: in the open pass, else in the encoder.
    /// The run of draws that is open goes into the pass first, so the part's stamps hold its
    /// draws.
    pub(crate) fn part_begin(&mut self, name: &'static str) -> Stamp {
        if !self.renderer.stamps.on() {
            return Stamp::NONE;
        }
        self.end_run();
        let stamps = self.renderer.stamps.clone();
        match (self.pass.as_mut(), self.encoder.as_mut()) {
            (Some(pass), _) => stamps.begin_in_pass(pass, name, StampKind::Part),
            (None, Some(encoder)) => stamps.begin(encoder, name, StampKind::Part),
            _ => Stamp::NONE,
        }
    }

    /// Ends a part that [`Player::part_begin`] started, after the draws of its run.
    pub(crate) fn part_end(&mut self, at: Stamp) {
        if !self.renderer.stamps.on() {
            return;
        }
        self.end_run();
        let stamps = self.renderer.stamps.clone();
        match (self.pass.as_mut(), self.encoder.as_mut()) {
            (Some(pass), _) => stamps.end_in_pass(pass, at),
            (None, Some(encoder)) => stamps.end(encoder, at),
            _ => {}
        }
    }
}

#[cfg(test)]
mod world_fog_tests {
    use super::world_fog;

    const LAST_DRAW: [u32; 4] = [1, 2, 3, 0];
    const SCENE: [u32; 4] = [4, 5, 6, 1];
    const OFF_COLOR: u32 = 7;

    #[test]
    fn a_record_with_fog_gives_its_fog() {
        assert_eq!(world_fog(Some((0, Some(SCENE))), OFF_COLOR, LAST_DRAW), (true, SCENE));
    }

    #[test]
    fn a_record_with_the_fog_off_keeps_it_off_with_the_colour_of_the_property() {
        assert_eq!(world_fog(Some((0, None)), OFF_COLOR, LAST_DRAW), (false, [OFF_COLOR, 0, 0, 0]));
    }

    #[test]
    fn only_no_record_falls_back_to_the_last_draw() {
        assert_eq!(world_fog(None, OFF_COLOR, LAST_DRAW), (true, LAST_DRAW));
    }
}
