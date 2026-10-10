//! Test: does wgpu build and run in a 32-bit Windows process, on Vulkan and on D3D12.
//!
//!     cargo run --release --target i686-pc-windows-msvc -- [draws per frame] [frames]
//!
//! For each backend the program opens a window, makes `draws` small draws in each frame with
//! one uniform buffer offset for each draw and a change of pipeline at each draw, and prints
//! the frame times and the memory of the process.

use std::num::NonZeroIsize;
use std::time::Instant;

use wgpu::rwh::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_FREE, MEM_RESERVE};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, PeekMessageW, RegisterClassW, ShowWindow,
    TranslateMessage, CW_USEDEFAULT, MSG, PM_REMOVE, SW_SHOW, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
/// The size of the uniform data of one draw. It is the largest offset alignment that a
/// device can ask for by default.
const SLOT: u64 = 256;

const SHADER: &str = r#"
struct Draw { offset: vec4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> draw: Draw;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var corner = array<vec2<f32>, 3>(vec2<f32>(-0.01, -0.01), vec2<f32>(0.01, -0.01), vec2<f32>(0.0, 0.01));
    return vec4<f32>(corner[index] + draw.offset.xy, 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return draw.color;
}
"#;

/// The memory of the process in megabytes: private bytes, and the address space that is
/// committed, reserved and the largest free block.
struct Memory {
    private: f64,
    committed: f64,
    reserved: f64,
    largest_free: f64,
}

fn memory() -> Memory {
    const MB: f64 = 1024.0 * 1024.0;
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        );
        let (mut committed, mut reserved, mut largest_free) = (0usize, 0usize, 0usize);
        let mut address = 0usize;
        loop {
            let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            if VirtualQuery(address as *const _, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
                break;
            }
            match info.State {
                MEM_COMMIT => committed += info.RegionSize,
                MEM_RESERVE => reserved += info.RegionSize,
                MEM_FREE => largest_free = largest_free.max(info.RegionSize),
                _ => {}
            }
            let next = address.wrapping_add(info.RegionSize);
            if next <= address {
                break;
            }
            address = next;
        }
        Memory {
            private: counters.PrivateUsage as f64 / MB,
            committed: committed as f64 / MB,
            reserved: reserved as f64 / MB,
            largest_free: largest_free as f64 / MB,
        }
    }
}

fn print_memory(label: &str) {
    let m = memory();
    println!(
        "  memory {label}: private {:.0} MB, committed {:.0} MB, reserved {:.0} MB, largest free block {:.0} MB",
        m.private, m.committed, m.reserved, m.largest_free
    );
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    DefWindowProcW(window, message, wparam, lparam)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

unsafe fn make_window(title: &str) -> (HWND, isize) {
    let instance = GetModuleHandleW(std::ptr::null());
    let class_name = wide("wgpu32");
    let mut class: WNDCLASSW = std::mem::zeroed();
    class.lpfnWndProc = Some(window_proc);
    class.hInstance = instance;
    class.lpszClassName = class_name.as_ptr();
    // A second call for the same class fails. That is correct.
    RegisterClassW(&class);
    let title = wide(title);
    let window = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        WIDTH as i32,
        HEIGHT as i32,
        0 as _,
        0 as _,
        instance,
        std::ptr::null(),
    );
    ShowWindow(window, SW_SHOW);
    (window, instance as isize)
}

unsafe fn pump_messages() {
    let mut message: MSG = std::mem::zeroed();
    while PeekMessageW(&mut message, 0 as _, 0, 0, PM_REMOVE) != 0 {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    blend: wgpu::BlendState,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(layout),
        vertex: wgpu::VertexState { module: shader, entry_point: "vs", compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn run(name: &str, backends: wgpu::Backends, draws: u32, frames: u32) -> Result<(), String> {
    println!("\n{name}");
    print_memory("before");
    let (window, hinstance) = unsafe { make_window(&format!("wgpu32 {name}")) };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..Default::default() });
    let mut handle = Win32WindowHandle::new(NonZeroIsize::new(window as isize).ok_or("no window")?);
    handle.hinstance = NonZeroIsize::new(hinstance);
    let surface = unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
            raw_window_handle: RawWindowHandle::Win32(handle),
        })
    }
    .map_err(|e| format!("create_surface: {e}"))?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .ok_or("no adapter for this backend")?;
    let info = adapter.get_info();
    println!("  adapter: {} ({:?}, {:?}), driver {} {}", info.name, info.backend, info.device_type, info.driver, info.driver_info);
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: Default::default(),
        },
        None,
    ))
    .map_err(|e| format!("request_device: {e}"))?;
    // The swap chain has the size of the client area. The window is larger: it has a frame.
    let mut client: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(window, &mut client) };
    let (width, height) = ((client.right - client.left).max(1) as u32, (client.bottom - client.top).max(1) as u32);
    let mut config = surface.get_default_config(&adapter, width, height).ok_or("the surface has no configuration")?;
    config.present_mode = wgpu::PresentMode::AutoNoVsync;
    surface.configure(&device, &config);
    print_memory("with a device");

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
    let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(32),
            },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&bind_layout],
        push_constant_ranges: &[],
    });
    let pipelines = [
        pipeline(&device, &layout, &shader, config.format, wgpu::BlendState::REPLACE),
        pipeline(&device, &layout, &shader, config.format, wgpu::BlendState::ALPHA_BLENDING),
    ];
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: draws as u64 * SLOT,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &uniforms, offset: 0, size: wgpu::BufferSize::new(32) }),
        }],
    });

    let mut data = vec![0u8; (draws as u64 * SLOT) as usize];
    let mut times = Vec::with_capacity(frames as usize);
    let mut reconfigured = 0;
    for frame in 0..frames {
        unsafe { pump_messages() };
        let start = Instant::now();
        for draw in 0..draws {
            let angle = draw as f32 * 0.1 + frame as f32 * 0.01;
            let radius = 0.1 + 0.85 * (draw as f32 / draws as f32);
            let values = [radius * angle.cos(), radius * angle.sin(), 0.0, 0.0, 0.3 + 0.7 * (draw % 7) as f32 / 7.0, 0.6, 0.9, 0.8f32];
            let at = (draw as u64 * SLOT) as usize;
            for (index, value) in values.iter().enumerate() {
                data[at + index * 4..at + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        queue.write_buffer(&uniforms, 0, &data);
        let target = match surface.get_current_texture() {
            Ok(target) => target,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                surface.configure(&device, &config);
                reconfigured += 1;
                surface.get_current_texture().map_err(|e| format!("get_current_texture: {e}"))?
            }
            Err(e) => return Err(format!("get_current_texture: {e}")),
        };
        let view = target.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            for draw in 0..draws {
                pass.set_pipeline(&pipelines[(draw & 1) as usize]);
                pass.set_bind_group(0, &bind_group, &[(draw as u64 * SLOT) as u32]);
                pass.draw(0..3, 0..1);
            }
        }
        queue.submit(Some(encoder.finish()));
        target.present();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        if frame == 0 {
            print_memory("after the first frame");
        }
    }
    print_memory(&format!("after {frames} frames"));
    println!("  swap chain {width} x {height}, configured again {reconfigured} times");

    // The first frames compile the pipelines. They are not in the statistics.
    let mut steady: Vec<f64> = times[times.len().min(20)..].to_vec();
    steady.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if !steady.is_empty() {
        let mean = steady.iter().sum::<f64>() / steady.len() as f64;
        println!(
            "  {draws} draws in a frame: mean {:.3} ms, median {:.3} ms, p99 {:.3} ms, max {:.3} ms; {:.2} microseconds for a draw",
            mean,
            steady[steady.len() / 2],
            steady[(steady.len() * 99 / 100).min(steady.len() - 1)],
            steady[steady.len() - 1],
            mean * 1000.0 / draws as f64
        );
    }
    drop(surface);
    unsafe { DestroyWindow(window) };
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let draws: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(5000);
    let frames: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(400);
    println!("wgpu32: pointer size {} bits, {draws} draws in a frame, {frames} frames", usize::BITS);
    for (name, backends) in [("Vulkan", wgpu::Backends::VULKAN), ("D3D12", wgpu::Backends::DX12)] {
        if let Err(problem) = run(name, backends, draws, frames) {
            println!("  FAIL: {problem}");
        }
    }
    print_memory("at the end");
}
