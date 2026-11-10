//! The byte census compares the functions of the renderer subsystem in memory with the
//! database.
//!
//! When the bytes of a function are different, another mod changed the function. That mod is
//! MWSE, MGE, or the Code Patch with other options than the database has. The port replicates
//! the database. If the port replaces a changed function, the change is lost. So each changed
//! function needs a decision.

use core::ffi::c_void;
use std::borrow::Cow;

use crate::gen::funcs::{Func, CODE, FUNCS};
use crate::hook::OwnWrite;

/// A run of bytes of a function that differ from the database.
pub struct Change {
    pub func: &'static Func,
    pub offset: usize,
    pub len: usize,
    /// The bytes that the plugin's jump displaced, which the census compared in place of the
    /// jump at the start of the function. Empty when the census compared memory only.
    pub under_jump: Vec<u8>,
    /// The run holds the plugin's jump at the entry, and another module changed that jump.
    pub own_jump_changed: bool,
}

impl Change {
    /// The function as the census compared it: memory, with [`Change::under_jump`] at its start.
    ///
    /// # Safety
    ///
    /// As for [`describe`]: the function is readable memory of the loaded Morrowind.exe.
    pub unsafe fn compared(&self) -> Vec<u8> {
        let mut bytes = core::slice::from_raw_parts(self.func.ea as *const u8, self.func.size).to_vec();
        bytes[..self.under_jump.len()].copy_from_slice(&self.under_jump);
        bytes
    }
}

/// The head of a function at a recheck.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head {
    /// The plugin wrote no jump at the entry.
    NoJump,
    /// The plugin's jump is at the entry as it wrote it.
    OwnJump,
    /// The plugin wrote a jump at the entry, and the bytes there are not that jump now.
    JumpChanged,
}

/// `live` as a recheck compares it, with the state of its head. With the plugin's own jump
/// intact at the entry, the bytes under the jump are those that it displaced: the jump never
/// shows, and a change that the trampoline carries shows as at the census of the install. When
/// the bytes at the entry are not the recorded jump, the live head shows whole.
pub fn compared<'a>(live: &'a [u8], own: Option<&OwnWrite>) -> (Cow<'a, [u8]>, Head) {
    let Some(own) = own else {
        return (Cow::Borrowed(live), Head::NoJump);
    };
    // A function shorter than the jump has the end of the jump in the padding after it.
    let covered = own.jump.len().min(live.len());
    if live[..covered] != own.jump[..covered] {
        return (Cow::Borrowed(live), Head::JumpChanged);
    }
    let mut bytes = live.to_vec();
    bytes[..covered].copy_from_slice(&own.displaced[..covered]);
    (Cow::Owned(bytes), Head::OwnJump)
}

/// The runs of [`runs`] on the bytes of [`compared`]. The head is put back before the runs are
/// found, because [`runs`] joins close differences: a change just past the jump must not join a
/// run of the jump.
pub fn runs_masked(live: &[u8], expected: &[u8], own: Option<&OwnWrite>) -> Vec<(usize, usize)> {
    runs(&compared(live, own).0, expected)
}

/// The runs where `live` differs from `expected`. Runs closer than 4 bytes are one run. A
/// patched address frequently has one or two bytes that are the same as in the old address.
pub fn runs(live: &[u8], expected: &[u8]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for i in 0..live.len().min(expected.len()) {
        if live[i] == expected[i] {
            continue;
        }
        match out.last_mut() {
            Some((start, len)) if i < *start + *len + 4 => *len = i - *start + 1,
            _ => out.push((i, 1)),
        }
    }
    out
}

pub fn expected(func: &Func) -> &'static [u8] {
    &CODE[func.code..func.code + func.size]
}

/// Every run of changed bytes in the functions of the table. With `hooked` (a recheck after the
/// install), the plugin's own jump at the entry of a function is compared as the bytes it
/// displaced ([`compared`]).
///
/// # Safety
///
/// The function table matches the loaded Morrowind.exe: each function of it is readable memory of
/// `size` bytes at `ea`.
pub unsafe fn changes(hooked: bool) -> Vec<Change> {
    let mut out = Vec::new();
    for func in FUNCS.iter() {
        let live = core::slice::from_raw_parts(func.ea as *const u8, func.size);
        let own = if hooked { crate::hook::own_write(func.ea) } else { None };
        let (bytes, head) = compared(live, own.as_ref());
        let under_jump = match (&own, head) {
            (Some(own), Head::OwnJump) => own.displaced[..own.displaced.len().min(func.size)].to_vec(),
            _ => Vec::new(),
        };
        let jump_len = own.as_ref().map_or(0, |own| own.jump.len());
        for (offset, len) in runs(&bytes, expected(func)) {
            let own_jump_changed = head == Head::JumpChanged && offset < jump_len;
            out.push(Change { func, offset, len, under_jump: under_jump.clone(), own_jump_changed });
        }
    }
    out
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExA(flags: u32, address: *const c_void, module: *mut usize) -> i32;
    fn GetModuleFileNameA(module: usize, name: *mut u8, size: u32) -> u32;
}

/// The base address of the module that holds `address`, or 0 for memory outside any module.
///
/// # Safety
///
/// No condition: any value of `address` is allowed. It is `unsafe` because of the call of the
/// Windows function.
pub unsafe fn module_base_of(address: usize) -> usize {
    let mut module = 0usize;
    // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | UNCHANGED_REFCOUNT
    if GetModuleHandleExA(4 | 2, address as *const c_void, &mut module) == 0 {
        return 0;
    }
    module
}

/// The file name of the module that holds `address`, or "?" for memory outside any module.
///
/// # Safety
///
/// No condition: any value of `address` is allowed. It is `unsafe` because of the calls of the
/// Windows functions.
pub unsafe fn module_of(address: usize) -> String {
    let module = module_base_of(address);
    if module == 0 {
        return "?".into();
    }
    let mut name = [0u8; 260];
    let len = GetModuleFileNameA(module, name.as_mut_ptr(), name.len() as u32) as usize;
    let path = String::from_utf8_lossy(&name[..len]).into_owned();
    path.rsplit(['\\', '/']).next().unwrap_or("?").to_string()
}

/// One line for a change. It gives the address, the bytes, and the target of a new call or
/// jump. The bytes are those that the census compared ([`Change::compared`]): under the
/// plugin's own jump, the displaced bytes, with a call's target from its old place.
///
/// # Safety
///
/// `change` came from [`changes`], so its function and run are readable memory of the loaded
/// Morrowind.exe.
pub unsafe fn describe(change: &Change) -> String {
    let bytes = change.compared();
    let at = change.func.ea + change.offset;
    let shown = change.len.min(12);
    let live = &bytes[change.offset..change.offset + shown];
    let old = &expected(change.func)[change.offset..change.offset + shown];
    let mut text = format!("{:#x} {}+{:#x} [{}]: {:02x?} was {:02x?}", at, change.func.name, change.offset, change.len, live, old);
    // This finds a call or a jump with a changed 4-byte target. The opcode is at the run or
    // one byte before it.
    for back in 0..2 {
        if change.offset < back {
            continue;
        }
        let opcode_offset = change.offset - back;
        let opcode = bytes[opcode_offset];
        if (opcode == 0xE8 || opcode == 0xE9) && opcode_offset + 5 <= change.func.size {
            let rel = i32::from_le_bytes([bytes[opcode_offset + 1], bytes[opcode_offset + 2], bytes[opcode_offset + 3], bytes[opcode_offset + 4]]);
            let target = (change.func.ea + opcode_offset + 5).wrapping_add(rel as usize);
            text.push_str(&format!(" -> {} {:#x} in {}", if opcode == 0xE8 { "call" } else { "jmp" }, target, module_of(target)));
            break;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_join_close_differences() {
        let expected = [0u8; 16];
        let mut live = [0u8; 16];
        assert!(runs(&live, &expected).is_empty());
        live[2] = 1;
        live[5] = 1; // This is 3 bytes after the first byte, so it is in the same run.
        live[12] = 1;
        assert_eq!(runs(&live, &expected), vec![(2, 4), (12, 1)]);
    }

    fn func_named(name: &str) -> &'static Func {
        FUNCS.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name} is in the table"))
    }

    /// The plugin's write at the entry of `func`, a jump with its fill of NOPs up to the stolen
    /// length, over `displaced`.
    fn own_write(func: &Func, displaced: &[u8]) -> OwnWrite {
        let mut jump = vec![0xE9, 0x10, 0x20, 0x30, 0x40];
        jump.resize(func.steal.max(5), 0x90);
        OwnWrite { displaced: displaced[..jump.len()].to_vec(), jump }
    }

    /// The function in memory: the database bytes with `own`'s jump at the entry.
    fn hooked(func: &Func, own: &OwnWrite) -> Vec<u8> {
        let mut live = expected(func).to_vec();
        live[..own.jump.len()].copy_from_slice(&own.jump);
        live
    }

    #[test]
    fn the_plugins_own_jump_is_no_change() {
        let func = func_named("NiDX8Renderer::deleting_dtor");
        assert_eq!(func.steal, 8);
        let own = own_write(func, expected(func));
        let live = hooked(func, &own);
        assert_eq!(compared(&live, Some(&own)).1, Head::OwnJump);
        assert!(runs_masked(&live, expected(func), Some(&own)).is_empty());
        // The fill NOP after a jump of a stolen length of 6 does not show either.
        let func = func_named("NiRenderer::MapWindowPointToBufferPoint");
        assert_eq!(func.steal, 6);
        let own = own_write(func, expected(func));
        assert!(runs_masked(&hooked(func, &own), expected(func), Some(&own)).is_empty());
    }

    #[test]
    fn a_write_into_the_plugins_jump_shows_the_whole_head() {
        let func = func_named("NiDX8Renderer::deleting_dtor");
        let own = own_write(func, expected(func));
        let mut live = hooked(func, &own);
        live[3] = 0xCC;
        assert_eq!(compared(&live, Some(&own)).1, Head::JumpChanged);
        assert_eq!(runs_masked(&live, expected(func), Some(&own))[0].0, 0, "the run starts at the entry");
    }

    #[test]
    fn a_write_just_past_the_jump_is_a_run_of_its_own() {
        // The pick ray bytes of MWSE start at +7, past a jump of 6 bytes.
        let func = func_named("NiRenderer::MapWindowPointToBufferPoint");
        let own = own_write(func, expected(func));
        let mut live = hooked(func, &own);
        live[7] ^= 0xFF;
        assert_eq!(runs_masked(&live, expected(func), Some(&own)), vec![(7, 1)]);
        // Without the head put back first, the jump and the write are one run.
        assert_eq!(runs(&live, expected(func))[0].0, 0);
    }

    #[test]
    fn a_change_under_the_jump_shows_as_at_the_install() {
        // The W25 form: MWSE sends the call at +3 of `deleting_dtor` to its own function before
        // the install, so the trampoline carries it and the jump covers it.
        let func = func_named("NiDX8Renderer::deleting_dtor");
        let mut before_install = expected(func).to_vec();
        assert_eq!(&before_install[..4], &[0x56, 0x8B, 0xF1, 0xE8]);
        before_install[4..8].copy_from_slice(&[0x38, 0xE5, 0x50, 0x03]);
        let at_install = runs(&before_install, expected(func));
        assert_eq!(at_install, vec![(4, 4)]);
        let own = own_write(func, &before_install);
        assert_eq!(runs_masked(&hooked(func, &own), expected(func), Some(&own)), at_install);
    }

    #[test]
    fn the_code_table_covers_every_function() {
        let last = FUNCS.iter().map(|f| f.code + f.size).max().unwrap();
        assert_eq!(last, CODE.len());
        for func in FUNCS.iter() {
            assert_eq!(&expected(func)[..func.size.min(16)], &func.head[..func.size.min(16)], "{}", func.name);
        }
    }
}
