//! Detours on engine functions.
//!
//! The entry of the engine function gets a jump to an entry thunk. The thunk calls the Rust
//! detour and puts ECX and EDX back, because callers in Morrowind.exe were compiled with
//! knowledge of what each callee leaves in the registers. A trampoline holds the displaced
//! entry instructions and jumps back, so the Rust side can still call the engine function.

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::gen::funcs::{Func, FUNCS};
use crate::win;

const ARENA_SIZE: usize = 0x20000;
static ARENA: AtomicUsize = AtomicUsize::new(0);
static ARENA_USED: AtomicUsize = AtomicUsize::new(0);

pub fn find(ea: usize) -> Option<&'static Func> {
    FUNCS.binary_search_by_key(&ea, |f| f.ea).ok().map(|i| &FUNCS[i])
}

unsafe fn arena_alloc(bytes: &[u8]) -> Option<usize> {
    let mut base = ARENA.load(Ordering::Relaxed);
    if base == 0 {
        base = win::VirtualAlloc(core::ptr::null_mut(), ARENA_SIZE, win::MEM_COMMIT_RESERVE, win::PAGE_EXECUTE_READWRITE) as usize;
        if base == 0 {
            return None;
        }
        ARENA.store(base, Ordering::Relaxed);
    }
    let used = ARENA_USED.load(Ordering::Relaxed);
    let aligned = (bytes.len() + 15) & !15;
    if used + aligned > ARENA_SIZE {
        return None;
    }
    ARENA_USED.store(used + aligned, Ordering::Relaxed);
    let addr = base + used;
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len());
    Some(addr)
}

fn rel32(from_next: usize, to: usize) -> [u8; 4] {
    (to.wrapping_sub(from_next) as u32).to_le_bytes()
}

/// How the function gets its stack arguments off the stack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cleanup {
    /// thiscall and stdcall: the function removes its arguments.
    Callee,
    /// cdecl: the caller removes them.
    Caller,
}

/// The code of an entry thunk for a detour with `stack_args` 4-byte stack arguments.
/// `at` is the address the thunk will have.
pub fn entry_thunk_code(at: usize, detour: usize, stack_args: usize, cleanup: Cleanup) -> Vec<u8> {
    let mut code = vec![0x52, 0x51]; // push edx; push ecx
    // The arguments again, last first. After each push the next one is at the same offset.
    let offset = 8 + 4 * stack_args;
    assert!(offset < 0x80);
    for _ in 0..stack_args {
        code.extend_from_slice(&[0xFF, 0x74, 0x24, offset as u8]); // push dword [esp+offset]
    }
    code.push(0xE8); // call detour
    let next = at + code.len() + 4;
    code.extend_from_slice(&rel32(next, detour));
    if cleanup == Cleanup::Caller && stack_args > 0 {
        // The detour is cdecl too and left the copies on the stack.
        code.extend_from_slice(&[0x83, 0xC4, (4 * stack_args) as u8]); // add esp, n
    }
    code.extend_from_slice(&[0x59, 0x5A]); // pop ecx; pop edx
    if cleanup == Cleanup::Callee && stack_args > 0 {
        code.push(0xC2); // ret n
        code.extend_from_slice(&((4 * stack_args) as u16).to_le_bytes());
    } else {
        code.push(0xC3);
    }
    code
}

/// Installs a detour on the engine function at `ea`. Returns the address to call for the
/// engine function itself.
pub unsafe fn install(ea: usize, detour: usize, stack_args: usize, cleanup: Cleanup) -> Result<usize, String> {
    let func = find(ea).ok_or_else(|| format!("0x{ea:x} is not in the function table"))?;
    if !func.pic {
        return Err("the entry instructions cannot be moved".into());
    }
    let relocs: Vec<usize> = func.relocs.iter().filter(|&&o| o != 0).map(|&o| o as usize).collect();
    install_raw(ea, &func.head[..func.steal], &relocs, detour, stack_args, cleanup)
}

/// As `install`, for a function outside the table. `head` holds the whole instructions the
/// jump displaces (all of the function when it is shorter than the jump); `relocs` are the
/// offsets in `head` of 32-bit relative call or jump targets.
pub unsafe fn install_raw(
    ea: usize,
    head: &[u8],
    relocs: &[usize],
    detour: usize,
    stack_args: usize,
    cleanup: Cleanup,
) -> Result<usize, String> {
    let live = core::slice::from_raw_parts(ea as *const u8, 16);
    if live[0] == 0xE9 && head[0] != 0xE9 {
        // Another mod has a jump here already. Keep its target.
        let rel = u32::from_le_bytes([live[1], live[2], live[3], live[4]]) as usize;
        let target = (ea + 5).wrapping_add(rel);
        let at = arena_alloc(&[0xE9, 0, 0, 0, 0]).ok_or("no memory for a trampoline")?;
        core::ptr::copy_nonoverlapping(rel32(at + 5, target).as_ptr(), (at + 1) as *mut u8, 4);
        return finish(ea, at, 5, detour, stack_args, cleanup);
    }
    let steal = head.len();
    if live[..steal] != *head {
        return Err(format!("entry bytes differ from the database: {:02x?} against {:02x?}", &live[..steal], head));
    }
    let mut code = head.to_vec();
    let whole = steal < 5;
    if whole {
        // The function is shorter than the jump. It ends in a return, and the jump may take
        // the padding after it, never the start of the next function.
        if live[steal..5].iter().any(|&b| b != 0x90 && b != 0xCC) {
            return Err("no padding after a function shorter than 5 bytes".into());
        }
    } else {
        code.extend_from_slice(&[0xE9, 0, 0, 0, 0]);
    }
    let at = arena_alloc(&code).ok_or("no memory for a trampoline")?;
    if !whole {
        core::ptr::copy_nonoverlapping(rel32(at + steal + 5, ea + steal).as_ptr(), (at + steal + 1) as *mut u8, 4);
    }
    for &offset in relocs {
        let rel = u32::from_le_bytes([live[offset], live[offset + 1], live[offset + 2], live[offset + 3]]) as usize;
        let target = (ea + offset + 4).wrapping_add(rel);
        core::ptr::copy_nonoverlapping(rel32(at + offset + 4, target).as_ptr(), (at + offset) as *mut u8, 4);
    }
    finish(ea, at, steal.max(5), detour, stack_args, cleanup)
}

unsafe fn finish(ea: usize, trampoline: usize, steal: usize, detour: usize, stack_args: usize, cleanup: Cleanup) -> Result<usize, String> {
    // Reserve the thunk's place first: its code depends on its own address.
    let size = entry_thunk_code(0, detour, stack_args, cleanup).len();
    let thunk = arena_alloc(&vec![0xCC; size]).ok_or("no memory for a thunk")?;
    let code = entry_thunk_code(thunk, detour, stack_args, cleanup);
    core::ptr::copy_nonoverlapping(code.as_ptr(), thunk as *mut u8, code.len());
    win::FlushInstructionCache(win::GetCurrentProcess(), ARENA.load(Ordering::Relaxed) as *const c_void, ARENA_SIZE);

    let mut jump = vec![0xE9];
    jump.extend_from_slice(&rel32(ea + 5, thunk));
    jump.resize(steal, 0x90);
    if !win::patch(ea, &jump) {
        return Err("could not write the jump".into());
    }
    Ok(trampoline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_table_is_sorted_and_findable() {
        assert!(FUNCS.windows(2).all(|w| w[0].ea < w[1].ea));
        let f = find(0x65ED60).expect("SetRenderState");
        assert_eq!(f.name, "NiRenderState::SetRenderState");
        assert!(f.steal >= 5 && f.pic);
        assert!(find(0x65ED61).is_none());
    }

    #[test]
    fn thunk_for_a_thiscall_with_three_stack_arguments() {
        let code = entry_thunk_code(0x1000, 0x2000, 3, Cleanup::Callee);
        assert_eq!(&code[..2], &[0x52, 0x51]);
        // three pushes of [esp+20]
        assert_eq!(&code[2..14], &[0xFF, 0x74, 0x24, 20, 0xFF, 0x74, 0x24, 20, 0xFF, 0x74, 0x24, 20]);
        assert_eq!(code[14], 0xE8);
        let rel = u32::from_le_bytes([code[15], code[16], code[17], code[18]]) as usize;
        assert_eq!(0x1000 + 19 + rel, 0x2000);
        assert_eq!(&code[19..], &[0x59, 0x5A, 0xC2, 12, 0]);
    }

    #[test]
    fn thunk_for_a_cdecl_function_leaves_the_arguments_to_the_caller() {
        let code = entry_thunk_code(0, 0x100, 2, Cleanup::Caller);
        assert_eq!(&code[code.len() - 6..], &[0x83, 0xC4, 8, 0x59, 0x5A, 0xC3]);
        let none = entry_thunk_code(0, 0x100, 0, Cleanup::Callee);
        assert_eq!(*none.last().unwrap(), 0xC3);
    }
}
