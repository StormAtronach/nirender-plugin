//! The world projection of a texture effect under a billboard that turns to the camera.
//!
//! The engine turns a billboard at its display (`NiBillboardNode::RotateToCamera` 0x6D3C00) and
//! updates its children there, so the projection of an effect under it changes with the camera
//! (`NiTextureEffect::UpdateProjection` 0x6E37C0). The plugin sends what does not change with
//! the camera ([`crate::tag::EFFECT_CHAIN`]): the world of the node above the first turning
//! billboard, the local transform of each node from it down to the effect, and the model
//! projection. The helper makes the projection from those and the camera of the frame.
//!
//! The camera is the one of the world scene. The main view is the only one that draws the table's
//! world shapes: the water's mirror replays only the sky classes (`reflect_sky`, `ReplayView::SKY`),
//! and the shadow maps do not use the table. A slot has one value for all views, so a sky slot in
//! the mirror takes the world camera's value too.
//!
//! The same steps make two values ([`Places`]). With the engine's stored places, each sum stored
//! as a float, they give the engine's words bit for bit: the unit tests and the live check. With
//! the exact places they give the stage of a draw: a map that takes the position is relative to
//! the origin of the table, as the geometry is, and holds far from the origin of the world.

/// The words of a transform: the rotation row by row, the translation, the scale.
pub const TRANSFORM_WORDS: usize = 13;
/// The words of a link of a record: the kind, then the local transform.
pub const LINK_WORDS: usize = 1 + TRANSFORM_WORDS;
/// The words of a record before its links: the id, the coordinate mode, the number of links, the
/// world of the node above the first turning billboard, then its exact place (3 doubles).
pub const HEAD_WORDS: usize = 3 + TRANSFORM_WORDS + 6;
/// The words of a record after its links: the model projection, matrix and translation.
pub const MODEL_WORDS: usize = 12;
/// The kind of a link that does not turn: a plain node, a billboard with no turn, the effect.
pub const NO_TURN: u32 = 3;
/// The most links a record can have: more nodes than a scene graph holds above an effect.
const MOST_LINKS: usize = 256;

/// A sphere map: its projection comes from the world rotation alone.
const COORDINATES_SPHERE: u32 = 2;

const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// A world projection: the matrix by rows, the translation.
pub type Projection = ([f32; 9], [f32; 3]);

/// What the test switch reads of the engine in a frame ([`crate::tag::EFFECT_TURN_CHECK`]).
pub struct EngineWords {
    pub id: u32,
    pub projection: Projection,
    pub camera_place: [f32; 3],
    /// The texture matrix of the engine's callbacks for the camera of the frame.
    pub stage: [f32; 16],
}

/// A map that takes the position of the vertex (coordinate modes 0 and 1): its stage is relative
/// to the origin of the table. A reflection or a normal has no place in it.
pub fn takes_position(coordinates: u32) -> bool {
    matches!(coordinates, 0 | 1)
}

/// How a place is kept between the steps.
#[derive(Clone, Copy, PartialEq)]
enum Places {
    /// As the engine stores it, a float.
    Stored,
    /// Exact.
    Exact,
}

impl Places {
    fn keep(self, value: f64) -> f64 {
        match self {
            Places::Stored => f64::from(value as f32),
            Places::Exact => value,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Camera {
    pub direction: [f32; 3],
    pub up: [f32; 3],
    pub right: [f32; 3],
    pub location: [f64; 3],
}

impl Camera {
    /// The camera of a view matrix of Direct3D, whose columns hold the camera's axes
    /// (`NiDX8Renderer::SetCameraData` 0x6AC620), at its exact place.
    pub fn of_view(view: &[f32; 16], location: [f64; 3]) -> Camera {
        Camera {
            right: [view[0], view[4], view[8]],
            up: [view[1], view[5], view[9]],
            direction: [view[2], view[6], view[10]],
            location,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform {
    pub rotation: [f32; 9],
    pub translation: [f32; 3],
    pub scale: f32,
}

impl Transform {
    fn of_words(words: &[u32]) -> Transform {
        Transform {
            rotation: core::array::from_fn(|i| f32::from_bits(words[i])),
            translation: core::array::from_fn(|i| f32::from_bits(words[9 + i])),
            scale: f32::from_bits(words[12]),
        }
    }
}

/// A node from the first turning billboard down to the effect.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Link {
    /// The billboard's mode (0 to 2), or [`NO_TURN`].
    pub kind: u32,
    pub local: Transform,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Chain {
    pub coordinates: u32,
    /// The world of the node above the first turning billboard, and its exact place.
    pub root: Transform,
    pub root_exact: [f64; 3],
    pub links: Vec<Link>,
    pub model_matrix: [f32; 9],
    pub model_translation: [f32; 3],
}

/// A world transform on the way down a chain.
#[derive(Clone, Copy)]
struct World {
    rotation: [f32; 9],
    place: [f64; 3],
    scale: f32,
}

impl Chain {
    /// The id and the chain of a record, or [`None`] for a record that does not hold its links.
    pub fn parse(words: &[u32]) -> Option<(u32, Chain)> {
        let count = *words.get(2)? as usize;
        if count > MOST_LINKS || words.len() < HEAD_WORDS + count * LINK_WORDS + MODEL_WORDS {
            return None;
        }
        let exact = &words[3 + TRANSFORM_WORDS..HEAD_WORDS];
        let links = words[HEAD_WORDS..]
            .as_chunks::<LINK_WORDS>()
            .0
            .iter()
            .take(count)
            .map(|link| Link {
                kind: link[0],
                local: Transform::of_words(&link[1..]),
            })
            .collect();
        let model = &words[HEAD_WORDS + count * LINK_WORDS..];
        let chain = Chain {
            coordinates: words[1],
            root: Transform::of_words(&words[3..]),
            root_exact: core::array::from_fn(|i| {
                f64::from_bits(u64::from(exact[i * 2]) | u64::from(exact[i * 2 + 1]) << 32)
            }),
            links,
            model_matrix: core::array::from_fn(|i| f32::from_bits(model[i])),
            model_translation: core::array::from_fn(|i| f32::from_bits(model[9 + i])),
        };
        Some((words[0], chain))
    }

    /// The world transform of the effect for this camera.
    fn effect_world(&self, camera: &Camera, places: Places, turned: bool) -> World {
        let place = match places {
            Places::Stored => self.root.translation.map(f64::from),
            Places::Exact => self.root_exact,
        };
        let mut world = World {
            rotation: self.root.rotation,
            place,
            scale: self.root.scale,
        };
        for link in &self.links {
            world = child_world(&world, &link.local, places);
            if turned && link.kind != NO_TURN {
                world.rotation = multiply(&world.rotation, &turn(link.kind, &world, camera, places));
            }
        }
        world
    }

    fn projection(&self, camera: &Camera, places: Places, origin: &[f64; 3], turned: bool) -> Projection {
        projection(
            self.coordinates,
            &self.model_matrix,
            &self.model_translation,
            &self.effect_world(camera, places, turned),
            origin,
            places,
        )
    }

    /// The projection as the engine has it, from the stored places and a camera at its stored place.
    pub fn stored_projection(&self, camera: &Camera) -> Projection {
        self.projection(camera, Places::Stored, &[0.0; 3], true)
    }

    /// The stored projection with no turn at all, for the null test of the check.
    pub fn unturned_projection(&self, camera: &Camera) -> Projection {
        self.projection(camera, Places::Stored, &[0.0; 3], false)
    }

    /// The projection from the exact places, in the space of the world.
    pub fn exact_projection(&self, camera: &Camera) -> Projection {
        self.projection(camera, Places::Exact, &[0.0; 3], true)
    }

    /// The projection of the table's stage: from the exact places, and relative to `origin` for a
    /// map that takes the position, whose vertex is relative to it too.
    pub fn stage_projection(&self, camera: &Camera, origin: &[f64; 3]) -> Projection {
        let origin = if takes_position(self.coordinates) { *origin } else { [0.0; 3] };
        self.projection(camera, Places::Exact, &origin, true)
    }

    /// The texture matrix of a draw at its place in the walk, which takes the vectors of camera
    /// space as the engine's callbacks do: each row of the projection along the camera's axes, and a
    /// position relative to the camera.
    pub fn camera_stage(&self, camera: &Camera) -> [f32; 16] {
        self.camera_stage_in(camera, Places::Exact)
    }

    fn camera_stage_in(&self, camera: &Camera, places: Places) -> [f32; 16] {
        let origin = if takes_position(self.coordinates) {
            camera.location
        } else {
            [0.0; 3]
        };
        let (matrix, translation) = self.projection(camera, places, &origin, true);
        let mut out = [0.0f32; 16];
        for (axis, along) in [camera.right, camera.up, camera.direction].iter().enumerate() {
            for row in 0..3 {
                out[axis * 4 + row] = (0..3)
                    .map(|k| f64::from(matrix[row * 3 + k]) * f64::from(along[k]))
                    .sum::<f64>() as f32;
            }
        }
        out[12..15].copy_from_slice(&translation);
        out
    }

    /// The mode of the last billboard that turns: the pair of the check's report.
    pub fn last_turn(&self) -> u32 {
        self.links
            .iter()
            .rev()
            .map(|link| link.kind)
            .find(|&kind| kind != NO_TURN)
            .unwrap_or(NO_TURN)
    }

    pub fn turning_billboards(&self) -> usize {
        self.links.iter().filter(|link| link.kind != NO_TURN).count()
    }
}

/// `a b`, each by rows (`matrix33::multiplyMatrix` 0x6E80A0).
fn multiply(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
    core::array::from_fn(|i| {
        let (row, column) = (i / 3, i % 3);
        (0..3)
            .map(|k| f64::from(a[row * 3 + k]) * f64::from(b[k * 3 + column]))
            .sum::<f64>() as f32
    })
}

/// `m v` (`matrix33::multiplyVector3` 0x6E8230).
fn apply(m: &[f32; 9], v: &[f64; 3], places: Places) -> [f64; 3] {
    core::array::from_fn(|row| places.keep((0..3).map(|k| f64::from(m[row * 3 + k]) * v[k]).sum::<f64>()))
}

/// `v m`, the vector in the axes of `m` (`matrix33_static::transpose_mul_vec3` 0x6E8290).
fn apply_transposed(m: &[f32; 9], v: &[f32; 3]) -> [f32; 3] {
    core::array::from_fn(|column| (0..3).map(|k| f64::from(m[k * 3 + column]) * f64::from(v[k])).sum::<f64>() as f32)
}

/// The world of a child (`NiAVObject::UpdateWorldData` 0x6FB350, with its parent).
fn child_world(parent: &World, local: &Transform, places: Places) -> World {
    let turned = apply(&parent.rotation, &local.translation.map(f64::from), places);
    let scaled = turned.map(|v| places.keep(v * f64::from(parent.scale)));
    World {
        rotation: multiply(&parent.rotation, &local.rotation),
        place: core::array::from_fn(|i| places.keep(parent.place[i] + scaled[i])),
        scale: (f64::from(local.scale) * f64::from(parent.scale)) as f32,
    }
}

/// The matrix that `RotateToCamera` 0x6D3C00 puts after the rotation of the billboard, for its
/// mode and its world before the turn.
fn turn(mode: u32, world: &World, camera: &Camera, places: Places) -> [f32; 9] {
    match mode {
        0 => turn_facing(world, camera),
        1 => turn_about_y(world, camera, places),
        2 => turn_to_axes(world, camera),
        _ => IDENTITY,
    }
}

/// Mode 0: faces the camera and keeps its up. Identity with the camera's up and right both
/// across the node's Y axis.
fn turn_facing(world: &World, camera: &Camera) -> [f32; 9] {
    let direction = apply_transposed(&world.rotation, &camera.direction).map(f64::from);
    let up = apply_transposed(&world.rotation, &camera.up).map(f64::from);
    let right = apply_transposed(&world.rotation, &camera.right).map(f64::from);

    let length = (up[1] * up[1] + right[1] * right[1]).sqrt();
    if length.is_nan() || length <= 1.0e-6 {
        return IDENTITY;
    }
    let cosine = up[1] / length;
    let sine = -right[1] / length;
    [
        (up[0] * sine + right[0] * cosine) as f32,
        (up[0] * cosine - right[0] * sine) as f32,
        -direction[0] as f32,
        (up[1] * sine + right[1] * cosine) as f32,
        (up[1] * cosine - right[1] * sine) as f32,
        -direction[1] as f32,
        (up[2] * sine + right[2] * cosine) as f32,
        (up[2] * cosine - right[2] * sine) as f32,
        -direction[2] as f32,
    ]
}

/// Mode 1: about the local Y axis, towards the camera's place, with the scale divided out.
/// Identity with the camera on that axis.
fn turn_about_y(world: &World, camera: &Camera, places: Places) -> [f32; 9] {
    let location = camera.location.map(|v| places.keep(v));
    let delta: [f32; 3] = core::array::from_fn(|i| (location[i] - world.place[i]) as f32);
    let local = apply_transposed(&world.rotation, &delta);
    let inverse = 1.0 / f64::from(world.scale);
    let x = f64::from((inverse * f64::from(local[0])) as f32);
    let z = inverse * f64::from(local[2]);

    let length = (z * z + x * x).sqrt();
    if length.is_nan() || length < 1.0e-12 {
        return IDENTITY;
    }
    let cosine = (z / length) as f32;
    let sine = (x / length) as f32;
    [cosine, 0.0, sine, 0.0, 1.0, 0.0, -sine, 0.0, cosine]
}

/// Mode 2: the rotation becomes the camera's axes.
fn turn_to_axes(world: &World, camera: &Camera) -> [f32; 9] {
    let direction = apply_transposed(&world.rotation, &camera.direction);
    let up = apply_transposed(&world.rotation, &camera.up);
    let right = apply_transposed(&world.rotation, &camera.right);
    [
        right[0],
        up[0],
        direction[0],
        right[1],
        up[1],
        direction[1],
        right[2],
        up[2],
        direction[2],
    ]
}

/// `NiTextureEffect::UpdateProjection` 0x6E37C0: a sphere map from the world rotation alone, each
/// other mode from the model projection, its translation for a position relative to `origin`.
fn projection(
    coordinates: u32,
    model_matrix: &[f32; 9],
    model_translation: &[f32; 3],
    world: &World,
    origin: &[f64; 3],
    places: Places,
) -> Projection {
    if coordinates == COORDINATES_SPHERE {
        let r = &world.rotation;
        let half = |v: f32| (f64::from(v) * 0.5) as f32;
        let minus_half = |v: f32| (f64::from(v) * -0.5) as f32;
        return (
            [
                half(r[2]),
                half(r[5]),
                half(r[8]),
                minus_half(r[1]),
                minus_half(r[4]),
                minus_half(r[7]),
                0.0,
                0.0,
                0.0,
            ],
            [0.5, 0.5, 0.0],
        );
    }
    let inverse = (1.0 / f64::from(world.scale)) as f32;
    let r = &world.rotation;
    let transposed = [r[0], r[3], r[6], r[1], r[4], r[7], r[2], r[5], r[8]];
    let matrix = multiply(model_matrix, &transposed).map(|v| (f64::from(v) * f64::from(inverse)) as f32);
    let relative: [f64; 3] = core::array::from_fn(|i| world.place[i] - origin[i]);
    let at = apply(&matrix, &relative, places);
    (
        matrix,
        core::array::from_fn(|i| places.keep(f64::from(model_translation[i]) - at[i]) as f32),
    )
}

/// The table's stage of an effect whose projection does not move in the play: its words of the
/// material record, relative to `origin` for a map that takes the position
/// (`T' = T + P origin`).
pub fn plain_stage(coordinates: u32, matrix: &[f32; 9], translation: &[f32; 3], origin: &[f64; 3]) -> [f32; 16] {
    if !takes_position(coordinates) {
        return texture_matrix(matrix, translation);
    }
    let moved = apply(matrix, origin, Places::Exact);
    let relative: [f32; 3] = core::array::from_fn(|i| (f64::from(translation[i]) + moved[i]) as f32);
    texture_matrix(matrix, &relative)
}

/// The texture matrix of Direct3D for a world projection, row by row: the coordinate is a row
/// vector times the matrix, so a column of the projection's matrix is a row here, and the
/// translation is the fourth row.
pub fn texture_matrix(matrix: &[f32; 9], translation: &[f32; 3]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for row in 0..3 {
        for column in 0..3 {
            out[row * 4 + column] = matrix[column * 3 + row];
        }
    }
    out[12..15].copy_from_slice(translation);
    out
}

/// The largest difference of a projection from the engine's words: the matrix, the translation.
pub fn difference(a: &Projection, b: &Projection) -> (f32, f32) {
    let largest = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| (p - q).abs()).fold(0.0f32, f32::max);
    (largest(&a.0, &b.0), largest(&a.1, &b.1))
}

/// What the live check of one pair found (`NIRENDER_EFFECT_TURN_CHECK`).
#[derive(Clone, Copy, Default)]
struct PairCheck {
    compared: u64,
    /// The projection from the stored places and the engine's stored camera place, against the
    /// engine's words: the maths.
    matrix: f32,
    translation: f32,
    /// The same from the exact places: how far the stage is from the engine's rounding.
    exact_matrix: f32,
    exact_translation: f32,
    /// The texture coordinate at the effect's place against its exact value: the stage relative
    /// to the origin, and the form before it, with the origin added back in floats. A map that
    /// takes the position only.
    relative_error: f32,
    world_error: f32,
    /// The stored projection with the camera of the frame before, with no turn, and with the chain
    /// of the frame before: the null tests. A moving camera shows in the first, a turn that counts in
    /// the second, and a read of the chain one frame late in the third.
    old_camera: f32,
    no_turn: f32,
    old_chain: f32,
    /// The stage of a draw at its place against the engine's callbacks: the first three rows, the
    /// translation; from the stored places, then from the exact ones as a draw takes it.
    stage_matrix: f32,
    stage_translation: f32,
    exact_stage_matrix: f32,
    exact_stage_translation: f32,
}

impl PairCheck {
    fn line(&self, name: &str) -> String {
        format!(
            "{name}: {} compared, largest difference {} in the matrix and {} in the translation; from the exact places {} and {}; a coordinate at the effect's place off by {} relative, {} in the form before; with the camera of the frame before {}, with no turn {}; the stage at its place {} and {}, from the exact places {} and {}; with the chain of the frame before {}",
            self.compared, self.matrix, self.translation, self.exact_matrix, self.exact_translation, self.relative_error, self.world_error, self.old_camera, self.no_turn, self.stage_matrix, self.stage_translation, self.exact_stage_matrix, self.exact_stage_translation, self.old_chain
        )
    }
}

/// The live check: the helper's projection against the engine's words after the billboard's
/// display, by the pair of the billboard's mode (the last row: a controller and no billboard) and
/// the coordinate mode, and the chains with more than one turning billboard apart.
#[derive(Default)]
pub struct TurnChecks {
    pairs: [[PairCheck; 5]; 4],
    nested: PairCheck,
    /// The frames with a comparison, and those whose camera differed from the frame before.
    frames: u64,
    moved_frames: u64,
    camera: Option<Camera>,
    previous: Option<Camera>,
    /// The cameras above at the place the engine stores.
    stored: Option<Camera>,
    previous_stored: Option<Camera>,
    origin: [f64; 3],
    /// The largest distance of the camera from the origin of the world.
    farthest: f64,
}

/// The largest difference of two texture matrices: the first three rows, the translation.
fn stage_difference(a: &[f32; 16], b: &[f32; 16]) -> (f32, f32) {
    let largest = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| (p - q).abs()).fold(0.0f32, f32::max);
    (largest(&a[..12], &b[..12]), largest(&a[12..15], &b[12..15]))
}

/// A coordinate from a stage at the effect's place: the matrix times the place, plus the
/// translation, in floats as the shader takes it.
fn coordinate_at(matrix: &[f32; 9], translation: &[f32; 3], place: &[f32; 3]) -> [f32; 3] {
    core::array::from_fn(|row| (0..3).map(|k| matrix[row * 3 + k] * place[k]).sum::<f32>() + translation[row])
}

impl TurnChecks {
    /// A frame's camera, with the place the engine stores for it, and the origin of the table,
    /// before its comparisons.
    pub fn next_frame(&mut self, camera: &Camera, stored_location: [f32; 3], origin: &[f64; 3]) {
        self.previous = self.camera;
        self.camera = Some(*camera);
        self.previous_stored = self.stored;
        self.stored = Some(Camera {
            location: stored_location.map(f64::from),
            ..*camera
        });
        self.origin = *origin;
        self.frames += 1;
        self.moved_frames += u64::from(self.previous.is_some_and(|previous| previous != *camera));
        self.farthest = self.farthest.max(camera.location.iter().map(|v| v * v).sum::<f64>().sqrt());
    }

    /// The projection of `chain` against the engine's words of this frame. `before` is its chain
    /// before the last record.
    pub fn compare(&mut self, chain: &Chain, before: &Chain, words: &EngineWords) {
        let (Some(camera), Some(stored_camera)) = (self.camera, self.stored) else {
            return;
        };
        let engine = &words.projection;
        let (matrix, translation) = difference(&chain.stored_projection(&stored_camera), engine);
        let (exact_matrix, exact_translation) = difference(&chain.exact_projection(&camera), engine);
        let old_camera = self
            .previous_stored
            .map_or(0.0, |previous| difference(&chain.stored_projection(&previous), engine).0);
        let no_turn = difference(&chain.unturned_projection(&stored_camera), engine).0;
        let old_chain = difference(&before.stored_projection(&stored_camera), engine).0;
        let (relative_error, world_error) = self.coordinate_errors(chain, &camera, engine);
        let (stage_matrix, stage_translation) =
            stage_difference(&chain.camera_stage_in(&stored_camera, Places::Stored), &words.stage);
        let (exact_stage_matrix, exact_stage_translation) = stage_difference(&chain.camera_stage(&camera), &words.stage);
        let pair = if chain.turning_billboards() > 1 {
            &mut self.nested
        } else {
            let (mode, coordinates) = (chain.last_turn() as usize, chain.coordinates as usize);
            let Some(pair) = self.pairs.get_mut(mode).and_then(|modes| modes.get_mut(coordinates)) else {
                return;
            };
            pair
        };
        pair.compared += 1;
        pair.matrix = pair.matrix.max(matrix);
        pair.translation = pair.translation.max(translation);
        pair.exact_matrix = pair.exact_matrix.max(exact_matrix);
        pair.exact_translation = pair.exact_translation.max(exact_translation);
        pair.relative_error = pair.relative_error.max(relative_error);
        pair.world_error = pair.world_error.max(world_error);
        pair.old_camera = pair.old_camera.max(old_camera);
        pair.no_turn = pair.no_turn.max(no_turn);
        pair.stage_matrix = pair.stage_matrix.max(stage_matrix);
        pair.stage_translation = pair.stage_translation.max(stage_translation);
        pair.exact_stage_matrix = pair.exact_stage_matrix.max(exact_stage_matrix);
        pair.exact_stage_translation = pair.exact_stage_translation.max(exact_stage_translation);
        pair.old_chain = pair.old_chain.max(old_chain);
    }

    /// The error of a coordinate at the effect's place, for the stage relative to the origin and
    /// for the form before it (the engine's world projection, with the origin added back to the
    /// relative place in floats, shader.wgsl before this change). Against the value in doubles.
    fn coordinate_errors(&self, chain: &Chain, camera: &Camera, engine: &Projection) -> (f32, f32) {
        if !takes_position(chain.coordinates) {
            return (0.0, 0.0);
        }
        let world = chain.effect_world(camera, Places::Exact, true);
        let relative: [f64; 3] = core::array::from_fn(|i| world.place[i] - self.origin[i]);
        let (matrix, translation) = chain.stage_projection(camera, &self.origin);
        let truth: [f64; 3] = core::array::from_fn(|row| {
            (0..3).map(|k| f64::from(matrix[row * 3 + k]) * relative[k]).sum::<f64>() + f64::from(translation[row])
        });
        let place = relative.map(|v| v as f32);
        let new_form = coordinate_at(&matrix, &translation, &place);
        let added: [f32; 3] = core::array::from_fn(|i| place[i] + self.origin[i] as f32);
        let old_form = coordinate_at(&engine.0, &engine.1, &added);
        let error = |form: [f32; 3]| (0..3).map(|i| (f64::from(form[i]) - truth[i]).abs()).fold(0.0f64, f64::max) as f32;
        (error(new_form), error(old_form))
    }

    /// The lines of the report; none without a comparison.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.frames == 0 {
            return lines;
        }
        lines.push(format!("effect turn check: {} frames, {} with another camera than the frame before; the camera at most {:.0} units from the origin of the world", self.frames, self.moved_frames, self.farthest));
        for (mode, modes) in self.pairs.iter().enumerate() {
            let billboard = if mode as u32 == NO_TURN {
                "no billboard".to_string()
            } else {
                format!("billboard mode {mode}")
            };
            for (coordinates, pair) in modes.iter().enumerate() {
                lines.push(pair.line(&format!("effect turn check: {billboard}, coordinate mode {coordinates}")));
            }
        }
        lines.push(self.nested.line("effect turn check: two turning billboards"));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vectors of `tools/effect_turn_vectors.lua`: the engine's own functions on made objects.
    const VECTORS: &str = include_str!("effect_turn_vectors.txt");

    fn floats(text: &str) -> Vec<f32> {
        text.split_whitespace()
            .map(|word| f32::from_bits(u32::from_str_radix(word, 16).unwrap()))
            .collect()
    }

    fn transform(v: &[f32]) -> Transform {
        Transform {
            rotation: v[..9].try_into().unwrap(),
            translation: v[9..12].try_into().unwrap(),
            scale: v[12],
        }
    }

    /// A case of the file: its name, the chain, the camera, the engine's world of the effect and
    /// its projection.
    struct Case {
        name: String,
        chain: Chain,
        camera: Camera,
        world: Transform,
        matrix: [f32; 9],
        translation: [f32; 3],
    }

    /// A line is `name|coordinate mode|kinds|root|the locals|model|camera|world|projection`, each
    /// float as the eight hex digits of its word.
    fn cases() -> Vec<Case> {
        VECTORS
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                let kinds: Vec<u32> = parts[2].split_whitespace().map(|k| k.parse().unwrap()).collect();
                let locals = floats(parts[4]);
                let model = floats(parts[5]);
                let camera = floats(parts[6]);
                let projection = floats(parts[8]);
                let root = transform(&floats(parts[3]));
                Case {
                    name: parts[0].to_string(),
                    chain: Chain {
                        coordinates: parts[1].parse().unwrap(),
                        root,
                        root_exact: root.translation.map(f64::from),
                        links: kinds
                            .iter()
                            .enumerate()
                            .map(|(i, &kind)| Link {
                                kind,
                                local: transform(&locals[i * TRANSFORM_WORDS..]),
                            })
                            .collect(),
                        model_matrix: model[..9].try_into().unwrap(),
                        model_translation: model[9..12].try_into().unwrap(),
                    },
                    camera: Camera {
                        direction: camera[..3].try_into().unwrap(),
                        up: camera[3..6].try_into().unwrap(),
                        right: camera[6..9].try_into().unwrap(),
                        location: [camera[9], camera[10], camera[11]].map(f64::from),
                    },
                    world: transform(&floats(parts[7])),
                    matrix: projection[..9].try_into().unwrap(),
                    translation: projection[9..12].try_into().unwrap(),
                }
            })
            .collect()
    }

    fn same_words(name: &str, what: &str, got: &[f32], want: &[f32]) {
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            assert_eq!(g.to_bits(), w.to_bits(), "{name}: {what}[{i}] is {g}, the engine has {w}");
        }
    }

    #[test]
    fn the_stored_maths_gives_the_engines_words() {
        let cases = cases();
        assert!(cases.len() >= 100, "{} cases", cases.len());
        for case in &cases {
            let world = case.chain.effect_world(&case.camera, Places::Stored, true);
            same_words(&case.name, "world rotation", &world.rotation, &case.world.rotation);
            let place = world.place.map(|v| v as f32);
            same_words(&case.name, "world translation", &place, &case.world.translation);
            same_words(&case.name, "world scale", &[world.scale], &[case.world.scale]);
            let (matrix, translation) = case.chain.stored_projection(&case.camera);
            same_words(&case.name, "matrix", &matrix, &case.matrix);
            same_words(&case.name, "translation", &translation, &case.translation);
        }
    }

    #[test]
    fn the_file_holds_every_pair_and_both_identity_branches() {
        let cases = cases();
        for mode in 0..3 {
            for coordinates in 0..5 {
                assert!(
                    cases
                        .iter()
                        .any(|c| c.chain.coordinates == coordinates && c.chain.last_turn() == mode),
                    "no case of billboard mode {mode} and coordinate mode {coordinates}"
                );
            }
        }
        for name in ["identity facing", "identity about y", "nested"] {
            assert!(cases.iter().any(|c| c.name.starts_with(name)), "no case {name}");
        }
    }

    /// An identity branch leaves the rotation as it was: the file's cases show that the engine
    /// took the branch, and the maths takes it too.
    #[test]
    fn an_identity_branch_does_not_turn() {
        for case in cases().iter().filter(|c| c.name.starts_with("identity")) {
            let world = case.chain.effect_world(&case.camera, Places::Stored, true);
            let plain = case.chain.effect_world(&case.camera, Places::Stored, false);
            assert_eq!(world.rotation, plain.rotation, "{}", case.name);
            assert_eq!(case.world.rotation, plain.rotation, "{} in the engine", case.name);
        }
    }

    /// The null test of the file: the same cases with no turn differ from the engine's values,
    /// so a test of this file can show a missing turn.
    #[test]
    fn without_the_turn_the_values_differ() {
        let cases = cases();
        let turning: Vec<&Case> = cases
            .iter()
            .filter(|c| !c.name.starts_with("identity") && c.chain.last_turn() != NO_TURN)
            .collect();
        assert!(!turning.is_empty());
        let same = turning
            .iter()
            .filter(|c| difference(&c.chain.unturned_projection(&c.camera), &(c.matrix, c.translation)).0 <= 1.0e-6);
        // A case with the camera along the node's own axes can turn by nothing.
        assert!(same.count() * 10 < turning.len());
    }

    /// The value in doubles of a stage at a place of the world.
    fn stage_value(matrix: &[f32; 9], translation: &[f32; 3], place: &[f64; 3]) -> [f64; 3] {
        core::array::from_fn(|row| {
            (0..3).map(|k| f64::from(matrix[row * 3 + k]) * place[k]).sum::<f64>() + f64::from(translation[row])
        })
    }

    /// Moving the origin moves the stage's translation and leaves the coordinate of a place of
    /// the world, up to the float step of the translation: a rebase changes no picture.
    #[test]
    fn a_stage_relative_to_the_origin_gives_the_same_coordinate() {
        let origin = [41000.0, -60000.0, 900.0];
        for case in cases().iter().filter(|c| takes_position(c.chain.coordinates)) {
            let absolute = case.chain.stage_projection(&case.camera, &[0.0; 3]);
            let relative = case.chain.stage_projection(&case.camera, &origin);
            assert_eq!(absolute.0, relative.0, "{}", case.name);
            let place = [41012.5, -60021.25, 925.75];
            let from_origin: [f64; 3] = core::array::from_fn(|i| place[i] - origin[i]);
            let a = stage_value(&absolute.0, &absolute.1, &place);
            let b = stage_value(&relative.0, &relative.1, &from_origin);
            for i in 0..3 {
                // Each translation is rounded to a float step of its own size.
                let size = absolute.1[i].abs().max(relative.1[i].abs()).max(1.0e-3);
                let step = f64::from(f32::from_bits(size.to_bits() + 1) - size);
                assert!((a[i] - b[i]).abs() <= step * 2.0, "{}: {} against {}", case.name, a[i], b[i]);
            }
        }
    }

    /// A map that takes a reflection or a normal has no place in it: the origin does not move it.
    #[test]
    fn a_map_without_the_position_stays_as_it_is() {
        for case in cases().iter().filter(|c| !takes_position(c.chain.coordinates)) {
            assert_eq!(
                case.chain.stage_projection(&case.camera, &[0.0; 3]),
                case.chain.stage_projection(&case.camera, &[100000.0, 2000.0, 3.0]),
                "{}",
                case.name
            );
        }
    }

    /// `T' = T + P origin` for an effect that does not move.
    #[test]
    fn a_plain_stage_moves_with_the_origin() {
        let matrix = [1.0 / 256.0, 0.0, 0.0, 0.0, 1.0 / 256.0, 0.0, 0.0, 0.0, 1.0 / 512.0];
        let translation = [0.5, 0.25, 1.0];
        let moved = plain_stage(0, &matrix, &translation, &[2560.0, -256.0, 512.0]);
        assert_eq!([moved[12], moved[13], moved[14]], [10.5, -0.75, 2.0]);
        assert_eq!(moved[0], matrix[0]);
        // A sphere map keeps its translation.
        let sphere = plain_stage(2, &matrix, &translation, &[2560.0, -256.0, 512.0]);
        assert_eq!([sphere[12], sphere[13], sphere[14]], translation);
    }

    /// The stage of a draw at its place, with its translation, takes a position of camera space to
    /// the value that the engine's world projection gives the same place of the world. A wrong origin
    /// fails here.
    #[test]
    fn the_stage_of_camera_space_gives_the_engines_coordinate() {
        for case in cases().iter().filter(|c| takes_position(c.chain.coordinates)) {
            let stage = case.chain.camera_stage_in(&case.camera, Places::Stored);
            let v = [12.5f64, -7.25, 180.0];
            let axes = [case.camera.right, case.camera.up, case.camera.direction];
            let place: [f64; 3] = core::array::from_fn(|k| {
                (0..3).map(|a| f64::from(axes[a][k]) * v[a]).sum::<f64>() + case.camera.location[k]
            });
            let want = stage_value(&case.matrix, &case.translation, &place);
            for row in 0..3 {
                let got: f64 =
                    (0..3).map(|a| v[a] * f64::from(stage[a * 4 + row])).sum::<f64>() + f64::from(stage[12 + row]);
                // Each side has a translation rounded to its float step.
                let size = case.translation[row].abs().max(stage[12 + row].abs()).max(1.0);
                let step = f64::from(f32::from_bits(size.to_bits() + 1) - size);
                assert!(
                    (got - want[row]).abs() <= step * 3.0 + 1.0e-3,
                    "{}: {got} against {}",
                    case.name,
                    want[row]
                );
            }
        }
    }

    /// The stage of a draw at its place takes vectors of camera space: it gives the same value as
    /// the projection of the same vector in the world, as the engine's callbacks do.
    #[test]
    fn the_stage_of_camera_space_is_the_projection_of_the_world() {
        for case in cases().iter().filter(|c| !c.name.starts_with("identity")) {
            let stage = case.chain.camera_stage(&case.camera);
            let (matrix, _) = case.chain.stage_projection(&case.camera, &case.camera.location);
            // A vector in camera space, then the same vector in the world.
            let v = [0.6f64, -0.3, 0.74];
            let axes = [case.camera.right, case.camera.up, case.camera.direction];
            let world: [f64; 3] = core::array::from_fn(|k| (0..3).map(|a| f64::from(axes[a][k]) * v[a]).sum());
            for row in 0..3 {
                let from_stage: f64 = (0..3).map(|a| v[a] * f64::from(stage[a * 4 + row])).sum();
                let from_world: f64 = (0..3).map(|k| f64::from(matrix[row * 3 + k]) * world[k]).sum();
                assert!(
                    (from_stage - from_world).abs() < 1.0e-5,
                    "{}: {from_stage} against {from_world}",
                    case.name
                );
            }
        }
    }

    /// The null test of the live check's two forms: far from the origin of the world the form with
    /// the origin added back in floats is off, and the relative stage is not.
    #[test]
    fn far_from_the_origin_the_form_before_is_off() {
        let case = cases()
            .into_iter()
            .find(|c| c.chain.coordinates == 0 && c.chain.last_turn() == 0)
            .unwrap();
        let mut chain = case.chain;
        chain.root_exact = [250000.5, -310000.25, 1200.125];
        chain.root.translation = chain.root_exact.map(|v| v as f32);
        let camera = Camera {
            location: [250100.0, -310100.0, 1300.0],
            ..case.camera
        };
        let mut checks = TurnChecks::default();
        checks.next_frame(&camera, camera.location.map(|v| v as f32), &camera.location);
        let engine = chain.stored_projection(&Camera {
            location: camera.location.map(|v| f64::from(v as f32)),
            ..camera
        });
        let (relative, world) = checks.coordinate_errors(&chain, &camera, &engine);
        assert!(relative < 1.0e-4, "the relative stage is off by {relative}");
        assert!(
            world > relative * 2.0,
            "the form before is off by {world}, the relative stage by {relative}"
        );
    }

    #[test]
    fn a_record_is_read_and_a_short_one_refused() {
        let mut words = vec![7u32, 2, 1];
        words.extend(core::iter::repeat_n(1.0f32.to_bits(), TRANSFORM_WORDS));
        for value in [10.5f64, -20.25, 30.125] {
            words.push(value.to_bits() as u32);
            words.push((value.to_bits() >> 32) as u32);
        }
        words.push(1);
        words.extend(core::iter::repeat_n(2.0f32.to_bits(), TRANSFORM_WORDS));
        words.extend(core::iter::repeat_n(3.0f32.to_bits(), MODEL_WORDS));
        let (id, chain) = Chain::parse(&words).unwrap();
        assert_eq!((id, chain.coordinates, chain.links.len(), chain.links[0].kind), (7, 2, 1, 1));
        assert_eq!(chain.root.scale, 1.0);
        assert_eq!(chain.root_exact, [10.5, -20.25, 30.125]);
        assert_eq!(chain.links[0].local.scale, 2.0);
        assert_eq!(chain.model_translation, [3.0; 3]);
        assert!(Chain::parse(&words[..words.len() - 1]).is_none());
        let mut many = words.clone();
        many[2] = u32::MAX;
        assert!(Chain::parse(&many).is_none());
    }

    #[test]
    fn the_camera_comes_from_the_columns_of_the_view() {
        let view = [1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0, 0.0, 7.0, 8.0, 9.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let camera = Camera::of_view(&view, [10.0, 20.0, 30.0]);
        assert_eq!(camera.right, [1.0, 4.0, 7.0]);
        assert_eq!(camera.up, [2.0, 5.0, 8.0]);
        assert_eq!(camera.direction, [3.0, 6.0, 9.0]);
        assert_eq!(camera.location, [10.0, 20.0, 30.0]);
    }

    #[test]
    fn the_texture_matrix_has_the_columns_in_rows() {
        let m = texture_matrix(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], &[10.0, 11.0, 12.0]);
        assert_eq!(
            m,
            [1.0, 4.0, 7.0, 0.0, 2.0, 5.0, 8.0, 0.0, 3.0, 6.0, 9.0, 0.0, 10.0, 11.0, 12.0, 0.0]
        );
    }
}
