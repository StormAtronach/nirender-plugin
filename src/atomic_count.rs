//! The count writes of `NiRefObject` on the property paths are locked writes (risk 56).
//!
//! The main thread and the cell loader thread (`game_threadLoadCells`) both run
//! `NiPropertyState::reset`, `NiPropertyState::ctor_copy`, `NiPointer::set`, `NiPointer::dtor`
//! and the property updates of the scene graph. Each of them changes a count with an unlocked
//! `inc` or `dec`, and the two threads write the count of a shared property in the same
//! microseconds. A lost change makes the count lower than the number of holders: the property
//! is freed while holders remain, and their releases at exit damage the heap. This module puts
//! a `lock` on each of these writes.
//!
//! Each site is 5 bytes of one of two forms, and the 5 bytes become a jump to a stub:
//! - `jz +3; inc dword [r+4]` (74 03 FF modrm 04): the stub is
//!   `jz back; lock inc dword [r+4]; back: jmp site+5`.
//! - `dec dword [ecx+4]; jnz +n` (FF 49 04 75 n): the stub is
//!   `lock dec dword [ecx+4]; jnz site+5+n; jmp site+5`.
//!
//! The stub leaves the flags as the original code does. No branch of the game lands inside the
//! 5 bytes, and the instruction after them does not read the flags \[V IDA, 2026-11-04\]. All
//! sites are checked before the first is written: one byte that differs refuses all of them.

use core::sync::atomic::{AtomicBool, Ordering};

/// One site: the address of its 5 bytes, the function, and what it counts.
struct Site {
    at: usize,
    function: &'static str,
    slot: &'static str,
}

const SITES: [Site; 18] = [
    Site { at: 0x6FE39F, function: "NiPropertyState::reset", slot: "default alpha" },
    Site { at: 0x6FE3BF, function: "NiPropertyState::reset", slot: "default fog" },
    Site { at: 0x6FE3DF, function: "NiPropertyState::reset", slot: "default material" },
    Site { at: 0x6FE3FF, function: "NiPropertyState::reset", slot: "default stencil" },
    Site { at: 0x6FE41F, function: "NiPropertyState::reset", slot: "default texturing" },
    Site { at: 0x6FE43F, function: "NiPropertyState::reset", slot: "default vertex colour" },
    Site { at: 0x6FE45F, function: "NiPropertyState::reset", slot: "default wireframe" },
    Site { at: 0x6FE489, function: "NiPropertyState::reset", slot: "default zbuffer" },
    Site { at: 0x6FE4B3, function: "NiPropertyState::reset", slot: "default renderer specific" },
    Site { at: 0x6FE2FC, function: "NiPropertyState::ctor_copy", slot: "each property of the copied state" },
    Site { at: 0x471B25, function: "NiPointer::set", slot: "the new object" },
    Site { at: 0x471B12, function: "NiPointer::set", slot: "the old object (release)" },
    Site { at: 0x40A886, function: "NiPointer::dtor", slot: "the object (release)" },
    Site { at: 0x6EB230, function: "NiAVObject::PushLocalProperties", slot: "the parent state" },
    Site { at: 0x6EB333, function: "NiAVObject::PushLocalProperties", slot: "the new state" },
    Site { at: 0x6C8D82, function: "NiNode::UpdateProperties", slot: "the state" },
    Site { at: 0x6F420F, function: "NiGeometry::UpdatePropertiesDownward", slot: "the geometry's state" },
    Site { at: 0x6EB125, function: "NiAVObject::UpdateProperties", slot: "the state" },
];

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// The registers that an `inc dword [r+4]` of a site uses (the modrm byte, mod 01, disp8).
const INC_MODRMS: [u8; 5] = [0x40, 0x41, 0x45, 0x46, 0x47];

/// The stub of a site whose 5 bytes are `bytes`, at `stub`; None for bytes of neither form.
fn stub_code(at: usize, bytes: &[u8; 5], stub: usize) -> Option<Vec<u8>> {
    let back = at + 5;
    let mut code = Vec::with_capacity(16);
    if bytes[0] == 0x74 && bytes[1] == 0x03 && bytes[2] == 0xFF && INC_MODRMS.contains(&bytes[3]) && bytes[4] == 0x04 {
        code.extend_from_slice(&[0x74, 0x04]); // jz over the lock inc
        code.extend_from_slice(&[0xF0, 0xFF, bytes[3], 0x04]); // lock inc dword [r+4]
    } else if bytes[..4] == [0xFF, 0x49, 0x04, 0x75] {
        let taken = back.wrapping_add(bytes[4] as i8 as isize as usize);
        code.extend_from_slice(&[0xF0, 0xFF, 0x49, 0x04]); // lock dec dword [ecx+4]
        code.extend_from_slice(&[0x0F, 0x85]); // jnz taken
        let next = stub + code.len() + 4;
        code.extend_from_slice(&(taken.wrapping_sub(next) as u32).to_le_bytes());
    } else {
        return None;
    }
    code.push(0xE9); // jmp back
    let next = stub + code.len() + 4;
    code.extend_from_slice(&(back.wrapping_sub(next) as u32).to_le_bytes());
    Some(code)
}

/// Locks the count writes of all sites, or none of them.
///
/// # Safety
///
/// The game code at the sites is the Morrowind.exe of the database, and no other thread runs
/// the 5 bytes of a site while it is written (the plugin's install, before any cell loads).
pub unsafe fn install() {
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    let mut live = Vec::with_capacity(SITES.len());
    for site in &SITES {
        let bytes: [u8; 5] = core::ptr::read_unaligned(site.at as *const [u8; 5]);
        if stub_code(site.at, &bytes, 0).is_none() {
            crate::log!(
                "atomic count: 0 of {} sites locked: the bytes at 0x{:x} ({}, {}) are {:02x?}, not the game's; all sites are refused",
                SITES.len(),
                site.at,
                site.function,
                site.slot,
                bytes
            );
            return;
        }
        live.push(bytes);
    }
    let mut locked = 0;
    for (site, bytes) in SITES.iter().zip(&live) {
        let Some(stub) = crate::hook::arena_alloc(&[0xCC; 16]) else {
            crate::log!("atomic count: no memory for a stub after {locked} sites");
            break;
        };
        let code = stub_code(site.at, bytes, stub).unwrap_or_default();
        core::ptr::copy_nonoverlapping(code.as_ptr(), stub as *mut u8, code.len());
        let mut jump = [0xE9, 0, 0, 0, 0];
        jump[1..].copy_from_slice(&(stub.wrapping_sub(site.at + 5) as u32).to_le_bytes());
        if !crate::win::patch(site.at, &jump) {
            crate::log!("atomic count: 0x{:x} could not be written after {locked} sites", site.at);
            break;
        }
        locked += 1;
    }
    INSTALLED.store(locked > 0, Ordering::Relaxed);
    crate::log!("atomic count: {locked} of {} sites locked (NiPropertyState, NiPointer and the property updates)", SITES.len());
}

/// True when the run of `len` bytes at `ea` is inside one of the sites' 5 bytes, once they are
/// written: the census does not count these as a change of another mod.
pub fn covers(ea: usize, len: usize) -> bool {
    INSTALLED.load(Ordering::Relaxed) && SITES.iter().any(|s| ea >= s.at && ea + len <= s.at + 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_of_an_inc_jumps_over_a_locked_inc_and_back() {
        let code = stub_code(0x6FE43F, &[0x74, 0x03, 0xFF, 0x47, 0x04], 0x1000).unwrap();
        assert_eq!(&code[..6], &[0x74, 0x04, 0xF0, 0xFF, 0x47, 0x04]);
        assert_eq!(code[6], 0xE9);
        let back = (0x1000 + 11usize).wrapping_add(u32::from_le_bytes(code[7..11].try_into().unwrap()) as usize);
        assert_eq!(back, 0x6FE444);
    }

    #[test]
    fn the_stub_of_a_dec_keeps_both_ways_of_its_jnz() {
        let code = stub_code(0x40A886, &[0xFF, 0x49, 0x04, 0x75, 0x0A], 0x2000).unwrap();
        assert_eq!(&code[..6], &[0xF0, 0xFF, 0x49, 0x04, 0x0F, 0x85]);
        let taken = (0x2000 + 10usize).wrapping_add(u32::from_le_bytes(code[6..10].try_into().unwrap()) as usize);
        assert_eq!(taken, 0x40A895);
        assert_eq!(code[10], 0xE9);
        let back = (0x2000 + 15usize).wrapping_add(u32::from_le_bytes(code[11..15].try_into().unwrap()) as usize);
        assert_eq!(back, 0x40A88B);
    }

    #[test]
    fn other_bytes_get_no_stub() {
        assert!(stub_code(0, &[0x74, 0x03, 0xFF, 0x4F, 0x04], 0).is_none());
        assert!(stub_code(0, &[0xF0, 0xFF, 0x49, 0x04, 0x75], 0).is_none());
        assert!(stub_code(0, &[0xE9, 0x00, 0x00, 0x00, 0x00], 0).is_none());
    }

    #[test]
    fn the_sites_do_not_overlap() {
        let mut starts: Vec<usize> = SITES.iter().map(|s| s.at).collect();
        starts.sort_unstable();
        assert!(starts.windows(2).all(|w| w[1] >= w[0] + 5));
    }
}
