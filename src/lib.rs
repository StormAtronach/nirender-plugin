//! nirender: a Rust replica of the `NiDX8Renderer` subsystem of Morrowind.exe.
//!
//! Each ported function sits behind a detour on the engine function and can run in three
//! modes: off (the engine function), shadow (both, compared) and live (the Rust function).
//! See `shadow` for the per-call comparison and `frame` for the whole-frame comparison.

// The plugin is 32-bit only: it lives inside Morrowind.exe.
#[cfg(not(target_pointer_width = "32"))]
compile_error!("nirender builds for i686-pc-windows-msvc only");

pub mod gen {
    pub mod funcs;
    pub mod layouts;
}

pub mod alloc;
pub mod log;
pub mod census;
pub mod device;
pub mod dump;
pub mod engine;
pub mod frame;
pub mod hook;
pub mod mem;
pub mod ports;
pub mod resource;
pub mod shadow;
pub mod site;
pub mod win;
pub mod x87;

// The test binary runs outside the game, where there is no lua51.dll to load.
#[cfg(not(test))]
pub mod lua;
