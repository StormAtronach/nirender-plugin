//! The ported functions. One module per engine class; `PORTS` lists every port.

use crate::shadow::PortState;

/// Declares the port of a `__thiscall` engine function.
///
/// It makes a module with `STATE`, the detour, and `engine(...)` to call the game code.
/// `regions` adds the memory the function may write for a call; `compare_eax` says whether
/// the value in EAX is a result.
macro_rules! port_thiscall {
    (
        $modname:ident, $addr:literal, $ename:literal,
        fn($this:ident $(, $a:ident : $t:ty)*) -> $r:ty,
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
                $crate::shadow::PortState::new($ename, $addr, 0 $(+ { let _ = stringify!($a); 1 })*, detour_addr);

            /// The engine function.
            #[allow(dead_code)]
            pub unsafe fn engine($this: usize $(, $a: $t)*) -> u32 {
                let f: unsafe extern "thiscall" fn(usize $(, $t)*) -> u32 =
                    core::mem::transmute(STATE.orig.load(Ordering::Relaxed));
                f($this $(, $a)*)
            }

            unsafe extern "thiscall" fn detour($this: usize $(, $a: $t)*) -> u32 {
                $crate::shadow::dispatch(
                    &STATE,
                    $cmp,
                    #[allow(unused_variables)]
                    |$set: &mut $crate::shadow::Regions| $regbody,
                    || -> $r { $rust($this $(, $a)*) },
                    || engine($this $(, $a)*),
                )
            }
        }
    };
}

/// Declares the port of a `__cdecl` engine function: the caller removes the arguments.
#[allow(unused_macros)]
macro_rules! port_cdecl {
    (
        $modname:ident, $addr:literal, $ename:literal,
        fn($($a:ident : $t:ty),*) -> $r:ty,
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

            pub static STATE: $crate::shadow::PortState = $crate::shadow::PortState::new_cdecl(
                $ename, $addr, 0 $(+ { let _ = stringify!($a); 1 })*, detour_addr);

            /// The engine function.
            #[allow(dead_code)]
            pub unsafe fn engine($($a: $t),*) -> u32 {
                let f: unsafe extern "C" fn($($t),*) -> u32 = core::mem::transmute(STATE.orig.load(Ordering::Relaxed));
                f($($a),*)
            }

            unsafe extern "C" fn detour($($a: $t),*) -> u32 {
                $crate::shadow::dispatch(
                    &STATE,
                    $cmp,
                    #[allow(unused_variables)]
                    |$set: &mut $crate::shadow::Regions| $regbody,
                    || -> $r { $rust($($a),*) },
                    || engine($($a),*),
                )
            }
        }
    };
}

pub mod base;
pub mod buffers;
pub mod ct_pipeline_state;
pub mod device_setup;
pub mod draw;
pub mod draw_misc;
pub mod geometry;
pub mod light_manager;
pub mod material;
pub mod render_state;
pub mod rendered;
pub mod renderer_frame;
pub mod system_desc;
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
    textures::PORTS,
    geometry::PORTS,
    device_setup::PORTS,
    buffers::PORTS,
    base::PORTS,
    draw_misc::PORTS,
    draw::PORTS,
    ct_pipeline_state::PORTS,
];

/// The call sites the ports serve through `site::call_target`. A module with such sites
/// adds its `SITES` here.
static SITE_LISTS: &[&[usize]] = &[
    draw_misc::SITES,
    draw::SITES,
    ct_pipeline_state::SITES,
];

/// True when a port calls the target that is in memory at the `call` at `site`.
pub fn serves_site(site: usize) -> bool {
    SITE_LISTS.iter().any(|list| list.contains(&site))
}

/// Bytes of the game code that another mod changes and that a port follows by reading
/// them from memory: (address, length). A change inside such a range does not lock the port.
///
/// - `NiDX8Renderer::RenderShape` +0x2F: MWSE's software skinning flag
///   (PatchNIDX8Renderer_RenderShape).
/// - `NiDX8TexturePass::setCTPipelineState` +0xFC: MWSE's base map texture coordinate fix
///   (PatchNiDX8TexturePass_BaseMapTexcoord).
/// - `NiDX8Renderer::ctor`: MWSE's hash map sizes ("Optimize renderer hash map lookups").
static FOLLOWED: &[(usize, usize)] =
    &[(0x6ACF1F, 8), (0x6B42AC, 6), (0x6A9AED, 9), (0x6A9B38, 4), (0x6A9B7E, 4), (0x6A9BC4, 4)];

/// True when a port reads the `len` bytes at `addr` from memory and acts on what they are.
pub fn follows(addr: usize, len: usize) -> bool {
    FOLLOWED.iter().any(|&(start, size)| addr >= start && addr + len <= start + size)
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
    fn every_port_is_a_known_function_with_the_right_stack_size() {
        let inventory = include_str!("../gen/funcs.rs");
        for port in all() {
            let func = hook::find(port.addr).unwrap_or_else(|| panic!("{} is not in the function table", port.name));
            assert_eq!(func.name, port.name);
            assert!(func.pic && (func.steal >= 5 || func.steal == func.size), "{} cannot take a detour", port.name);
            if port.cdecl {
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
