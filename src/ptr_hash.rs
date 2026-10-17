//! A hasher for the tables keyed by an engine pointer (`instance_stream`). The standard
//! `RandomState` (SipHash) cost about 0.05 ms a frame in the lookups of the display hook and
//! the table walk (the VTune profile of the quicksave, 2026-10-17). The keys are addresses the
//! game allocates, not input an attacker chooses, so a multiply suffices; the high half of the
//! product is folded into the low half, so that the bucket index (low bits) and the control
//! byte (high bits) both mix every bit of an aligned pointer.

use core::hash::{BuildHasherDefault, Hasher};

#[derive(Default, Clone, Copy)]
pub struct PtrHasher(u64);

impl Hasher for PtrHasher {
    fn finish(&self) -> u64 {
        let h = self.0.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^ (h >> 29)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.0 = (self.0.rotate_left(32) ^ value as u64).wrapping_mul(0x100_0000_01b3);
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(32) ^ value).wrapping_mul(0x100_0000_01b3);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

pub type PtrBuild = BuildHasherDefault<PtrHasher>;
pub type HashMap<K, V> = std::collections::HashMap<K, V, PtrBuild>;
pub type HashSet<K> = std::collections::HashSet<K, PtrBuild>;

#[cfg(test)]
mod tests {
    use super::*;
    use core::hash::BuildHasher;

    #[test]
    fn aligned_pointers_spread_over_the_low_and_the_high_bits() {
        let build = PtrBuild::default();
        let mut low = std::collections::HashSet::new();
        let mut high = std::collections::HashSet::new();
        for i in 0..256usize {
            let h = build.hash_one(0x1000_0000usize + i * 16);
            low.insert(h & 0xFF);
            high.insert(h >> 57);
        }
        assert!(low.len() > 150, "low bits: {} of 256", low.len());
        assert!(high.len() > 100, "high 7 bits: {} of 128", high.len());
    }

    #[test]
    fn a_map_works() {
        let mut map: HashMap<usize, u32> = HashMap::default();
        for i in 0..1000usize {
            map.insert(i * 4, i as u32);
        }
        assert!((0..1000usize).all(|i| map.get(&(i * 4)) == Some(&(i as u32))));
    }
}
