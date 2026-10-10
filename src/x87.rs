//! Float sequences of the game code that keep intermediate results on the x87 stack.
//!
//! The result of such a sequence depends on the precision that the FPU control word selects.
//! So the port runs the same instructions and does not do the arithmetic in SSE.

use core::arch::asm;

/// The x87 control word. Bits 8 and 9 give the precision: 0 is 24 bits, 2 is 53 bits and 3
/// is 64 bits.
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

/// The operation of `_ftol` on ST(0). It stores ST(0) as an integer with the rounding mode set
/// to truncate. It gives the low 32 bits in EAX. `{s}` is a scratch block. It has the two
/// control words at +0 and +4 and the 64-bit integer at +8.
///
/// `_ftol` of the game (0x72769E) is a jump to the import from MSVCRT.DLL.
///
/// This is a copy of `ftol_asm!` of `ports::renderer_frame`. Use it inside `asm!` with a
/// register operand `s`, and give EAX as an output.
macro_rules! ftol_asm {
    () => {
        concat!(
            "fnstcw word ptr [{s}]
",
            "movzx eax, word ptr [{s}]
",
            "or eax, 0x0C00
",
            "mov word ptr [{s} + 4], ax
",
            "fldcw word ptr [{s} + 4]
",
            "fistp qword ptr [{s} + 8]
",
            "fldcw word ptr [{s}]
",
            "mov eax, dword ptr [{s} + 8]
",
        )
    };
}
#[allow(unused_imports)]
pub(crate) use ftol_asm;

/// `fild qword n; fmul dword x; call _ftol`. It multiplies an unsigned 32-bit count by a float.
/// `x` is the bits of the float.
#[allow(dead_code)]
pub fn ftol_count_times(n: u32, x: u32) -> u32 {
    let mut s = [0u32, 0, 0, 0, n, 0, x];
    let result: u32;
    unsafe {
        asm!(
            "fild qword ptr [{s} + 16]",
            "fmul dword ptr [{s} + 24]",
            ftol_asm!(),
            s = in(reg) s.as_mut_ptr(),
            out("eax") result,
            clobber_abi("C"),
            options(nostack),
        );
    }
    result
}

/// `fld a; fsub b; fimul dword n; call _ftol`. `fimul` reads `n` as a signed integer. `a` and
/// `b` are the bits of the floats.
#[allow(dead_code)]
pub fn ftol_difference_times_int(a: u32, b: u32, n: u32) -> u32 {
    let mut s = [0u32, 0, 0, 0, a, b, n];
    let result: u32;
    unsafe {
        asm!(
            "fld dword ptr [{s} + 16]",
            "fsub dword ptr [{s} + 20]",
            "fimul dword ptr [{s} + 24]",
            ftol_asm!(),
            s = in(reg) s.as_mut_ptr(),
            out("eax") result,
            clobber_abi("C"),
            options(nostack),
        );
    }
    result
}

/// `fld a; fmul b; call _ftol`. `a` and `b` are the bits of the floats.
#[allow(dead_code)]
pub fn ftol_product(a: u32, b: u32) -> u32 {
    let mut s = [0u32, 0, 0, 0, a, b];
    let result: u32;
    unsafe {
        asm!(
            "fld dword ptr [{s} + 16]",
            "fmul dword ptr [{s} + 20]",
            ftol_asm!(),
            s = in(reg) s.as_mut_ptr(),
            out("eax") result,
            clobber_abi("C"),
            options(nostack),
        );
    }
    result
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
    fn ftol_truncates() {
        let f255 = 255.0f32.to_bits();
        // The rounding mode of the test process is "nearest". `_ftol` truncates.
        assert_eq!(ftol_product(2.75f32.to_bits(), 1.0f32.to_bits()), 2);
        assert_eq!(ftol_product((-2.75f32).to_bits(), 1.0f32.to_bits()) as i32, -2);
        assert_eq!(ftol_product(0.999f32.to_bits(), f255), 254);
        assert_eq!(ftol_count_times(800, 0.25f32.to_bits()), 200);
        assert_eq!(ftol_count_times(3, 0.5f32.to_bits()), 1);
        // The count is unsigned.
        assert_eq!(ftol_count_times(0x8000_0000, 1.0f32.to_bits()), 0x8000_0000);
        assert_eq!(ftol_difference_times_int(1.0f32.to_bits(), 0.75f32.to_bits(), 600), 150);
        assert_eq!(ftol_difference_times_int(0.0f32.to_bits(), 0.5f32.to_bits(), 3) as i32, -1);
        // `_ftol` puts the control word back.
        let before = control_word();
        ftol_product(1.5f32.to_bits(), 1.0f32.to_bits());
        assert_eq!(control_word(), before);
    }

    #[test]
    fn control_word_reads() {
        assert_ne!(control_word(), 0);
    }
}
