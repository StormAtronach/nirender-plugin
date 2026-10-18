//! nirender is a Rust replica of the `NiDX8Renderer` subsystem of Morrowind.exe.
//!
//! Each port has a detour on its engine function and runs in one of three modes. The mode
//! off runs the engine function. The mode shadow runs the two functions and compares them.
//! The mode live runs the Rust function.
//! See `shadow` for the per-call comparison and `frame` for the whole-frame comparison.

// The plugin is 32-bit only, because it runs inside Morrowind.exe.
#[cfg(not(target_pointer_width = "32"))]
compile_error!("nirender builds for i686-pc-windows-msvc only");

pub mod gen {
    pub mod funcs;
    pub mod layouts;
}

pub mod alloc;
pub mod callers;
pub mod capture;
pub mod camera_space;
pub mod cores;
pub mod log;
pub mod census;
pub mod click_time;
pub mod d3d;
pub mod deferred_destroy;
pub mod deferred_unload;
pub mod worker_events;
pub mod device;
pub mod dump;
pub mod engine;
pub mod frame;
pub mod geometry_stream;
pub mod hook;
pub mod install;
pub mod animation;
pub mod live_walk;
pub mod instance_stream;
pub mod link;
pub mod link_client;
pub mod light_set;
pub mod light_stream;
pub mod material_stream;
pub mod mem;
pub mod overlay;
pub mod perf;
pub mod ptr_hash;
pub mod ports;
pub mod resource;
pub mod script_names;
pub mod shadow;
pub mod shape_stream;
pub mod site;
pub mod sync;
pub mod win;
pub mod x87;

// The test binary runs outside the game, where there is no lua51.dll to load.
#[cfg(not(test))]
pub mod lua;
pub mod own_device;
pub mod own_host;
