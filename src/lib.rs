//! The plugin akulakhan.dll: the renderer of Morrowind.exe, replaced.
//!
//! The game gets a Direct3D 8 of the plugin and makes no device (`own_device`). The ports
//! (`ports`) are Rust functions that take the place of the `NiDX8Renderer` subsystem, one
//! for each engine function, through a detour on the function (`hook`, `install`). A port
//! runs its Rust code on the main thread; on another thread it runs the engine function
//! ([`shadow::dispatch`]), and the cell worker's calls reach the plugin as events
//! (`worker_events`).
//!
//! What the game draws goes as records into a ring in shared memory (`link`, `own_host`,
//! `capture`): the geometry, the materials, the shapes, the lights and the draws of a frame
//! (the `*_stream` modules), with the world matrices made in 64-bit arithmetic relative to
//! the camera (`camera_space`). A shape of the world is a retained instance of the helper
//! (`instance_stream`): the plugin skips its display and sends a record only when it
//! changes. The helper, numidium.exe of MGE XE G7, reads the ring and draws the frame with
//! wgpu (the crate in `host/`).
//!
//! `lua` is the table `nirender` of the MWSE mod. `census` compares the engine functions in
//! memory with the database before the install and refuses the renderer when another mod
//! changed a port's bytes.
//!
//! The comparison rig (the cargo feature `rig`): a port also runs in the mode shadow (the
//! Rust and the engine function both, compared call by call, `shadow`) and the frame
//! comparison (`frame`, `dump`, `perf`, the recorder in `device` and `resource`). The rig
//! is for a test run with a real Direct3D 8 device, never for a player.

// The plugin is 32-bit only, because it runs inside Morrowind.exe.
#[cfg(not(target_pointer_width = "32"))]
compile_error!("nirender builds for i686-pc-windows-msvc only");

pub mod gen {
    pub mod funcs;
    pub mod layouts;
}

#[cfg(feature = "rig")]
pub mod alloc;
#[cfg(feature = "rig")]
pub mod callers;
pub mod capture;
pub mod camera_space;
pub mod checked;
pub mod entry_trace;
pub mod cores;
pub mod crash;
pub mod log;
pub mod main_thread;
pub mod census;
pub mod d3d;
pub mod freed_fill;
pub mod worker_events;
pub mod world_lights;
pub mod device;
#[cfg(feature = "rig")]
pub mod dump;
pub mod engine;
#[cfg(feature = "rig")]
pub mod frame;
pub mod geometry_stream;
pub mod hook;
pub mod install;
pub mod animation;
pub mod instance_stream;
pub mod link;
pub mod link_client;
pub mod light_stream;
pub mod material_stream;
pub mod map_fog;
pub mod map_tiles;
pub mod mem;
pub mod mwse_native;
pub mod overlay;
pub mod pacing;
pub mod world_walk;
pub mod queue_flush;
#[cfg(feature = "rig")]
pub mod perf;
pub mod ptr_hash;
pub mod ports;
#[cfg(feature = "rig")]
pub mod resource;
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
