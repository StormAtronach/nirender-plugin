//! The line sources of the debug views: the set of shape kinds that the line pass draws on top
//! of the frame (the lights, the shadow boxes, the bounds, the shapes that Lua sends). A source
//! is one bit, so any number of sources can be on together and beside a shader view.
//!
//! The crate and the program that draws more through [`crate::Extension`] share this type:
//! bits 0 to 15 are named here; bits 16 and up are free for the extension's own sources. The
//! names are the words of the tune `debug lines <name> on|off` and of the replay argument
//! `--debug-lines <name,name>`.
//!
//! The ids of the shader views ([`view_id`]) are here too: the extension owns the views and
//! gives the id of the frame's view in [`crate::FrameParameters::debug_view`].

use core::fmt;

/// The ids of the shader views of the debug views. At most one is on in a frame.
pub mod view_id {
    pub const NONE: u32 = 0;
    pub const LIGHTS: u32 = 1;
    pub const SHADOW_LAYERS: u32 = 2;
    pub const SHADOW_MAP: u32 = 3;
    pub const WATER: u32 = 4;
    pub const POST: u32 = 5;
    /// Each draw writes the part of the fog that it applied, as grey (W441).
    pub const FOG: u32 = 6;
}

/// The world variant of a shader view: the view itself for a view with code in the world shader
/// (lights, shadow layers, fog), else [`view_id::NONE`]. A draw of the world takes the variant of
/// its frame, and the world shader's text names the views of the variants (`VIEW_LIGHTS`,
/// `VIEW_SHADOW_LAYERS`, `VIEW_FOG` in shader.wgsl).
pub fn world_variant(view: u32) -> u32 {
    match view {
        view_id::LIGHTS | view_id::SHADOW_LAYERS | view_id::FOG => view,
        _ => view_id::NONE,
    }
}

/// A set of line sources, one bit for each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineSources(u32);

impl LineSources {
    /// No source.
    pub const NONE: Self = Self(0);
    /// The place and the radius of each light.
    pub const LIGHTS: Self = Self(1);
    /// The boxes of the shadow layers.
    pub const SHADOW_BOXES: Self = Self(2);
    /// The bounds of the particle systems and of the instances.
    pub const BOUNDS: Self = Self(4);
    /// The shapes that a mod or a test sends from Lua.
    pub const LUA: Self = Self(8);

    /// Each named source with its name.
    const NAMED: [(Self, &'static str); 4] = [
        (Self::LIGHTS, "lights"),
        (Self::SHADOW_BOXES, "shadow-boxes"),
        (Self::BOUNDS, "bounds"),
        (Self::LUA, "lua"),
    ];

    /// The set of these bits. Bits without a name here are kept: they are the extension's.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The bits of the set, the extension's too.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// True when no source is on.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every source of `other` is in this set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Turns the sources of `other` on or off.
    pub fn set(&mut self, other: Self, on: bool) {
        if on {
            self.0 |= other.0;
        } else {
            self.0 &= !other.0;
        }
    }

    /// The source of a name, or `None` for a name that this crate does not know.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::NAMED.iter().find(|(_, n)| *n == name).map(|(source, _)| *source)
    }

    /// The names of the named sources in this set, in the order of their bits.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMED
            .into_iter()
            .filter(move |(source, _)| self.contains(*source))
            .map(|(_, name)| name)
    }

    /// The set of a list of names separated by commas (the replay argument). An empty list
    /// is no source. The error names the first word that is not a source.
    pub fn parse_list(list: &str) -> Result<Self, UnknownSource> {
        let mut sources = Self::NONE;
        for word in list.split(',').map(str::trim).filter(|word| !word.is_empty()) {
            let source = Self::from_name(word).ok_or_else(|| UnknownSource(word.to_owned()))?;
            sources.set(source, true);
        }
        Ok(sources)
    }
}

/// A word that names no line source, with the names that are known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownSource(pub String);

impl fmt::Display for UnknownSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let known: Vec<&str> = LineSources::NAMED.iter().map(|(_, name)| *name).collect();
        write!(f, "no line source is named {:?} (the sources: {})", self.0, known.join(", "))
    }
}

impl std::error::Error for UnknownSource {}

/// Where the line pass draws a segment: behind what the frame drew at its pixel or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineGroup {
    /// The segment is hidden where the depth of the frame is nearer than it.
    Tested,
    /// The segment is drawn over everything.
    OnTop,
}

/// One segment of the line pass: two places in the world space of the game (64-bit, as the
/// helper keeps its world places), a colour (red, green, blue, alpha) and its group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: [f64; 3],
    pub b: [f64; 3],
    pub color: [u8; 4],
    pub group: LineGroup,
}

/// The segments of a box.
pub const BOX_SEGMENTS: usize = 12;
/// The segments of one circle of a sphere.
pub const CIRCLE_SEGMENTS: usize = 16;
/// The segments of a sphere: three circles, one in each plane of the axes.
pub const SPHERE_SEGMENTS: usize = 3 * CIRCLE_SEGMENTS;

/// The one list of the line pass for a frame. Each source adds its segments to it; the pass
/// knows only segments. A box, a sphere and the box of a projection are functions that add
/// segments.
#[derive(Clone, Debug, Default)]
pub struct LineList {
    segments: Vec<Segment>,
}

impl LineList {
    /// The segments of the frame so far.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// The number of segments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// True when the list has no segment.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Takes out every segment, for the next frame.
    pub fn clear(&mut self) {
        self.segments.clear();
    }

    /// Adds one segment.
    pub fn line(&mut self, a: [f64; 3], b: [f64; 3], color: [u8; 4], group: LineGroup) {
        self.segments.push(Segment { a, b, color, group });
    }

    /// Adds the 12 edges of a box of 8 corners. The index of a corner has a bit for each axis
    /// of the box: 1 the first, 2 the second, 4 the third (a corner and the corner with one
    /// bit more are the ends of an edge).
    pub fn box_corners(&mut self, corners: &[[f64; 3]; 8], color: [u8; 4], group: LineGroup) {
        for from in 0..8usize {
            for bit in [1usize, 2, 4] {
                if from & bit == 0 {
                    self.line(corners[from], corners[from | bit], color, group);
                }
            }
        }
    }

    /// Adds a box with its edges on the axes of the world, from its least and its most corner.
    pub fn aligned_box(&mut self, least: [f64; 3], most: [f64; 3], color: [u8; 4], group: LineGroup) {
        let corners = core::array::from_fn(|i| {
            core::array::from_fn(|axis| if i & (1 << axis) == 0 { least[axis] } else { most[axis] })
        });
        self.box_corners(&corners, color, group);
    }

    /// Adds a sphere as three circles of [`CIRCLE_SEGMENTS`] segments, one in each plane of the
    /// axes of the world.
    pub fn sphere(&mut self, centre: [f64; 3], radius: f64, color: [u8; 4], group: LineGroup) {
        let point = |plane: usize, step: usize| -> [f64; 3] {
            let angle = core::f64::consts::TAU * step as f64 / CIRCLE_SEGMENTS as f64;
            let (sin, cos) = angle.sin_cos();
            let mut place = centre;
            place[plane] += radius * cos;
            place[(plane + 1) % 3] += radius * sin;
            place
        };
        for plane in 0..3 {
            for step in 0..CIRCLE_SEGMENTS {
                self.line(point(plane, step), point(plane, step + 1), color, group);
            }
        }
    }

    /// Adds the box that a projection draws into: the corners of its clip space (x and y from
    /// -1 to 1, the depth from 0 to 1, as Direct3D has it) taken back into the world through
    /// the inverse of `view_projection` (the layout of Direct3D: a position is a row vector,
    /// `position * view_projection`). False, and nothing added, when the matrix has no inverse.
    pub fn projection_box(&mut self, view_projection: &[f32; 16], color: [u8; 4], group: LineGroup) -> bool {
        let Some(inverse) = inverse(view_projection) else {
            return false;
        };
        let mut corners = [[0.0f64; 3]; 8];
        for (i, corner) in corners.iter_mut().enumerate() {
            let clip = [
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { 0.0 } else { 1.0 },
                1.0,
            ];
            let world: [f64; 4] =
                core::array::from_fn(|column| (0..4).map(|row| clip[row] * inverse[row * 4 + column]).sum());
            if world[3].abs() < 1e-12 {
                return false;
            }
            *corner = [world[0] / world[3], world[1] / world[3], world[2] / world[3]];
        }
        self.box_corners(&corners, color, group);
        true
    }
}

/// The inverse of a 4 by 4 matrix (16 numbers, row after row), in 64-bit, or `None` when it
/// has none.
fn inverse(m: &[f32; 16]) -> Option<[f64; 16]> {
    let a: [f64; 16] = m.map(f64::from);
    let at = |row: usize, column: usize| a[row * 4 + column];
    let s0 = at(0, 0) * at(1, 1) - at(1, 0) * at(0, 1);
    let s1 = at(0, 0) * at(1, 2) - at(1, 0) * at(0, 2);
    let s2 = at(0, 0) * at(1, 3) - at(1, 0) * at(0, 3);
    let s3 = at(0, 1) * at(1, 2) - at(1, 1) * at(0, 2);
    let s4 = at(0, 1) * at(1, 3) - at(1, 1) * at(0, 3);
    let s5 = at(0, 2) * at(1, 3) - at(1, 2) * at(0, 3);
    let c5 = at(2, 2) * at(3, 3) - at(3, 2) * at(2, 3);
    let c4 = at(2, 1) * at(3, 3) - at(3, 1) * at(2, 3);
    let c3 = at(2, 1) * at(3, 2) - at(3, 1) * at(2, 2);
    let c2 = at(2, 0) * at(3, 3) - at(3, 0) * at(2, 3);
    let c1 = at(2, 0) * at(3, 2) - at(3, 0) * at(2, 2);
    let c0 = at(2, 0) * at(3, 1) - at(3, 0) * at(2, 1);
    let determinant = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
    if determinant.abs() < 1e-30 {
        return None;
    }
    let d = 1.0 / determinant;
    Some([
        (at(1, 1) * c5 - at(1, 2) * c4 + at(1, 3) * c3) * d,
        (-at(0, 1) * c5 + at(0, 2) * c4 - at(0, 3) * c3) * d,
        (at(3, 1) * s5 - at(3, 2) * s4 + at(3, 3) * s3) * d,
        (-at(2, 1) * s5 + at(2, 2) * s4 - at(2, 3) * s3) * d,
        (-at(1, 0) * c5 + at(1, 2) * c2 - at(1, 3) * c1) * d,
        (at(0, 0) * c5 - at(0, 2) * c2 + at(0, 3) * c1) * d,
        (-at(3, 0) * s5 + at(3, 2) * s2 - at(3, 3) * s1) * d,
        (at(2, 0) * s5 - at(2, 2) * s2 + at(2, 3) * s1) * d,
        (at(1, 0) * c4 - at(1, 1) * c2 + at(1, 3) * c0) * d,
        (-at(0, 0) * c4 + at(0, 1) * c2 - at(0, 3) * c0) * d,
        (at(3, 0) * s4 - at(3, 1) * s2 + at(3, 3) * s0) * d,
        (-at(2, 0) * s4 + at(2, 1) * s2 - at(2, 3) * s0) * d,
        (-at(1, 0) * c3 + at(1, 1) * c1 - at(1, 2) * c0) * d,
        (at(0, 0) * c3 - at(0, 1) * c1 + at(0, 2) * c0) * d,
        (-at(3, 0) * s3 + at(3, 1) * s1 - at(3, 2) * s0) * d,
        (at(2, 0) * s3 - at(2, 1) * s1 + at(2, 2) * s0) * d,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for (source, name) in LineSources::NAMED {
            assert_eq!(LineSources::from_name(name), Some(source));
            assert_eq!(source.names().collect::<Vec<_>>(), [name]);
        }
        assert_eq!(LineSources::from_name("Lights"), None);
    }

    #[test]
    fn set_and_contains() {
        let mut sources = LineSources::NONE;
        assert!(sources.is_empty());
        sources.set(LineSources::LIGHTS, true);
        sources.set(LineSources::BOUNDS, true);
        assert!(sources.contains(LineSources::LIGHTS));
        assert!(!sources.contains(LineSources::SHADOW_BOXES));
        assert!(!sources.contains(LineSources::from_bits(LineSources::LIGHTS.bits() | LineSources::LUA.bits())));
        sources.set(LineSources::LIGHTS, false);
        assert_eq!(sources, LineSources::BOUNDS);
    }

    #[test]
    fn parse_list() {
        assert_eq!(LineSources::parse_list(""), Ok(LineSources::NONE));
        assert_eq!(LineSources::parse_list("lights, lua").map(LineSources::bits), Ok(1 | 8));
        assert_eq!(LineSources::parse_list("lights,lights").map(LineSources::bits), Ok(1));
        assert_eq!(
            LineSources::parse_list("lights,wires"),
            Err(UnknownSource("wires".to_owned()))
        );
    }

    /// The plugin counts the segments of the Lua shapes before the stream with the numbers of
    /// the record's file; they are the numbers of the shapes here.
    #[test]
    fn the_record_counts_the_segments_of_the_shapes() {
        use crate::debug_shapes_format::Kind;
        let mut list = LineList::default();
        list.line([0.0; 3], [1.0; 3], RED, LineGroup::Tested);
        assert_eq!(Kind::Line.segments() as usize, list.len());
        assert_eq!(Kind::Box.segments() as usize, BOX_SEGMENTS);
        assert_eq!(Kind::Sphere.segments() as usize, SPHERE_SEGMENTS);
    }

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn length(segment: &Segment) -> f64 {
        (0..3).map(|i| (segment.b[i] - segment.a[i]).powi(2)).sum::<f64>().sqrt()
    }

    /// A box has 12 edges, 4 along each axis, each the length of the box on that axis.
    #[test]
    fn a_box_has_twelve_edges_of_its_sides() {
        let mut list = LineList::default();
        list.aligned_box([0.0, 0.0, 0.0], [1.0, 2.0, 4.0], RED, LineGroup::Tested);
        assert_eq!(list.len(), BOX_SEGMENTS);
        let mut lengths: Vec<f64> = list.segments().iter().map(length).collect();
        lengths.sort_by(f64::total_cmp);
        assert_eq!(lengths, [1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 4.0, 4.0, 4.0, 4.0]);
        assert!(list.segments().iter().all(|s| s.color == RED && s.group == LineGroup::Tested));
    }

    /// A sphere is 48 segments, each end on the sphere, far from the origin too.
    #[test]
    fn a_sphere_is_three_circles_on_its_radius() {
        let mut list = LineList::default();
        let centre = [150_000.5, -200_000.25, 1000.0];
        list.sphere(centre, 512.0, RED, LineGroup::OnTop);
        assert_eq!(list.len(), SPHERE_SEGMENTS);
        for segment in list.segments() {
            for end in [segment.a, segment.b] {
                let radius = (0..3).map(|i| (end[i] - centre[i]).powi(2)).sum::<f64>().sqrt();
                assert!((radius - 512.0).abs() < 1e-6, "radius {radius}");
            }
        }
        list.clear();
        assert!(list.is_empty());
    }

    /// The inverse times the matrix is the identity.
    #[test]
    fn the_inverse_undoes_the_matrix() {
        let m: [f32; 16] = [
            2.0, 0.5, 0.0, 1.0, -1.0, 3.0, 0.25, 0.0, 0.0, 1.0, 4.0, -2.0, 5.0, -3.0, 1.0, 1.0,
        ];
        let inv = inverse(&m).expect("an inverse");
        for row in 0..4 {
            for column in 0..4 {
                let value: f64 = (0..4).map(|k| f64::from(m[row * 4 + k]) * inv[k * 4 + column]).sum();
                let identity = if row == column { 1.0 } else { 0.0 };
                assert!((value - identity).abs() < 1e-9, "({row}, {column}) {value}");
            }
        }
        assert_eq!(inverse(&[0.0; 16]), None);
    }

    /// The box of an orthographic projection is the box that it draws: here x from -100 to
    /// 100, y from -50 to 50 and the depth from 10 to 1010 along -z (a view of the identity,
    /// `D3DXMatrixOrthoRH`, so the depth is 0 at z = -10 and 1 at z = -1010).
    #[test]
    fn the_box_of_an_orthographic_projection_is_its_volume() {
        let (width, height, near, far) = (200.0f32, 100.0f32, 10.0f32, 1010.0f32);
        let mut m = [0.0f32; 16];
        m[0] = 2.0 / width;
        m[5] = 2.0 / height;
        m[10] = 1.0 / (near - far);
        m[14] = near / (near - far);
        m[15] = 1.0;
        let mut list = LineList::default();
        assert!(list.projection_box(&m, RED, LineGroup::OnTop));
        assert_eq!(list.len(), BOX_SEGMENTS);
        let mut ends: Vec<[f64; 3]> = list.segments().iter().flat_map(|s| [s.a, s.b]).collect();
        for end in &mut ends {
            *end = end.map(|v| (v * 1000.0).round() / 1000.0);
        }
        for x in [-100.0, 100.0] {
            for y in [-50.0, 50.0] {
                for z in [-10.0, -1010.0] {
                    assert!(ends.contains(&[x, y, z]), "corner {x} {y} {z}");
                }
            }
        }
        assert!(!list.projection_box(&[0.0; 16], RED, LineGroup::OnTop));
        assert_eq!(list.len(), BOX_SEGMENTS, "a matrix with no inverse adds nothing");
    }

    #[test]
    fn extension_bits_are_kept() {
        let sources = LineSources::from_bits(1 << 16 | 2);
        assert_eq!(sources.bits(), 1 << 16 | 2);
        assert_eq!(sources.names().collect::<Vec<_>>(), ["shadow-boxes"]);
    }

    /// Only the views with code in the world shader make a world variant, and the world shader
    /// names them with the ids here.
    #[test]
    fn the_world_variants_are_the_lights_the_shadow_layers_and_the_fog() {
        assert_eq!(world_variant(view_id::LIGHTS), view_id::LIGHTS);
        assert_eq!(world_variant(view_id::SHADOW_LAYERS), view_id::SHADOW_LAYERS);
        assert_eq!(world_variant(view_id::FOG), view_id::FOG);
        for view in [view_id::NONE, view_id::SHADOW_MAP, view_id::WATER, view_id::POST, 99] {
            assert_eq!(world_variant(view), view_id::NONE, "view {view}");
        }
        let shader = include_str!("shader.wgsl");
        assert!(shader.contains(&format!("const VIEW_LIGHTS: u32 = {}u;", view_id::LIGHTS)));
        assert!(shader.contains(&format!("const VIEW_SHADOW_LAYERS: u32 = {}u;", view_id::SHADOW_LAYERS)));
        assert!(shader.contains(&format!("const VIEW_FOG: u32 = {}u;", view_id::FOG)));
    }
}
