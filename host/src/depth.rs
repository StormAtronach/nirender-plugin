//! The one decode of a depth value, for every shader that reads a depth: the merge of the
//! first person, and in MGE XE the depth frame of the post chain, the water, the blend and the
//! caustics. A projection with the layout of Direct3D gives the depth `d = a + b / z` for a
//! distance `z` along the view, with `a` and `b` its places 10 and 14: the depth terms
//! ([`terms`]). A reader takes the terms of the projection that drew its depth, and the WGSL
//! functions of [`WGSL`]; a change of the depth convention changes this module only.

/// The depth terms of a projection with the layout of Direct3D: `[a, b]` of `d = a + b / z`.
pub fn terms(projection: &[f32; 16]) -> [f32; 2] {
    [projection[10], projection[14]]
}

/// The WGSL functions of the depth. `t` is the depth terms of a projection ([`terms`]).
pub const WGSL: &str = "
// True where nothing was drawn: the depth has the value of its clear.
fn depth_empty(d: f32) -> bool {
    return d >= 1.0;
}

// The distance along the view of a depth `d` that the projection with the terms `t` drew.
fn view_distance(d: f32, t: vec2f) -> f32 {
    return t.y / (d - t.x);
}

// The depth of a distance `z` along the view, for the projection with the terms `t`.
fn depth_at_distance(z: f32, t: vec2f) -> f32 {
    return t.x + t.y / z;
}
";
