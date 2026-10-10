//! Call sites of the game code that other mods redirect.
//!
//! MGE XE and MWSE change the target of some `call` instructions inside the functions this
//! crate ports. A port that calls its callee directly would drop that change. So at such a
//! site the port calls whatever the instruction in memory calls now: the callee of the game
//! when nobody changed it, the function of the mod when a mod did.
//!
//! A port lists the sites it serves this way in the `SITES` of its module. The installer
//! compares that list with the census: a function with a change that its port does not
//! serve is locked, and stays game code in that install.

use crate::census;
use crate::gen::funcs::Func;

/// Where the `call rel32` at `site` goes, as it is in memory now.
#[inline(always)]
pub unsafe fn call_target(site: usize) -> usize {
    debug_assert_eq!(*(site as *const u8), 0xE8);
    let rel = ((site + 1) as *const i32).read_unaligned();
    (site + 5).wrapping_add(rel as usize)
}

/// What a run of changed bytes in a function is.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind {
    /// Only the target of a `call` changed. The value is the address of the instruction.
    RedirectedCall(usize),
    /// Anything else: new instructions, a jump at the entry, changed constants.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook;

    #[test]
    fn a_changed_call_target_is_a_redirected_call() {
        // NiDX8Renderer::DrawPrimitive: MGE XE changes the target of the call at +0x167.
        let func = hook::find(0x6AEB80).unwrap();
        let mut live = census::expected(func).to_vec();
        assert_eq!(live[0x167], 0xE8);
        live[0x168..0x16C].copy_from_slice(&[0x64, 0x08, 0xE5, 0x7A]);
        assert_eq!(classify(func, &live, 0x168, 4), Kind::RedirectedCall(0x6AEB80 + 0x167));
        // A change of only the high bytes of the target is still that call.
        assert_eq!(classify(func, &live, 0x16A, 2), Kind::RedirectedCall(0x6AEB80 + 0x167));
        // A run that goes past the instruction is not.
        assert_eq!(classify(func, &live, 0x168, 6), Kind::Rewrite);
    }

    #[test]
    fn a_jump_at_the_entry_is_a_rewrite() {
        // NiDX8VertexBufferManager::PackSkinnedVB: MGE XE puts a jump at the entry.
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
