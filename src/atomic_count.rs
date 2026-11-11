//! Locked count writes of `NiRefObject` (risk 56): the increments and the `NiPointer` releases
//! that the ledger saw on the property paths, `NiObject::release`, the inline releases of the
//! same functions, and the changes of the renderer's current states.
//!
//! The main thread and the cell loader thread (`game_threadLoadCells`) both run
//! `NiPropertyState::reset`, `NiPropertyState::ctor_copy`, `NiPointer::set`, `NiPointer::dtor`
//! and the property updates of the scene graph. Each of them changes a count with an unlocked
//! `inc` or `dec`, and the two threads write the count of a shared property in the same
//! microseconds. A lost change makes the count lower than the number of holders: the property
//! is freed while holders remain, and their releases at exit damage the heap. This module puts
//! a `lock` on each of these writes. `NiObject::release` (0x404630) is the same release as
//! `NiPointer::dtor`, and `NiPropertyState::reset` calls it for 7 of its slots.
//!
//! `NiGeometry::pushStateToRenderer` (0x6F3F40) and `NiRenderer::BeginBatch` (0x6F5380) make a
//! shape's states the renderer's current ones, each with the same plain releases and
//! increments; `NiDX8Renderer::dtor` releases the current property state.
//!
//! Each site is one of five forms, and its first 5 bytes become a jump to a stub:
//! - `jz +3; inc dword [r+4]` (74 03 FF modrm 04), 5 bytes: the stub is
//!   `jz back; lock inc dword [r+4]; back: jmp site+5`.
//! - `test r, r; mov [b+d], r; jz +3; inc dword [r+4]`, 10 bytes: the 3 bytes after the `inc`
//!   are a branch target, so the site starts at the `test`. The stub is the `test` and the
//!   `mov` as they are, then the first form, with `jmp site+10`.
//! - `dec dword [ecx+4]; jnz +n` (FF 49 04 75 n), 5 bytes: the stub is
//!   `lock dec dword [ecx+4]; jnz site+5+n; jmp site+5`.
//! - The plain inline release `mov eax, [r+4]; dec eax; mov [r+4], eax`, 7 bytes before a
//!   `jnz`: the stub is `mov eax, -1; lock xadd [r+4], eax; dec eax; jmp site+7`.
//! - The interleaved inline release `mov c, [eax+4]; mov ecx, eax; add eax, 4; dec c;
//!   (mov edx, c); test edx, edx; mov [eax], c`, 13 or 15 bytes before a `jnz`: the stub is
//!   `mov ecx, eax; mov c, -1; lock xadd [eax+4], c; dec c; add eax, 4; (mov edx, c);
//!   test edx, edx; jmp site+len`.
//!
//! The stub leaves the registers, the count and the flags as the original code does, with one
//! difference: after the plain form, CF is the carry of the `xadd` and not the CF from before.
//! The code after the `jnz` of each plain site writes CF or calls before it reads CF. No branch
//! of the game lands inside a site after its first byte \[V IDA, 2026-11-04\]. The bytes of a
//! site after its jump are filled with `nop`. All sites are checked before the first is
//! written: one byte that differs refuses all of them.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// One site: the address of its bytes, their number, the function, and what it counts.
struct Site {
    at: usize,
    len: usize,
    function: &'static str,
    slot: &'static str,
}

const SITES: [Site; 39] = [
    Site {
        at: 0x6FE39F,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default alpha",
    },
    Site {
        at: 0x6FE3BF,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default fog",
    },
    Site {
        at: 0x6FE3DF,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default material",
    },
    Site {
        at: 0x6FE3FF,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default stencil",
    },
    Site {
        at: 0x6FE41F,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default texturing",
    },
    Site {
        at: 0x6FE43F,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default vertex colour",
    },
    Site {
        at: 0x6FE45F,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default wireframe",
    },
    Site {
        at: 0x6FE475,
        len: 7,
        function: "NiPropertyState::reset",
        slot: "the old zbuffer (release)",
    },
    Site {
        at: 0x6FE489,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default zbuffer",
    },
    Site {
        at: 0x6FE49F,
        len: 7,
        function: "NiPropertyState::reset",
        slot: "the old renderer specific (release)",
    },
    Site {
        at: 0x6FE4B3,
        len: 5,
        function: "NiPropertyState::reset",
        slot: "default renderer specific",
    },
    Site {
        at: 0x6FE2E6,
        len: 7,
        function: "NiPropertyState::ctor_copy",
        slot: "the old property (release)",
    },
    Site {
        at: 0x6FE2FC,
        len: 5,
        function: "NiPropertyState::ctor_copy",
        slot: "each property of the copied state",
    },
    Site {
        at: 0x471B25,
        len: 5,
        function: "NiPointer::set",
        slot: "the new object",
    },
    Site {
        at: 0x471B12,
        len: 5,
        function: "NiPointer::set",
        slot: "the old object (release)",
    },
    Site {
        at: 0x40A886,
        len: 5,
        function: "NiPointer::dtor",
        slot: "the object (release)",
    },
    Site {
        at: 0x404630,
        len: 5,
        function: "NiObject::release",
        slot: "the object (release)",
    },
    Site {
        at: 0x6EB230,
        len: 5,
        function: "NiAVObject::PushLocalProperties",
        slot: "the parent state",
    },
    Site {
        at: 0x6EB30D,
        len: 15,
        function: "NiAVObject::PushLocalProperties",
        slot: "the parent state (release)",
    },
    Site {
        at: 0x6EB333,
        len: 5,
        function: "NiAVObject::PushLocalProperties",
        slot: "the new state",
    },
    Site {
        at: 0x6EB349,
        len: 7,
        function: "NiAVObject::PushLocalProperties",
        slot: "the new state (release)",
    },
    Site {
        at: 0x6C8D82,
        len: 5,
        function: "NiNode::UpdateProperties",
        slot: "the state",
    },
    Site {
        at: 0x6C8D94,
        len: 15,
        function: "NiNode::UpdateProperties",
        slot: "the local state (release)",
    },
    Site {
        at: 0x6C8E05,
        len: 7,
        function: "NiNode::UpdateProperties",
        slot: "the state (release)",
    },
    Site {
        at: 0x6F41F6,
        len: 7,
        function: "NiGeometry::UpdatePropertiesDownward",
        slot: "the geometry's old state (release)",
    },
    Site {
        at: 0x6F420F,
        len: 5,
        function: "NiGeometry::UpdatePropertiesDownward",
        slot: "the geometry's state",
    },
    Site {
        at: 0x6F4224,
        len: 15,
        function: "NiGeometry::UpdatePropertiesDownward",
        slot: "the state argument (release)",
    },
    Site {
        at: 0x6EB125,
        len: 5,
        function: "NiAVObject::UpdateProperties",
        slot: "the state",
    },
    Site {
        at: 0x6EB137,
        len: 13,
        function: "NiAVObject::UpdateProperties",
        slot: "the local state (release)",
    },
    Site {
        at: 0x6EB19A,
        len: 7,
        function: "NiAVObject::UpdateProperties",
        slot: "the state (release)",
    },
    Site {
        at: 0x6F3F5E,
        len: 7,
        function: "NiGeometry::pushStateToRenderer",
        slot: "the old current property state (release)",
    },
    Site {
        at: 0x6F3F6D,
        len: 10,
        function: "NiGeometry::pushStateToRenderer",
        slot: "the shape's property state",
    },
    Site {
        at: 0x6F3F88,
        len: 7,
        function: "NiGeometry::pushStateToRenderer",
        slot: "the old current effect state (release)",
    },
    Site {
        at: 0x6F3F97,
        len: 10,
        function: "NiGeometry::pushStateToRenderer",
        slot: "the shape's effect state",
    },
    Site {
        at: 0x6F5393,
        len: 7,
        function: "NiRenderer::BeginBatch",
        slot: "the old current property state (release)",
    },
    Site {
        at: 0x6F53A2,
        len: 10,
        function: "NiRenderer::BeginBatch",
        slot: "the batch's property state",
    },
    Site {
        at: 0x6F53BB,
        len: 7,
        function: "NiRenderer::BeginBatch",
        slot: "the old current effect state (release)",
    },
    Site {
        at: 0x6F53CA,
        len: 10,
        function: "NiRenderer::BeginBatch",
        slot: "the batch's effect state",
    },
    Site {
        at: 0x6A9EEB,
        len: 7,
        function: "NiDX8Renderer::dtor",
        slot: "the current property state (release)",
    },
];

/// The most bytes of a site, and one more for the `jnz` after an inline release.
const READ_LEN: usize = 16;

/// The most bytes of a stub (the interleaved form is 25).
const STUB_LEN: usize = 32;

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// The stub of each site that the install wrote, by its index in [`SITES`]; 0 for a site not
/// written. The bytes that the install wrote are `site_code` of the site and its stub.
static STUBS: [AtomicUsize; SITES.len()] = [const { AtomicUsize::new(0) }; SITES.len()];

/// The sites, as bits by their index, whose foreign bytes the log named.
static TOLD: AtomicU64 = AtomicU64::new(0);
// TOLD has one bit for each site.
const _: () = assert!(SITES.len() <= 64);

/// The registers that an `inc dword [r+4]` of a site uses (the modrm byte, mod 01, disp8).
const INC_MODRMS: [u8; 5] = [0x40, 0x41, 0x45, 0x46, 0x47];

/// The modrm bytes of `[r+4]` with reg eax in a plain inline release (r is ecx, ebp or esi).
const PLAIN_MODRMS: [u8; 3] = [0x41, 0x45, 0x46];

/// The registers that hold the count in an interleaved inline release: edx, esi and edi.
const INTERLEAVED_REGS: [u8; 3] = [2, 6, 7];

const EDX: u8 = 2;

/// The registers that hold the state in a store before an increment: esi and edi.
const STORE_REGS: [u8; 2] = [6, 7];

/// The length of the site at the start of `bytes` and its stub at `stub`, for a site at `at`;
/// None for bytes of no form.
fn stub_code(at: usize, bytes: &[u8; READ_LEN], stub: usize) -> Option<(usize, Vec<u8>)> {
    let mut code = Vec::with_capacity(STUB_LEN);
    let len = if let Some(len) = inc_or_dec(at, bytes, stub, &mut code) {
        len
    } else if let Some(len) = plain_release(bytes, &mut code) {
        len
    } else if let Some(len) = store_then_inc(bytes, &mut code) {
        len
    } else {
        interleaved_release(bytes, &mut code)?
    };
    code.push(0xE9); // jmp back
    let next = stub + code.len() + 4;
    code.extend_from_slice(&((at + len).wrapping_sub(next) as u32).to_le_bytes());
    Some((len, code))
}

/// The 5-byte forms: `jz +3; inc dword [r+4]` and `dec dword [ecx+4]; jnz +n`.
fn inc_or_dec(at: usize, bytes: &[u8; READ_LEN], stub: usize, code: &mut Vec<u8>) -> Option<usize> {
    if bytes[0] == 0x74 && bytes[1] == 0x03 && bytes[2] == 0xFF && INC_MODRMS.contains(&bytes[3]) && bytes[4] == 0x04 {
        code.extend_from_slice(&[0x74, 0x04]); // jz over the lock inc
        code.extend_from_slice(&[0xF0, 0xFF, bytes[3], 0x04]); // lock inc dword [r+4]
        return Some(5);
    }
    if bytes[..4] == [0xFF, 0x49, 0x04, 0x75] {
        let taken = (at + 5).wrapping_add(bytes[4] as i8 as isize as usize);
        code.extend_from_slice(&[0xF0, 0xFF, 0x49, 0x04]); // lock dec dword [ecx+4]
        code.extend_from_slice(&[0x0F, 0x85]); // jnz taken
        let next = stub + code.len() + 4;
        code.extend_from_slice(&(taken.wrapping_sub(next) as u32).to_le_bytes());
        return Some(5);
    }
    None
}

/// `mov eax, [r+4]; dec eax; mov [r+4], eax; jnz`: 7 bytes before the `jnz`.
fn plain_release(bytes: &[u8; READ_LEN], code: &mut Vec<u8>) -> Option<usize> {
    let modrm = bytes[1];
    let form = [0x8B, modrm, 0x04, 0x48, 0x89, modrm, 0x04, 0x75];
    if !PLAIN_MODRMS.contains(&modrm) || bytes[..8] != form {
        return None;
    }
    code.extend_from_slice(&[0xB8, 0xFF, 0xFF, 0xFF, 0xFF]); // mov eax, -1
    code.extend_from_slice(&[0xF0, 0x0F, 0xC1, modrm, 0x04]); // lock xadd [r+4], eax
    code.push(0x48); // dec eax
    Some(7)
}

/// `test r, r; mov [b+d8], r; jz +3; inc dword [r+4]`: 10 bytes. r is esi or edi.
fn store_then_inc(bytes: &[u8; READ_LEN], code: &mut Vec<u8>) -> Option<usize> {
    let r = bytes[1] & 7;
    let store = bytes[3];
    let form = bytes[0] == 0x85
        && STORE_REGS.contains(&r)
        && bytes[1] == 0xC0 | (r << 3) | r
        && bytes[2] == 0x89
        && store >> 6 == 1 // mod 01: [b+d8]
        && (store >> 3) & 7 == r
        && store & 7 != 4 // no SIB byte
        && bytes[5..10] == [0x74, 0x03, 0xFF, 0x40 | r, 0x04];
    if !form {
        return None;
    }
    code.extend_from_slice(&bytes[..5]); // test r, r; mov [b+d8], r
    code.extend_from_slice(&[0x74, 0x04]); // jz over the lock inc
    code.extend_from_slice(&[0xF0, 0xFF, 0x40 | r, 0x04]); // lock inc dword [r+4]
    Some(10)
}

/// `mov c, [eax+4]; mov ecx, eax; add eax, 4; dec c; (mov edx, c); test edx, edx; mov [eax], c;
/// jnz`: 15 bytes before the `jnz`, or 13 when c is edx and the `mov edx, c` is absent.
fn interleaved_release(bytes: &[u8; READ_LEN], code: &mut Vec<u8>) -> Option<usize> {
    let reg = (bytes[1] >> 3) & 7;
    if !INTERLEAVED_REGS.contains(&reg) {
        return None;
    }
    let mut form = vec![0x8B, 0x40 | (reg << 3), 0x04, 0x8B, 0xC8, 0x83, 0xC0, 0x04, 0x48 + reg];
    if reg != EDX {
        form.extend_from_slice(&[0x8B, 0xD0 + reg]);
    }
    form.extend_from_slice(&[0x85, 0xD2, 0x89, reg << 3, 0x75]);
    if bytes[..form.len()] != form[..] {
        return None;
    }
    code.extend_from_slice(&[0x8B, 0xC8]); // mov ecx, eax
    code.extend_from_slice(&[0xB8 + reg, 0xFF, 0xFF, 0xFF, 0xFF]); // mov c, -1
    code.extend_from_slice(&[0xF0, 0x0F, 0xC1, 0x40 | (reg << 3), 0x04]); // lock xadd [eax+4], c
    code.push(0x48 + reg); // dec c
    code.extend_from_slice(&[0x83, 0xC0, 0x04]); // add eax, 4
    if reg != EDX {
        code.extend_from_slice(&[0x8B, 0xD0 + reg]); // mov edx, c
    }
    code.extend_from_slice(&[0x85, 0xD2]); // test edx, edx
    Some(form.len() - 1)
}

/// The bytes that replace a site of `len` bytes at `at`: a jump to `stub`, then `nop`.
fn site_code(at: usize, len: usize, stub: usize) -> Vec<u8> {
    let mut code = vec![0x90; len];
    code[0] = 0xE9;
    code[1..5].copy_from_slice(&(stub.wrapping_sub(at + 5) as u32).to_le_bytes());
    code
}

/// Locks the count writes of all sites, or none of them.
///
/// # Safety
///
/// The game code at the sites is the Morrowind.exe of the database, and no other thread runs
/// the bytes of a site while it is written (the plugin's install, before any cell loads).
pub unsafe fn install() {
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    let mut live = Vec::with_capacity(SITES.len());
    for site in &SITES {
        let bytes: [u8; READ_LEN] = core::ptr::read_unaligned(site.at as *const [u8; READ_LEN]);
        if stub_code(site.at, &bytes, 0).map(|(len, _)| len) != Some(site.len) {
            crate::log!(
                "atomic count: 0 of {} sites locked: the bytes at 0x{:x} ({}, {}) are {:02x?}, not the game's; all sites are refused",
                SITES.len(),
                site.at,
                site.function,
                site.slot,
                &bytes[..site.len]
            );
            return;
        }
        live.push(bytes);
    }
    let mut locked = 0;
    for ((site, bytes), written) in SITES.iter().zip(&live).zip(&STUBS) {
        let Some(stub) = crate::hook::arena_alloc(&[0xCC; STUB_LEN]) else {
            crate::log!("atomic count: no memory for a stub after {locked} sites");
            break;
        };
        let (_, code) = stub_code(site.at, bytes, stub).unwrap_or_default();
        core::ptr::copy_nonoverlapping(code.as_ptr(), stub as *mut u8, code.len());
        if !crate::win::patch(site.at, &site_code(site.at, site.len, stub)) {
            crate::log!("atomic count: 0x{:x} could not be written after {locked} sites", site.at);
            break;
        }
        written.store(stub, Ordering::Relaxed);
        locked += 1;
    }
    INSTALLED.store(locked > 0, Ordering::Relaxed);
    crate::log!(
        "atomic count: {locked} of {} sites locked (NiPropertyState, NiPointer, NiObject::release, the property updates and their inline releases, the current states)",
        SITES.len()
    );
}

/// True when the run of `len` bytes at `ea` is inside one site that the install wrote, and the
/// site holds the bytes that the install wrote there: the census does not count the run as a
/// change of another mod. A written site with other bytes (a mod wrote over the plugin's jump)
/// is not covered, and the log names it once.
pub fn covers(ea: usize, len: usize) -> bool {
    let stubs: [usize; SITES.len()] = core::array::from_fn(|i| STUBS[i].load(Ordering::Relaxed));
    // SAFETY: a site with a stub is in the code of the game, which stays mapped and readable for
    // the life of the process.
    let read = |address: usize| unsafe { *(address as *const u8) };
    match covering_site(&stubs, ea, len, read) {
        Covering::Own => true,
        Covering::Outside => false,
        Covering::Foreign(index) => {
            if TOLD.fetch_or(1u64 << index, Ordering::Relaxed) & (1u64 << index) == 0 {
                let site = &SITES[index];
                let live: Vec<u8> = (site.at..site.at + site.len).map(read).collect();
                crate::log!(
                    "atomic count: the site at 0x{:x} ({}, {}) holds {:02x?}, not the plugin's jump: another mod wrote it; the census counts its function as changed",
                    site.at,
                    site.function,
                    site.slot,
                    live
                );
            }
            false
        }
    }
}

/// What a run of changed bytes is to the sites.
#[derive(Debug, PartialEq, Eq)]
enum Covering {
    /// Wholly inside a written site that holds the plugin's bytes.
    Own,
    /// Wholly inside a written site, whose bytes are not the plugin's: the index of the site.
    Foreign(usize),
    /// Not wholly inside any written site.
    Outside,
}

/// The rule of [`covers`], with the stubs of the sites (0 for a site not written) and the read of
/// a byte given.
fn covering_site(stubs: &[usize; SITES.len()], ea: usize, len: usize, read: impl Fn(usize) -> u8) -> Covering {
    let inside = SITES
        .iter()
        .zip(stubs)
        .enumerate()
        .find(|(_, (site, &stub))| stub != 0 && ea >= site.at && ea + len <= site.at + site.len);
    let Some((index, (site, &stub))) = inside else {
        return Covering::Outside;
    };
    let own = site_code(site.at, site.len, stub);
    if own.iter().enumerate().all(|(i, &byte)| read(site.at + i) == byte) {
        Covering::Own
    } else {
        Covering::Foreign(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 16 bytes read at a site: `head`, then `nop`.
    fn read(head: &[u8]) -> [u8; READ_LEN] {
        let mut bytes = [0x90; READ_LEN];
        bytes[..head.len()].copy_from_slice(head);
        bytes
    }

    /// The target of the rel32 that ends at `end` of a stub at `stub`.
    fn target(code: &[u8], stub: usize, end: usize) -> usize {
        (stub + end).wrapping_add(u32::from_le_bytes(code[end - 4..end].try_into().unwrap()) as usize)
    }

    #[test]
    fn the_stub_of_an_inc_jumps_over_a_locked_inc_and_back() {
        let (len, code) = stub_code(0x6FE43F, &read(&[0x74, 0x03, 0xFF, 0x47, 0x04]), 0x1000).unwrap();
        assert_eq!(len, 5);
        assert_eq!(&code[..6], &[0x74, 0x04, 0xF0, 0xFF, 0x47, 0x04]);
        assert_eq!(code[6], 0xE9);
        assert_eq!(target(&code, 0x1000, 11), 0x6FE444);
    }

    #[test]
    fn the_stub_of_a_dec_keeps_both_ways_of_its_jnz() {
        let (len, code) = stub_code(0x40A886, &read(&[0xFF, 0x49, 0x04, 0x75, 0x0A]), 0x2000).unwrap();
        assert_eq!(len, 5);
        assert_eq!(&code[..6], &[0xF0, 0xFF, 0x49, 0x04, 0x0F, 0x85]);
        assert_eq!(target(&code, 0x2000, 10), 0x40A895);
        assert_eq!(code[10], 0xE9);
        assert_eq!(target(&code, 0x2000, 15), 0x40A88B);
    }

    #[test]
    fn the_stub_of_object_release_keeps_both_ways_of_its_jnz() {
        let (_, code) = stub_code(0x404630, &read(&[0xFF, 0x49, 0x04, 0x75, 0x0A]), 0x3000).unwrap();
        assert_eq!(target(&code, 0x3000, 10), 0x40463F);
        assert_eq!(target(&code, 0x3000, 15), 0x404635);
    }

    #[test]
    fn the_stub_of_a_plain_release_is_a_locked_xadd_and_goes_back_to_the_jnz() {
        // 0x6EB349, NiAVObject::PushLocalProperties: mov eax, [ebp+4]; dec eax; mov [ebp+4], eax; jnz +9
        let site = [0x8B, 0x45, 0x04, 0x48, 0x89, 0x45, 0x04, 0x75, 0x09];
        let (len, code) = stub_code(0x6EB349, &read(&site), 0x4000).unwrap();
        assert_eq!(len, 7);
        assert_eq!(
            &code[..11],
            &[0xB8, 0xFF, 0xFF, 0xFF, 0xFF, 0xF0, 0x0F, 0xC1, 0x45, 0x04, 0x48]
        );
        assert_eq!(code[11], 0xE9);
        assert_eq!(code.len(), 16);
        assert_eq!(target(&code, 0x4000, 16), 0x6EB350);
    }

    #[test]
    fn the_stub_of_an_interleaved_release_keeps_its_registers_and_its_test() {
        // 0x6F4224, NiGeometry::UpdatePropertiesDownward, the count in esi
        let site = [
            0x8B, 0x70, 0x04, 0x8B, 0xC8, 0x83, 0xC0, 0x04, 0x4E, 0x8B, 0xD6, 0x85, 0xD2, 0x89, 0x30, 0x75,
        ];
        let (len, code) = stub_code(0x6F4224, &read(&site), 0x5000).unwrap();
        assert_eq!(len, 15);
        let order = [
            0x8B, 0xC8, // mov ecx, eax
            0xBE, 0xFF, 0xFF, 0xFF, 0xFF, // mov esi, -1
            0xF0, 0x0F, 0xC1, 0x70, 0x04, // lock xadd [eax+4], esi
            0x4E, // dec esi
            0x83, 0xC0, 0x04, // add eax, 4
            0x8B, 0xD6, // mov edx, esi
            0x85, 0xD2, // test edx, edx
            0xE9,
        ];
        assert_eq!(&code[..order.len()], &order);
        assert_eq!(code.len(), 25);
        assert_eq!(target(&code, 0x5000, 25), 0x6F4233);
    }

    #[test]
    fn the_interleaved_release_with_the_count_in_edx_has_no_move_to_edx() {
        // 0x6EB137, NiAVObject::UpdateProperties
        let site = [
            0x8B, 0x50, 0x04, 0x8B, 0xC8, 0x83, 0xC0, 0x04, 0x4A, 0x85, 0xD2, 0x89, 0x10, 0x75,
        ];
        let (len, code) = stub_code(0x6EB137, &read(&site), 0x6000).unwrap();
        assert_eq!(len, 13);
        let order = [
            0x8B, 0xC8, // mov ecx, eax
            0xBA, 0xFF, 0xFF, 0xFF, 0xFF, // mov edx, -1
            0xF0, 0x0F, 0xC1, 0x50, 0x04, // lock xadd [eax+4], edx
            0x4A, // dec edx
            0x83, 0xC0, 0x04, // add eax, 4
            0x85, 0xD2, // test edx, edx
            0xE9,
        ];
        assert_eq!(&code[..order.len()], &order);
        assert_eq!(target(&code, 0x6000, code.len()), 0x6EB144);
    }

    #[test]
    fn the_interleaved_release_with_the_count_in_edi() {
        // 0x6C8D94, NiNode::UpdateProperties
        let site = [
            0x8B, 0x78, 0x04, 0x8B, 0xC8, 0x83, 0xC0, 0x04, 0x4F, 0x8B, 0xD7, 0x85, 0xD2, 0x89, 0x38, 0x75,
        ];
        let (len, code) = stub_code(0x6C8D94, &read(&site), 0x7000).unwrap();
        assert_eq!(len, 15);
        assert_eq!(
            &code[2..13],
            &[0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF0, 0x0F, 0xC1, 0x78, 0x04, 0x4F]
        );
        assert_eq!(&code[16..18], &[0x8B, 0xD7]);
    }

    #[test]
    fn the_stub_of_a_store_then_inc_keeps_the_store_and_locks_the_inc() {
        // 0x6F3F6D, NiGeometry::pushStateToRenderer: test esi, esi; mov [ebx+0Ch], esi; jz +3; inc dword [esi+4]
        let site = [0x85, 0xF6, 0x89, 0x73, 0x0C, 0x74, 0x03, 0xFF, 0x46, 0x04];
        let (len, code) = stub_code(0x6F3F6D, &read(&site), 0x8000).unwrap();
        assert_eq!(len, 10);
        assert_eq!(
            &code[..11],
            &[0x85, 0xF6, 0x89, 0x73, 0x0C, 0x74, 0x04, 0xF0, 0xFF, 0x46, 0x04]
        );
        assert_eq!(code[11], 0xE9);
        assert_eq!(target(&code, 0x8000, 16), 0x6F3F77);
    }

    #[test]
    fn the_store_then_inc_of_begin_batch_uses_edi_and_esi() {
        // 0x6F53CA, NiRenderer::BeginBatch: test edi, edi; mov [esi+10h], edi; jz +3; inc dword [edi+4]
        let site = [0x85, 0xFF, 0x89, 0x7E, 0x10, 0x74, 0x03, 0xFF, 0x47, 0x04];
        let (len, code) = stub_code(0x6F53CA, &read(&site), 0x9000).unwrap();
        assert_eq!(len, 10);
        assert_eq!(&code[5..11], &[0x74, 0x04, 0xF0, 0xFF, 0x47, 0x04]);
        assert_eq!(target(&code, 0x9000, 16), 0x6F53D4);
    }

    #[test]
    fn a_store_then_inc_of_another_register_gets_no_stub() {
        // The test of esi, the increment of edi.
        assert!(stub_code(0, &read(&[0x85, 0xF6, 0x89, 0x73, 0x0C, 0x74, 0x03, 0xFF, 0x47, 0x04]), 0).is_none());
        // A store with a SIB byte.
        assert!(stub_code(0, &read(&[0x85, 0xF6, 0x89, 0x74, 0x0C, 0x74, 0x03, 0xFF, 0x46, 0x04]), 0).is_none());
    }

    #[test]
    fn a_site_is_a_jump_then_nop() {
        let code = site_code(0x6F4224, 15, 0x5000);
        assert_eq!(code.len(), 15);
        assert_eq!(code[0], 0xE9);
        assert_eq!(
            (0x6F4224usize + 5).wrapping_add(u32::from_le_bytes(code[1..5].try_into().unwrap()) as usize),
            0x5000
        );
        assert!(code[5..].iter().all(|&b| b == 0x90));
    }

    #[test]
    fn other_bytes_get_no_stub() {
        assert!(stub_code(0, &read(&[0x74, 0x03, 0xFF, 0x4F, 0x04]), 0).is_none());
        assert!(stub_code(0, &read(&[0xF0, 0xFF, 0x49, 0x04, 0x75]), 0).is_none());
        assert!(stub_code(0, &read(&[0xE9, 0x00, 0x00, 0x00, 0x00]), 0).is_none());
        // A plain release without the jnz after it, and one through eax.
        assert!(stub_code(0, &read(&[0x8B, 0x41, 0x04, 0x48, 0x89, 0x41, 0x04, 0x74]), 0).is_none());
        assert!(stub_code(0, &read(&[0x8B, 0x40, 0x04, 0x48, 0x89, 0x40, 0x04, 0x75]), 0).is_none());
        // An interleaved release that stores another register.
        let site = [
            0x8B, 0x70, 0x04, 0x8B, 0xC8, 0x83, 0xC0, 0x04, 0x4E, 0x8B, 0xD6, 0x85, 0xD2, 0x89, 0x38, 0x75,
        ];
        assert!(stub_code(0, &read(&site), 0).is_none());
    }

    #[test]
    fn the_stubs_fit_their_room() {
        let site = [
            0x8B, 0x70, 0x04, 0x8B, 0xC8, 0x83, 0xC0, 0x04, 0x4E, 0x8B, 0xD6, 0x85, 0xD2, 0x89, 0x30, 0x75,
        ];
        assert!(stub_code(0, &read(&site), 0).unwrap().1.len() <= STUB_LEN);
        assert!(SITES.iter().all(|s| s.len >= 5 && s.len < READ_LEN));
    }

    #[test]
    fn the_sites_do_not_overlap() {
        let mut sites: Vec<(usize, usize)> = SITES.iter().map(|s| (s.at, s.len)).collect();
        sites.sort_unstable();
        assert!(sites.windows(2).all(|w| w[1].0 >= w[0].0 + w[0].1));
    }

    /// A census after the lock (W127): a run inside a written site that holds the plugin's jump
    /// is covered; the same run after one byte of the site changed is not, and names the site; a
    /// run that crosses the end of a site, or lies in a site not written, is not.
    #[test]
    fn a_site_is_covered_only_with_the_plugins_own_bytes() {
        let index = SITES.iter().position(|s| s.len == 7).unwrap();
        let site = &SITES[index];
        let stub = 0x1000_0000;
        let mut stubs = [0usize; SITES.len()];
        stubs[index] = stub;
        // The memory of the site, from its address; 0xCC past it.
        fn reader(memory: &[u8], at: usize) -> impl Fn(usize) -> u8 + '_ {
            move |address| memory.get(address.wrapping_sub(at)).copied().unwrap_or(0xCC)
        }
        let mut memory = site_code(site.at, site.len, stub);

        assert_eq!(
            covering_site(&stubs, site.at, site.len, reader(&memory, site.at)),
            Covering::Own
        );
        assert_eq!(covering_site(&stubs, site.at + 5, 2, reader(&memory, site.at)), Covering::Own);
        assert_eq!(
            covering_site(&stubs, site.at + 5, 3, reader(&memory, site.at)),
            Covering::Outside
        );
        assert_eq!(
            covering_site(&[0; SITES.len()], site.at, site.len, reader(&memory, site.at)),
            Covering::Outside
        );

        memory[6] = 0x42;
        assert_eq!(
            covering_site(&stubs, site.at + 6, 1, reader(&memory, site.at)),
            Covering::Foreign(index)
        );
        assert_eq!(
            covering_site(&stubs, site.at, 1, reader(&memory, site.at)),
            Covering::Foreign(index)
        );
    }
}
