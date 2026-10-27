//! The depth convention of the world view, and the one decode of a depth value for every
//! shader that reads a depth: the merge of the first person, the resolve of a multisampled
//! depth, and in MGE XE the depth frame of the post chain, the water, the blend and the
//! caustics.
//!
//! A projection with the layout of Direct3D gives the depth `d = a + b / z` for a distance `z`
//! along the view, with `a` and `b` its places 10 and 14: the depth terms ([`terms`]). With
//! a float depth the depth is reversed ([`reversed`]): near is 1 and far is 0 (`d' = 1 - d`),
//! which spreads the float's precision over the whole view (docs: notes/depth/design.md,
//! option B). The reversal is made where a projection goes to the GPU
//! ([`gpu_projection`]); the CPU keeps the projection of the game as it is. Each pipeline
//! that tests the depth of the world view takes its compare from [`compare`], each clear of
//! that depth its value from [`clear`]. A target of its own with its own views (the shadow
//! map) keeps the plain convention.

use std::sync::atomic::{AtomicBool, Ordering};

static REVERSED: AtomicBool = AtomicBool::new(false);

/// Sets the convention once, when the renderer makes its device: reversed with a float depth.
pub fn set_reversed(on: bool) {
    REVERSED.store(on, Ordering::Relaxed);
}

/// True when the depth of the world view is reversed: near 1, far 0.
pub fn reversed() -> bool {
    REVERSED.load(Ordering::Relaxed)
}

/// A projection with the layout of Direct3D as the GPU takes it: the depth column (places 2,
/// 6, 10, 14) becomes w less depth (places 3, 7, 11, 15) when the depth is reversed. Right for
/// a perspective and for an orthographic projection.
pub fn gpu_projection(projection: &[f32; 16]) -> [f32; 16] {
    let mut p = *projection;
    if reversed() {
        for row in 0..4 {
            p[row * 4 + 2] = projection[row * 4 + 3] - projection[row * 4 + 2];
        }
    }
    p
}

/// The depth terms of a perspective projection with the layout of Direct3D, as the GPU draws
/// with it ([`gpu_projection`]): `[a, b]` of `d = a + b / z`.
pub fn terms(projection: &[f32; 16]) -> [f32; 2] {
    plane_terms(projection[10], projection[14])
}

/// The depth terms as the GPU draws with them for the terms `a`, `b` of a projection with the
/// layout of Direct3D (`d = a + b / z`): `[1 - a, -b]` when the depth is reversed.
pub fn plane_terms(a: f32, b: f32) -> [f32; 2] {
    if reversed() { [1.0 - a, -b] } else { [a, b] }
}

/// The compare of a depth test of the world view, given for the plain convention (less is
/// nearer).
pub fn compare(function: wgpu::CompareFunction) -> wgpu::CompareFunction {
    use wgpu::CompareFunction::*;
    if !reversed() {
        return function;
    }
    match function {
        Less => Greater,
        LessEqual => GreaterEqual,
        Greater => Less,
        GreaterEqual => LessEqual,
        other => other,
    }
}

/// The depth bias of a pipeline of the world view, given for the plain convention: away from
/// the camera is up there, down when the depth is reversed.
pub fn bias(bias: wgpu::DepthBiasState) -> wgpu::DepthBiasState {
    if !reversed() {
        return bias;
    }
    wgpu::DepthBiasState { constant: -bias.constant, slope_scale: -bias.slope_scale, clamp: -bias.clamp }
}

/// The value of a clear of the depth of the world view, given for the plain convention.
pub fn clear(value: f32) -> f32 {
    if reversed() { 1.0 - value } else { value }
}

/// The WGSL functions of the depth. `t` is the depth terms of a projection ([`terms`]).
pub fn wgsl() -> String {
    format!(
        "
// The depth of the world view is reversed (near 1, far 0) when this is true.
const DEPTH_REVERSED: bool = {};

// True where nothing was drawn: the depth has the value of its clear.
fn depth_empty(d: f32) -> bool {{
    if DEPTH_REVERSED {{
        return d <= 0.0;
    }}
    return d >= 1.0;
}}

// The nearer of two depths.
fn depth_nearer(a: f32, b: f32) -> f32 {{
    if DEPTH_REVERSED {{
        return max(a, b);
    }}
    return min(a, b);
}}

// The distance along the view of a depth `d` that the projection with the terms `t` drew.
fn view_distance(d: f32, t: vec2f) -> f32 {{
    return t.y / (d - t.x);
}}

// The depth of a distance `z` along the view, for the projection with the terms `t`.
fn depth_at_distance(z: f32, t: vec2f) -> f32 {{
    return t.x + t.y / z;
}}
",
        reversed()
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_reversed_terms_decode_the_reversed_depth_of_a_distance() {
        let (near, far) = (1.0f32, 7170.75f32);
        let mut p = [0.0f32; 16];
        p[10] = far / (far - near);
        p[11] = 1.0;
        p[14] = -near * far / (far - near);
        // The reversed column, by hand: w less depth.
        let reversed = [1.0 - p[10], -p[14]];
        for z in [1.0f32, 10.0, 500.0, 7000.0] {
            let d = reversed[0] + reversed[1] / z;
            assert!((d - (1.0 - (p[10] + p[14] / z))).abs() < 1e-6);
            assert!(((reversed[1] / (d - reversed[0])) - z).abs() / z < 1e-3);
        }
    }
}
