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
}

/// The world variant of a shader view: the view itself for a view with code in the world shader
/// (lights, shadow layers), else [`view_id::NONE`]. A draw of the world takes the variant of its
/// frame, and the world shader's text names the views of the variants (`VIEW_LIGHTS`,
/// `VIEW_SHADOW_LAYERS` in shader.wgsl).
pub fn world_variant(view: u32) -> u32 {
    match view {
        view_id::LIGHTS | view_id::SHADOW_LAYERS => view,
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

    #[test]
    fn extension_bits_are_kept() {
        let sources = LineSources::from_bits(1 << 16 | 2);
        assert_eq!(sources.bits(), 1 << 16 | 2);
        assert_eq!(sources.names().collect::<Vec<_>>(), ["shadow-boxes"]);
    }

    /// Only the views with code in the world shader make a world variant, and the world shader
    /// names them with the ids here.
    #[test]
    fn the_world_variants_are_the_lights_and_the_shadow_layers() {
        assert_eq!(world_variant(view_id::LIGHTS), view_id::LIGHTS);
        assert_eq!(world_variant(view_id::SHADOW_LAYERS), view_id::SHADOW_LAYERS);
        for view in [view_id::NONE, view_id::SHADOW_MAP, view_id::WATER, view_id::POST, 99] {
            assert_eq!(world_variant(view), view_id::NONE, "view {view}");
        }
        let shader = include_str!("shader.wgsl");
        assert!(shader.contains(&format!("const VIEW_LIGHTS: u32 = {}u;", view_id::LIGHTS)));
        assert!(shader.contains(&format!("const VIEW_SHADOW_LAYERS: u32 = {}u;", view_id::SHADOW_LAYERS)));
    }
}
