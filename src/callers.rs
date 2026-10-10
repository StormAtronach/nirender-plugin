//! The census of the callers of the device: which code calls each method of the
//! `IDirect3DDevice8` of the game.
//!
//! The goal of the project is a game that makes no D3D8 call. The plugin has the functions of
//! the renderer subsystem. This census finds the other code that calls the device: video,
//! screenshots, gamma, code of other mods.
//!
//! The wrappers of `device.rs` call `count` with the return address of each device call.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::gen::funcs::FUNCS;

/// Who a caller is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Caller {
    /// A function of the renderer subsystem: game code that the plugin has a port for. The
    /// value is the address of the function.
    Renderer(usize),
    /// The Rust code of this plugin.
    Plugin,
    /// Other code. The value is the return address of the call.
    Other(usize),
}

static COUNTS: Mutex<Option<HashMap<(usize, Caller), u64>>> = Mutex::new(None);

/// The function of the renderer subsystem that `address` is in.
fn renderer_function(address: usize) -> Option<usize> {
    // The table has the functions in the order of the database, not of the addresses.
    FUNCS.iter().find(|f| address >= f.ea && address < f.ea + f.size).map(|f| f.ea)
}

/// True when `address` is in a function of the renderer subsystem.
#[cfg(test)]
fn in_renderer(address: usize) -> bool {
    renderer_function(address).is_some()
}

fn in_plugin(address: usize) -> bool {
    let (start, end) = plugin_range();
    address >= start && address < end
}

/// The address range of this DLL: from its base to the end of its image.
fn plugin_range() -> (usize, usize) {
    use std::sync::OnceLock;
    static RANGE: OnceLock<(usize, usize)> = OnceLock::new();
    *RANGE.get_or_init(|| unsafe {
        let here = plugin_range as *const () as usize;
        let mut base = 0usize;
        if GetModuleHandleExA(4 | 2, here as *const core::ffi::c_void, &mut base) == 0 || base == 0 {
            return (0, 0);
        }
        // The size of the image is in the optional header of the PE file.
        let pe = base + *((base + 0x3C) as *const u32) as usize;
        let size = *((pe + 0x50) as *const u32) as usize;
        (base, base + size)
    })
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExA(flags: u32, address: *const core::ffi::c_void, module: *mut usize) -> i32;
}

/// Counts one device call. `return_address` is where the call returns to.
pub fn count(slot: usize, return_address: usize) {
    let caller = if in_plugin(return_address) {
        Caller::Plugin
    } else if let Some(function) = renderer_function(return_address) {
        Caller::Renderer(function)
    } else {
        Caller::Other(return_address)
    };
    if let Ok(mut counts) = COUNTS.lock() {
        *counts.get_or_insert_with(HashMap::new).entry((slot, caller)).or_insert(0) += 1;
    }
}

/// Starts or stops the census. `start` clears the counts.
pub fn set(on: bool) {
    if on {
        if let Ok(mut counts) = COUNTS.lock() {
            *counts = Some(HashMap::new());
        }
    }
    crate::device::set_caller_census(on);
}

/// The counts as text. The first line has the totals. Then one line for each method and
/// caller that is not the renderer subsystem and not the plugin:
/// `method<TAB>module<TAB>address<TAB>count`.
pub unsafe fn report() -> String {
    let counts = match COUNTS.lock() {
        Ok(counts) => counts.clone().unwrap_or_default(),
        Err(_) => return "the census has no counts".to_string(),
    };
    let total = |kind: fn(&Caller) -> bool| counts.iter().filter(|((_, c), _)| kind(c)).map(|(_, n)| n).sum::<u64>();
    let mut text = format!(
        "renderer {} plugin {} other {}\n",
        total(|c| matches!(c, Caller::Renderer(_))),
        total(|c| *c == Caller::Plugin),
        total(|c| matches!(c, Caller::Other(_)))
    );
    // The game code of the renderer subsystem that made a call: with the Rust code on, this
    // is code that a port did not replace.
    let mut renderer: Vec<(usize, usize, u64)> = counts
        .iter()
        .filter_map(|((slot, caller), n)| match caller {
            Caller::Renderer(function) => Some((*function, *slot, *n)),
            _ => None,
        })
        .collect();
    renderer.sort();
    for (function, slot, n) in renderer {
        let name = FUNCS.iter().find(|f| f.ea == function).map_or("?", |f| f.name);
        text.push_str(&format!("{}\trenderer {}\t{:#x}\t{}\n", crate::device::slot_name(slot), name, function, n));
    }
    // The calls of the Rust code, for each method: the D3D8 calls that the plugin still makes.
    let mut plugin: Vec<(usize, u64)> = counts
        .iter()
        .filter_map(|((slot, caller), n)| match caller {
            Caller::Plugin => Some((*slot, *n)),
            _ => None,
        })
        .collect();
    plugin.sort();
    for (slot, n) in plugin {
        text.push_str(&format!("{}\tplugin\t-\t{}\n", crate::device::slot_name(slot), n));
    }
    let mut other: Vec<(usize, usize, u64)> = counts
        .iter()
        .filter_map(|((slot, caller), n)| match caller {
            Caller::Other(address) => Some((*slot, *address, *n)),
            _ => None,
        })
        .collect();
    other.sort();
    for (slot, address, n) in other {
        let module = crate::census::module_of(address);
        let name = module.rsplit(['\\', '/']).next().unwrap_or("?").to_string();
        text.push_str(&format!("{}\t{}\t{:#x}\t{}\n", crate::device::slot_name(slot), name, address, n));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_renderer_address_is_in_the_renderer() {
        let first = &FUNCS[0];
        assert!(in_renderer(first.ea));
        assert!(in_renderer(first.ea + first.size - 1));
        assert!(!in_renderer(0x401000));
    }
}
