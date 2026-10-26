//! Call sites of the game code that other mods redirect.
//!
//! MGE XE and MWSE change the target of some `call` instructions inside the functions that
//! this crate ports. A port that calls its callee directly drops that change. So at such a
//! site the port calls the target that the instruction in memory has now. That target is the
//! callee of the game when no mod changed it. It is the function of the mod when a mod
//! changed it.
//!
//! A port lists the sites that it serves this way in the `SITES` of its module. The installer
//! compares that list with the census. When a function has a change that its port does not
//! serve, the installer locks the port. The function stays game code in that install.

use crate::census;
use crate::gen::funcs::Func;

/// True when the instruction at `site` is a `call rel32`. The Morrowind Code Patch puts the
/// call of one of its functions at some places where the game calls a method of the device.
/// Without that option of the Code Patch, the game code has its own instruction there.
#[cfg(not(test))]
#[inline(always)]
pub unsafe fn is_call(site: usize) -> bool {
    *(site as *const u8) == 0xE8
}

/// The unit tests have no game code: each site has the call of the Code Patch.
#[cfg(test)]
pub unsafe fn is_call(_site: usize) -> bool {
    true
}

/// The target of the `call rel32` at `site`, as it is in memory now.
#[inline(always)]
pub unsafe fn call_target(site: usize) -> usize {
    debug_assert_eq!(*(site as *const u8), 0xE8);
    let rel = ((site + 1) as *const i32).read_unaligned();
    (site + 5).wrapping_add(rel as usize)
}

/// The kind of a run of changed bytes in a function.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind {
    /// Only the target of a `call` changed. The value is the address of the instruction.
    RedirectedCall(usize),
    /// Any other change: new instructions, a jump at the entry, changed constants.
    Rewrite,
}

/// Classifies the run of `len` changed bytes at `offset` of `func`, given the bytes in memory.
pub fn classify(func: &Func, live: &[u8], offset: usize, len: usize) -> Kind {
    let expected = census::expected(func);
    // The opcode is up to 4 bytes before the first changed byte of the target.
    for back in 1..=4 {
        if offset < back {
            break;
        }
        let opcode = offset - back;
        let inside = offset + len <= opcode + 5;
        if inside && expected[opcode] == 0xE8 && live[opcode] == 0xE8 {
            return Kind::RedirectedCall(func.ea + opcode);
        }
    }
    Kind::Rewrite
}

// ---- macros that call the target of a call site
//
// A module that uses these macros has a function `target(site: usize) -> usize` in scope. The
// macros call it for the address of the callee. It is `call_target`, or a function of the
// unit tests of the module that gives a stand-in.

/// The type `u32`, one time for each argument of a macro. The other macros use it.
macro_rules! u32_of {
    ($x:expr) => {
        u32
    };
}
pub(crate) use u32_of;

/// Calls the target of the `call` at `$site` as a `__thiscall` function. `$this` is a `usize`
/// and goes in ECX. Each other argument goes on the stack as a `u32`. The macro returns EAX.
macro_rules! site_thiscall {
    ($site:expr, $this:expr $(, $a:expr)* $(,)?) => {{
        let _call = $crate::entry_trace::call_out();
        let f: unsafe extern "thiscall" fn(usize $(, $crate::site::u32_of!($a))*) -> u32 = core::mem::transmute(target($site));
        f($this $(, $a as u32)*)
    }};
}
pub(crate) use site_thiscall;

/// Calls the target of the `call` at `$site` as a `__cdecl` function. Each argument goes on
/// the stack as a `u32`. The macro returns EAX.
macro_rules! site_cdecl {
    ($site:expr $(, $a:expr)* $(,)?) => {{
        let _call = $crate::entry_trace::call_out();
        let f: unsafe extern "C" fn($($crate::site::u32_of!($a)),*) -> u32 = core::mem::transmute(target($site));
        f($($a as u32),*)
    }};
}
pub(crate) use site_cdecl;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook;

    #[test]
    fn a_changed_call_target_is_a_redirected_call() {
        // In NiDX8Renderer::DrawPrimitive, MGE XE changes the target of the call at +0x167.
        let func = hook::find(0x6AEB80).unwrap();
        let mut live = census::expected(func).to_vec();
        assert_eq!(live[0x167], 0xE8);
        live[0x168..0x16C].copy_from_slice(&[0x64, 0x08, 0xE5, 0x7A]);
        assert_eq!(classify(func, &live, 0x168, 4), Kind::RedirectedCall(0x6AEB80 + 0x167));
        // A change of only the high bytes of the target is still that call.
        assert_eq!(classify(func, &live, 0x16A, 2), Kind::RedirectedCall(0x6AEB80 + 0x167));
        // A run that goes past the instruction is not that call.
        assert_eq!(classify(func, &live, 0x168, 6), Kind::Rewrite);
    }

    #[test]
    fn a_jump_at_the_entry_is_a_rewrite() {
        // In NiDX8VertexBufferManager::PackSkinnedVB, MGE XE puts a jump at the entry.
        let func = hook::find(0x6BE2B0).unwrap();
        let mut live = census::expected(func).to_vec();
        live[..6].copy_from_slice(&[0xE9, 0x4B, 0xC4, 0xE4, 0x7A, 0x90]);
        assert_eq!(classify(func, &live, 0, 6), Kind::Rewrite);
    }

    #[test]
    fn call_target_reads_the_instruction() {
        let code: [u8; 5] = [0xE8, 0x10, 0x00, 0x00, 0x00];
        let at = code.as_ptr() as usize;
        assert_eq!(unsafe { call_target(at) }, at + 5 + 0x10);
    }
}
