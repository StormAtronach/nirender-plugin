//! The program `nirender-host`: its arguments, its log, and the replay of a capture file
//! (`run`, `run_with`).

use super::*;

pub(crate) fn option(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

extern "system" {
    fn SetStdHandle(which: u32, handle: *mut core::ffi::c_void) -> i32;
}

/// Sends the text of this program, and the text of a failure, into a file.
pub(crate) fn log_to(path: &str) {
    use std::os::windows::io::IntoRawHandle;
    if let Ok(file) = std::fs::File::create(path) {
        let handle = file.into_raw_handle();
        // STD_OUTPUT_HANDLE and STD_ERROR_HANDLE
        unsafe {
            SetStdHandle(-11i32 as u32, handle as _);
            SetStdHandle(-12i32 as u32, handle as _);
        }
    }
}

/// Runs the renderer as the program `nirender-host` does. `args` are the arguments of the
/// command line, with the name of the program first.
///
/// With `--live <name>` it draws the stream of the shared memory with that name into a
/// window until the stream ends: this is the call for a program that has the renderer as a
/// part, for example the helper of MGE XE. Without it, it draws a frame of a capture file
/// into a picture file.
pub fn run(args: Vec<String>) {
    run_with(args, None)
}

/// `run` with a program that draws more than the game does: in the live mode, and for a
/// capture file.
pub fn run_with(args: Vec<String>, extension: Option<Box<dyn Extension>>) {
    if let Some(path) = option(&args, "--log") {
        log_to(&path);
    }
    println!("nirender-host v{}{}", env!("CARGO_PKG_VERSION"), extension.as_ref().map_or(String::new(), |e| format!(", {}", e.name())));
    let backends = match option(&args, "--backend").as_deref() {
        Some("dx12") => wgpu::Backends::DX12,
        Some("vulkan") => wgpu::Backends::VULKAN,
        _ => wgpu::Backends::PRIMARY,
    };
    if let Some(name) = option(&args, "--live") {
        return live::run(&name, backends, &args, extension);
    }
    if args.len() < 3 {
        eprintln!("use: nirender-host <capture file> <picture.png> [--backend vulkan|dx12] [--frame N | --frames-at N,M,...]");
        eprintln!("     nirender-host --live <name of the shared memory> [--backend vulkan|dx12] [--frames N] [--shot picture.png]");
        std::process::exit(2);
    }
    let file = std::fs::read(&args[1]).expect("the capture file");
    assert_eq!(&file[..8], b"NIRCAP01", "not a capture file");
    let mut records = Vec::new();
    let mut at = 8;
    while at + 8 <= file.len() {
        let (tag, length) = (word(&file, at), word(&file, at + 4) as usize);
        records.push((tag, &file[at + 8..at + 8 + length]));
        at += 8 + length;
    }
    let frames = records.iter().filter(|(tag, _)| *tag == tag::FRAME_END).count();
    // `--frames-at`: several frames of one replay, each into `<picture>-<frame as 5 digits>.png`
    // (the frames count from 0, as `--frame`). One start of the helper for a capture of many
    // stops, in place of one for each.
    let frames_at: Vec<usize> = option(&args, "--frames-at").map(|text| text.split(',').filter_map(|n| n.trim().parse().ok()).collect()).unwrap_or_default();
    let wanted = match frames_at.iter().max() {
        Some(&last) => last,
        None => option(&args, "--frame").and_then(|text| text.parse().ok()).unwrap_or(frames.saturating_sub(1)),
    };
    // The size of the back buffer: the first record, or the largest viewport of an old capture.
    let (mut width, mut height) = (0u32, 0u32);
    for (tag, data) in &records {
        if *tag == tag::HEADER {
            (width, height) = (word(data, 0), word(data, 4));
            break;
        }
        if *tag == tag::CALL && word(data, 0) == CALL_VIEWPORT {
            let viewport = &data[16 + word(data, 12) as usize * 4..];
            if viewport.len() >= 24 {
                width = width.max(word(viewport, 0) + word(viewport, 8));
                height = height.max(word(viewport, 4) + word(viewport, 12));
            }
        }
    }
    if width == 0 || height == 0 {
        (width, height) = (1280, 720);
    }
    println!("{} records, {frames} frames, frame {wanted} into {width} x {height}", records.len());

    let mut player = Player::new(Renderer::new(backends, None, wgpu::TextureFormat::Rgba8Unorm, extension_shaders(extension.as_deref()), extension.as_ref().map_or(0, |extension| extension.texture_slots())).0);
    // The filter of the extension's settings, as in the live mode; the argument is for a test.
    player.renderer.anisotropy =
        option(&args, "--anisotropy").and_then(|text| text.parse().ok()).or(extension.as_ref().map(|extension| extension.anisotropy())).unwrap_or(1);
    if let Some(mut extension) = extension {
        extension.offline();
        player.extension = Some(extension);
    }
    let device = player.renderer.device.clone();
    let target = target_texture(&device, width, height, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let depth = target_texture(&device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
    let mut frame = 0;
    for (tag, data) in &records {
        // Each frame up to the wanted one is drawn: the game can draw into a rendered texture
        // in one frame and use it in the frames after that.
        if frame <= wanted && !player.drawing() {
            player.begin_frame(&target, Rc::new(depth.create_view(&Default::default())), false);
        }
        if player.feed(*tag, data) {
            if player.drawing() {
                let mut read = None;
                let this_one = if frames_at.is_empty() { frame == wanted } else { frames_at.contains(&frame) };
                player.end_frame(|encoder| {
                    if this_one {
                        read = Some(copy_picture(&device, encoder, &target, width, height));
                    }
                });
                if let Some((buffer, row)) = read {
                    let path = if frames_at.is_empty() { args[2].clone() } else { format!("{}-{:05}.png", args[2], frame) };
                    save_picture(&device, &buffer, row, width, height, false, &path);
                    buffer.unmap();
                }
            }
            frame += 1;
        }
    }
    player.print_counts();
}
