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
static ONE_BUFFER: AtomicBool = AtomicBool::new(false);

/// Sets the convention once, when the renderer makes its device: reversed with a float depth.
pub fn set_reversed(on: bool) {
    REVERSED.store(on, Ordering::Relaxed);
}

/// True when the depth of the world view is reversed: near 1, far 0.
pub fn reversed() -> bool {
    REVERSED.load(Ordering::Relaxed)
}

/// Sets the one-buffer path once, when the renderer makes its device: only with the reversed
/// depth. Temporary: the switch `MGE_XE_ONE_DEPTH` while step 6 of the depth design is tested;
/// the path and copy 2 of the depth go together with the switch when step 6 is accepted.
pub fn set_one_buffer(on: bool) {
    ONE_BUFFER.store(on && reversed(), Ordering::Relaxed);
}

/// True when the extension's draws before the world (the distant land, its statics and
/// glows) and the game's draws share one depth: no clear between them, and every perspective
/// projection of the world view with an infinite far plane ([`world_projection`]).
pub fn one_buffer() -> bool {
    ONE_BUFFER.load(Ordering::Relaxed)
}

/// A projection of the world view with the depth convention of the frame, still in the plain
/// convention: with [`one_buffer`], a perspective projection gets an infinite far plane (`d =
/// 1 - n / z`; reversed, `d = n / z`), so that every pass compares depths with one rule and
/// its own near plane. An orthographic projection stays as it is.
pub fn world_projection(projection: &[f32; 16]) -> [f32; 16] {
    if one_buffer() { infinite_far(projection) } else { *projection }
}

/// The projection with the same near plane and an infinite far plane, for a perspective; an
/// orthographic projection as it is.
fn infinite_far(projection: &[f32; 16]) -> [f32; 16] {
    let mut p = *projection;
    let perspective = p[11] == 1.0 && p[15] == 0.0 && p[10] != 0.0;
    if perspective {
        let near = -p[14] / p[10];
        p[10] = 1.0;
        p[14] = -near;
    }
    p
}

/// A projection with the layout of Direct3D as the GPU takes it ([`world_projection`]): the
/// depth column (places 2, 6, 10, 14) becomes w less depth (places 3, 7, 11, 15) when the
/// depth is reversed. Right for a perspective and for an orthographic projection.
pub fn gpu_projection(projection: &[f32; 16]) -> [f32; 16] {
    let world = world_projection(projection);
    let mut p = world;
    if reversed() {
        for row in 0..4 {
            p[row * 4 + 2] = world[row * 4 + 3] - world[row * 4 + 2];
        }
    }
    p
}

/// The depth terms of a perspective projection with the layout of Direct3D, as the GPU draws
/// with it ([`gpu_projection`]): `[a, b]` of `d = a + b / z`.
pub fn terms(projection: &[f32; 16]) -> [f32; 2] {
    let world = world_projection(projection);
    plane_terms(world[10], world[14])
}

/// The fault injections of the replay mode's `--depth-control` (step 6 of the depth design):
/// the near plane of the distant land as a part of the game's, its sink near the camera on or
/// off, and a raise of the distant land in units. Never set in a live run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub near_scale: Option<f32>,
    pub sink: bool,
    pub raise: f32,
}

impl Default for Control {
    fn default() -> Self {
        Control { near_scale: None, sink: true, raise: 0.0 }
    }
}

impl Control {
    /// Reads `k=<near scale>,sink=on|off,raise=<units>`, each part optional.
    pub fn parse(text: &str) -> Result<Control, String> {
        let mut control = Control::default();
        for part in text.split(',').map(str::trim).filter(|part| !part.is_empty()) {
            let (key, value) = part.split_once('=').ok_or_else(|| format!("{part}: not key=value"))?;
            match key {
                "k" => control.near_scale = Some(value.parse().map_err(|_| format!("{part}: not a number"))?),
                "sink" => {
                    control.sink = match value {
                        "on" => true,
                        "off" => false,
                        _ => return Err(format!("{part}: on or off")),
                    }
                }
                "raise" => control.raise = value.parse().map_err(|_| format!("{part}: not a number"))?,
                _ => return Err(format!("{part}: the keys are k, sink and raise")),
            }
        }
        Ok(control)
    }
}

static CONTROL: std::sync::OnceLock<Control> = std::sync::OnceLock::new();

/// Sets the fault injections of a replay, once, before its first frame.
pub fn set_control(control: Control) {
    let _ = CONTROL.set(control);
}

/// The fault injections of the replay, or none.
pub fn control() -> Control {
    CONTROL.get().copied().unwrap_or_default()
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

    #[test]
    fn an_infinite_far_keeps_the_near_plane_and_reverses_to_near_over_distance() {
        let (near, far) = (4.0f32, 7168.0f32);
        let mut p = [0.0f32; 16];
        p[0] = 1.3;
        p[5] = 1.7;
        p[10] = far / (far - near);
        p[11] = 1.0;
        p[14] = -near * far / (far - near);
        let q = super::infinite_far(&p);
        assert_eq!((q[0], q[5], q[11], q[15]), (p[0], p[5], 1.0, 0.0));
        assert!((-q[14] / q[10] - near).abs() < 1e-3);
        // Reversed, the depth is near / z at every distance: past the old far plane too.
        for z in [near, 100.0, 7168.0, 57344.0, 300000.0] {
            let d = (1.0 - q[10]) + (-q[14]) / z;
            assert!((d - near / z).abs() < 1e-6);
            assert!(d > 0.0);
        }
    }

    #[test]
    fn an_orthographic_projection_keeps_its_depth() {
        let mut p = [0.0f32; 16];
        p[0] = 0.01;
        p[5] = 0.01;
        p[10] = 0.001;
        p[14] = 0.5;
        p[15] = 1.0;
        assert_eq!(super::infinite_far(&p), p);
    }

    #[test]
    fn the_depth_control_reads_its_parts() {
        use super::Control;
        assert_eq!(Control::parse("").unwrap(), Control::default());
        assert_eq!(Control::parse("k=1,sink=off,raise=50").unwrap(), Control { near_scale: Some(1.0), sink: false, raise: 50.0 });
        assert_eq!(Control::parse(" k=0.9975 ").unwrap().near_scale, Some(0.9975));
        assert!(Control::parse("sink=maybe").is_err());
        assert!(Control::parse("depth=1").is_err());
        assert!(Control::parse("k").is_err());
    }
}
