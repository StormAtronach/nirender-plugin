//! MWSE's reference count locks (W329, D314): the 40 count sites of the engine that MWSE locks,
//! as the census meets them.
//!
//! MWSE's `NI::RefCountLocks` (MWSE `NIRefCountLocks.cpp`) writes a jump to a locked stub over the
//! first 5 bytes of each site and fills the rest of the site with `nop`. The plugin locks no count
//! itself. Four of the sites' functions have ports (`NiGeometry::pushStateToRenderer`,
//! `NiRenderer::BeginBatch`, `NiDX8Renderer::dtor` and `NiGeometry::UpdatePropertiesDownward`),
//! and the census locks a port whose function another mod changed. So the census asks [`covers`]:
//! a run of changed bytes wholly inside one site that holds a jump and then `nop` to its end is
//! MWSE's count lock, not another mod's change. The check of an MWSE.dll before a deploy
//! (`mwse_sites`) asks [`is_site`] for the same reason.
//!
//! The list `SITES` is a copy of the addresses and lengths of MWSE's table. MWSE's install line in
//! MWSE.log names every address it locked; a test compares that line with this list in both
//! directions. The rule does not check where the jump goes: another mod's jump at a site reads as
//! MWSE's. MWSE's own check refuses all sites when one does not hold the game's bytes, and its
//! log says so.

use core::sync::atomic::{AtomicU64, Ordering};

/// One site: the address of its bytes and their number.
struct Site {
    at: usize,
    len: usize,
}

/// The sites of MWSE's `NI::RefCountLocks::sites`, in its order.
const SITES: [Site; 40] = [
    Site { at: 0x6FE39F, len: 5 },
    Site { at: 0x6FE3BF, len: 5 },
    Site { at: 0x6FE3DF, len: 5 },
    Site { at: 0x6FE3FF, len: 5 },
    Site { at: 0x6FE41F, len: 5 },
    Site { at: 0x6FE43F, len: 5 },
    Site { at: 0x6FE45F, len: 5 },
    Site { at: 0x6FE475, len: 7 },
    Site { at: 0x6FE489, len: 5 },
    Site { at: 0x6FE49F, len: 7 },
    Site { at: 0x6FE4B3, len: 5 },
    Site { at: 0x6FE2E6, len: 7 },
    Site { at: 0x6FE2FC, len: 5 },
    Site { at: 0x471B25, len: 5 },
    Site { at: 0x471B12, len: 5 },
    Site { at: 0x40A886, len: 5 },
    Site { at: 0x404630, len: 5 },
    Site { at: 0x6EB230, len: 5 },
    Site { at: 0x6EB30D, len: 15 },
    Site { at: 0x6EB333, len: 5 },
    Site { at: 0x6EB349, len: 7 },
    Site { at: 0x6C8D82, len: 5 },
    Site { at: 0x6C8D94, len: 15 },
    Site { at: 0x6C8E05, len: 7 },
    Site { at: 0x6F41F6, len: 7 },
    Site { at: 0x6F420F, len: 5 },
    Site { at: 0x6F4224, len: 15 },
    Site { at: 0x6EB125, len: 5 },
    Site { at: 0x6EB137, len: 13 },
    Site { at: 0x6EB19A, len: 7 },
    Site { at: 0x6F3F5E, len: 7 },
    Site { at: 0x6F3F6D, len: 10 },
    Site { at: 0x6F3F88, len: 7 },
    Site { at: 0x6F3F97, len: 10 },
    Site { at: 0x6F5393, len: 7 },
    Site { at: 0x6F53A2, len: 10 },
    Site { at: 0x6F53BB, len: 7 },
    Site { at: 0x6F53CA, len: 10 },
    Site { at: 0x6A9EEB, len: 7 },
    Site { at: 0x6EAC1A, len: 7 },
];

/// The sites, as bits by their index, whose other bytes the log named.
static TOLD: AtomicU64 = AtomicU64::new(0);
// TOLD has one bit for each site.
const _: () = assert!(SITES.len() <= 64);

const JMP: u8 = 0xE9;
const NOP: u8 = 0x90;

/// True when the bytes of `site`, read by `read`, are a jump and then `nop` to its end.
fn holds_lock(site: &Site, read: &impl Fn(usize) -> u8) -> bool {
    read(site.at) == JMP && (site.at + 5..site.at + site.len).all(|address| read(address) == NOP)
}

/// True when `at` is the address of one of MWSE's count lock sites.
pub fn is_site(at: usize) -> bool {
    SITES.iter().any(|site| site.at == at)
}

/// The addresses of the sites, in the order of MWSE's table.
pub fn site_addresses() -> impl Iterator<Item = usize> {
    SITES.iter().map(|site| site.at)
}

/// Writes one line on the state of the sites: all locked by MWSE, none, or some.
pub fn log_state() {
    // SAFETY: the sites are in the code of the game, which stays mapped and readable for the life
    // of the process.
    let read = |address: usize| unsafe { *(address as *const u8) };
    let locked = SITES.iter().filter(|site| holds_lock(site, &read)).count();
    match locked {
        0 => crate::log!("count locks: the count sites are not locked (MWSE's reference count locks are off or absent)"),
        n if n == SITES.len() => crate::log!("count locks: {n} of {n} count sites hold MWSE's lock (a jump, then nop)"),
        n => crate::log!(
            "count locks: {n} of {} count sites hold a jump, then nop; the others do not",
            SITES.len()
        ),
    }
}

/// True when the run of `len` bytes at `ea` is wholly inside one site that holds a jump and then
/// `nop`: the census does not count the run as a change of another mod. A run inside a site with
/// other bytes is counted, and the log names the site once.
pub fn covers(ea: usize, len: usize) -> bool {
    // SAFETY: as in `log_state`.
    let read = |address: usize| unsafe { *(address as *const u8) };
    match covering_site(ea, len, &read) {
        Covering::Lock => true,
        Covering::Outside => false,
        Covering::Other(index) => {
            if TOLD.fetch_or(1u64 << index, Ordering::Relaxed) & (1u64 << index) == 0 {
                let site = &SITES[index];
                let live: Vec<u8> = (site.at..site.at + site.len).map(read).collect();
                crate::log!(
                    "count locks: the count site at 0x{:x} holds {:02x?}, not a jump and then nop; the census counts its function as changed",
                    site.at,
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
    /// Wholly inside a site that holds a jump and then `nop`.
    Lock,
    /// Wholly inside a site with other bytes: the index of the site.
    Other(usize),
    /// Not wholly inside any site.
    Outside,
}

/// The rule of [`covers`], with the read of a byte given.
fn covering_site(ea: usize, len: usize, read: &impl Fn(usize) -> u8) -> Covering {
    let inside = SITES
        .iter()
        .enumerate()
        .find(|(_, site)| ea >= site.at && ea + len <= site.at + site.len);
    match inside {
        None => Covering::Outside,
        Some((_, site)) if holds_lock(site, read) => Covering::Lock,
        Some((index, _)) => Covering::Other(index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The memory of one site from its address, the bytes `site` there; 0xCC past it.
    fn reader(memory: &[u8], at: usize) -> impl Fn(usize) -> u8 + '_ {
        move |address| memory.get(address.wrapping_sub(at)).copied().unwrap_or(0xCC)
    }

    /// A site as MWSE writes it: a jump, then `nop`.
    fn locked(len: usize) -> Vec<u8> {
        let mut code = vec![NOP; len];
        code[..5].copy_from_slice(&[JMP, 0x12, 0x34, 0x56, 0x78]);
        code
    }

    #[test]
    fn a_run_inside_a_locked_site_is_covered() {
        let site = SITES.iter().find(|s| s.len == 15).unwrap();
        let memory = locked(site.len);
        let read = reader(&memory, site.at);
        assert_eq!(covering_site(site.at, site.len, &read), Covering::Lock);
        assert_eq!(covering_site(site.at + 1, 4, &read), Covering::Lock);
        assert_eq!(covering_site(site.at + 5, 10, &read), Covering::Lock);
    }

    #[test]
    fn a_run_past_the_end_of_a_site_is_not_covered() {
        let site = SITES.iter().find(|s| s.len == 7).unwrap();
        let memory = locked(site.len);
        let read = reader(&memory, site.at);
        assert_eq!(covering_site(site.at + 5, 3, &read), Covering::Outside);
        assert_eq!(covering_site(site.at - 1, 2, &read), Covering::Outside);
    }

    #[test]
    fn a_site_with_other_bytes_is_counted_and_named() {
        let index = SITES.iter().position(|s| s.len == 7).unwrap();
        let site = &SITES[index];
        // The game's plain release, unlocked.
        let plain = [0x8B, 0x41, 0x04, 0x48, 0x89, 0x41, 0x04];
        assert_eq!(covering_site(site.at, 1, &reader(&plain, site.at)), Covering::Other(index));
        // A jump without the nop after it.
        let mut memory = locked(site.len);
        memory[6] = 0x42;
        assert_eq!(covering_site(site.at, 5, &reader(&memory, site.at)), Covering::Other(index));
    }

    #[test]
    fn the_sites_are_mwses_forty_and_do_not_overlap() {
        assert_eq!(SITES.len(), 40);
        assert!(SITES.iter().any(|s| s.at == 0x6EAC1A && s.len == 7));
        assert!(SITES.iter().all(|s| s.len >= 5 && s.len < 16));
        let mut sites: Vec<(usize, usize)> = SITES.iter().map(|s| (s.at, s.len)).collect();
        sites.sort_unstable();
        assert!(sites.windows(2).all(|w| w[1].0 >= w[0].0 + w[0].1));
    }
}
