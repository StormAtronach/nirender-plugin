//! Float sequences of the game code that keep intermediate results on the x87 stack.
//!
//! The result of such a sequence depends on the precision the FPU control word selects, so
//! the port runs the same instructions instead of doing the arithmetic in SSE.

use core::arch::asm;

/// The x87 control word. Bits 8-9 are the precision: 0 = 24 bits, 2 = 53 bits, 3 = 64 bits.
pub fn control_word() -> u16 {
    let mut word: u16 = 0;
    unsafe {
        asm!("fnstcw word ptr [{p}]", p = in(reg) &mut word, options(nostack));
    }
    word
}

/// Fog start and end as `NiDX8RenderState::updateFogStates` (0x6B8A55) computes them.
/// Returns the bits of the two floats.
pub fn fog_range(density: f32, frustum_distance: f32, frustum_far: f32, inverse_max_fog: f32) -> (u32, u32) {
    let input = [density, frustum_distance, frustum_far, inverse_max_fog];
    let mut output = [0u32; 2];
    unsafe {
        asm!(
            "fld dword ptr [{i}]",
            "fmul dword ptr [{i} + 4]",
            "fld dword ptr [{i} + 8]",
            "fsub st, st(1)",
            "fstp dword ptr [{o}]",
            "fmul dword ptr [{i} + 12]",
            "fadd dword ptr [{i} + 8]",
            "fstp dword ptr [{o} + 4]",
            i = in(reg) input.as_ptr(),
            o = in(reg) output.as_mut_ptr(),
            clobber_abi("C"),
            options(nostack),
        );
    }
    (output[0], output[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fog_range_on_exact_values() {
        // 0.5 * 8 = 4; start = 100 - 4; end = 4 * 0.25 + 100
        let (start, end) = fog_range(0.5, 8.0, 100.0, 0.25);
        assert_eq!(f32::from_bits(start), 96.0);
        assert_eq!(f32::from_bits(end), 101.0);
    }

    #[test]
    fn control_word_reads() {
        assert_ne!(control_word(), 0);
    }
}
