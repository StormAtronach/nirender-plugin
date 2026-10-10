//! The byte census: the functions of the renderer subsystem in memory against the IDA database.
//!
//! A function whose bytes differ was changed by another mod (MWSE, MGE, the Code Patch with
//! other options than the database has). The Rust port replicates the database; replacing a
//! changed function would drop the change, so each one needs a decision.

use crate::gen::funcs::{Func, FUNCS};

pub fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 0x811C_9DC5u32;
    for &b in bytes {
        hash = (hash ^ b as u32).wrapping_mul(0x0100_0193);
    }
    hash
}

/// The functions whose bytes in memory are not the bytes of the database.
pub unsafe fn changed() -> Vec<&'static Func> {
    FUNCS.iter().filter(|f| fnv1a(core::slice::from_raw_parts(f.ea as *const u8, f.size)) != f.hash).collect()
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
}
