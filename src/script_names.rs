//! The search for the next token in `ScriptRecord::resolveGlobalIDs` (plan 8g, 2026-10-18).
//!
//! The engine resolves the names of a script expression in place: a name token (a lower case
//! letter, a length byte, the name) becomes the upper case letter, the length, the record's
//! pointer at +2 and the fill 0x40 over the rest of the name. After a token the function finds
//! the next one with `strchr(at, ' ')` (the call at 0x4FF2A0). On an expression resolved
//! before, `at` is the letter of a resolved token, and the search crosses its length and its
//! pointer byte by byte: a 0x20 there, followed by a token letter, is taken for a new token.
//! The function then copies a "name" whose length byte it reads as a signed char (a byte of
//! 0x80 or more copies about 4 GB: the crash of the save load in the g1 pass), or looks up a
//! false name and writes over the real token (the "Unable to find ... "@@@@" warnings).
//! Which bytes a pointer holds depends on the addresses of the records.
//!
//! The fix replaces that one call: at the letter of a token that is resolved (the letter after
//! a space, its length, and the fill after the pointer), the search starts after the token.
//! Anywhere else it is `strchr`.

/// The call `call [strchr]` (the import at 0x74627C) that the fix replaces with a call of
/// `next_token` and a `nop`.
const CALL_SITE: usize = 0x4FF2A0;
const CALL_BYTES: [u8; 6] = [0xFF, 0x15, 0x7C, 0x62, 0x74, 0x00];

/// The letters of a resolved token: the race, sound, cell, dialogue, global script, object,
/// reference and faction, and a global variable (the upper case of its type).
const RESOLVED: &[u8] = b"ABCDMORTFLS";
/// The fill of a resolved token after its pointer.
const FILL: u8 = 0x40;
/// The bytes of the pointer of a resolved token.
const POINTER: usize = 4;
/// The longest name of a token (an id of the game is shorter).
const MOST_LENGTH: usize = 63;

use core::sync::atomic::{AtomicU64, Ordering};

/// The resolved tokens crossed by their length, and those where the engine's search would
/// have taken a false token.
static CROSSED: AtomicU64 = AtomicU64::new(0);
static PREVENTED: AtomicU64 = AtomicU64::new(0);

/// The line of the report.
pub fn report() -> String {
    format!(
        "script names: resolved tokens crossed by their length {}, false tokens prevented {}",
        CROSSED.load(Ordering::Relaxed),
        PREVENTED.load(Ordering::Relaxed)
    )
}

/// The bytes after a resolved token at `at`, or `None` when `at` is not one. Reads only the
/// token's bytes and the one after it.
unsafe fn after_resolved(at: *const u8) -> Option<*const u8> {
    if !RESOLVED.contains(&*at) || *at.sub(1) != b' ' {
        return None;
    }
    let length = *at.add(1) as usize;
    if length < POINTER || length > MOST_LENGTH {
        return None;
    }
    // The fill after the pointer, then the end of the token.
    for i in 2 + POINTER..2 + length {
        if *at.add(i) != FILL {
            return None;
        }
    }
    let end = at.add(2 + length);
    if *end != b' ' && *end != 0 {
        return None;
    }
    Some(end)
}

/// The engine's `strchr(at, c)`, except that a resolved token at `at` is crossed by its
/// length. The call site passes the arguments of `strchr` (cdecl).
unsafe extern "C" fn next_token(at: *const u8, c: i32) -> *const u8 {
    let resolved = after_resolved(at);
    if let Some(end) = resolved {
        CROSSED.fetch_add(1, Ordering::Relaxed);
        // The engine's search would have stopped at a space inside the token: a false token
        // when a letter the function acts on follows it.
        let mut q = at.add(1);
        while q < end {
            if *q == b' ' && b"GXabcdflmorst".contains(&*q.add(1)) {
                PREVENTED.fetch_add(1, Ordering::Relaxed);
                break;
            }
            q = q.add(1);
        }
    }
    let mut p = resolved.unwrap_or(at);
    let c = c as u8;
    loop {
        if *p == c {
            return p;
        }
        if *p == 0 {
            return core::ptr::null();
        }
        p = p.add(1);
    }
}

/// Puts the call of `next_token` at the call site, when the site holds the engine's call.
pub unsafe fn install() {
    let site = core::slice::from_raw_parts(CALL_SITE as *const u8, CALL_BYTES.len());
    if site != CALL_BYTES {
        crate::log!("script names: the call at {CALL_SITE:#x} is not the engine's ({site:02x?}): the search is not replaced");
        return;
    }
    let target = next_token as *const () as usize;
    let relative = target.wrapping_sub(CALL_SITE + 5) as u32;
    let mut bytes = [0x90u8; 6];
    bytes[0] = 0xE8;
    bytes[1..5].copy_from_slice(&relative.to_le_bytes());
    if crate::win::patch(CALL_SITE, &bytes) {
        crate::log!("script names: the search for the next token at {CALL_SITE:#x} steps over the resolved tokens");
    } else {
        crate::log!("script names: the call at {CALL_SITE:#x} could not be written");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn next(bytes: &[u8], at: usize) -> Option<usize> {
        let found = next_token(bytes.as_ptr().add(at), b' ' as i32);
        (!found.is_null()).then(|| found.offset_from(bytes.as_ptr()) as usize)
    }

    #[test]
    fn a_resolved_token_is_crossed_by_its_length() {
        // " D\x08" + a pointer with " a\x90" in it + the fill + " >= 1" + the end.
        let mut bytes = b" D\x08\x20\x61\x90\x0f@@@@ >= 1\0".to_vec();
        bytes[3] = 0x20;
        unsafe {
            // At the letter: the space after the token, not the one in the pointer.
            assert_eq!(next(&bytes, 1), Some(11));
        }
    }

    #[test]
    fn anything_else_is_strchr() {
        let bytes = b" X\x20\x10 == 1\0";
        unsafe {
            // An opcode with 0x20 in it: the engine's search, unchanged.
            assert_eq!(next(bytes, 1), Some(2));
            // No space left: null.
            assert_eq!(next(bytes, 9), None);
        }
        // A name not resolved: the engine's search.
        let name = b" d\x08HT Slave >= 1\0";
        unsafe {
            assert_eq!(next(name, 1), Some(5));
        }
        // A letter without the fill after the pointer is not a resolved token.
        let short = b" D\x05\x11\x22\x33\x44Q >= 1\0";
        unsafe {
            assert_eq!(next(short, 1), Some(8));
        }
    }
}
