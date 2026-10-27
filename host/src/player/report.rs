//! The marks and the counts of a run, printed for the log.

use crate::*;

impl Player {
    /// Prints the counts since the last mark, for one step of a scenario.
    pub(crate) fn mark(&mut self, text: &str) {
        let counts = &self.renderer.counts;
        let frames = counts.frames - self.mark_frames;
        let draws = counts.draws - self.mark_draws;
        let runs = counts.pipeline_runs - self.mark_runs;
        let busy: u64 = counts.times.iter().zip(self.mark_times).map(|(now, before)| *now - before).sum();
        let per_frame = |time: u64| ticks::duration(time).as_secs_f64() * 1000.0 / frames.max(1) as f64;
        println!(
            "mark {text}: {frames} frames, {:.1} draws in {:.1} pipeline runs for each frame, busy {:.3} ms for each frame:",
            draws as f64 / frames.max(1) as f64,
            runs as f64 / frames.max(1) as f64,
            per_frame(busy)
        );
        for ((name, now), before) in TIME_NAMES.iter().zip(counts.times).zip(self.mark_times) {
            println!("  {:6.3}  {name}", per_frame(now - before));
        }
        let timed = counts.gpu_frames - self.mark_gpu.2;
        if timed != 0 {
            let per_frame = |now: f64, before: f64| (now - before) / 1e6 / timed as f64;
            println!("  gpu: {:.3} ms in the passes, {:.3} ms the frame, for each of {timed} timed frames", per_frame(counts.gpu_passes, self.mark_gpu.0), per_frame(counts.gpu_frame, self.mark_gpu.1));
        }
        let work = gpu::work();
        let delta: Vec<u64> = work.iter().zip(self.mark_work).map(|(now, before)| now - before).collect();
        println!(
            "  work: {:.2} submits, {:.3} ms in them; {:.2} bind groups made for the frame, {:.3} ms in them; for each frame",
            delta[0] as f64 / frames.max(1) as f64,
            delta[1] as f64 / 1e6 / frames.max(1) as f64,
            delta[2] as f64 / frames.max(1) as f64,
            delta[3] as f64 / 1e6 / frames.max(1) as f64
        );
        let passes = gpu::passes();
        let delta: [u64; gpu::PASS_COUNTS] = core::array::from_fn(|i| passes[i] - self.mark_passes[i]);
        println!("  {}", gpu::pass_line(&delta, frames as u64));
        self.mark_passes = passes;
        println!("  {}", self.renderer.held_line());
        let errors = gpu::errors();
        if errors != self.mark_errors {
            println!("  gpu errors: {}", errors - self.mark_errors);
        }
        self.mark_work = work;
        self.mark_errors = errors;
        self.mark_gpu = (counts.gpu_passes, counts.gpu_frame, counts.gpu_frames);
        self.mark_times = counts.times;
        self.mark_frames = counts.frames;
        self.mark_draws = counts.draws;
        self.mark_runs = counts.pipeline_runs;
    }

    pub(crate) fn print_counts(&self) {
        let c = &self.renderer.counts;
        println!("draws {} in {} pipeline runs, pipelines {}, textures {}, samplers {}", c.draws, c.pipeline_runs, self.renderer.pipelines.len(), c.textures, self.renderer.samplers.len());
        println!(
            "geometry: {} records ({:.1} MB; {} of a known geometry written into its ranges, {} into new ranges because a draw of the frame named the old), {} draws from them, {} records that did not parse, {} draws of more index entries than their geometry has",
            c.geometry_records,
            c.geometry_bytes as f64 / 1e6,
            c.geometry_in_place,
            c.geometry_drawn_in_frame,
            c.geometry_draws,
            c.geometry_bad,
            c.geometry_short
        );
        println!("material: {} records; {} records of any tag shorter than their fixed part, skipped", c.material_records, c.short_records);
        println!("{}", self.renderer.held_line());
        println!("shapes: {} records, {} draws from them, {} without a geometry or a material", c.shape_records, c.shape_draws, c.shape_missing);
        println!("instances: {} records and {} moves ({} checked against a full record, {} different), {} draws from them, {} draws of an instance not known, {} shape twins skipped, {} blends of the engine merged into the sorted list", c.instance_records, c.instance_moves, c.moves_checked, c.moves_different, c.instance_draws, c.instance_missing, c.shape_twins, c.merged_blends);
        println!(
            "sorted queue: {} places of the table's blends drawn in the engine's order, {} of an instance not known, {} outside the world scene, {} of an instance the table does not sort; {} sorted blends of the table without a place, put between by depth ({} of them in frames with no place at all); {} places after the frame's first sorted blends; {} items of the table's blends before a depth clear after the world (the queue still open)",
            c.queued_places, c.queued_unknown, c.queued_outside, c.queued_not_sorted, c.sorted_without_place, c.sorted_without_place_no_places, c.queued_late_places, c.blends_before_clear
        );
        println!(
            "walk places: {} places of the table's blends in the engine's walk, {} of an instance not known, {} outside a world view, {} after the table's main view; {} blends drawn in place by the engine that had no place, drawn by bucket; {} places whose walk order differs from their bucket order",
            c.walk_places, c.walk_unknown, c.walk_outside, c.walk_late, self.retained.counts.walk_without_place, self.retained.counts.walk_reordered
        );
        println!(
            "scene targets: {} scenes named their target, {} of them another target than the SetRenderTarget calls gave; {} draws of a world view with an orthographic projection",
            c.scene_targets, c.scene_target_changed, c.orthographic_world_draws
        );
        let r = &self.retained.counts;
        println!(
            "retained: {} instances in {} slots and {} buckets; {} drawn ({} sorted blends), {} culled, {} hidden, {} calls in {} views; {} moved in place, {} made again for a geometry sent again; {} without a geometry, {} without a material; the origin moved {} times",
            r.instances, r.slots, r.buckets, r.drawn, r.sorted, r.culled, r.hidden, r.calls, r.views, r.moves, r.refreshed, r.no_geometry, r.no_material, r.rebases
        );
        let (morph_steps, morph_skipped) = self.renderer.morph_counts();
        println!(
            "morphs: {} records of targets, {} steps made on the GPU, {} not made, {} for a geometry not held",
            c.morph_targets, morph_steps, morph_skipped, c.morph_missing
        );
        println!(
            "uv controllers: {} records with the state of the vertices, {} steps, {} for a geometry not held",
            c.uv_bases, c.uv_steps, c.uv_missing
        );
        println!(
            "retained instance uploads: {} whole, {} runs of changed entries with {:.1} MB{}",
            c.instance_uploads_whole,
            c.instance_upload_runs,
            c.instance_upload_bytes as f64 / 1e6,
            if switches().instance_upload_all { " (MGE_XE_INSTANCE_UPLOAD=all: the segment goes whole above 64 changed slots)" } else { "" }
        );
        println!("rules of the engine not followed: {} shapes with flat shading, {} shapes with specular lighting", c.shape_flat, c.shape_specular);
        println!("skinned shapes {}, particle systems {} ({} of them from INSTANCE_PARTICLES records)", c.shape_skinned, c.shape_particles, c.instance_particles);
        match self.renderer.particle_quads_made() {
            Some((systems, particles)) => println!("particle quads made on the GPU: {systems} systems, {particles} particles"),
            None => println!("particle quads made on the GPU: none ({})", if switches().particle_quads_cpu { "MGE_XE_PARTICLE_QUADS=cpu" } else { "no system yet" }),
        }
        println!("shapes held for the extension {}", c.held);
        println!("classes of the extension: {} calls at a draw of the class, {} calls in frames without a draw of the class", c.class_calls, c.class_calls_without_draw);
        println!(
            "lights: {} records, {} destroyed, {} known now, {} lights of a shape without a record",
            c.light_records,
            c.lights_gone,
            self.renderer.lights_known.len(),
            c.light_missing
        );
        println!(
            "clusters: {:.1} lights and {:.0} entries for each frame, {} frames with more lights than a mask holds; {:.3} ms of the helper for each frame (with the world mode's shade of the table's lights)",
            c.cluster_lights as f64 / c.frames.max(1) as f64,
            c.cluster_entries as f64 / c.frames.max(1) as f64,
            c.cluster_frames_many,
            ticks::duration(c.times[TIME_CLUSTERS]).as_secs_f64() * 1000.0 / c.frames.max(1) as f64
        );
        println!("lights for the extension: {:.1} for each frame", c.extension_lights as f64 / c.frames.max(1) as f64);
        println!(
            "copies for the extension: {:.2} of the picture and {:.2} of the depth for each frame",
            c.picture_copies as f64 / c.frames.max(1) as f64,
            c.depth_copies as f64 / c.frames.max(1) as f64
        );
        // With `MGE_XE_GPU_TIME=1`: the GPU time of all timed frames (a replay has no marks).
        if c.gpu_frames != 0 {
            println!("gpu: {:.3} ms in the passes, {:.3} ms the frame, for each of {} timed frames", c.gpu_passes / 1e6 / c.gpu_frames as f64, c.gpu_frame / 1e6 / c.gpu_frames as f64, c.gpu_frames);
            // The named spans (`stamps::Stamps`) by kind, the most time first. The parts do not
            // overlap one another; a part's time holds the copies and the details inside it.
            let frames = c.gpu_frames as f64;
            let ms = |time: f64| time / 1e6 / frames;
            let mut named = c.gpu_named.clone();
            named.sort_by(|a, b| b.2.total_cmp(&a.2));
            let of = |kind: stamps::StampKind| named.iter().filter(move |(_, k, _, _)| *k == kind);
            let parts: f64 = of(stamps::StampKind::Part).map(|(_, _, time, _)| time).sum();
            println!("gpu by part: {:.3} ms of the frame's {:.3} ms in named parts, {:.3} ms in none", ms(parts), ms(c.gpu_frame), ms(c.gpu_frame - parts));
            for (name, _, time, count) in of(stamps::StampKind::Part) {
                println!("  {:7.3} ms  {name} ({:.2} for each frame)", ms(*time), *count as f64 / frames);
            }
            let copies: f64 = of(stamps::StampKind::Outside).map(|(_, _, time, _)| time).sum();
            println!("gpu copies and resolves: {:.3} ms (one made inside a part, for a class of draws or the held draws, is in that part's time too)", ms(copies));
            for (name, _, time, count) in of(stamps::StampKind::Outside) {
                println!("  {:7.3} ms  {name} ({:.2} for each frame)", ms(*time), *count as f64 / frames);
            }
            if of(stamps::StampKind::Detail).next().is_some() {
                println!("gpu details (inside a part, or in an encoder of their own before the frame):");
                for (name, _, time, count) in of(stamps::StampKind::Detail) {
                    println!("  {:7.3} ms  {name} ({:.2} for each frame)", ms(*time), *count as f64 / frames);
                }
            }
        }
        println!("not drawn: other target {}, vertex format {}", c.other_target, c.no_layout);
        println!(
            "not complete: draws with a texture that is not in the stream {}, more than 6 stages {}; draws with a made or moved texture coordinate {}; the most lights of a draw {}",
            c.missing_textures, c.more_stages, c.coordinate_work, c.most_lights
        );
        println!("texture levels written again {}, not written {}; textures with a block format made as pixels {}", c.texture_updates, c.texture_updates_lost, c.textures_decoded);
        for (reason, count) in &c.lost_reasons {
            println!("  not written: {reason}: {count}");
        }
        println!(
            "rendered textures made {}, draws into them {}, copies of pixels into them {}, pixel reads {}",
            c.render_textures, c.target_draws, c.target_copies, c.pixel_reads
        );
        println!("map tiles copied {}, tile reads given to the game {}, tiles not copied {}", c.tile_copies, c.tile_reads, c.tiles_lost);
        println!("draws in screen coordinates {}, not drawn {}", c.screen_draws, c.screen_draws_lost);
        println!("copies of a draw without room in the records of the frame {}", c.no_record);
        println!(
            "draw records: room for {} in one binding; draws not drawn past it {}, retained slots past it {}",
            self.renderer.record_limit, c.records_over_limit, c.retained_over_limit
        );
        println!("indirect calls {}, draws in them {}, draws one by one for want of room {}", c.indirect_calls, c.indirect_draws, c.indirect_over);
        println!(
            "texture arrays: kept from the frame before in {} frames, laid out again in {}, groups made {}; the most textures of a frame {}, stages over the room {} (of them retained textures with no pinned slot {}; room for {}, pinned at most {}), textures that came again for a handle with a slot {}, pinned slots freed {}, dropped from the end {} (pinned now {}, free {}, the most {}), stages drawn from a slot of the frame until their pinned slot came {}, retained textures pinned before their record {}, pinned slots in use that took another texture {} with other facts and {} with the same",
            c.arrays_kept,
            c.arrays_laid_out,
            c.array_groups_made,
            c.most_frame_textures,
            c.over_array_cap,
            c.pinned_over,
            self.renderer.array_cap,
            self.renderer.array_cap.saturating_sub(crate::FRAME_RESERVE + 1),
            c.textures_replaced,
            c.textures_unpinned,
            c.pinned_dropped,
            self.renderer.pinned_textures.len(),
            self.renderer.pinned_free.len(),
            c.pinned_most,
            c.pinned_late,
            c.pinned_before_record,
            c.pinned_replaced[0],
            c.pinned_replaced[1]
        );
        println!(
            "geometry pools: {:.1} MB of vertices in {} buffers of {:.0} MB in all, {:.1} MB of indexes in {} buffers of {:.0} MB in all",
            self.renderer.vertex_pool.used as f64 / 1e6,
            self.renderer.vertex_pool.chunks(),
            self.renderer.vertex_pool.capacity() as f64 / 1e6,
            self.renderer.index_pool.used as f64 / 1e6,
            self.renderer.index_pool.chunks(),
            self.renderer.index_pool.capacity() as f64 / 1e6
        );
        for (format, count) in &c.texture_formats {
            println!("texture format {format:#x} not made: {count}");
        }
    }
}
