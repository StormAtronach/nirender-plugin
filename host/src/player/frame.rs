//! The frame and the pass of `Player`: begin, the targets, the draws into the pass, the end
//! and the pictures. The struct is in lib.rs; the other concerns are beside this file.

use crate::*;

impl Player {
    pub(crate) fn new(renderer: Renderer) -> Player {
        Player {
            renderer,
            instances: HashMap::new(),
            retained_on: switches().retained,
            retained: RetainedSet::new(),
            retained_values: None,
            stream: Stream {
                view: [0.0; 16],
                projection: [0.0; 16],
                viewport: None,
                back_buffer: true,
                origin: None,
                interior: false,
                sky_hidden: false,
                fog_range: None,
                object_class: 0,
                sky: None,
                // D3DFILL_SOLID is 3.
                shape: ShapeFacts { geometry: 0, fvf: 0, stride: 0, world: [0.0; 16], relative: false, material: [0.0; 17], ambient: 0, fill: 3, lights: Vec::new() },
                shape_fog: None,
                fog: [0; 4],
                ambient_source: 0,
                shape_bones: None,
                shape_bound: None,
                shape_queued: false,
                queue_marked: false,
                world_walk: false,
                scene_target: false,
                queue_flush: false,
            },
            encoder: None,
            pass: None,
            color: None,
            depth: None,
            back_buffer: None,
            back_view: None,
            ms_color: None,
            ms_depth: None,
            pass_samples: 1,
            sent_in_part: false,
            last_picture: None,
            reply: None,
            tile_out: None,
            tile_reads: Vec::new(),
            tile_pipelines: HashMap::new(),
            shots: Vec::new(),
            early_pictures: Vec::new(),
            overlay_record: None,
            film_shot: None,
            film_shot_at: 1,
            film_draws: 0,
            extension: None,
            items: Vec::new(),
            world_draws: Vec::new(),
            held_draws: Vec::new(),
            taken: Vec::new(),
            frame_lights: Vec::new(),
            frame_bones: Vec::new(),
            frame_instances: Vec::new(),
            parameters: None,
            plan: plan::Plan::default(),
            frame_sorted: Vec::new(),
            world_near: None,
            replaced: 0,
            takes: 0,
            shadow_reach: 0.0,
            plane_ahead: None,
            split: None,
            posted: false,
            world_matrices: None,
            world_sun: None,
            retained_sun: None,
            world_sun_logged: 0,
            world_ambient: [0.0; 3],
            target: None,
            wants_texture: false,
            pass_target: None,
            target_size: (0, 0),
            viewport: None,
            before: None,
            before_kept: false,
            picture_copy: None,
            depth_copy: None,
            world_depth: None,
            world_depth_kept: false,
            world_depth_projection: None,
            first_person_drawn: false,
            before_depth: None,
            before_depth_kept: false,
            mark_times: [0; TIME_COUNT],
            mark_frames: 0,
            mark_draws: 0,
            mark_runs: 0,
            mark_gpu: (0.0, 0.0, 0),
            mark_work: [0; 4],
            mark_errors: 0,
            bound: BoundGroups::default(),
            pass_pipeline: None,
            pass_stencil: None,
            pass_vertices: None,
            pass_indexes: None,
            pass_shared: false,
            pass_arrays: false,
            run_args: Vec::new(),
            frame_args: Vec::new(),
            size: (0, 0),
            frame_size: (0, 0),
        }
    }

    /// Starts a pass on the targets of the frame: it draws on what is there, or it clears first.
    pub(crate) fn open_pass(&mut self, clear_color: Option<wgpu::Color>, clear_depth: Option<f32>, clear_stencil: Option<u32>) {
        if let Some(pass) = self.pass.as_mut() {
            flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
        }
        drop(self.pass.take());
        let rendered = self.pass_target.and_then(|handle| self.renderer.targets.get(&handle));
        // A texture that the game draws into has one sample; the targets of the frame have
        // `Renderer::samples`, and the pass resolves its colour into the back buffer.
        let views = match rendered {
            Some(texture) => Some((texture.view.clone(), texture.depth.clone(), texture.size, None, 1)),
            None => self.color.clone().zip(self.depth.clone()).map(|(color, depth)| (color, depth, self.frame_size, self.back_view.clone(), self.renderer.samples)),
        };
        let (Some(encoder), Some((color, depth, size, resolve, samples))) = (self.encoder.as_mut(), views) else { return };
        self.target_size = size;
        self.pass_samples = samples;
        let (color, depth) = (&*color, &*depth);
        let stamps = self.renderer.timing.as_mut().and_then(Timing::next_pass);
        let pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("game pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    depth_slice: None,
                    resolve_target: resolve.as_deref(),
                    ops: wgpu::Operations { load: clear_color.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations { load: clear_depth.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store }),
                    stencil_ops: Some(wgpu::Operations { load: clear_stencil.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store }),
                }),
                timestamp_writes: stamps.map(|index| wgpu::RenderPassTimestampWrites {
                    query_set: &self.renderer.timing.as_ref().unwrap().set,
                    beginning_of_pass_write_index: Some(2 * index),
                    end_of_pass_write_index: Some(2 * index + 1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        self.pass = Some(pass);
        self.forget_bound();
        self.apply_viewport();
    }

    /// With more than one sample: writes the back buffer into the multisampled colour target,
    /// after a write of the back buffer outside the pass (`Extension::post`). The pass stays
    /// closed. Nothing with one sample.
    pub(crate) fn load_color(&mut self) {
        if self.back_view.is_none() {
            return;
        }
        if let Some(pass) = self.pass.as_mut() {
            flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
        }
        drop(self.pass.take());
        let (Some(multisample), Some(source), Some(target), Some(encoder)) = (self.renderer.multisample.as_ref(), self.back_view.as_ref(), self.color.as_ref(), self.encoder.as_mut()) else { return };
        multisample.load_color(&self.renderer.device, encoder, source, target);
        self.renderer.counts.picture_copies += 1;
    }

    /// Ends the run of draws that is open in the pass, if any.
    pub(crate) fn end_run(&mut self) {
        if let Some(pass) = self.pass.as_mut() {
            flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
        }
    }

    /// The state of the pass is not known: a new pass, or one that the extension drew into.
    pub(crate) fn forget_bound(&mut self) {
        self.bound.forget();
        self.pass_pipeline = None;
        self.pass_stencil = None;
        self.pass_vertices = None;
        self.pass_indexes = None;
        self.pass_shared = false;
        self.pass_arrays = false;
    }

    /// Gives the viewport of the stream to the pass, cut to the size of the targets.
    pub(crate) fn apply_viewport(&mut self) {
        let (Some(pass), Some((rectangle, range))) = (self.pass.as_mut(), self.viewport) else { return };
        let (width, height) = (self.target_size.0 as f32, self.target_size.1 as f32);
        let (x, y) = (rectangle[0].min(width - 1.0).max(0.0), rectangle[1].min(height - 1.0).max(0.0));
        let (w, h) = (rectangle[2].min(width - x), rectangle[3].min(height - y));
        if w >= 1.0 && h >= 1.0 {
            pass.set_viewport(x, y, w, h, range[0], range[1]);
        }
    }

    /// Starts a frame that draws into these targets. Until the next start, the records
    /// change only the state and the resources. With `keep`, the frame starts with the
    /// picture that is in the colour target: the game can read the last frame before it
    /// clears the target.
    pub(crate) fn begin_frame(&mut self, color: &wgpu::Texture, depth: Rc<wgpu::TextureView>, keep: bool) {
        self.frame_size = (color.width(), color.height());
        self.items.clear();
        self.world_draws.clear();
        self.frame_lights.clear();
        self.renderer.extension_lights.clear();
        self.frame_bones.clear();
        self.frame_instances.clear();
        self.frame_instances.push(Instance { world: IDENTITY, record: [0; 4] });
        self.renderer.begin_frame_arrays();
        self.renderer.pin_into_frame();
        self.retained_settle_late();
        self.renderer.scene_slots_used = 1;
        // The scene record of the last frame stays until the next one comes: the world's
        // facts are taken at the world's start, before the walk's record of the frame. Do not
        // clear it here. A stream without the record keeps None.
        self.run_args.clear();
        self.frame_args.clear();
        self.renderer.indirect_written = 0;
        self.parameters = None;
        self.plan.frame_start();
        self.held_draws.clear();
        self.taken.clear();
        self.frame_sorted.clear();
        self.retained.walk.frame_start();
        self.world_near = None;
        self.replaced = 0;
        self.plane_ahead = None;
        self.split = None;
        self.posted = false;
        self.world_matrices = None;
        self.world_depth_kept = false;
        self.world_depth_projection = None;
        self.first_person_drawn = false;
        self.before_depth_kept = false;
        self.target = None;
        self.pass_target = None;
        self.before_kept = false;
        self.stream.fog_range = None;
        self.stream.sky = None;
        self.stream.back_buffer = true;
        self.sent_in_part = false;
        let mut encoder = self.renderer.device.create_command_encoder(&Default::default());
        if let Some(timing) = self.renderer.timing.as_mut() {
            timing.begin(&mut encoder);
        }
        self.encoder = Some(encoder);
        // The ranges of the geometry replaced in the frame before: the GPU is done with it.
        for range in std::mem::take(&mut self.renderer.retired) {
            self.renderer.vertex_pool.give(&range);
        }
        for range in std::mem::take(&mut self.renderer.retired_indexes) {
            self.renderer.index_pool.give(&range);
        }
        self.renderer.record_staging.clear();
        self.renderer.record_count = 0;
        self.renderer.record_written = 0;
        self.renderer.record_extras = 0;
        self.renderer.instance_extras = 0;
        let back_view = Rc::new(color.create_view(&Default::default()));
        self.back_buffer = Some(color.clone());
        if self.renderer.samples > 1 {
            // The draws go into the multisampled targets; the colour resolves into the back
            // buffer at the end of each pass. The multisampled colour keeps the last frame
            // between the frames, so a frame that keeps the picture starts from it.
            let (width, height) = (color.width(), color.height());
            if self.ms_color.as_ref().is_none_or(|ms| (ms.width(), ms.height(), ms.format()) != (width, height, color.format())) {
                self.ms_color = Some(multisampled_texture(&self.renderer.device, width, height, color.format(), wgpu::TextureUsages::RENDER_ATTACHMENT, self.renderer.samples));
                self.ms_depth = Some(multisampled_texture(&self.renderer.device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, self.renderer.samples));
            }
            self.color = Some(Rc::new(self.ms_color.as_ref().unwrap().create_view(&Default::default())));
            self.depth = Some(Rc::new(self.ms_depth.as_ref().unwrap().create_view(&Default::default())));
            self.back_view = Some(back_view);
        } else {
            self.color = Some(back_view);
            self.depth = Some(depth);
            self.back_view = None;
        }
        if keep {
            self.open_pass(None, None, None);
        } else {
            self.open_pass(Some(wgpu::Color::BLACK), Some(1.0), Some(0));
        }
    }

    /// Sends the commands of the frame up to now to the GPU. The frame goes on after `between`.
    pub(crate) fn flush<T>(&mut self, between: impl FnOnce(&mut Player) -> T) -> T {
        self.run_items();
        let in_frame = self.drawing();
        if let Some(pass) = self.pass.as_mut() {
            flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
        }
        drop(self.pass.take());
        if let Some(encoder) = self.encoder.take() {
            // The first part of a frame: the back buffer has the last complete picture
            // until these commands run. A copy of it stays for the window (see
            // `shown_picture`).
            let copy = (in_frame && !std::mem::replace(&mut self.sent_in_part, true)).then(|| self.keep_picture()).flatten();
            self.renderer.upload_indirect(&self.frame_args);
            crate::gpu::submit(&self.renderer.queue, self.renderer.commands.take().into_iter().chain(copy).chain([encoder.finish()]));
        }
        let result = between(self);
        if in_frame {
            self.encoder = Some(self.renderer.device.create_command_encoder(&Default::default()));
            self.open_pass(None, None, None);
        }
        result
    }

    /// The commands that copy the back buffer into `last_picture`.
    pub(crate) fn keep_picture(&mut self) -> Option<wgpu::CommandBuffer> {
        let source = self.back_buffer.as_ref()?;
        let (width, height) = (source.width(), source.height());
        if self.last_picture.as_ref().is_none_or(|kept| (kept.width(), kept.height()) != (width, height)) {
            self.last_picture = Some(target_texture(&self.renderer.device, width, height, source.format(), wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC));
        }
        let mut encoder = self.renderer.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(source.as_image_copy(), self.last_picture.as_ref()?.as_image_copy(), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
        Some(encoder.finish())
    }

    /// The last complete picture of the game: the back buffer, or the copy from before the
    /// frame in progress when a part of that frame is in the back buffer.
    pub(crate) fn shown_picture(&self) -> Option<&wgpu::Texture> {
        if self.sent_in_part { self.last_picture.as_ref() } else { self.back_buffer.as_ref() }
    }

    /// The pixels of a rendered texture, or of the back buffer with the handle 0: the width,
    /// the height and 4 bytes for each pixel, blue first.
    pub(crate) fn read_pixels(&mut self, handle: u32) -> Option<(u32, u32, Vec<u8>)> {
        let texture = match handle {
            0 => self.back_buffer.clone()?,
            _ => self.renderer.targets.get(&handle)?.texture.clone(),
        };
        if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let (width, height) = (texture.width(), texture.height());
        let blue_first = matches!(texture.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        self.flush(|player| {
            let device = player.renderer.device.clone();
            let mut encoder = device.create_command_encoder(&Default::default());
            let (buffer, row) = copy_picture(&device, &mut encoder, &texture, width, height);
            player.renderer.upload_indirect(&player.frame_args);
            crate::gpu::submit(&player.renderer.queue, player.renderer.commands.take().into_iter().chain([encoder.finish()]));
            // A device lost during the read: no pixels, and the game gets a width of 0.
            let mapped_ok = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = mapped_ok.clone();
            buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| flag.store(result.is_ok(), std::sync::atomic::Ordering::Release));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            if !mapped_ok.load(std::sync::atomic::Ordering::Acquire) || crate::gpu::lost() {
                return None;
            }
            let Ok(mapped) = buffer.slice(..).get_mapped_range() else { return None };
            let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
            for y in 0..height as usize {
                pixels.extend_from_slice(&mapped[y * row..y * row + width as usize * 4]);
            }
            if !blue_first {
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
            }
            player.renderer.counts.pixel_reads += 1;
            Some((width, height, pixels))
        })
    }

    pub(crate) fn drawing(&self) -> bool {
        self.encoder.is_some()
    }

    /// Ends the frame and sends its commands to the GPU. `before_submit` can add commands,
    /// for example a copy of the picture.
    pub(crate) fn end_frame(&mut self, before_submit: impl FnOnce(&mut wgpu::CommandEncoder)) {
        self.post_item();
        self.run_items();
        // The pictures that an extension wants and did not get after its post-processing.
        if let Some(extension) = self.extension.as_mut() {
            self.shots.extend(extension.pictures().into_iter().map(|request| request.path));
        }
        let clock = ticks::Clock::now();
        if let Some(pass) = self.pass.as_mut() {
            flush_run(pass, &mut self.renderer, &mut self.run_args, &mut self.frame_args);
        }
        drop(self.pass.take());
        self.color = None;
        self.depth = None;
        self.back_view = None;
        if let Some(mut encoder) = self.encoder.take() {
            before_submit(&mut encoder);
            let renderer = &mut self.renderer;
            let slot = renderer.timing.as_mut().and_then(|timing| timing.end(&renderer.device, &mut encoder, &mut renderer.counts));
            renderer.upload_indirect(&self.frame_args);
            crate::gpu::submit(&renderer.queue, renderer.commands.take().into_iter().chain([encoder.finish()]));
            if let (Some(timing), Some(slot)) = (renderer.timing.as_ref(), slot) {
                timing.map(slot);
            }
        }
        let blue_first = matches!(self.renderer.target_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        for (buffer, row, width, height, path) in std::mem::take(&mut self.early_pictures) {
            save_picture(&self.renderer.device, &buffer, row, width, height, blue_first, &path);
        }
        self.renderer.counts.times[TIME_END] += clock.elapsed();
    }

}
