//! The ports. Each engine class has one module. `PORTS` lists every port.

use crate::shadow::PortState;

/// The body of `port_thiscall!` and `port_cdecl!`. It makes a module with `STATE`, the detour,
/// and `engine(...)` to call the game code. `$call` is the closure that runs the Rust
/// function; it gets the proof of the main thread (`main_thread::MainThread`).
macro_rules! port_impl {
    (
        $modname:ident, $addr:literal, $ename:literal, $abi:literal, $cleanup:ident, $stack_args:expr,
        ($($p:ident : $pt:ty),*) -> $r:ty, $cmp:literal, |$set:ident| $regbody:expr, $call:expr
    ) => {
        pub mod $modname {
            #[allow(unused_imports)]
            use super::*;
            use core::sync::atomic::Ordering;

            fn detour_addr() -> usize {
                detour as *const () as usize
            }

            pub static STATE: $crate::shadow::PortState = $crate::shadow::PortState::new(
                $ename, $addr, $stack_args, $crate::hook::Cleanup::$cleanup, detour_addr);

            /// The engine function.
            ///
            /// # Safety
            ///
            /// The port is installed: `STATE.orig` is 0 until its detour is in place, and a
            /// call before then jumps to address 0. The arguments are the ones the engine
            /// function at the port's address takes, as valid as the game passes them (a
            /// `this` or a pointer to a live engine object of its class); nothing here checks
            /// them. Which thread may call the engine function is its own rule, not known
            /// here; the detour calls it with the game's own arguments on the game's thread.
            #[track_caller]
            pub unsafe fn engine($($p: $pt),*) -> u32 {
                let _call = $crate::entry_trace::call_out();
                let f: unsafe extern $abi fn($($pt),*) -> u32 = core::mem::transmute(STATE.orig.load(Ordering::Relaxed));
                f($($p),*)
            }

            unsafe extern $abi fn detour($($p: $pt),*) -> u32 {
                let any_thread = STATE.any_thread.load(Ordering::Relaxed);
                if any_thread != 0 {
                    let args: &[usize] = &[$($crate::shadow::Word::word($p)),*];
                    $crate::shadow::run_any_thread(any_thread, args);
                }
                let before = STATE.before.load(Ordering::Relaxed);
                if before != 0 {
                    let args: &[usize] = &[$($crate::shadow::Word::word($p)),*];
                    $crate::shadow::run_before(before, args);
                }
                $crate::shadow::dispatch(
                    &STATE,
                    $cmp,
                    #[allow(unused_variables)]
                    |$set: &mut $crate::shadow::Regions| $regbody,
                    $call,
                    || engine($($p),*),
                )
            }
        }
    };
}

/// Declares the port of a `__thiscall` engine function. The function removes its stack
/// arguments.
///
/// `regions` adds the memory that the function can write for a call. `compare_eax` tells if
/// the value in EAX is a result. `rust` names the Rust function; `rust_main` names one that
/// takes the proof of the main thread as its first argument (a port that reaches a table
/// of the plugin).
macro_rules! port_thiscall {
    (
        $modname:ident, $addr:literal, $ename:literal,
        fn($this:ident $(, $a:ident : $t:ty)*) -> $r:ty,
        compare_eax = $cmp:literal,
        regions = |$set:ident| $regbody:expr,
        rust = $rust:path
    ) => {
        port_impl!(
            $modname, $addr, $ename, "thiscall", Callee, 0 $(+ { let _ = stringify!($a); 1 })*,
            ($this: usize $(, $a: $t)*) -> $r, $cmp, |$set| $regbody,
            |_proof: &$crate::main_thread::MainThread| -> $r { $rust($this $(, $a)*) }
        );
    };
    (
        $modname:ident, $addr:literal, $ename:literal,
        fn($this:ident $(, $a:ident : $t:ty)*) -> $r:ty,
        compare_eax = $cmp:literal,
        regions = |$set:ident| $regbody:expr,
        rust_main = $rust:path
    ) => {
        port_impl!(
            $modname, $addr, $ename, "thiscall", Callee, 0 $(+ { let _ = stringify!($a); 1 })*,
            ($this: usize $(, $a: $t)*) -> $r, $cmp, |$set| $regbody,
            |proof: &$crate::main_thread::MainThread| -> $r { $rust(proof, $this $(, $a)*) }
        );
    };
}

/// Declares the port of a `__thiscall` engine function that returns a `float`: the value is
/// on the x87 stack (ST(0)), not in EAX. The detour and the call of the engine function have
/// the float type, so the compiler loads and pops ST(0); the comparison uses the bits.
macro_rules! port_thiscall_float {
    (
        $modname:ident, $addr:literal, $ename:literal,
        fn($this:ident) -> f32,
        compare_eax = $cmp:literal,
        regions = |$set:ident| $regbody:expr,
        rust = $rust:path
    ) => {
        pub mod $modname {
            #[allow(unused_imports)]
            use super::*;
            use core::sync::atomic::Ordering;

            fn detour_addr() -> usize {
                detour as *const () as usize
            }

            pub static STATE: $crate::shadow::PortState =
                $crate::shadow::PortState::new($ename, $addr, 0, $crate::hook::Cleanup::Callee, detour_addr);

            /// The engine function.
            ///
            /// # Safety
            ///
            /// The port is installed: `STATE.orig` is 0 until its detour is in place, and a
            /// call before then jumps to address 0. `this` points at a live engine object of
            /// the class of the function at the port's address, as the game passes it;
            /// nothing here checks it. Which thread may call it is the engine function's own
            /// rule, not known here; the detour calls it on the game's thread.
            #[track_caller]
            pub unsafe fn engine($this: usize) -> f32 {
                let _call = $crate::entry_trace::call_out();
                let f: unsafe extern "thiscall" fn(usize) -> f32 = core::mem::transmute(STATE.orig.load(Ordering::Relaxed));
                f($this)
            }

            unsafe extern "thiscall" fn detour($this: usize) -> f32 {
                let any_thread = STATE.any_thread.load(Ordering::Relaxed);
                if any_thread != 0 {
                    $crate::shadow::run_any_thread(any_thread, &[$this]);
                }
                let before = STATE.before.load(Ordering::Relaxed);
                if before != 0 {
                    $crate::shadow::run_before(before, &[$this]);
                }
                f32::from_bits($crate::shadow::dispatch(
                    &STATE,
                    $cmp,
                    #[allow(unused_variables)]
                    |$set: &mut $crate::shadow::Regions| $regbody,
                    |_proof: &$crate::main_thread::MainThread| -> f32 { $rust($this) },
                    || engine($this).to_bits(),
                ))
            }
        }
    };
}

/// Declares the port of a `__cdecl` engine function. The caller removes the arguments.
/// `rust_main` as in `port_thiscall!`.
macro_rules! port_cdecl {
    (
        $modname:ident, $addr:literal, $ename:literal,
        fn($($a:ident : $t:ty),*) -> $r:ty,
        compare_eax = $cmp:literal,
        regions = |$set:ident| $regbody:expr,
        rust = $rust:path
    ) => {
        port_impl!(
            $modname, $addr, $ename, "C", Caller, 0 $(+ { let _ = stringify!($a); 1 })*,
            ($($a: $t),*) -> $r, $cmp, |$set| $regbody,
            |_proof: &$crate::main_thread::MainThread| -> $r { $rust($($a),*) }
        );
    };
    (
        $modname:ident, $addr:literal, $ename:literal,
        fn($($a:ident : $t:ty),*) -> $r:ty,
        compare_eax = $cmp:literal,
        regions = |$set:ident| $regbody:expr,
        rust_main = $rust:path
    ) => {
        port_impl!(
            $modname, $addr, $ename, "C", Caller, 0 $(+ { let _ = stringify!($a); 1 })*,
            ($($a: $t),*) -> $r, $cmp, |$set| $regbody,
            |proof: &$crate::main_thread::MainThread| -> $r { $rust(proof $(, $a)*) }
        );
    };
}

pub mod base;
pub mod buffers;
pub mod ct_pipeline_state;
pub mod device_setup;
pub mod draw;
pub mod draw_misc;
pub mod geometry;
pub mod hash_map;
pub mod light_manager;
pub mod material;
pub mod ni;
pub mod render_state;
pub mod rendered;
pub mod animation;
pub mod state_update;
pub mod scene;
pub mod renderer_frame;
pub mod system_desc;
#[cfg(not(test))]
pub mod texture_link;
pub mod texture_pass_a;
pub mod texture_pass_b;
pub mod texture_pipeline;
pub mod textures;

/// The ports of every module. A new module adds its `PORTS` here.
static MODULES: &[&[&PortState]] = &[
    render_state::PORTS,
    material::PORTS,
    texture_pass_a::PORTS,
    light_manager::PORTS,
    texture_pipeline::PORTS,
    renderer_frame::PORTS,
    texture_pass_b::PORTS,
    system_desc::PORTS,
    rendered::PORTS,
    scene::PORTS,
    animation::PORTS,
    state_update::PORTS,
    textures::PORTS,
    #[cfg(not(test))]
    texture_link::PORTS,
    geometry::PORTS,
    device_setup::PORTS,
    buffers::PORTS,
    base::PORTS,
    draw_misc::PORTS,
    draw::PORTS,
    ct_pipeline_state::PORTS,
];

/// The call sites that the ports serve through `site::call_target`. A module with such sites
/// adds its `SITES` here.
static SITE_LISTS: &[&[usize]] = &[
    draw_misc::SITES,
    draw::SITES,
    ct_pipeline_state::SITES,
    device_setup::SITES,
    // `NiDX8TextureStage::updateRenderer`: `texture_pipeline` reads the target of this call.
    // The Morrowind Code Patch option "Bump/reflect map local lighting" (feature 131) changes it.
    &[0x6C1BA4],
    // `NiDX8TexturePass::setBlendingState`: `texture_pass_b` reads the target of this call.
    // The Code Patch option "Enchant glow in fog fix" (feature 12) changes it.
    &[texture_pass_b::SET_ALPHA_SITE],
];

/// True when a port calls the target that is in memory at the `call` at `site`.
pub fn serves_site(site: usize) -> bool {
    SITE_LISTS.iter().any(|list| list.contains(&site))
}

/// A place in the game code that another mod can change and that a port follows. The port
/// reads the bytes from memory and does what they say.
enum Followed {
    /// An operand of an instruction. The port uses the value that is in memory.
    Operand { at: usize, len: usize },
    /// Bytes that a known patch writes. The port knows these bytes and the bytes of the
    /// database. It does not follow other bytes.
    Patch { at: usize, bytes: &'static [u8] },
    /// A `call` and a `nop` that a known patch puts in place of 6 bytes of code. The port does
    /// in Rust what the function of the patch does.
    InsertedCall { at: usize },
    /// Bytes that a known patch writes around the address of its own function: `head`, 4
    /// bytes of address, `tail`. The port calls that function. The function must lie in the
    /// module of the patch (`callee`): a patch of the same shape with another target is not
    /// known.
    CallOfPatch { at: usize, head: &'static [u8], tail: &'static [u8], callee: Callee },
}

/// The name that [`follows`] gets for an address in this plugin's own DLL (its file name changes
/// with the build).
pub const THIS_PLUGIN: &str = "this plugin";

/// The 4 bytes of a [`Followed::CallOfPatch`] and the modules where the function that they name
/// may lie. A module is a file name, compared without the case.
enum Callee {
    /// The address of the function.
    Address(&'static [&'static str]),
    /// The distance from the end of the 4 bytes to the function: a `call rel32`.
    Distance(&'static [&'static str]),
}

impl Callee {
    /// True when the 4 bytes at `at` name a function in one of the modules. `module_of` gives
    /// the module of an address.
    fn accepts(&self, at: usize, read: &impl Fn(usize) -> u8, module_of: &impl Fn(usize) -> String) -> bool {
        let value = u32::from_le_bytes([read(at), read(at + 1), read(at + 2), read(at + 3)]) as usize;
        let (target, modules) = match *self {
            Callee::Address(modules) => (value, modules),
            Callee::Distance(modules) => ((at + 4).wrapping_add(value), modules),
        };
        let module = module_of(target);
        modules.iter().any(|name| name.eq_ignore_ascii_case(&module))
    }
}

static FOLLOWED: &[Followed] = &[
    // `NiDX8Renderer::RenderShape`: the software skinning flag of MWSE
    // (PatchNIDX8Renderer_RenderShape). `draw::render_shape` reads the bytes.
    Followed::Patch { at: 0x6ACF1F, bytes: &[0x90, 0x66, 0xF7, 0x46, 0x36, 0x00, 0x02, 0x75] },
    // `NiDX8TexturePass::setCTPipelineState`: the base map texture coordinate fix of MWSE
    // (PatchNiDX8TexturePass_BaseMapTexcoord).
    Followed::InsertedCall { at: 0x6B42AC },
    // `NiDX8Renderer::ctor`: the hash map sizes that MWSE changes ("Optimize renderer hash map
    // lookups"). The first operand is the bucket count. The others are sizes in bytes.
    Followed::Operand { at: 0x6A9AED, len: 4 },
    Followed::Operand { at: 0x6A9AF2, len: 4 },
    Followed::Operand { at: 0x6A9B38, len: 4 },
    Followed::Operand { at: 0x6A9B7E, len: 4 },
    Followed::Operand { at: 0x6A9BC4, len: 4 },
    // `NiDX8SourceTextureData_static::Create`: the two calls and the three stack offsets of
    // MGE XE (`patchLoadTexture2D`). `texture_link::create` calls the two functions.
    // The two functions are in MGE XE's d3d8.dll, or in this plugin when it writes the same patch
    // itself (`texture_link`).
    Followed::CallOfPatch { at: 0x6BFC4B, head: &[0x52, 0x55, 0x56, 0x53, 0xB8], tail: &[0xFF, 0xD0, 0xEB, 0x0E], callee: Callee::Address(&["d3d8.dll", THIS_PLUGIN]) },
    Followed::CallOfPatch {
        at: 0x6BFD3B,
        head: &[0x8B, 0x54, 0x24, 0x10, 0x52, 0x55, 0x56, 0xB8],
        tail: &[0xFF, 0xD0, 0xEB, 0x08],
        callee: Callee::Address(&["d3d8.dll", THIS_PLUGIN]),
    },
    Followed::Patch { at: 0x6BFCC7, bytes: &[0x18] },
    Followed::Patch { at: 0x6BFCD7, bytes: &[0x18] },
    Followed::Patch { at: 0x6BFCE3, bytes: &[0x24] },
    // The draw calls of the device that the Morrowind Code Patch option "Show transparent
    // clothes in the inventory" (feature 11) replaces with calls of its draw functions. The
    // database has the calls of the Code Patch. These are the bytes of the game without the
    // option: `site_draw_indexed_primitive` and `site_draw_primitive` of `draw` and
    // `draw_misc` call the device when the site has no `call rel32`.
    // `NiDX8Renderer::RenderPoints` and `NiDX8Renderer::RenderLines`: `call [edx+11Ch]`.
    Followed::Patch { at: 0x6AD84A, bytes: &[0xFF, 0x92, 0x1C, 0x01, 0x00, 0x00] },
    Followed::Patch { at: 0x6ADC91, bytes: &[0xFF, 0x92, 0x1C, 0x01, 0x00, 0x00] },
    // `NiDX8Renderer::DrawPrimitive`: the device and the vtable in other registers, then
    // `call [edx+11Ch]`.
    Followed::Patch {
        at: 0x6AEF04,
        bytes: &[0x8B, 0x4E, 0x24, 0x8B, 0x40, 0x2C, 0x57, 0x8B, 0x11, 0x55, 0x53, 0x6A, 0x00, 0x50, 0x51, 0xFF, 0x92, 0x1C, 0x01, 0x00, 0x00],
    },
    // `NiDX8Renderer::DrawSkinnedPrimitive2`: `call [ecx+11Ch]` and `call [ecx+118h]`, and the
    // jumps around them.
    Followed::Patch {
        at: 0x6AF230,
        bytes: &[
            0x74, 0x17, 0x33, 0xD2, 0x66, 0x8B, 0x55, 0xF4, 0x52, 0x8B, 0x54, 0x24, 0x44, 0x6A, 0x00, 0x52, 0x50, 0xFF, 0x91, 0x1C, 0x01, 0x00, 0x00,
            0xEB, 0x0C, 0x8B, 0x54, 0x24, 0x40, 0x52, 0x50, 0xFF, 0x91, 0x18, 0x01, 0x00, 0x00,
        ],
    },
    // `NiDX8MaterialManager::UpdateToDevice`: the database has the call that the Morrowind
    // Code Patch puts there (MCP-011). These are the bytes of the game without the patch.
    // `material::update_to_device` reads the bytes.
    Followed::Patch { at: 0x6C2686, bytes: &[0x15] },
    Followed::Patch { at: 0x6C2696, bytes: &[0xFF, 0x92, 0xA8, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00] },
    // `NiDX8MaterialManager::dtor`: the patch of MWSE for a default material that is null (the
    // engine's exit handlers released it). A `call` and 25 `nop` in place of 30 bytes of code,
    // and 3 `nop` in place of the write `mov [eax+18h], ebp`. `base::clear_default_material_entry`
    // does the same: it skips the write when the default material is null. The function is in MWSE.dll.
    Followed::CallOfPatch { at: 0x6C2578, head: &[0xE8], tail: &[0x90; 25], callee: Callee::Distance(&["MWSE.dll"]) },
    Followed::Patch { at: 0x6C259B, bytes: &[0x90, 0x90, 0x90] },
    // `NiDX8TextureData::FindMatchingPixelFormat`: four displacements that the Morrowind Code
    // Patch changes (MCP-039). `textures` reads them.
    Followed::Operand { at: 0x6B70A8, len: 1 },
    Followed::Operand { at: 0x6B70DF, len: 1 },
    Followed::Operand { at: 0x6B7357, len: 1 },
    Followed::Operand { at: 0x6B737A, len: 1 },
    // `NiRenderer::MapWindowPointToBufferPoint`: the UI scale patch of MWSE (at a request) or of
    // MGE XE on Direct3D 9. `base::renderer_map_window_point_to_buffer_point` asks MWSE.
    Followed::Patch { at: base::UI_SCALE_PATCH_AT, bytes: &base::UI_SCALE_PATCH },
];

/// The records of the Morrowind Code Patch that write inside a ported function: the feature,
/// the first address and the number of bytes. The Code Patch 2.4 has these 11, and the 2.5
/// beta 19 has the same. A record of `UpdateToDevice` ends one byte after the function. The
/// test `every_code_patch_record_in_a_port_is_followed` of `install` checks the patch files
/// of the Code Patch on the machine against this table.
pub static CODE_PATCH_RECORDS: &[(u32, usize, usize)] = &[
    (11, 0x6AD84A, 5),  // NiDX8Renderer::RenderPoints
    (11, 0x6ADC91, 5),  // NiDX8Renderer::RenderLines
    (11, 0x6AEF05, 19), // NiDX8Renderer::DrawPrimitive
    (11, 0x6AF231, 35), // NiDX8Renderer::DrawSkinnedPrimitive2
    (12, 0x6B5664, 3),  // NiDX8TexturePass::setBlendingState
    (39, 0x6B70A8, 1),  // NiDX8TextureData::FindMatchingPixelFormat
    (39, 0x6B70DF, 1),
    (39, 0x6B7357, 1),
    (39, 0x6B737A, 1),
    (131, 0x6C1BA5, 4), // NiDX8TextureStage::updateRenderer
    (11, 0x6C2686, 25), // NiDX8MaterialManager::UpdateToDevice
];

/// The name of an option of the Morrowind Code Patch in the installer of the Code Patch.
pub fn code_patch_option(feature: u32) -> &'static str {
    match feature {
        11 => "Show transparent clothes in the inventory",
        12 => "Enchant glow in fog fix",
        39 => "Better quality splash and title screens",
        131 => "Bump/reflect map local lighting",
        _ => "an option that is not known",
    }
}

/// The feature of the Code Patch whose record has bytes in the `len` bytes from `at`.
pub fn code_patch_feature(at: usize, len: usize) -> Option<u32> {
    CODE_PATCH_RECORDS.iter().find(|&&(_, start, count)| at < start + count && start < at + len).map(|&(feature, _, _)| feature)
}

/// True when a port follows a change of the byte at `addr`. `read` gives the byte that is in
/// memory at an address, and `module_of` the file name of the module that holds an address
/// ([`THIS_PLUGIN`] for this plugin, "?" for none). A call of a known patch must name a function in
/// the module of that patch.
pub fn follows(addr: usize, read: impl Fn(usize) -> u8, module_of: impl Fn(usize) -> String) -> bool {
    FOLLOWED.iter().any(|followed| match *followed {
        Followed::Operand { at, len } => (at..at + len).contains(&addr),
        Followed::Patch { at, bytes } => {
            (at..at + bytes.len()).contains(&addr) && bytes.iter().enumerate().all(|(i, &b)| read(at + i) == b)
        }
        Followed::InsertedCall { at } => (at..at + 6).contains(&addr) && read(at) == 0xE8 && read(at + 5) == 0x90,
        Followed::CallOfPatch { at, head, tail, ref callee } => {
            let tail_at = at + head.len() + 4;
            (at..tail_at + tail.len()).contains(&addr)
                && head.iter().enumerate().all(|(i, &b)| read(at + i) == b)
                && tail.iter().enumerate().all(|(i, &b)| read(tail_at + i) == b)
                && callee.accepts(at + head.len(), &read, &module_of)
        }
    })
}

/// The ports whose Rust runs the whole engine function and only notes what it changed (the
/// new property and effect states of a shape). The game code past their stolen bytes runs as
/// it is in memory, so a change there by another mod is kept.
static WRAPPING: &[&[&PortState]] = &[state_update::PORTS];

/// True when the port at `addr` runs the whole engine function (`WRAPPING`).
pub fn wraps_engine(addr: usize) -> bool {
    WRAPPING.iter().any(|module| module.iter().any(|port| port.addr == addr))
}

/// Every port.
pub fn all() -> impl Iterator<Item = &'static PortState> {
    MODULES.iter().flat_map(|module| module.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook;

    #[test]
    fn a_port_follows_known_patches_only() {
        let database = |_addr: usize| 0u8;
        let no_module = |_addr: usize| "?".to_string();
        // An operand: any value.
        assert!(follows(0x6A9AED, database, no_module));
        assert!(follows(0x6A9AF5, database, no_module));
        assert!(!follows(0x6A9AF1, database, no_module), "the opcode between two operands is not an operand");
        // The skinning check: only the bytes of MWSE.
        let mwse = [0x90u8, 0x66, 0xF7, 0x46, 0x36, 0x00, 0x02, 0x75];
        assert!(follows(0x6ACF20, |addr| mwse[addr - 0x6ACF1F], no_module));
        assert!(!follows(0x6ACF20, |addr| if addr == 0x6ACF20 { 0x67 } else { mwse[addr - 0x6ACF1F] }, no_module));
        // The inserted call: a call and a nop.
        let call = [0xE8u8, 1, 2, 3, 4, 0x90];
        assert!(follows(0x6B42AD, |addr| call[addr - 0x6B42AC], no_module));
        assert!(!follows(0x6B42AD, database, no_module));
        assert!(!follows(0x400000, database, no_module));
    }

    /// The memory of a `CallOfPatch`: `head`, the 4 bytes, `tail`, at `at`.
    fn patch_bytes(at: usize, head: &[u8], four: u32, tail: &[u8]) -> impl Fn(usize) -> u8 {
        let mut bytes = head.to_vec();
        bytes.extend_from_slice(&four.to_le_bytes());
        bytes.extend_from_slice(tail);
        move |addr| bytes[addr - at]
    }

    /// Modules at fixed places: MGE XE's d3d8.dll above 0x1000_0000, MWSE.dll above 0x2000_0000, this plugin above 0x3000_0000, the
    /// game below.
    fn module_at(addr: usize) -> String {
        match addr {
            0x3000_0000.. => THIS_PLUGIN.to_string(),
            0x2000_0000.. => "MWSE.dll".to_string(),
            0x1000_0000.. => "D3D8.DLL".to_string(),
            0x40_0000..=0x80_0000 => "Morrowind.exe".to_string(),
            _ => "?".to_string(),
        }
    }

    #[test]
    fn the_call_of_a_known_patch_must_name_a_function_of_the_module_of_the_patch() {
        // The two calls of MGE XE: the address in the 4 bytes.
        let (head, tail) = (&[0x52, 0x55, 0x56, 0x53, 0xB8][..], &[0xFF, 0xD0, 0xEB, 0x0E][..]);
        let at = 0x6BFC4B;
        for (target, wanted, what) in [
            (0x1000_1000u32, true, "a function of d3d8.dll (the case does not matter)"),
            (0x3000_1000, true, "a function of this plugin (it writes the same patch)"),
            (0x2000_1000, false, "a function of MWSE.dll is not the patch of MGE XE"),
            (0x0040_1000, false, "the right opcode and a target in the game itself"),
            (0, false, "a null target"),
            (0x0010_0000, false, "a target in no module"),
        ] {
            assert_eq!(follows(at + 5, patch_bytes(at, head, target, tail), module_at), wanted, "{what}");
        }
        // The second call has its own head.
        let (head, tail) = (&[0x8B, 0x54, 0x24, 0x10, 0x52, 0x55, 0x56, 0xB8][..], &[0xFF, 0xD0, 0xEB, 0x08][..]);
        assert!(follows(0x6BFD3B + 8, patch_bytes(0x6BFD3B, head, 0x1000_2000, tail), module_at));
        assert!(!follows(0x6BFD3B + 8, patch_bytes(0x6BFD3B, head, 0x0040_2000, tail), module_at), "the second call with a target in the game");
        // The patch of MWSE: a `call rel32` in place of 30 bytes; the distance is counted from the end of the 4 bytes.
        let at = 0x6C2578;
        let distance_to = |target: u32| target.wrapping_sub(at as u32 + 5);
        let mwse = |target: u32| patch_bytes(at, &[0xE8], distance_to(target), &[0x90; 25]);
        assert!(follows(at, mwse(0x2000_1000), module_at), "a function of MWSE.dll");
        assert!(follows(at + 4, mwse(0x2000_1000), module_at), "any byte of the run");
        assert!(!follows(at, mwse(0x1000_1000), module_at), "a call into d3d8.dll is not the patch of MWSE");
        assert!(!follows(at, mwse(0x0040_1000), module_at), "the right opcode and a target in the game");
        assert!(!follows(at, mwse(0), module_at), "a call to address 0");
        // The bytes around the 4 still count.
        let bad_tail = |addr: usize| if addr == at + 17 { 0x91 } else { mwse(0x2000_1000)(addr) };
        assert!(!follows(at, bad_tail, module_at), "a changed nop");
    }

    #[test]
    fn a_port_that_runs_the_engine_function_is_known() {
        assert!(wraps_engine(0x6F41B0), "NiGeometry::UpdatePropertiesDownward");
        assert!(wraps_engine(0x6F4270), "NiGeometry::UpdateEffectsDownward");
        assert!(!wraps_engine(0x6E4F90), "NiTriShape::Display is a port of Rust");
    }

    #[test]
    fn a_change_in_a_record_of_the_code_patch_names_its_option() {
        assert_eq!(code_patch_feature(0x6B5665, 1), Some(12));
        assert_eq!(code_patch_feature(0x6B5660, 6), Some(12), "a run that starts before the record");
        assert_eq!(code_patch_feature(0x6B5667, 2), None, "after the record");
        assert_eq!(code_patch_option(12), "Enchant glow in fog fix");
        for &(feature, _, _) in CODE_PATCH_RECORDS {
            assert_ne!(code_patch_option(feature), code_patch_option(0), "feature {feature} has a name");
        }
    }

    #[test]
    fn every_port_is_a_known_function_with_the_right_stack_size() {
        let inventory = include_str!("../gen/funcs.rs");
        for port in all() {
            let func = hook::find(port.addr).unwrap_or_else(|| panic!("{} is not in the function table", port.name));
            assert_eq!(func.name, port.name);
            assert!(func.pic && (func.steal >= 5 || func.steal == func.size), "{} cannot take a detour", port.name);
            if port.cleanup == crate::hook::Cleanup::Caller {
                assert_eq!(func.argsize, 0, "{}: a cdecl function removes no arguments", port.name);
            } else {
                assert_eq!(func.argsize, port.stack_args * 4, "{}: stack arguments", port.name);
            }
            assert!(inventory.contains(port.name));
        }
        let mut addrs: Vec<usize> = all().map(|p| p.addr).collect();
        addrs.sort();
        addrs.dedup();
        assert_eq!(addrs.len(), all().count(), "a function is listed twice");
    }
}
