//! Reads and writes of engine memory at an offset from a base address.
//!
//! The ports work on the engine objects themselves. The offsets come from `gen::layouts`.

/// # Safety
///
/// `base + off` is the address of `size_of::<T>()` readable bytes that hold a valid `T`. The
/// address need not be aligned.
#[inline(always)]
pub unsafe fn rd<T: Copy>(base: usize, off: usize) -> T {
    ((base + off) as *const T).read_unaligned()
}

/// # Safety
///
/// `base + off` is the address of `size_of::<T>()` writable bytes that nothing else reads or writes
/// during the call. The address need not be aligned. The old value is not dropped.
#[inline(always)]
pub unsafe fn wr<T>(base: usize, off: usize, value: T) {
    ((base + off) as *mut T).write_unaligned(value)
}

/// A float that went through `fld` and `fstp` in the game code. The x87 load makes a
/// signalling NaN quiet. Every other value comes back with the same bits.
#[inline(always)]
pub fn x87_pass(bits: u32) -> u32 {
    let is_nan = bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0;
    if is_nan {
        bits | 0x0040_0000
    } else {
        bits
    }
}

/// The FNV-1a hash of `bytes`.
pub fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 0x811C_9DC5u32;
    for &b in bytes {
        hash = (hash ^ b as u32).wrapping_mul(0x0100_0193);
    }
    hash
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
    fn x87_pass_changes_only_signalling_nans() {
        assert_eq!(x87_pass(1.5f32.to_bits()), 1.5f32.to_bits());
        assert_eq!(x87_pass(0x7F80_0000), 0x7F80_0000); // infinity
        assert_eq!(x87_pass(0x7FC0_0001), 0x7FC0_0001); // quiet NaN
        assert_eq!(x87_pass(0x7F80_0001), 0x7FC0_0001); // signalling NaN
        assert_eq!(x87_pass(0xFFA0_0000), 0xFFE0_0000);
    }

    #[test]
    fn rd_wr_round_trip() {
        let mut block = [0u8; 16];
        let base = block.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(base, 5, 0xDEAD_BEEF);
            assert_eq!(rd::<u32>(base, 5), 0xDEAD_BEEF);
            assert_eq!(rd::<u8>(base, 5), 0xEF);
        }
    }
}
