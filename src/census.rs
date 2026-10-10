//! The byte census: the functions of the renderer subsystem in memory against the IDA database.
//!
//! A function whose bytes differ was changed by another mod (MWSE, MGE, the Code Patch with
//! other options than the database has). The Rust port replicates the database; replacing a
//! changed function would drop the change, so each one needs a decision.

use core::ffi::c_void;

use crate::gen::funcs::{Func, CODE, FUNCS};

pub fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 0x811C_9DC5u32;
    for &b in bytes {
        hash = (hash ^ b as u32).wrapping_mul(0x0100_0193);
    }
    hash
}

/// A run of bytes of a function that differ from the database.
pub struct Change {
    pub func: &'static Func,
    pub offset: usize,
    pub len: usize,
}

/// The runs where `live` differs from `expected`. Runs closer than 4 bytes are one run: a
/// patched address often shares a byte or two with the old one.
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

/// Every run of changed bytes in the functions of the table.
pub unsafe fn changes() -> Vec<Change> {
    let mut out = Vec::new();
    for func in FUNCS.iter() {
        let live = core::slice::from_raw_parts(func.ea as *const u8, func.size);
        for (offset, len) in runs(live, expected(func)) {
            out.push(Change { func, offset, len });
        }
    }
    out
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExA(flags: u32, address: *const c_void, module: *mut usize) -> i32;
    fn GetModuleFileNameA(module: usize, name: *mut u8, size: u32) -> u32;
}

/// The file name of the module that holds `address`, or "?" for memory outside any module.
pub unsafe fn module_of(address: usize) -> String {
    let mut module = 0usize;
    // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | UNCHANGED_REFCOUNT
    if GetModuleHandleExA(4 | 2, address as *const c_void, &mut module) == 0 || module == 0 {
        return "?".into();
    }
    let mut name = [0u8; 260];
    let len = GetModuleFileNameA(module, name.as_mut_ptr(), name.len() as u32) as usize;
    let path = String::from_utf8_lossy(&name[..len]).into_owned();
    path.rsplit(['\\', '/']).next().unwrap_or("?").to_string()
}

/// One line for a change: where, the bytes, and where a new call or jump goes.
pub unsafe fn describe(change: &Change) -> String {
    let at = change.func.ea + change.offset;
    let shown = change.len.min(12);
    let live = core::slice::from_raw_parts(at as *const u8, shown);
    let old = &expected(change.func)[change.offset..change.offset + shown];
    let mut text = format!("{:#x} {}+{:#x} [{}]: {:02x?} was {:02x?}", at, change.func.name, change.offset, change.len, live, old);
    // A call or jump whose 4-byte target changed: the opcode is at the run or one byte before it.
    for back in 0..2 {
        if change.offset < back {
            continue;
        }
        let opcode_at = at - back;
        let opcode = *(opcode_at as *const u8);
        if (opcode == 0xE8 || opcode == 0xE9) && change.offset - back + 5 <= change.func.size {
            let rel = (opcode_at as *const u8).add(1).cast::<i32>().read_unaligned();
            let target = (opcode_at + 5).wrapping_add(rel as usize);
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
    fn fnv1a_known_values() {
        assert_eq!(fnv1a(b""), 0x811C_9DC5);
        assert_eq!(fnv1a(b"a"), 0xE40C_292C);
        assert_eq!(fnv1a(b"foobar"), 0xBF9C_F968);
    }

    #[test]
    fn runs_join_close_differences() {
        let expected = [0u8; 16];
        let mut live = [0u8; 16];
        assert!(runs(&live, &expected).is_empty());
        live[2] = 1;
        live[5] = 1; // 3 bytes after the first: same run
        live[12] = 1;
        assert_eq!(runs(&live, &expected), vec![(2, 4), (12, 1)]);
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
