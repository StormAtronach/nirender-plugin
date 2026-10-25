//! The UV controllers of the engine (`NiUVController`) as a texture transform of the draws.
//!
//! The engine's controller rewrites one texture set of its geometry data in place at each
//! update, from its last offsets and tilings to the new ones, and gives the data a new
//! revision (`NiUVController::Update` 0x7220E0). From the set as the data had it at the
//! state (offset 0, tiling 1) the result is `u = tU u0 + (1 - tU) / 2 - oU` and
//! `v = tV v0 + (1 - tV) / 2 + oV`. The plugin sends the vertices once with the state they
//! were in (`base`) and then only the new state (`now`, [`crate::tag::GEOMETRY_UV`]); the
//! draws of the geometry take the step from `base` to `now` as a texture transform of the
//! stages that read the set.

use crate::data::DrawData;

/// The UV state of a geometry: the texture set of its controller, the state of the
/// vertices the helper holds, and the controller's last state. A state is the U offset, the
/// V offset, the U tiling and the V tiling.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct UvAnimation {
    pub(crate) set: u32,
    pub(crate) base: [f32; 4],
    pub(crate) now: [f32; 4],
}

impl UvAnimation {
    /// The texture transform from `base` to `now`, as Direct3D's matrix of a stage with two
    /// parts (`u' = u m11 + v m21 + m31`, `v' = u m12 + v m22 + m32`), or [`None`] when the
    /// step changes nothing or a tiling of `base` is 0.
    pub(crate) fn matrix(&self) -> Option<[f32; 16]> {
        let [base_u, base_v, base_tu, base_tv] = self.base;
        let [now_u, now_v, now_tu, now_tv] = self.now;
        if base_tu == 0.0 || base_tv == 0.0 || self.base == self.now {
            return None;
        }
        let ru = now_tu / base_tu;
        let rv = now_tv / base_tv;
        let mut m = IDENTITY;
        m[0] = ru;
        m[8] = (1.0 - ru) * 0.5 - (now_u - ru * base_u);
        m[5] = rv;
        m[9] = (1.0 - rv) * 0.5 + (now_v - rv * base_v);
        Some(m)
    }
}

const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// The product `a b` of two matrices of Direct3D (row vectors: `v a b`).
fn multiply(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    core::array::from_fn(|i| {
        let (row, column) = (i / 4, i % 4);
        (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum()
    })
}

/// The coordinates and the texture transforms of the stages of a draw before a UV
/// transform, to compose a later step on.
#[derive(Clone)]
pub(crate) struct StageBase {
    coordinates: [[u32; 4]; 6],
    transforms: [[f32; 16]; 6],
}

impl StageBase {
    pub(crate) fn of(data: &DrawData) -> StageBase {
        StageBase { coordinates: data.stages.map(|stage| stage.coordinate), transforms: data.texture_transforms }
    }

    pub(crate) fn restore(&self, data: &mut DrawData) {
        for (stage, coordinate) in data.stages.iter_mut().zip(self.coordinates) {
            stage.coordinate = coordinate;
        }
        data.texture_transforms = self.transforms;
    }
}

/// Puts the step of `uv` before the texture transform of each stage of `data` that takes its
/// coordinate from the vertex set of `uv`. `data` has no step of `uv` yet.
pub(crate) fn compose(data: &mut DrawData, uv: &UvAnimation) {
    if let Some(m) = uv.matrix() {
        compose_matrix(data, uv.set, &m);
    }
}

pub(crate) fn compose_matrix(data: &mut DrawData, set: u32, m: &[f32; 16]) {
    let stages = (data.counts[1] as usize).min(data.stages.len());
    for i in 0..stages {
        let coordinate = data.stages[i].coordinate;
        // From the vertex (0), from this set.
        if coordinate[0] != 0 || coordinate[1] != set {
            continue;
        }
        if coordinate[2] == 0 {
            data.texture_transforms[i] = *m;
            data.stages[i].coordinate[2] = 2;
        } else {
            data.texture_transforms[i] = multiply(m, &data.texture_transforms[i]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One update of the engine's controller (`NiUVController::Update` 0x7220E0) on one
    /// coordinate: from the state `old` to the state `new`.
    fn engine_step(uv: [f32; 2], old: [f32; 4], new: [f32; 4]) -> [f32; 2] {
        let [old_u, old_v, old_tu, old_tv] = old;
        let [new_u, new_v, new_tu, new_tv] = new;
        let ru = new_tu / old_tu;
        let du = new_u - ru * old_u;
        let rv = new_tv / old_tv;
        let dv = new_v - rv * old_v;
        [ru * uv[0] + (1.0 - ru) * 0.5 - du, rv * uv[1] + (1.0 - rv) * 0.5 + dv]
    }

    fn apply(m: &[f32; 16], uv: [f32; 2]) -> [f32; 2] {
        [uv[0] * m[0] + uv[1] * m[4] + m[8], uv[0] * m[1] + uv[1] * m[5] + m[9]]
    }

    #[test]
    fn the_matrix_gives_the_engines_coordinates_after_several_updates() {
        let states = [[0.0, 0.0, 1.0, 1.0], [0.1, -0.2, 1.0, 1.0], [0.35, 0.4, 2.0, 0.5], [-0.7, 0.05, 0.25, 3.0]];
        let start = [[0.0f32, 0.0], [1.0, 1.0], [0.3, 0.8], [-2.5, 4.0]];
        // The helper holds the vertices of the state `base`; the engine goes on from there.
        for base in 0..states.len() {
            let mut held: Vec<[f32; 2]> = start.to_vec();
            for step in 1..=base {
                held = held.iter().map(|&uv| engine_step(uv, states[step - 1], states[step])).collect();
            }
            let mut engine = held.clone();
            for now in base + 1..states.len() {
                engine = engine.iter().map(|&uv| engine_step(uv, states[now - 1], states[now])).collect();
                let m = UvAnimation { set: 0, base: states[base], now: states[now] }.matrix().unwrap();
                for (h, e) in held.iter().zip(&engine) {
                    let got = apply(&m, *h);
                    assert!((got[0] - e[0]).abs() < 1e-4 && (got[1] - e[1]).abs() < 1e-4, "base {base} now {now}: {got:?} against {e:?}");
                }
            }
        }
    }

    #[test]
    fn no_step_and_a_zero_tiling_give_no_matrix() {
        let state = [0.2, 0.3, 1.5, 0.5];
        assert!(UvAnimation { set: 0, base: state, now: state }.matrix().is_none());
        assert!(UvAnimation { set: 0, base: [0.0, 0.0, 0.0, 1.0], now: state }.matrix().is_none());
    }

    #[test]
    fn compose_touches_only_the_stages_of_the_set_from_the_vertex() {
        let mut data: DrawData = bytemuck::Zeroable::zeroed();
        data.counts[1] = 3;
        data.stages[0].coordinate = [0, 0, 0, 0];
        data.stages[1].coordinate = [0, 1, 0, 0];
        data.stages[2].coordinate = [2, 0, 3, 0];
        let uv = UvAnimation { set: 0, base: [0.0, 0.0, 1.0, 1.0], now: [0.25, 0.0, 1.0, 1.0] };
        compose(&mut data, &uv);
        assert_eq!(data.stages[0].coordinate[2], 2);
        assert_eq!(data.texture_transforms[0][8], -0.25);
        assert_eq!(data.stages[1].coordinate[2], 0);
        assert_eq!(data.stages[2].coordinate[2], 3);
        assert_eq!(data.texture_transforms[2], [0.0; 16]);
    }

    #[test]
    fn a_stage_transform_goes_after_the_step() {
        let mut data: DrawData = bytemuck::Zeroable::zeroed();
        data.counts[1] = 1;
        data.stages[0].coordinate = [0, 0, 2, 0];
        // The stage doubles u.
        let mut stage = IDENTITY;
        stage[0] = 2.0;
        data.texture_transforms[0] = stage;
        let uv = UvAnimation { set: 0, base: [0.0, 0.0, 1.0, 1.0], now: [0.25, 0.0, 1.0, 1.0] };
        compose(&mut data, &uv);
        // (u - 0.25) * 2 for u = 1.
        assert_eq!(apply(&data.texture_transforms[0], [1.0, 0.0])[0], 1.5);
    }
}
