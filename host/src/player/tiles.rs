//! The map tiles of the game (tag `COPY_TILE`, `src/map_tiles.rs` of the plugin). The game
//! draws a tile of its local map into a rendered texture. This program puts a copy of that
//! texture with alpha 1 into the texture of the tile, and reads the copy without a wait: the
//! pixels go into the tile memory of the game when the GPU has them (`deliver_tiles`).
//!
//! The copy is a draw, not a texture copy: the texture of the tile has the format of its D3D8
//! format, which can differ from the format of the rendered texture, and alpha must be 1, as
//! the game makes it in its read (`WorldControllerRenderTarget::renderMap` 0x42F6F0).

use crate::*;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// A read of the pixels of a tile that waits for the GPU.
pub(crate) struct TileRead {
    ticket: u32,
    buffer: wgpu::Buffer,
    /// The length of a row in the buffer.
    row: usize,
    width: u32,
    height: u32,
    blue_first: bool,
    /// 0 while the buffer is not mapped, 1 when it is, 2 when the mapping failed.
    state: Arc<AtomicU8>,
}

const TILE_SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the target.
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureLoad(source, vec2<i32>(position.xy), 0).rgb, 1.0);
}
"#;

impl Player {
    /// The pipeline that copies a tile into a texture of `format`.
    fn tile_pipeline(&mut self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        if let Some(pipeline) = self.tile_pipelines.get(&format) {
            return pipeline.clone();
        }
        let device = &self.renderer.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("map tile copy"), source: wgpu::ShaderSource::Wgsl(TILE_SHADER.into()) });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map tile copy"),
            layout: None,
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.tile_pipelines.insert(format, pipeline.clone());
        pipeline
    }

    /// The record `COPY_TILE`: the texture `texture` of a tile gets a copy of the rendered
    /// texture `target` with alpha 1. With a ticket other than 0 and a game to answer, the
    /// pixels of the copy go to the game later.
    pub(crate) fn copy_tile(&mut self, ticket: u32, target: u32, texture: u32) {
        let source = self.renderer.targets.get(&target).map(|target| target.texture.clone());
        let d3d_format = self.renderer.texture_objects.get(&texture).map(|(_, format)| *format);
        let format = d3d_format.and_then(d3d::texture_format).filter(|&(format, block, bytes)| {
            block == 0 && bytes == 4 && matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm)
        });
        let (Some(source), Some(d3d_format), Some((format, _, _))) = (source, d3d_format, format) else {
            self.renderer.counts.tiles_lost += 1;
            return;
        };
        let (width, height) = (source.width(), source.height());
        let pipeline = self.tile_pipeline(format);
        let read = ticket != 0 && self.tile_out.is_some();
        // The copy goes after the draws of the tile, which are in the commands of the frame.
        let made = self.flush(|player| {
            let device = player.renderer.device.clone();
            let tile = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("map tile"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tile.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map tile copy"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source.create_view(&Default::default())) }],
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map tile copy") });
            {
                crate::gpu::note_other_pass();
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("map tile copy"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            let buffer = read.then(|| copy_picture(&device, &mut encoder, &tile, width, height));
            player.renderer.upload_indirect(&player.frame_args);
            crate::gpu::submit(&player.renderer.queue, player.renderer.commands.take().into_iter().chain([encoder.finish()]));
            if let Some((buffer, row)) = buffer {
                let state = Arc::new(AtomicU8::new(0));
                let mapped = state.clone();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| mapped.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release));
                let blue_first = matches!(format, wgpu::TextureFormat::Bgra8Unorm);
                player.tile_reads.push(TileRead { ticket, buffer, row, width, height, blue_first, state });
            }
            (tile, view)
        });
        let (tile, view) = made;
        self.renderer.set_texture(texture, Rc::new(view));
        self.renderer.texture_objects.insert(texture, (tile, d3d_format));
        self.renderer.counts.tile_copies += 1;
    }

    /// The device is lost: each tile read in progress gets the answer "no pixels" (a width of
    /// 0), because its mapping can never finish, and the game must not wait for it.
    pub(crate) fn fail_tile_reads(&mut self) {
        for read in std::mem::take(&mut self.tile_reads) {
            if let Some(out) = self.tile_out.as_mut() {
                out(read.ticket, 0, 0, &[]);
            }
        }
    }

    /// The device is lost, and the record `tag` is dropped: a request that the game waits for
    /// gets the answer "no pixels" at once.
    pub(crate) fn answer_lost(&mut self, tag: u32, data: &[u8]) {
        match tag {
            tag::READ_PIXELS if data.len() >= 8 => {
                if let Some(reply) = self.reply.as_mut() {
                    reply(word(data, 0), super::feed::reply_generation(data), 0, 0, &[]);
                }
            }
            tag::COPY_TILE if data.len() >= 12 && word(data, 0) != 0 => {
                if let Some(out) = self.tile_out.as_mut() {
                    out(word(data, 0), 0, 0, &[]);
                }
            }
            _ => {}
        }
    }

    /// Gives the game the pixels of the tile reads that the GPU finished, in their order.
    /// It does not wait.
    pub(crate) fn deliver_tiles(&mut self) {
        if self.tile_reads.is_empty() {
            return;
        }
        if crate::gpu::lost() {
            self.fail_tile_reads();
            return;
        }
        let _ = self.renderer.device.poll(wgpu::PollType::Poll);
        while let Some(read) = self.tile_reads.first() {
            let state = read.state.load(Ordering::Acquire);
            if state == 0 {
                break;
            }
            let read = self.tile_reads.remove(0);
            let Some(out) = self.tile_out.as_mut() else { continue };
            if state != 1 {
                out(read.ticket, 0, 0, &[]);
                continue;
            }
            let mapped = read.buffer.slice(..).get_mapped_range().expect("the bytes of a tile");
            let mut pixels = Vec::with_capacity(read.width as usize * read.height as usize * 4);
            for y in 0..read.height as usize {
                pixels.extend_from_slice(&mapped[y * read.row..y * read.row + read.width as usize * 4]);
            }
            drop(mapped);
            read.buffer.unmap();
            if !read.blue_first {
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
            }
            out(read.ticket, read.width, read.height, &pixels);
            self.renderer.counts.tile_reads += 1;
        }
    }
}
