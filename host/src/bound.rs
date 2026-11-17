//! The bounds of the record path's draws: a sphere that holds every vertex of a draw, in the
//! draw's space. A draw lit for each vertex takes the world's point lights that reach it
//! ([`crate::lights::world_lights_in_reach`]).
//!
//! A bound only has to hold the vertices. A larger one adds lights, and the vertex loop of the
//! shader tests the range of each light at each vertex. So a sphere here is the middle of a box
//! and the farthest point from it, not the smallest sphere.
//!
//! The geometry's sphere is in model space, made when its `GEOMETRY` record comes. A draw takes
//! it through its world matrix, or through the matrices that blend its vertices: then each
//! vertex is a sum of `w_i M_i v` with weights that are not negative and sum to 1, so it lies
//! within `s r` of the hull of the points `M_i c` (`c` and `r` the model sphere, `s` the largest
//! scale of the matrices). A geometry whose weights break that rule gets no sphere.

/// A sphere: its centre and its radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sphere {
    pub(crate) centre: [f32; 3],
    pub(crate) radius: f32,
}

/// The model sphere of a geometry, with what its weights allow. A vertex `p = sum w_i q_i` of
/// points `q_i` within `R` of a centre `C` lies within `spread * R + excess * |C|` of it:
/// `p - C = sum w_i (q_i - C) + (sum w_i - 1) C`. With the slack of the rule a weight can be a
/// little below 0, so `spread` (the largest `sum |w_i|`) can be a little above 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ModelBound {
    pub(crate) sphere: Sphere,
    /// The largest `sum |w_i|` of a vertex, the implied last weight with them; 1 with no weights.
    pub(crate) spread: f32,
    /// The largest `|sum w_i - 1|` of a vertex: 0 for a blend whose last weight is 1 less the
    /// others.
    pub(crate) excess: f32,
}

/// The slack of the weight rule: a weight down to `-SLACK`, a sum up to `1 + SLACK`.
const SLACK: f32 = 0.001;
/// The vertex format of a geometry with a list of influences (`influences::FVF_INFLUENCES`).
const FVF_INFLUENCES: u32 = crate::influences::FVF_INFLUENCES;
/// The bytes of an entry of the list of influences: the bone and its weight.
const ENTRY_BYTES: usize = crate::influences::ENTRY_BYTES as usize;
/// The bytes of a vertex before its first weight: the position.
const WEIGHTS_AT: usize = 12;

/// The sphere around `points`: the middle of their box, and the farthest point from it.
/// [`None`] with no point, or with a point that is not finite.
pub(crate) fn around<I>(points: I) -> Option<Sphere>
where
    I: Iterator<Item = [f32; 3]> + Clone,
{
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for point in points.clone() {
        if !point.iter().all(|value| value.is_finite()) {
            return None;
        }
        for axis in 0..3 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    if low[0] > high[0] {
        return None;
    }
    let centre: [f32; 3] = core::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
    let radius = points.map(|point| distance(point, centre)).fold(0.0, f32::max);
    Some(Sphere { centre, radius })
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn float_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

/// The positions of vertices of `stride` bytes: the first three floats of each.
fn positions(vertices: &[u8], stride: usize) -> impl Iterator<Item = [f32; 3]> + Clone + '_ {
    vertices
        .chunks_exact(stride)
        .map(|vertex| core::array::from_fn(|axis| float_at(vertex, axis * 4)))
}

/// The weights that a vertex of the format holds after its position (`d3d::attributes`): one
/// float for each beta of the format, less the one of the matrix indexes.
fn weights_of(fvf: u32) -> usize {
    let betas = ((fvf & 0xE) as usize).saturating_sub(4) / 2;
    if betas != 0 && fvf & 0x1000 != 0 {
        betas - 1
    } else {
        betas
    }
}

/// The `sum |w_i|` of a vertex of a blend whose last weight is 1 less the others, with that
/// last weight. [`None`] when a weight, the last too, is below `-SLACK`.
fn blend_spread(weights: impl Iterator<Item = f32>) -> Option<f32> {
    let (mut sum, mut spread) = (0.0f32, 0.0f32);
    for weight in weights {
        if weight < -SLACK {
            return None;
        }
        sum += weight;
        spread += weight.abs();
    }
    let last = 1.0 - sum;
    (last >= -SLACK).then_some(spread + last.abs())
}

/// The largest `sum |w_i|` and `|sum w_i - 1|` of the vertices' entries of the list of
/// influences. [`None`] when a weight is below `-SLACK` or a sum is not within `SLACK` of 1. The
/// vertex holds its first entry and its count in its first two weights, as the record has
/// them (before `influences::moved_vertices`).
fn list_spread(vertices: &[u8], stride: usize, list: &[u8]) -> Option<(f32, f32)> {
    let entries = list.len() / ENTRY_BYTES;
    let (mut spread, mut excess) = (0.0f32, 0.0f32);
    for vertex in vertices.chunks_exact(stride) {
        let (first, count) = (float_at(vertex, WEIGHTS_AT), float_at(vertex, WEIGHTS_AT + 4));
        if !(first >= 0.0 && count >= 0.0) || first as usize + count as usize > entries {
            return None;
        }
        let (mut sum, mut absolute) = (0.0f32, 0.0f32);
        for entry in first as usize..first as usize + count as usize {
            let weight = float_at(list, entry * ENTRY_BYTES + 4);
            if weight < -SLACK {
                return None;
            }
            sum += weight;
            absolute += weight.abs();
        }
        if (sum - 1.0).abs() > SLACK {
            return None;
        }
        spread = spread.max(absolute);
        excess = excess.max((sum - 1.0).abs());
    }
    Some((spread, excess))
}

/// The model bound of a geometry record, with the format `fvf`, its vertices and its list of
/// influences. [`None`] for transformed vertices (not of the world), for a geometry with no
/// vertex, and for weights that break the rule of the module.
pub(crate) fn geometry_bound(fvf: u32, stride: u32, vertices: &[u8], list: Option<&[u8]>) -> Option<ModelBound> {
    let position = fvf & 0xE;
    let stride = stride as usize;
    if position == 0x4 || position == 0 || stride < 12 {
        return None;
    }
    let (spread, excess) = if fvf & FVF_INFLUENCES != 0 {
        if stride < WEIGHTS_AT + 8 {
            return None;
        }
        list_spread(vertices, stride, list?)?
    } else {
        let weights = weights_of(fvf).min(3);
        if weights == 0 {
            (1.0, 0.0)
        } else {
            if stride < WEIGHTS_AT + weights * 4 {
                return None;
            }
            let mut spread = 0.0f32;
            for vertex in vertices.chunks_exact(stride) {
                spread = spread.max(blend_spread((0..weights).map(|i| float_at(vertex, WEIGHTS_AT + i * 4)))?);
            }
            (spread, 0.0)
        }
    };
    Some(ModelBound {
        sphere: around(positions(vertices, stride))?,
        spread: spread.max(1.0),
        excess,
    })
}

/// A point through a matrix of the shader's layout (columns of four; the translation in
/// 12 to 14).
fn apply(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
}

/// The largest length of the matrix's three axes. A transform of the engine is a rotation times
/// one scale, so this is how far it stretches a length.
fn axis_scale(m: &[f32; 16]) -> f32 {
    (0..3)
        .map(|c| (m[c * 4].powi(2) + m[c * 4 + 1].powi(2) + m[c * 4 + 2].powi(2)).sqrt())
        .fold(0.0, f32::max)
}

/// A model sphere through a world matrix.
pub(crate) fn transformed(sphere: &Sphere, m: &[f32; 16]) -> Sphere {
    Sphere {
        centre: apply(m, sphere.centre),
        radius: sphere.radius * axis_scale(m),
    }
}

/// A model bound through the matrices that blend its vertices (the bones of a skinned shape,
/// or the draw's own world matrices). Each point `M_i v` lies within `R = rho + s r` of the
/// centre of the points `M_i c` (`rho` their radius); the blend of them within
/// `spread * R + excess * |C|` ([`ModelBound`]).
pub(crate) fn blended(model: &ModelBound, matrices: &[[f32; 16]]) -> Option<Sphere> {
    let scale = matrices.iter().map(axis_scale).fold(0.0, f32::max);
    let points = around(matrices.iter().map(|m| apply(m, model.sphere.centre)))?;
    let reach = points.radius + scale * model.sphere.radius;
    Some(Sphere {
        centre: points.centre,
        radius: model.spread * reach + model.excess * distance(points.centre, [0.0; 3]),
    })
}

/// The sphere of a record draw in the space where the shader sums its vertices (shader.wgsl,
/// the vertex stage): `blend` is the record's `flags[3]`. 0: through the world matrix. 1 to 3:
/// the vertex blend of the draw's own matrices `world[0..=blend]`. 4 and 5: the bones of the
/// shape in the frame's palette, `palette`, which the packed weights or the list name.
/// [`None`] for another blend, or a skinned draw without its bones.
pub(crate) fn draw_sphere(
    model: &ModelBound,
    blend: u32,
    world: &[[f32; 16]; 4],
    palette: Option<&[[f32; 16]]>,
) -> Option<Sphere> {
    match blend {
        0 => Some(transformed(&model.sphere, &world[0])),
        1..=3 => blended(model, &world[..=blend as usize]),
        4 | 5 => blended(model, palette.filter(|bones| !bones.is_empty())?),
        _ => None,
    }
}

/// The sphere that holds two spheres.
pub(crate) fn union(a: &Sphere, b: &Sphere) -> Sphere {
    let gap = distance(a.centre, b.centre);
    if gap + b.radius <= a.radius {
        return *a;
    }
    if gap + a.radius <= b.radius {
        return *b;
    }
    let radius = (gap + a.radius + b.radius) * 0.5;
    // The centre on the line from a to b, at `radius - a.radius` from a's centre.
    let t = (radius - a.radius) / gap;
    Sphere {
        centre: core::array::from_fn(|axis| a.centre[axis] + (b.centre[axis] - a.centre[axis]) * t),
        radius,
    }
}

/// The sphere of the positions of a morph target: three floats for each vertex.
pub(crate) fn target_sphere(floats: &[f32]) -> Option<Sphere> {
    around(floats.as_chunks::<3>().0.iter().copied())
}

/// The sphere of a morph's sum `sum w_t T_t`: each position lies within `sum |w_t| r_t` of
/// `sum w_t c_t`.
pub(crate) fn morphed(targets: &[Sphere], weights: &[f32]) -> Sphere {
    let pairs = || targets.iter().zip(weights);
    Sphere {
        centre: core::array::from_fn(|axis| pairs().map(|(target, weight)| weight * target.centre[axis]).sum()),
        radius: pairs().map(|(target, weight)| weight.abs() * target.radius).sum(),
    }
}

/// The model sphere of a particle system from its record: the positions (20 bytes each: three
/// floats, the size, the colour), widened by the farthest corner of a quad,
/// `radius * size * max(|right + up|, |up - right|)`.
pub(crate) fn particle_sphere(radius: f32, right: [f32; 3], up: [f32; 3], particles: &[u8]) -> Option<Sphere> {
    const BYTES: usize = 20;
    let centres = around(positions(particles, BYTES))?;
    let size = particles
        .as_chunks::<BYTES>()
        .0
        .iter()
        .map(|particle| float_at(particle, 12).abs())
        .fold(0.0, f32::max);
    let sum: [f32; 3] = core::array::from_fn(|i| right[i] + up[i]);
    let difference: [f32; 3] = core::array::from_fn(|i| up[i] - right[i]);
    let corner = distance(sum, [0.0; 3]).max(distance(difference, [0.0; 3]));
    Some(Sphere {
        centre: centres.centre,
        radius: centres.radius + radius.abs() * size * corner,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex_bytes(points: &[[f32; 3]], extra: &[f32]) -> Vec<u8> {
        points
            .iter()
            .flat_map(|p| p.iter().chain(extra).flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>())
            .collect()
    }

    fn holds(sphere: &Sphere, point: [f32; 3]) -> bool {
        distance(point, sphere.centre) <= sphere.radius * (1.0 + 1e-5) + 1e-4
    }

    const POINTS: [[f32; 3]; 5] = [
        [1.0, 2.0, 3.0],
        [-40.0, 7.5, 0.0],
        [12.0, -3.0, 55.0],
        [0.0, 0.0, -20.0],
        [33.0, 33.0, 33.0],
    ];

    #[test]
    fn the_sphere_of_a_geometry_holds_every_position() {
        let bytes = vertex_bytes(&POINTS, &[0.5]);
        let model = geometry_bound(0x2, 16, &bytes, None).unwrap();
        assert_eq!((model.spread, model.excess), (1.0, 0.0));
        let sphere = model.sphere;
        assert!(POINTS.iter().all(|&p| holds(&sphere, p)));
        // The null: a sphere smaller by 1 percent misses one.
        let smaller = Sphere {
            radius: sphere.radius * 0.99,
            ..sphere
        };
        assert!(!POINTS.iter().all(|&p| holds(&smaller, p)));
        // Transformed vertices are not of the world.
        assert_eq!(geometry_bound(0x4, 16, &bytes, None), None);
    }

    /// A rotation about z by `angle`, scaled by `scale`, then moved by `place`: one column of
    /// the shader's layout on each line.
    #[rustfmt::skip]
    fn matrix(angle: f32, scale: f32, place: [f32; 3]) -> [f32; 16] {
        let (s, c) = angle.sin_cos();
        [
            c * scale, s * scale, 0.0, 0.0, //
            -s * scale, c * scale, 0.0, 0.0, //
            0.0, 0.0, scale, 0.0, //
            place[0], place[1], place[2], 1.0,
        ]
    }

    #[test]
    fn a_scaled_world_matrix_scales_the_radius() {
        let sphere = Sphere {
            centre: [1.0, 0.0, 0.0],
            radius: 2.0,
        };
        let moved = transformed(&sphere, &matrix(0.0, 3.0, [10.0, 0.0, 0.0]));
        assert_eq!(moved.centre, [13.0, 0.0, 0.0]);
        assert_eq!(moved.radius, 6.0);
    }

    /// The blend of the shader: `w_0 M_0 v + ... + (1 - sum) M_n v`.
    fn blend(matrices: &[[f32; 16]], weights: &[f32], p: [f32; 3]) -> [f32; 3] {
        let last = 1.0 - weights.iter().sum::<f32>();
        let mut out = [0.0; 3];
        for (m, w) in matrices.iter().zip(weights.iter().copied().chain([last])) {
            let q = apply(m, p);
            for axis in 0..3 {
                out[axis] += w * q[axis];
            }
        }
        out
    }

    #[test]
    fn a_skin_that_turns_a_bone_stays_in_its_bound() {
        // Two bones, three weights each (XYZB4 with the indexes: 0x100C); the second bone
        // turned and moved.
        let weights: [[f32; 3]; 5] = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.5, 0.5, 0.0],
            [0.25, 0.75, 0.0],
            [0.9, 0.1, 0.0],
        ];
        let bytes: Vec<u8> = POINTS
            .iter()
            .zip(weights)
            .flat_map(|(p, w)| {
                p.iter()
                    .chain(&w)
                    .flat_map(|f| f.to_le_bytes())
                    .chain([0u8, 1, 0, 0])
                    .collect::<Vec<u8>>()
            })
            .collect();
        let model = geometry_bound(0x100C, 28, &bytes, None).unwrap();
        assert_eq!(model.spread, 1.0);
        let bones = [matrix(0.0, 1.0, [0.0; 3]), matrix(1.2, 1.0, [30.0, -10.0, 5.0])];
        let bound = blended(&model, &bones).unwrap();
        let posed: Vec<[f32; 3]> = POINTS.iter().zip(weights).map(|(&p, w)| blend(&bones, &w[..1], p)).collect();
        assert!(posed.iter().all(|&p| holds(&bound, p)));
        // The null: the bound without the widening `s r` misses one.
        let narrow = around(bones.iter().map(|m| apply(m, model.sphere.centre))).unwrap();
        assert!(!posed.iter().all(|&p| holds(&narrow, p)));
    }

    #[test]
    fn a_negative_weight_gives_no_bound() {
        let bytes = vertex_bytes(&[[0.0; 3]], &[1.2, -0.2, 0.0, 0.0]);
        assert_eq!(geometry_bound(0x100C, 28, &bytes, None), None);
        // The null: weights of the rule give a bound.
        let bytes = vertex_bytes(&[[0.0; 3]], &[0.8, 0.2, 0.0, 0.0]);
        assert!(geometry_bound(0x100C, 28, &bytes, None).is_some());
    }

    /// Verdict 583 addendum 1, C2: a weight just under 0 that the slack lets in puts the vertex
    /// outside the hull of its bones' points; the spread holds it.
    #[test]
    fn a_weight_in_the_slack_stays_in_the_spread_bound() {
        let weights = [1.0008, -0.0008, 0.0];
        let bytes = vertex_bytes(&[[0.0; 3]], &[weights[0], weights[1], weights[2], 0.0]);
        let model = geometry_bound(0x100C, 28, &bytes, None).unwrap();
        assert!((model.spread - 1.0016).abs() < 1e-6);
        let bones = [matrix(0.0, 1.0, [0.0; 3]), matrix(0.0, 1.0, [10000.0, 0.0, 0.0])];
        // The vertex at the first bone's origin; the second bone 10000 away takes -0.0008.
        let p = blend(&bones, &weights[..1], [0.0; 3]);
        assert!((p[0] + 8.0).abs() < 1e-3);
        let bound = blended(&model, &bones).unwrap();
        assert!(holds(&bound, p));
        // The null: the bound without the spread misses it.
        let unscaled = blended(&ModelBound { spread: 1.0, ..model }, &bones).unwrap();
        assert!(!holds(&unscaled, p));
    }

    /// Verdict 583 addendum 2: a list whose sum is off 1 moves the vertex by that share of its
    /// distance from the origin of the blend's space. The matrices are in the draw's space,
    /// so the term takes `|C|` there: the shape at absolute (100, 0, 0) and the space's origin
    /// at (30000, 0, 0). The sum is 1.0009, inside the slack of 0.001 also after the rounding
    /// of f32 (1.001 itself rounds just over it).
    #[test]
    fn the_excess_of_a_list_is_taken_in_the_space_of_the_blend() {
        let format = 0x100C | FVF_INFLUENCES;
        let bytes = vertex_bytes(&[[0.0; 3]], &[0.0, 2.0, 0.0, 0.0]);
        let model = geometry_bound(format, 28, &bytes, Some(&list(&[0.6, 0.4009]))).unwrap();
        assert!((model.excess - 0.0009).abs() < 1e-6);
        let (shape, origin) = ([100.0f64, 0.0, 0.0], [30000.0f64, 0.0, 0.0]);
        let place: [f32; 3] = core::array::from_fn(|i| (shape[i] - origin[i]) as f32);
        let bones = [matrix(0.0, 1.0, place), matrix(0.0, 1.0, place)];
        // The blend of the list: each weight times its bone's point, with no last weight.
        let p: [f32; 3] = core::array::from_fn(|axis| (0.6 + 0.4009) * apply(&bones[0], [0.0; 3])[axis]);
        let bound = blended(&model, &bones).unwrap();
        assert!(holds(&bound, p));
        // The null: the term with `|C|` of the absolute place misses it. The bones' points are
        // one point and the model sphere has no radius, so the term is the whole radius.
        let absolute = Sphere {
            radius: model.excess * 100.0,
            ..bound
        };
        assert!(!holds(&absolute, p));
    }

    fn list(weights: &[f32]) -> Vec<u8> {
        weights
            .iter()
            .enumerate()
            .flat_map(|(bone, w)| (bone as u32).to_le_bytes().into_iter().chain(w.to_le_bytes()))
            .collect()
    }

    #[test]
    fn a_software_skin_with_a_sum_off_one_gives_no_bound() {
        let format = 0x100C | FVF_INFLUENCES;
        // One vertex with two entries from entry 0.
        let bytes = vertex_bytes(&[[1.0, 2.0, 3.0]], &[0.0, 2.0, 0.0, 0.0]);
        assert_eq!(geometry_bound(format, 28, &bytes, Some(&list(&[0.7, 0.5]))), None);
        // The null: a sum of 1 gives a bound.
        assert!(geometry_bound(format, 28, &bytes, Some(&list(&[0.7, 0.3]))).is_some());
    }

    #[test]
    fn a_morph_with_a_negative_weight_stays_in_its_bound() {
        let targets: [[[f32; 3]; 4]; 3] = [
            [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]],
            [[1.0, 1.0, 0.0], [-3.0, 0.0, 2.0], [0.0, 4.0, -4.0], [2.0, 2.0, 2.0]],
            [[0.0, 0.0, 5.0], [0.0, -6.0, 0.0], [7.0, 0.0, 0.0], [-1.0, -1.0, -1.0]],
        ];
        let weights = [1.0, -0.8, 0.6];
        let spheres: Vec<Sphere> = targets.iter().map(|t| target_sphere(&t.concat()).unwrap()).collect();
        let bound = morphed(&spheres, &weights);
        let sums: Vec<[f32; 3]> = (0..4)
            .map(|v| core::array::from_fn(|axis| (0..3).map(|t| weights[t] * targets[t][v][axis]).sum()))
            .collect();
        assert!(sums.iter().all(|&p| holds(&bound, p)));
        // The null: the radius with `w_t` in place of `|w_t|` misses one.
        let signed = Sphere {
            radius: spheres.iter().zip(weights).map(|(s, w)| w * s.radius).sum(),
            ..bound
        };
        assert!(!sums.iter().all(|&p| holds(&signed, p)));
    }

    #[test]
    fn the_union_holds_both_spheres() {
        let a = Sphere {
            centre: [0.0; 3],
            radius: 1.0,
        };
        let b = Sphere {
            centre: [10.0, 0.0, 0.0],
            radius: 2.0,
        };
        let both = union(&a, &b);
        assert!(holds(&both, [-1.0, 0.0, 0.0]) && holds(&both, [12.0, 0.0, 0.0]));
        assert_eq!(both.radius, 6.5);
        // A sphere inside the other gives the other.
        let inside = Sphere {
            centre: [0.5, 0.0, 0.0],
            radius: 0.2,
        };
        assert_eq!(union(&a, &inside), a);
    }

    #[test]
    fn every_corner_of_a_particle_system_lies_in_its_bound() {
        let particles: Vec<u8> = [([0.0, 0.0, 0.0], 1.0), ([50.0, 10.0, -5.0], 2.5), ([-20.0, 30.0, 8.0], 0.5)]
            .iter()
            .flat_map(|(p, size): &([f32; 3], f32)| {
                p.iter()
                    .chain([size])
                    .flat_map(|f| f.to_le_bytes())
                    .chain([255u8; 4])
                    .collect::<Vec<u8>>()
            })
            .collect();
        let (radius, right, up) = (4.0, [0.6, 0.8, 0.0], [0.0, 0.0, 1.0]);
        let bound = particle_sphere(radius, right, up, &particles).unwrap();
        let corners: Vec<[f32; 3]> = particles
            .chunks_exact(20)
            .flat_map(|particle| {
                let p: [f32; 3] = core::array::from_fn(|axis| float_at(particle, axis * 4));
                let size = float_at(particle, 12);
                let s: [f32; 3] = core::array::from_fn(|i| (right[i] + up[i]) * radius * size);
                let d: [f32; 3] = core::array::from_fn(|i| (up[i] - right[i]) * radius * size);
                [
                    core::array::from_fn::<f32, 3, _>(|i| p[i] - s[i]),
                    core::array::from_fn(|i| p[i] - d[i]),
                    core::array::from_fn(|i| p[i] + s[i]),
                    core::array::from_fn(|i| p[i] + d[i]),
                ]
            })
            .collect();
        assert!(corners.iter().all(|&c| holds(&bound, c)));
        // The null: the sphere of the positions alone misses a corner.
        let centres = around(positions(&particles, 20)).unwrap();
        assert!(!corners.iter().all(|&c| holds(&centres, c)));
    }
}
