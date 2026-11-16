//! A fragment shader for each stage state of the draws (W293, option E2 of W279). The world
//! shader's `fs` interprets the Direct3D stages of a draw for each pixel: it loads the codes
//! of each stage from the draw's record and switches on them. A stage key holds the codes that
//! `fs` switches on (the stage count, the colour and alpha operations and their arguments, the
//! alpha test function), and the facts of the draw that choose the branches of `fs` and of the
//! extension's shade (the bits of `counts.w` in [`SHADE_BITS`], the lighting and normal flags,
//! whether the draw takes the scene's values, whether it adds to the picture, whether it is
//! flat-shaded, and whether it has a plane). For each key
//! the helper makes a fragment shader from the world shader's own text, with those codes and
//! facts as constants and the reads of the stages past the count left out. The arithmetic is the
//! same; the switches, the branches that the key's draws cannot take and the record loads go.
//!
//! The world shader stays as the uber-shader: it draws a key until that key's pipeline is made,
//! when a pipeline fails, past the cap on the keys, and with `MGE_XE_STAGE_SHADERS=0`. It is
//! also the reference of the look test (W293).
//!
//! The uber-shader draws the same picture, so a rewrite or a pipeline that fails shows in no
//! picture: only the speed goes. Three things make it loud (W523). The rewrite searches the
//! parts that it needs one time in the crate's own text only, so the text of an extension or of
//! a mod's file cannot make it fail ([`fragment_source`]), and it names the part that refused
//! ([`RewriteRefusal`]). The report counts the failures and the draws on the uber-shader by
//! cause ([`StageShaders::report`]). And a replay, which makes each pipeline at its first use,
//! ends with an exit code of its own when it has one of them ([`StageShaders::replay_fault`]); a
//! live session only reports them.
//!
//! A live helper makes the pipelines on a thread of its own, so a new key costs a few frames
//! of the uber-shader and not a hitch [I: wgpu's own locks can still hold the frame]. A
//! replay makes them at once, so that its frames are the same from the first. The pipelines
//! of a live session go to a list in the user's folder (`%LOCALAPPDATA%\MGE XE`, never the
//! game folder), one file for each world shader and format of the list. The next start makes
//! them in its first frames, behind the keys of first use, and the list's keys take at most half
//! of the cap. A list that cannot be read, that a build with another world shader wrote, or that
//! cannot be written, leaves each key to its first use, with a line in the log.
//!
//! A live start with no list sends the uber-shader's own pipelines of a fixed set ([`UBER_SET`]) to
//! the thread. They are the keys of the uber-shader that the frame thread made in a start with no
//! list. A start with a list does not send them: the list's pipelines go to the thread only while
//! it has few jobs, so the set would hold them back. The frame thread takes such a pipeline at the key's first draw, and makes a key of the
//! set itself only when the thread has not given it back yet. The report counts both.
//!
//! A debug view with code in the world shader ([`crate::debug_lines::world_variant`]) draws
//! with a variant of the world shader's text. A module is kept by its stage key and its variant,
//! so a draw of the view never takes a module of normal play, or the reverse. The pipelines of
//! a variant do not go to the list, and a change of the variant drops them.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc;

use crate::data::{DrawData, PipelineKey};

/// The most stage keys of a session; a draw of a key past them takes the uber-shader.
pub(crate) const MAX_KEYS: usize = 256;
const STAGES: usize = 6;
/// The first line of the list, then [`LIST_FORMAT`] and the hash of the world shader's text.
const LIST_HEADER: &str = "numidium stage pipelines";
/// The format of the list's lines; it is in the list's file name too ([`list_name`]). 3: the
/// key's shade bits have [`crate::NO_LIGHTS_BIT`]. 4: the key has the add bit (`adds`). 5: the
/// key has the flat and the plane bits, and its shade bits have the sky bits 4 and 8 (W300).
const LIST_FORMAT: u32 = 5;
/// The frames between two writes of a changed list.
const SAVE_FRAMES: u32 = 600;
/// The list's pipelines go to the thread while it has fewer jobs than this, so that a key of
/// first use waits behind at most this many of them.
const LIST_BATCH: usize = 4;
/// The stage keys that the list may take; the rest of the cap is for the keys of first use.
const LIST_KEYS: usize = MAX_KEYS / 2;
/// The words of a stage key in the list: 9 facts, then 7 codes for each stage.
const KEY_FACTS: usize = 9;
const KEY_WORDS: usize = KEY_FACTS + STAGES * 7;

/// The samples of a key of [`UBER_SET`] that draws into the frame's targets. [`StageShaders::start`] puts the
/// frame's count in its place: it is the user's multisample setting.
const FRAME_SAMPLES: u32 = 0;
/// The base of a key of [`UBER_SET`] that draws into the frame's targets: a filled triangle list with the
/// stencil off, drawn by the uber-shader in normal play.
const UBER: PipelineKey = PipelineKey {
    fvf: 0,
    stride: 0,
    topology: 4,
    blend: [0; 3],
    depth: [0; 3],
    cull: 0,
    fill: 3,
    stencil: [0; 7],
    color_write: 0,
    samples: FRAME_SAMPLES,
    stages: 0,
    debug: 0,
};
/// The base of a key of [`UBER_SET`] that draws into a texture of the game: one sample.
const TEXTURE: PipelineKey = PipelineKey { samples: 1, ..UBER };
/// The stencil states of [`UBER_SET`], in Direct3D's numbers: enable, fail, depth fail, pass, function, mask, write
/// mask. The test always passes and the pass replaces the value.
const STENCIL_WRITE: [u32; 7] = [1, 1, 1, 3, 8, u32::MAX, u32::MAX];
/// The test always passes, and a depth fail increments the value (saturated).
const STENCIL_INCREMENT: [u32; 7] = [1, 1, 4, 1, 8, u32::MAX, u32::MAX];
/// The test always passes, and a depth fail decrements the value (saturated).
const STENCIL_DECREMENT: [u32; 7] = [1, 1, 5, 1, 8, u32::MAX, u32::MAX];
/// The test passes where the value is not the reference; nothing is written.
const STENCIL_NOT_EQUAL: [u32; 7] = [1, 1, 1, 1, 6, u32::MAX, u32::MAX];

/// The first key of [`UBER_SET`] with `frame_samples`: the key whose pipeline shows that a world
/// shader's text holds on the device (W317, `Renderer::settle_world_shader`).
pub(crate) fn first_world_key(frame_samples: u32) -> PipelineKey {
    PipelineKey {
        samples: frame_samples,
        ..UBER_SET[0]
    }
}

/// The fixed set of the uber-shader's pipelines (D306), in their order of first use: the keys that the frame thread
/// made of the uber-shader in a live start with no list (run 20261111-193018-d306-step1: the test interior, Seyda
/// Neen, the Census and Excise Office, Vivec). [`StageShaders::start`] sends them to the thread before the list, so
/// that the first start after a change of the world shader does not make them on the frame thread in play. A key of
/// the set that the frame needs before the thread gives it back is made on the frame thread, as before.
#[rustfmt::skip]
const UBER_SET: [PipelineKey; 48] = [
    PipelineKey { fvf: 322, stride: 24, blend: [0, 2, 1], depth: [1, 0, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 6], depth: [1, 0, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 6], depth: [1, 0, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 322, stride: 24, blend: [1, 5, 6], depth: [1, 0, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 274, stride: 32, blend: [0, 2, 1], depth: [1, 1, 8], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 338, stride: 36, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 2], depth: [1, 0, 4], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 2], depth: [1, 0, 4], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 322, stride: 24, blend: [1, 2, 2], depth: [1, 0, 8], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 15, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 4444, stride: 52, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 15, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 6], depth: [1, 1, 4], cull: 2, color_write: 15, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 8, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 6], depth: [1, 1, 4], cull: 2, color_write: 15, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 8, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 4380, stride: 48, blend: [1, 5, 6], depth: [1, 1, 4], cull: 2, color_write: 15, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 4380, stride: 48, blend: [1, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 8, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 6], depth: [1, 1, 4], cull: 1, color_write: 15, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 2, 1], depth: [1, 1, 4], cull: 1, color_write: 8, stencil: STENCIL_WRITE, ..TEXTURE },
    PipelineKey { fvf: 338, stride: 36, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 274, stride: 32, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 2], depth: [1, 0, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 2], depth: [1, 0, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 9, 2], depth: [1, 0, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 3, 2], depth: [1, 0, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 6], depth: [1, 1, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 6], depth: [1, 1, 4], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 6], depth: [1, 0, 4], cull: 1, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 4380, stride: 48, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 338, stride: 36, blend: [0, 2, 1], depth: [1, 1, 4], cull: 1, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 6], depth: [1, 1, 4], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 6], depth: [1, 0, 8], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 6], depth: [1, 0, 4], cull: 3, color_write: 7, ..TEXTURE },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 6], depth: [1, 1, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 4444, stride: 52, blend: [0, 2, 1], depth: [1, 1, 4], cull: 1, color_write: 7, ..UBER },
    PipelineKey { fvf: 4380, stride: 48, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 4636, stride: 56, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 274, stride: 32, blend: [0, 2, 1], depth: [1, 1, 4], cull: 3, color_write: 7, ..UBER },
    PipelineKey { fvf: 4444, stride: 52, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 338, stride: 36, blend: [0, 2, 1], depth: [1, 1, 4], cull: 3, color_write: 7, ..UBER },
    PipelineKey { fvf: 258, stride: 20, blend: [1, 5, 6], depth: [1, 0, 4], cull: 3, color_write: 7, stencil: STENCIL_INCREMENT, ..UBER },
    PipelineKey { fvf: 258, stride: 20, blend: [1, 5, 6], depth: [1, 0, 4], cull: 2, color_write: 7, stencil: STENCIL_DECREMENT, ..UBER },
    PipelineKey { fvf: 258, stride: 20, blend: [1, 1, 3], depth: [1, 0, 8], cull: 2, color_write: 7, stencil: STENCIL_NOT_EQUAL, ..UBER },
    PipelineKey { fvf: 274, stride: 32, blend: [1, 5, 2], depth: [1, 0, 8], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 786, stride: 48, blend: [1, 5, 2], depth: [1, 0, 8], cull: 2, color_write: 7, ..UBER },
    PipelineKey { fvf: 338, stride: 36, blend: [1, 5, 6], depth: [1, 0, 4], cull: 2, color_write: 15, ..TEXTURE },
    PipelineKey { fvf: 4444, stride: 52, blend: [0, 2, 1], depth: [1, 1, 4], cull: 2, color_write: 15, ..TEXTURE },
];
/// The bits of `counts.w` in a stage key: each one chooses a branch of `fs` or of the
/// extension's shade (1 the extension shades, 2, the sky's [`SKY_BITS`], 16, 32 the sun's
/// shadow, 128 the light for each pixel, 512, and this crate's [`crate::CLOUDS_HOOK_BIT`], 1024:
/// the clouds with the extension's colour, [`crate::CORNER_LIGHT_BIT`], 16384: a particle lit
/// at its corners, and [`crate::NO_LIGHTS_BIT`], 32768: no light loop). A draw of the retained
/// table also takes the bits of its kind from the scene (`draw_counts_w`): those stay values of
/// the frame, but for the sky's bits. The key keeps the bits in a `u16`.
const SHADE_BITS: u32 =
    1 | 2 | SKY_BITS | 16 | 32 | 128 | 512 | crate::CLOUDS_HOOK_BIT | crate::CORNER_LIGHT_BIT | crate::NO_LIGHTS_BIT;
/// The sky's bits of `counts.w`: 4 the dome, 8 the shadow of a moon. Only the stream sets them
/// (`stream::sky_bits`). The scene's bits of a table's kind never have them (`retained::apply_shade`
/// takes them out), so the key's text takes them out of the scene's part too ([`fixed_once`]),
/// and a table's draw has them as constants.
pub(crate) const SKY_BITS: u32 = 4 | 8;
const _: () = assert!(SHADE_BITS <= u16::MAX as u32, "the stage key keeps the shade bits in a u16");

/// The codes that `fs` switches on for a draw. Only the stages below `count` are kept; the
/// rest are 0, so that two draws that differ only in an unused stage share a key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(crate) struct StageKey {
    count: u8,
    /// The colour operation and its arguments 1, 2 and 0 of each stage.
    color: [[u8; 4]; STAGES],
    /// The alpha operation and its arguments 1 and 2 of each stage.
    alpha: [[u8; 3]; STAGES],
    /// The alpha test function, 0 for none.
    alpha_test: u8,
    /// The bits of [`SHADE_BITS`] in the record's `counts.w`.
    shade: u16,
    /// `flags.x` (the lighting) and `flags.y` (the vertex has a normal) of the record.
    lit: u8,
    normal: u8,
    /// Bit 4 of `light_range.w`: the draw takes the scene's values (`from_scene`).
    scene: bool,
    /// Bit 64 of `light_range.w`: the draw adds to the picture ([`crate::adds_to_picture`]),
    /// and `fogged` makes it darker in the fog.
    adds: bool,
    /// Bit 16 of `light_range.w`: the draw is flat-shaded ([`crate::flat_shading_bit`]).
    flat: bool,
    /// The draw has a plane: the record's `clip_plane` is not 0. A draw with the scene's values
    /// has none: the scene's plane is always 0 (the test `each_write_of_a_plane_is_known`). The
    /// record of a draw takes its plane after its key, so the places that give it one take the
    /// key again ([`StageShaders::id_with_plane`]).
    plane: bool,
}

impl StageKey {
    /// The key of a draw's record, or None when a code does not fit (the uber-shader draws it).
    pub(crate) fn of(data: &DrawData) -> Option<StageKey> {
        let count = data.counts[1];
        if count as usize > STAGES {
            return None;
        }
        let small = |value: u32| u8::try_from(value).ok();
        let scene = data.light_range[3] & 4 != 0;
        let mut key = StageKey {
            count: count as u8,
            alpha_test: small(data.more[2])?,
            shade: (data.counts[3] & SHADE_BITS) as u16,
            lit: small(data.flags[0])?,
            normal: small(data.flags[1])?,
            scene,
            adds: data.light_range[3] & crate::ADDS_BIT != 0,
            flat: data.light_range[3] & 16 != 0,
            plane: !scene && data.clip_plane != [0.0; 4],
            ..Default::default()
        };
        for stage in 0..count as usize {
            let s = &data.stages[stage];
            for part in 0..4 {
                key.color[stage][part] = small(s.color[part])?;
            }
            for part in 0..3 {
                key.alpha[stage][part] = small(s.alpha[part])?;
            }
        }
        Some(key)
    }

    /// The key as a line of numbers for the list.
    fn words(&self) -> Vec<u32> {
        let mut words = vec![
            self.count as u32,
            self.alpha_test as u32,
            self.shade as u32,
            self.lit as u32,
            self.normal as u32,
            self.scene as u32,
            self.adds as u32,
            self.flat as u32,
            self.plane as u32,
        ];
        for stage in 0..STAGES {
            words.extend(self.color[stage].iter().map(|&v| v as u32));
            words.extend(self.alpha[stage].iter().map(|&v| v as u32));
        }
        words
    }

    fn from_words(words: &[u32]) -> Option<StageKey> {
        if words.len() != KEY_WORDS {
            return None;
        }
        let small = |value: u32| u8::try_from(value).ok();
        let shade = u16::try_from(words[2])
            .ok()
            .filter(|&shade| shade as u32 & !SHADE_BITS == 0)?;
        let flag = |word: u32| match word {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        let mut key = StageKey {
            count: small(words[0])?,
            alpha_test: small(words[1])?,
            shade,
            lit: small(words[3])?,
            normal: small(words[4])?,
            scene: flag(words[5])?,
            adds: flag(words[6])?,
            flat: flag(words[7])?,
            plane: flag(words[8])?,
            ..Default::default()
        };
        // A draw with the scene's values has no plane of its own ([`StageKey::of`]).
        if key.count as usize > STAGES || (key.scene && key.plane) {
            return None;
        }
        for stage in 0..STAGES {
            let at = KEY_FACTS + stage * 7;
            for part in 0..4 {
                key.color[stage][part] = small(words[at + part])?;
            }
            for part in 0..3 {
                key.alpha[stage][part] = small(words[at + 4 + part])?;
            }
        }
        Some(key)
    }
}

/// The world shader's `fs` for one key, made from the world shader's text: the reads of the
/// stages at and past the count give white, the stage loop becomes one step for each stage with
/// its codes as constants, the alpha test has its function as a constant, and the draw's facts
/// of the key are constants ([`fixed_once`], [`fixed_reads`]). A [`RewriteRefusal`] when the text does
/// not have a part that this rewrite expects (then the uber-shader draws the key).
///
/// The parts that the rewrite needs one time are searched in the crate's own part of the text
/// only: the text before [`crate::request::EXTENSION_BEGIN`]. The text of an extension or of a
/// mod's file is not ours, and a second copy of such a part in it changes nothing. The first
/// such line is the boundary, so a copy of the line itself in a mod's text changes nothing
/// either. A text with no such line is the crate's own as a whole.
pub(crate) fn fragment_source(world: &str, key: &StageKey) -> Result<String, RewriteRefusal> {
    let (own, extension) = match world.split_once(crate::request::EXTENSION_BEGIN) {
        Some((own, extension)) => (own, Some(extension)),
        None => (world, None),
    };
    let mut source = own.to_string();
    // The reads: each stage's line starts so; stage 0's read is kept (a draw with no stage has no
    // use for it, and the compiler drops it).
    for stage in (key.count as usize).max(1)..STAGES {
        let start = format!("    texels[{stage}] = bump_light(");
        let at = one(&source, &start, || format!("the read of stage {stage}"))?;
        let end = at + line_length(&source[at..]);
        source.replace_range(at..end, &format!("    texels[{stage}] = vec4f(1.0);"));
    }
    // The reads below the count: after a stage that is no bump stage (22, 23), `bump_light` gives
    // the texel and `bump_move` gives no move, so the read is the texture's own.
    for stage in 1..key.count as usize {
        let before = stage - 1;
        if matches!(key.color[before][0], 22 | 23) {
            continue;
        }
        let start = format!("    texels[{stage}] = bump_light({before}u, texels[{before}], ");
        let at = one(&source, &start, || {
            format!("the read of stage {stage} after the bump light of stage {before}")
        })?;
        let end = at + line_length(&source[at..]);
        let Some(sample) = source[at + start.len()..end].strip_suffix(");") else {
            return Err(RewriteRefusal::count(
                format!("the close of the read of stage {stage}"),
                0,
                ONE,
            ));
        };
        let moved = format!(" + bump_move({before}u, texels[{before}])");
        let moves = sample.matches(&moved).count();
        if moves != 1 {
            return Err(RewriteRefusal::count(
                format!("the bump move in the read of stage {stage}"),
                moves,
                ONE,
            ));
        }
        let read = format!("    texels[{stage}] = {};", sample.replacen(&moved, "", 1));
        source.replace_range(at..end, &read);
    }

    // The stage loop: its code must be the code that the steps copy.
    let (start, end) = stage_loop_range(&source)?;
    if without_comments(&source[start..end]) != STAGE_LOOP {
        return Err(RewriteRefusal {
            part: "the stage loop".to_string(),
            found: 1,
            wanted: "its code is not the code that the steps copy",
        });
    }
    let mut steps = String::new();
    for stage in 0..key.count as usize {
        let [op, a1, a2, a0] = key.color[stage];
        let [alpha_op, alpha1, alpha2] = key.alpha[stage];
        steps.push_str(&format!("    {{\n        let texel = texels[{stage}];\n"));
        steps.push_str(&format!(
            "        let color = operation({op}u, argument({a1}u, diffuse, current, texel), argument({a2}u, diffuse, current, texel), argument({a0}u, diffuse, current, texel), diffuse, current, texel);\n"
        ));
        steps.push_str("        var alpha = current;\n");
        // As the loop: a bump stage (22, 23) and the alpha operation 1 keep the current alpha.
        if alpha_op != 1 && op != 22 && op != 23 {
            steps.push_str(&format!(
                "        alpha = operation({alpha_op}u, argument({alpha1}u, diffuse, current, texel), argument({alpha2}u, diffuse, current, texel), vec4f(0.0), diffuse, current, texel);\n"
            ));
        }
        steps.push_str("        current = vec4f(color.rgb, alpha.a);\n");
        steps.push_str("        if (draw_counts_w() & 128u) == 0u {\n            current = clamp(current, vec4f(0.0), vec4f(1.0));\n        }\n    }\n");
    }
    source.replace_range(start..end, &steps);

    // The alpha test: the draw's, and the same test on the colour of the clouds hook.
    for (indent, value, part) in [
        ("    ", "current", "the alpha test of the draw"),
        ("        ", "cloud", "the alpha test of the clouds hook"),
    ] {
        let test = format!("{indent}if draws[record].more.z != 0u && !compare(draws[record].more.z, floor({value}.a * 255.0 + 0.5), f32(draws[record].more.w)) {{\n{indent}    discard;\n{indent}}}\n");
        one(&source, &test, || part.to_string())?;
        let fixed_test = match key.alpha_test {
            0 => String::new(),
            function => format!("{indent}if !compare({function}u, floor({value}.a * 255.0 + 0.5), f32(draws[record].more.w)) {{\n{indent}    discard;\n{indent}}}\n"),
        };
        source = source.replacen(&test, &fixed_test, 1);
    }
    let own = fixed_once(&source, key)?;
    let whole = match extension {
        Some(extension) => format!("{own}{}{extension}", crate::request::EXTENSION_BEGIN),
        None => own,
    };
    fixed_reads(&whole, key)
}

/// Why the rewrite of a stage key's text failed: the part of the world shader's text that the
/// rewrite looked for, how many times the text has it, and what the rewrite wanted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewriteRefusal {
    /// The part, in words.
    pub part: String,
    /// How many times the text has the part.
    pub found: usize,
    /// What the rewrite wanted of it, in words.
    pub wanted: &'static str,
}

/// The rewrite wanted the part one time.
const ONE: &str = "1 wanted";
/// The rewrite wanted the part one time or more.
const SOME: &str = "1 or more wanted";

impl RewriteRefusal {
    fn count(part: String, found: usize, wanted: &'static str) -> RewriteRefusal {
        RewriteRefusal { part, found, wanted }
    }
}

impl std::fmt::Display for RewriteRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {} in the text, {}", self.part, self.found, self.wanted)
    }
}

/// Where the one copy of `part_text` is in `text`, or the refusal with the count of its copies.
fn one(text: &str, part_text: &str, part: impl Fn() -> String) -> Result<usize, RewriteRefusal> {
    match text.matches(part_text).count() {
        1 => Ok(text.find(part_text).expect("a text that matches one time is found")),
        found => Err(RewriteRefusal::count(part(), found, ONE)),
    }
}

/// The length of the first line of `text`, without its line end.
fn line_length(text: &str) -> usize {
    text.find('\n').unwrap_or(text.len())
}

/// The crate's own text with the facts of the key that the text tests at one place as constants,
/// so that the compiler leaves out the branches that the key's draws cannot take: `from_scene`
/// returns the key's scene bit, the test of `fogged` for a draw that adds to the picture is the
/// key's add bit, the plane's test of `fs` is false for a key without a plane, and the scene's
/// part of `draw_counts_w` never has [`SKY_BITS`]. A [`RewriteRefusal`] when the text does not have one
/// of these parts one time.
fn fixed_once(source: &str, key: &StageKey) -> Result<String, RewriteRefusal> {
    let scene_test = "fn from_scene() -> bool {\n    return (draws[record].light_range.w & 4u) != 0u;\n}\n";
    one(source, scene_test, || "the function from_scene".to_string())?;
    let source = source.replacen(
        scene_test,
        &format!("fn from_scene() -> bool {{\n    return {};\n}}\n", key.scene),
        1,
    );

    // The one test of the add bit is in `fogged`, which `fs` and the extension's finish call.
    let adds_test = format!("(draws[record].light_range.w & {}u) != 0u", crate::ADDS_BIT);
    one(&source, &adds_test, || "the test of the add bit in fogged".to_string())?;
    let source = source.replacen(&adds_test, &key.adds.to_string(), 1);

    // The plane's test of `fs`: a draw without a plane keeps no read, test or `discard` of it.
    let plane_test = "any(clip_plane != vec4f(0.0))";
    one(&source, plane_test, || "the plane's test of fs".to_string())?;
    let source = if key.plane {
        source
    } else {
        source.replacen(plane_test, "false", 1)
    };

    // The scene's part of `draw_counts_w` never has the sky's bits.
    let scene_part = "scene.counts[k >> 2u][k & 3u]";
    one(&source, scene_part, || "the scene's part of draw_counts_w".to_string())?;
    Ok(source.replacen(scene_part, &format!("({scene_part} & {}u)", !SKY_BITS), 1))
}

/// The whole text, the extension's part too, with the facts of the key that the text reads at
/// many places as constants: each read of the flat bit is the key's, a read of the record's
/// `counts.w` has the key's bits of [`SHADE_BITS`] as constants (the scene's bits of a retained
/// draw stay values of the frame), and the reads of `flags.x` and `flags.y` are the key's values.
/// Each read is replaced on purpose, so a copy in the extension's text is a constant too. A
/// [`RewriteRefusal`] when the text has no such read at all.
fn fixed_reads(source: &str, key: &StageKey) -> Result<String, RewriteRefusal> {
    // Each read of the flat bit: the start of `fs`, and the vertex stage's and the extension's
    // tests of a draw with no lights.
    let flat_read = "draws[record].light_range.w & 16u";
    if !source.contains(flat_read) {
        return Err(RewriteRefusal::count("a read of the flat bit".to_string(), 0, SOME));
    }
    let source = source.replace(flat_read, &format!("{}u", if key.flat { 16 } else { 0 }));

    let counts_w = "draws[record].counts.w";
    if !source.contains(counts_w) {
        return Err(RewriteRefusal::count("a read of counts.w".to_string(), 0, SOME));
    }
    let source = source.replace(
        counts_w,
        &format!("((draws[record].counts.w & {}u) | {}u)", !SHADE_BITS, key.shade),
    );

    let no_read = |field: &str| RewriteRefusal::count(format!("a read of {field}"), 0, SOME);
    let source =
        replace_field(&source, "draws[record].flags.x", &format!("{}u", key.lit)).ok_or_else(|| no_read("flags.x"))?;
    replace_field(&source, "draws[record].flags.y", &format!("{}u", key.normal)).ok_or_else(|| no_read("flags.y"))
}

/// The text with each read of a field replaced (a read, not a longer name that starts with it,
/// such as a swizzle); None when the text has no read of it.
fn replace_field(source: &str, field: &str, value: &str) -> Option<String> {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    let mut found = false;
    while let Some(at) = rest.find(field) {
        let after = &rest[at + field.len()..];
        let whole = !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
        out.push_str(&rest[..at]);
        out.push_str(if whole { value } else { field });
        found |= whole;
        rest = after;
    }
    out.push_str(rest);
    found.then_some(out)
}

/// The code of the stage loop of the world shader's `fs` without its comments, as the steps of
/// [`fragment_source`] copy it. An edit of the loop's code no longer matches: the rewrite then
/// refuses the text (the uber-shader draws every key), and the test of each key fails until the
/// steps follow the edit.
const STAGE_LOOP: &str = r#"    for (var i = 0u; i < draws[record].counts.y; i++) {
        let stage = draws[record].stages[i];
        let texel = texels[i];
        let color = operation(stage.color.x, argument(stage.color.y, diffuse, current, texel),
            argument(stage.color.z, diffuse, current, texel), argument(stage.color.w, diffuse, current, texel), diffuse, current, texel);
        var alpha = current;
        if stage.alpha.x != 1u && stage.color.x != 22u && stage.color.x != 23u {
            alpha = operation(stage.alpha.x, argument(stage.alpha.y, diffuse, current, texel),
                argument(stage.alpha.z, diffuse, current, texel), vec4f(0.0), diffuse, current, texel);
        }
        current = vec4f(color.rgb, alpha.a);
        if (draw_counts_w() & 128u) == 0u {
            current = clamp(current, vec4f(0.0), vec4f(1.0));
        }
    }
"#;

/// The mark before the stage loop of `fs` in the world shader's text, and the line after the
/// loop. The loop is what stands between the mark's line and the end line, so the lines that
/// follow the loop are free: a new line there does not move the range.
const STAGE_LOOP_BEGIN: &str = "    // STAGE LOOP BEGIN";
const STAGE_LOOP_END: &str = "    // STAGE LOOP END\n";

/// Where the stage loop of `fs` is in the world shader's text: from the line after the line of
/// [`STAGE_LOOP_BEGIN`] to the line of [`STAGE_LOOP_END`]. Each mark must be in the text one time.
fn stage_loop_range(source: &str) -> Result<(usize, usize), RewriteRefusal> {
    let begin = one(source, STAGE_LOOP_BEGIN, || "the begin mark of the stage loop".to_string())?;
    let end = one(source, STAGE_LOOP_END, || "the end mark of the stage loop".to_string())?;
    let start = (begin + line_length(&source[begin..]) + 1).min(source.len());
    if start > end {
        return Err(RewriteRefusal::count(
            "the end mark of the stage loop after its begin mark".to_string(),
            0,
            ONE,
        ));
    }
    Ok((start, end))
}

/// The kinds of stage key that [`stage_rewrite_refusals`] tries, each with its name: one key for
/// each fact that changes the rewrite's steps, and one with every bit of [`SHADE_BITS`].
fn kinds() -> Vec<(&'static str, StageKey)> {
    let mut plain = StageKey {
        count: 1,
        shade: 1 | 128,
        lit: 1,
        normal: 1,
        ..Default::default()
    };
    // Stage 0: MODULATE of the texture and the diffuse; the later stages: ADD of the texture and
    // the current colour.
    plain.color[0] = [4, 2, 0, 1];
    plain.alpha[0] = [4, 2, 0];
    let mut every_stage = StageKey {
        count: STAGES as u8,
        alpha_test: 5,
        ..plain
    };
    for stage in 1..STAGES {
        every_stage.color[stage] = [7, 2, 1, 1];
        every_stage.alpha[stage] = [1, 1, 0];
    }
    let shade = |bits: u32| StageKey {
        shade: bits as u16,
        ..plain
    };
    vec![
        ("a plain key", plain),
        ("a key with no stage", StageKey { count: 0, ..plain }),
        ("a key with every stage and an alpha test", every_stage),
        ("a key with a plane", StageKey { plane: true, ..plain }),
        ("a flat key", StageKey { flat: true, ..plain }),
        ("a key of the scene", StageKey { scene: true, ..plain }),
        ("a key that adds to the picture", StageKey { adds: true, ..plain }),
        ("a key of the sky", shade(SKY_BITS)),
        ("a key with the clouds hook's bit", shade(crate::CLOUDS_HOOK_BIT)),
        ("a key with no lights", shade(crate::NO_LIGHTS_BIT)),
        ("a key with the light at the corners", shade(crate::CORNER_LIGHT_BIT)),
        ("a key with every shade bit", shade(SHADE_BITS)),
    ]
}

/// The kinds of stage key whose text the rewrite cannot make from a world shader's text
/// ([`crate::world_shader_source`], or its tier C form [`crate::tier_c_shader_source`]), each as
/// "(the kind): (the part that refused)". Empty when the rewrite makes a text for each kind.
///
/// For a test of an extension's text with no GPU. The helper draws with the crate's text and the
/// extension's, and a rewrite that fails on that text fails only at run time, where the
/// uber-shader then draws every key.
pub fn stage_rewrite_refusals(world_source: &str) -> Vec<String> {
    kinds()
        .into_iter()
        .filter_map(|(kind, key)| {
            fragment_source(world_source, &key)
                .err()
                .map(|refusal| format!("{kind}: {refusal}"))
        })
        .collect()
}

/// The lines of a text that are not comments, each with its line end.
fn without_comments(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(|line| [line, "\n"])
        .collect()
}

/// The hash of a text, for the list's check that it was written for this world shader.
fn text_hash(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn pipeline_words(key: &PipelineKey) -> Vec<u32> {
    let mut words = vec![key.fvf, key.stride, key.topology];
    words.extend(key.blend);
    words.extend(key.depth);
    words.push(key.cull);
    words.push(key.fill);
    words.extend(key.stencil);
    words.push(key.color_write);
    words.push(key.samples);
    words
}

fn pipeline_from_words(words: &[u32]) -> Option<PipelineKey> {
    if words.len() != 21 {
        return None;
    }
    Some(PipelineKey {
        fvf: words[0],
        stride: words[1],
        topology: words[2],
        blend: words[3..6].try_into().ok()?,
        depth: words[6..9].try_into().ok()?,
        cull: words[9],
        fill: words[10],
        stencil: words[11..18].try_into().ok()?,
        color_write: words[18],
        samples: words[19],
        // Word 20 is the place of the stage key's id, which the list does not keep.
        stages: 0,
        // The list keeps no variant.
        debug: 0,
    })
}

/// The name of a fragment module: the id of its stage key and its world variant
/// (`PipelineKey::stages`, `PipelineKey::debug`).
type ModuleId = (u32, u32);

fn module_id(key: &PipelineKey) -> ModuleId {
    (key.stages, key.debug)
}

/// A pipeline to make on the thread: its key, and the fragment module's text when the thread
/// has no module of its stage key and variant yet.
struct Job {
    key: PipelineKey,
    source: Option<String>,
}

struct Done {
    key: PipelineKey,
    pipeline: Option<wgpu::RenderPipeline>,
}

/// The stage keys of the session and their pipelines that are not made yet.
pub(crate) struct StageShaders {
    pub(crate) on: bool,
    /// A replay makes a key's pipeline at its first use, on the frame's thread.
    sync: bool,
    world_source: String,
    world_hash: u64,
    /// The world variant of the frames (`PipelineKey::debug`), 0 in normal play, and the world
    /// shader's text of that variant; None when the variant's text cannot be made.
    variant: u32,
    variant_source: Option<String>,
    ids: HashMap<StageKey, u32>,
    /// The keys by id; id 0 is the uber-shader.
    keys: Vec<StageKey>,
    /// The fragment modules of a replay; None for a module that failed.
    modules: HashMap<ModuleId, Option<wgpu::ShaderModule>>,
    /// The modules that the thread has.
    sent_modules: HashSet<ModuleId>,
    /// The modules whose text the rewrite cannot make.
    failed_sources: HashSet<ModuleId>,
    pending: HashSet<PipelineKey>,
    failed: HashSet<PipelineKey>,
    jobs: Option<mpsc::Sender<Job>>,
    done: Option<mpsc::Receiver<Done>>,
    list: Option<PathBuf>,
    /// The pipelines of the list not yet sent to the thread (`stages` 0), with their stage keys.
    queued: VecDeque<(PipelineKey, StageKey)>,
    /// The pipelines made in this session, with their stage keys, for the list.
    made: HashSet<(PipelineKey, StageKey)>,
    changed: bool,
    frames_since_save: u32,
    // Counts for the report.
    pub(crate) made_count: u32,
    pub(crate) from_list: u32,
    /// The list's pipelines left out: their stage key is past the list's share of the cap.
    pub(crate) list_left_out: u32,
    /// The stage keys whose text the rewrite could not make, by id, each with its refusal.
    refused: BTreeMap<u32, String>,
    /// The stage keys with a text whose module or pipeline failed, by id.
    failed_pipelines: BTreeSet<u32>,
    /// The draws with a stage key that the uber-shader drew, by cause: the key's rewrite failed, its module or
    /// pipeline failed, or the thread had not made its pipeline yet (a live session only).
    draws_refused: u64,
    draws_failed: u64,
    draws_not_made: u64,
    pub(crate) past_cap: u64,
    /// The draws whose record has a code that does not fit a stage key: the uber-shader draws them.
    pub(crate) no_key: u64,
    /// The keys of [`UBER_SET`] in this session, in its order, with the frame's samples in place. A search of it
    /// comes only at the first draw of a key of the uber-shader.
    uber_set: Vec<PipelineKey>,
    /// The pipelines of the set that the thread made, until their first draw.
    uber_ready: HashMap<PipelineKey, wgpu::RenderPipeline>,
    /// The keys of the set that have drawn.
    uber_drawn: HashSet<PipelineKey>,
    /// The keys of the set whose pipeline the thread made before their first draw.
    uber_ahead: u32,
    /// The keys of the set that drew first with a pipeline of the frame thread: the sign that the set is late or stale.
    uber_late: u32,
    /// The draws that took their key again after their record took a plane ([`StageShaders::id_with_plane`]): the split
    /// copies, and the draws of a view with a plane.
    again_split: u64,
    again_view: u64,
    /// The stage keys that a draw asked a pipeline for while it was not ready: filled on the miss path only, so that
    /// the frame thread pays no insert for each draw. In a session that starts with no list, the keys not in it are
    /// the keys that no draw asked for (the key without a plane of a draw that is drawn only as its copies with a
    /// plane): each costs its place under the cap and no module. In a session with a list, a key whose pipeline the
    /// list made first is not in it either.
    not_ready: HashSet<u32>,
}

impl StageShaders {
    /// The stage shaders of a session, made at their first use (a replay) until [`StageShaders::start`].
    pub(crate) fn new(on: bool, world_source: &str) -> StageShaders {
        StageShaders {
            on,
            sync: true,
            world_source: world_source.to_string(),
            world_hash: text_hash(world_source),
            variant: 0,
            variant_source: None,
            ids: HashMap::new(),
            keys: vec![StageKey::default()],
            modules: HashMap::new(),
            sent_modules: HashSet::new(),
            failed_sources: HashSet::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            jobs: None,
            done: None,
            list: None,
            queued: VecDeque::new(),
            made: HashSet::new(),
            changed: false,
            frames_since_save: 0,
            made_count: 0,
            from_list: 0,
            list_left_out: 0,
            refused: BTreeMap::new(),
            failed_pipelines: BTreeSet::new(),
            draws_refused: 0,
            draws_failed: 0,
            draws_not_made: 0,
            past_cap: 0,
            no_key: 0,
            uber_set: Vec::new(),
            uber_ready: HashMap::new(),
            uber_drawn: HashSet::new(),
            uber_ahead: 0,
            uber_late: 0,
            again_split: 0,
            again_view: 0,
            not_ready: HashSet::new(),
        }
    }

    /// The stage shaders of a new world text in the same session (W317's switch of the world shader).
    /// The stage keys keep their ids: the retained table keeps the id of each shape for its life. The
    /// counts of the key table go with it: a shape that got id 0 past the cap or with no key keeps the
    /// 0 and does not ask again. All that comes from the text starts again: the modules, the refusals,
    /// the list and the thread. The old value drops here, and its drop saves the old text's list.
    pub(crate) fn switched(mut self, world_source: &str) -> StageShaders {
        // StageShaders has a Drop, so a struct update cannot move fields out of a value: each kept field is
        // set on the new value.
        let mut next = StageShaders::new(self.on, world_source);
        next.ids = std::mem::take(&mut self.ids);
        next.keys = std::mem::take(&mut self.keys);
        next.past_cap = self.past_cap;
        next.no_key = self.no_key;
        next
    }

    /// The id of the stage key of a record that took a plane after its key: the split copy of a draw, or a draw of a
    /// view with a plane (`view`). The plane is a fact of the key ([`StageKey::plane`]).
    pub(crate) fn id_with_plane(&mut self, data: &DrawData, view: bool) -> u32 {
        if view {
            self.again_view += 1;
        } else {
            self.again_split += 1;
        }
        self.id_of(data)
    }

    /// The id of a draw's stage key, 0 (the uber-shader) when the feature is off, the key does
    /// not fit, or the cap is reached.
    pub(crate) fn id_of(&mut self, data: &DrawData) -> u32 {
        if !self.on {
            return 0;
        }
        let Some(key) = StageKey::of(data) else {
            self.no_key += 1;
            return 0;
        };
        self.intern(key)
    }

    fn intern(&mut self, key: StageKey) -> u32 {
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        if self.keys.len() > MAX_KEYS {
            self.past_cap += 1;
            return 0;
        }
        let id = self.keys.len() as u32;
        self.keys.push(key);
        self.ids.insert(key, id);
        id
    }

    /// The world shader's text of a variant: the text of normal play for 0, else the text of
    /// the frames' variant. None for an older variant, or when the variant's text cannot be made.
    pub(crate) fn world_text(&self, variant: u32) -> Option<&str> {
        match variant {
            0 => Some(&self.world_source),
            _ if variant == self.variant => self.variant_source.as_deref(),
            _ => None,
        }
    }

    fn source_of(&self, (id, variant): ModuleId) -> Result<String, RewriteRefusal> {
        let absent = |part: &str| RewriteRefusal::count(part.to_string(), 0, ONE);
        let world = self
            .world_text(variant)
            .ok_or_else(|| absent("the world shader's text of the variant"))?;
        let key = self.keys.get(id as usize).ok_or_else(|| absent("the stage key of the id"))?;
        fragment_source(world, key)
    }

    /// The world variant of the frames ([`crate::debug_lines::world_variant`]). A change drops
    /// what the stage shaders keep of the old variant: its modules, the marks of its modules that
    /// the thread has, and its pending and failed pipelines. A pipeline of the old variant that
    /// the thread gives back later is discarded ([`StageShaders::receive`]); the caller drops
    /// the pipelines that it keeps.
    pub(crate) fn set_variant(&mut self, variant: u32) {
        if variant == self.variant {
            return;
        }
        let old = self.variant;
        self.variant = variant;
        self.variant_source = match variant {
            0 => None,
            _ => {
                let source = crate::request::world_variant_source(&self.world_source, variant);
                if source.is_none() {
                    println!(
                        "debug view: the world shader has no one DEBUG_VIEW line; variant {variant} draws as normal play"
                    );
                }
                source
            }
        };
        if old != 0 {
            self.modules.retain(|&(_, v), _| v != old);
            self.sent_modules.retain(|&(_, v)| v != old);
            self.failed_sources.retain(|&(_, v)| v != old);
            self.pending.retain(|key| key.debug != old);
            self.failed.retain(|key| key.debug != old);
        }
    }

    /// Starts the thread that makes the pipelines of a live session, and gives it the pipelines of
    /// the list, or with no list the keys of [`UBER_SET`] (`frame_samples`: the samples of the
    /// frame's targets).
    pub(crate) fn start(&mut self, maker: crate::renderer::PipelineMaker, frame_samples: u32) {
        if !self.on {
            return;
        }
        let (jobs, job_receiver) = mpsc::channel::<Job>();
        let (done_sender, done) = mpsc::channel::<Done>();
        let spawned = std::thread::Builder::new().name("stage pipelines".into()).spawn(move || {
            let mut modules: HashMap<ModuleId, Option<wgpu::ShaderModule>> = HashMap::new();
            while let Ok(job) = job_receiver.recv() {
                let id = module_id(&job.key);
                if let Some(source) = job.source {
                    // The first module of a variant drops the modules of the variant before it.
                    let variant = id.1;
                    if variant != 0 {
                        modules.retain(|&(_, v), _| v == 0 || v == variant);
                    }
                    modules.insert(id, maker.module("stage shaders", &source));
                }
                let pipeline = modules
                    .get(&id)
                    .and_then(|module| module.as_ref())
                    .and_then(|module| maker.make(job.key, module));
                if done_sender.send(Done { key: job.key, pipeline }).is_err() {
                    break;
                }
            }
        });
        if spawned.is_err() {
            println!("stage shaders: the thread of the pipelines cannot start; the uber-shader draws every key");
            self.on = false;
            return;
        }
        self.jobs = Some(jobs);
        self.done = Some(done);
        self.sync = false;
        self.list = list_path(self.world_hash);
        self.load_list();
        self.send_uber_set(frame_samples);
    }

    /// Sends the keys of [`UBER_SET`] to the thread, with `frame_samples` in place of
    /// [`FRAME_SAMPLES`], when no pipeline of the list waits. The first job carries the world
    /// shader's own text as the module of the uber-shader. The list does not keep these keys
    /// ([`StageShaders::receive`]).
    fn send_uber_set(&mut self, frame_samples: u32) {
        // `send_queued` sends the list only while fewer than `LIST_BATCH` jobs
        // wait, so the set's jobs would hold the list back.
        if !self.queued.is_empty() {
            return;
        }
        let Some(jobs) = self.jobs.clone() else { return };
        let mut source = Some(self.world_source.clone());
        for key in UBER_SET {
            let samples = if key.samples == FRAME_SAMPLES {
                frame_samples
            } else {
                key.samples
            };
            let key = PipelineKey { samples, ..key };
            // With one sample for the frame, a key of the frame equals a key of a texture.
            if self.uber_set.contains(&key) {
                continue;
            }
            self.uber_set.push(key);
            if jobs
                .send(Job {
                    key,
                    source: source.take(),
                })
                .is_err()
            {
                return;
            }
            self.pending.insert(key);
        }
        self.sent_modules.insert(module_id(&UBER));
    }

    /// The pipeline of a key of the uber-shader in normal play, when the thread made it as a key
    /// of [`UBER_SET`] before its first draw; None otherwise, and the caller makes it.
    pub(crate) fn uber_pipeline(&mut self, key: PipelineKey) -> Option<wgpu::RenderPipeline> {
        if !self.uber_set.contains(&key) || !self.uber_drawn.insert(key) {
            return None;
        }
        let pipeline = self.uber_ready.remove(&key);
        if pipeline.is_some() {
            self.uber_ahead += 1;
        } else {
            self.uber_late += 1;
        }
        pipeline
    }

    fn load_list(&mut self) {
        let Some(path) = self.list.clone() else {
            println!("stage shaders: no folder for the list (LOCALAPPDATA); each key is made at its first use");
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => {
                println!(
                    "stage shaders: no list at {}; each key is made at its first use",
                    path.display()
                );
                return;
            }
        };
        let mut lines = text.lines();
        let header = format!("{LIST_HEADER} {LIST_FORMAT} {:016x}", self.world_hash);
        if lines.next() != Some(header.as_str()) {
            println!(
                "stage shaders: the list at {} is of another world shader or format; it is not used",
                path.display()
            );
            return;
        }
        let mut dropped = 0;
        for line in lines {
            let words: Option<Vec<u32>> = line.split_whitespace().map(|word| word.parse().ok()).collect();
            let parsed = words
                .as_deref()
                .filter(|words| words.len() == 21 + KEY_WORDS)
                .and_then(|words| Some((pipeline_from_words(&words[..21])?, StageKey::from_words(&words[21..])?)));
            let Some(entry) = parsed else {
                dropped += 1;
                continue;
            };
            self.queued.push_back(entry);
        }
        if dropped != 0 {
            println!("stage shaders: {dropped} lines of the list at {} not read", path.display());
        }
        println!(
            "stage shaders: {} pipelines of the list at {} are made behind the keys of first use",
            self.queued.len(),
            path.display()
        );
    }

    /// Sends the list's pipelines to the thread while it has fewer than [`LIST_BATCH`] jobs. A
    /// pipeline whose stage key would pass [`LIST_KEYS`] is left out, and the next save drops it.
    fn send_queued(&mut self) {
        while self.pending.len() < LIST_BATCH {
            let Some((pipeline, stage)) = self.queued.pop_front() else {
                return;
            };
            if self.made.contains(&(pipeline, stage)) {
                continue;
            }
            let id = match self.ids.get(&stage) {
                Some(&id) => id,
                None if self.keys.len() <= LIST_KEYS => self.intern(stage),
                None => {
                    self.list_left_out += 1;
                    continue;
                }
            };
            let key = PipelineKey { stages: id, ..pipeline };
            if id != 0 && !self.pending.contains(&key) && self.send(key) {
                self.from_list += 1;
            }
        }
    }

    /// Sends a pipeline to the thread; false when the thread is gone.
    fn send(&mut self, key: PipelineKey) -> bool {
        // The module's text goes with the first pipeline of its stage key and variant; a key
        // whose text the rewrite cannot make is not sent.
        let id = module_id(&key);
        let source = if self.sent_modules.contains(&id) {
            None
        } else if self.failed_sources.contains(&id) {
            return false;
        } else {
            match self.source_of(id) {
                Ok(source) => Some(source),
                Err(refusal) => {
                    self.note_refusal(id, &refusal);
                    return false;
                }
            }
        };
        let Some(jobs) = &self.jobs else { return false };
        if jobs.send(Job { key, source }).is_err() {
            return false;
        }
        self.sent_modules.insert(id);
        self.pending.insert(key);
        true
    }

    /// Takes the pipelines that the thread made.
    pub(crate) fn receive(&mut self, pipelines: &mut rustc_hash::FxHashMap<PipelineKey, std::rc::Rc<wgpu::RenderPipeline>>) {
        let Some(done) = &self.done else { return };
        // The failed pipelines are noted after the loop, which holds the channel.
        let mut failed = Vec::new();
        while let Ok(Done { key, pipeline }) = done.try_recv() {
            self.pending.remove(&key);
            // A key of the uber set waits for its first draw ([`StageShaders::uber_pipeline`]). When the frame thread
            // made it first, the thread's copy is not needed.
            if key.stages == 0 {
                match pipeline {
                    Some(pipeline) if !self.uber_drawn.contains(&key) => {
                        self.uber_ready.insert(key, pipeline);
                    }
                    Some(_) => {}
                    None => {
                        println!("stage shaders: a pipeline of the uber set failed; the frame makes it at its first draw")
                    }
                }
                continue;
            }
            // A pipeline of a variant that a change dropped (`set_variant`) is discarded.
            if key.debug != 0 && key.debug != self.variant {
                continue;
            }
            match pipeline {
                Some(pipeline) => {
                    pipelines.insert(key, std::rc::Rc::new(pipeline));
                    self.made_count += 1;
                    // The list keeps the pipelines of normal play only.
                    if key.debug != 0 {
                        continue;
                    }
                    if let Some(stage) = self.keys.get(key.stages as usize) {
                        if self.made.insert((PipelineKey { stages: 0, ..key }, *stage)) {
                            self.changed = true;
                        }
                    }
                }
                None => failed.push(key),
            }
        }
        for key in failed {
            self.note_failed_pipeline(key);
        }
        self.send_queued();
    }

    /// A stage key whose text the rewrite cannot make: the uber-shader draws it. One line for each key and
    /// variant, with the part that refused.
    fn note_refusal(&mut self, id: ModuleId, refusal: &RewriteRefusal) {
        if self.failed_sources.insert(id) {
            self.refused.entry(id.0).or_insert_with(|| refusal.to_string());
            println!(
                "stage shaders: the rewrite of stage key {} failed ({refusal}); the uber-shader draws it",
                id.0
            );
        }
    }

    /// A pipeline of a stage key that failed: the uber-shader draws it. A key whose rewrite failed has its own
    /// line and count ([`StageShaders::note_refusal`]); this is a key that has a text.
    fn note_failed_pipeline(&mut self, key: PipelineKey) {
        if self.failed.insert(key) && !self.failed_sources.contains(&module_id(&key)) {
            self.failed_pipelines.insert(key.stages);
            println!(
                "stage shaders: the pipeline of stage key {} failed; the uber-shader draws it",
                key.stages
            );
        }
    }

    /// Counts a draw with a stage key that the uber-shader draws, by its cause.
    fn count_uber_draw(&mut self, key: &PipelineKey) {
        if self.failed_sources.contains(&module_id(key)) {
            self.draws_refused += 1;
        } else if self.failed.contains(key) {
            self.draws_failed += 1;
        } else {
            self.draws_not_made += 1;
        }
    }

    /// The pipeline of a key with a stage key, or None: then the caller draws with the uber-shader.
    pub(crate) fn pipeline(
        &mut self,
        key: PipelineKey,
        maker: &crate::renderer::PipelineMaker,
        pipelines: &mut rustc_hash::FxHashMap<PipelineKey, std::rc::Rc<wgpu::RenderPipeline>>,
    ) -> Option<std::rc::Rc<wgpu::RenderPipeline>> {
        if let Some(pipeline) = pipelines.get(&key) {
            return Some(pipeline.clone());
        }
        self.not_ready.insert(key.stages);
        if self.failed.contains(&key) {
            self.count_uber_draw(&key);
            return None;
        }
        if self.sync {
            let id = module_id(&key);
            if !self.modules.contains_key(&id) {
                let module = match self.source_of(id) {
                    Ok(source) => maker.module("stage shaders", &source),
                    Err(refusal) => {
                        self.note_refusal(id, &refusal);
                        None
                    }
                };
                self.modules.insert(id, module);
            }
            let pipeline = self
                .modules
                .get(&id)
                .and_then(|module| module.as_ref())
                .and_then(|module| maker.make(key, module));
            let Some(pipeline) = pipeline else {
                self.note_failed_pipeline(key);
                self.count_uber_draw(&key);
                return None;
            };
            let pipeline = std::rc::Rc::new(pipeline);
            pipelines.insert(key, pipeline.clone());
            self.made_count += 1;
            return Some(pipeline);
        }
        if !self.pending.contains(&key) && !self.send(key) {
            self.note_failed_pipeline(key);
        }
        self.count_uber_draw(&key);
        None
    }

    /// The fault of a replay, or None. A replay makes each pipeline at its first use, so there every draw with
    /// a stage key has its own pipeline, and a capture of ours never passes the cap of the keys. A failed
    /// rewrite, a failed pipeline, a draw with a stage key on the uber-shader and a draw past the cap are each
    /// a fault there: the picture is the same, and only the speed is lost, so nothing else shows it. The line
    /// has every failed key and each count. A live session has no such fault: there the thread makes the
    /// pipelines behind the draws, and the helper never ends for this.
    pub(crate) fn replay_fault(&self) -> Option<String> {
        let uber_draws = self.draws_refused + self.draws_failed + self.draws_not_made;
        let none = self.refused.is_empty() && self.failed_pipelines.is_empty() && uber_draws == 0 && self.past_cap == 0;
        if !self.sync || none {
            return None;
        }
        Some(format!(
            "stage shaders: a fault of the replay: {}; {}; the uber-shader drew {uber_draws} draws that have a stage key ({} after a failed rewrite, {} after a failed pipeline, {} with a pipeline not made); {} draws past the cap of {MAX_KEYS} keys",
            self.refused_text(),
            self.failed_pipelines_text(),
            self.draws_refused,
            self.draws_failed,
            self.draws_not_made,
            self.past_cap
        ))
    }

    /// The stage shaders as a check ([`crate::checks`]): the stage keys judged, and the faults.
    /// A failed rewrite and a failed pipeline are faults in every session. In a replay, which
    /// makes each pipeline at its first use, a draw with a stage key on the uber-shader and a
    /// draw past the cap are faults too ([`StageShaders::replay_fault`]).
    pub(crate) fn check(&self) -> crate::checks::Check {
        let failed = (self.refused.len() + self.failed_pipelines.len()) as u64;
        let of_a_replay = match self.sync {
            true => self.draws_refused + self.draws_failed + self.draws_not_made + self.past_cap,
            false => 0,
        };
        crate::checks::Check::new(
            crate::checks::STAGE_SHADERS,
            (self.keys.len() - 1) as u64,
            failed + of_a_replay,
        )
    }

    /// "N rewrites failed", with each key and the refusal of each.
    fn refused_text(&self) -> String {
        let mut text = format!("{} rewrites failed", self.refused.len());
        for (index, (id, refusal)) in self.refused.iter().enumerate() {
            text.push_str(if index == 0 { " (" } else { "; " });
            text.push_str(&format!("stage key {id}: {refusal}"));
        }
        if !self.refused.is_empty() {
            text.push(')');
        }
        text
    }

    /// "N pipelines failed", with the keys.
    fn failed_pipelines_text(&self) -> String {
        let ids: Vec<String> = self.failed_pipelines.iter().map(u32::to_string).collect();
        match ids.is_empty() {
            true => "0 pipelines failed".to_string(),
            false => format!("{} pipelines failed (stage keys {})", ids.len(), ids.join(", ")),
        }
    }

    /// After a frame: writes the list when it changed, at most every [`SAVE_FRAMES`] frames.
    pub(crate) fn after_frame(&mut self) {
        self.frames_since_save += 1;
        if self.changed && self.frames_since_save >= SAVE_FRAMES {
            self.save();
        }
    }

    pub(crate) fn save(&mut self) {
        self.frames_since_save = 0;
        if !self.changed {
            return;
        }
        self.changed = false;
        let Some(path) = &self.list else { return };
        let mut text = format!("{LIST_HEADER} {LIST_FORMAT} {:016x}\n", self.world_hash);
        let entries: HashSet<&(PipelineKey, StageKey)> = self.made.iter().chain(self.queued.iter()).collect();
        for (pipeline, stage) in entries {
            let words: Vec<String> = pipeline_words(pipeline)
                .into_iter()
                .chain([0])
                .chain(stage.words())
                .map(|w| w.to_string())
                .collect();
            text.push_str(&words.join(" "));
            text.push('\n');
        }
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(path, text));
        if let Err(error) = written {
            println!(
                "stage shaders: the list at {} cannot be written ({error}); the next start makes each key at its first use",
                path.display()
            );
            self.list = None;
        }
    }

    /// The line of the report: the keys and the pipelines, then what failed, each kind with its keys, then the
    /// draws that the uber-shader took, by cause. A replay with a number other than 0 after "pipelines made"
    /// (but the draws with no key) ends with [`StageShaders::replay_fault`]; a live session only reports, and the
    /// draws with a pipeline not made are its cold start (the list says whether it had one).
    pub(crate) fn report(&self) -> String {
        let made = match (self.sync, self.from_list) {
            (true, _) => "made at the first use".to_string(),
            (false, 0) => "made on a thread, with no list: a cold start".to_string(),
            (false, _) => "made on a thread, with a list".to_string(),
        };
        format!(
            "stage shaders: {} ({made}), {} stage keys (cap {MAX_KEYS}), {} pipelines made ({} sent from the list, {} of the list left out past its share of the cap); {}; {}; draws on the uber-shader with a stage key: {} after a failed rewrite, {} after a failed pipeline, {} with a pipeline not made; {} draws past the cap, {} draws with no key",
            if self.on { "on" } else { "off" },
            self.keys.len() - 1,
            self.made_count,
            self.from_list,
            self.list_left_out,
            self.refused_text(),
            self.failed_pipelines_text(),
            self.draws_refused,
            self.draws_failed,
            self.draws_not_made,
            self.past_cap,
            self.no_key
        )
    }

    /// The facts of the keys of W300: how many keys have each one, the keys that asked for a pipeline that was not
    /// ready ([`StageShaders::not_ready`]), the modules made (a replay's here, a live session's sent to the thread; not
    /// the uber-shader's), and the draws that took their key again with a plane.
    pub(crate) fn facts_report(&self) -> String {
        let keys = &self.keys[1..];
        let count = |fact: fn(&StageKey) -> bool| keys.iter().filter(|key| fact(key)).count();
        let asked = self.not_ready.iter().filter(|&&id| id != 0).count();
        let modules = self
            .modules
            .iter()
            .filter(|(id, module)| id.0 != 0 && module.is_some())
            .count()
            + self.sent_modules.iter().filter(|id| id.0 != 0).count();
        format!(
            "stage key facts: {} with a plane, {} flat, {} of the sky (bit 4 or 8), {} of the scene (the table); {} keys asked for a pipeline that was not ready in this session, {} did not (cap {MAX_KEYS}); {} stage modules made; draws that took their key again with a plane: {} split copies, {} of a view",
            count(|key| key.plane),
            count(|key| key.flat),
            count(|key| key.shade as u32 & SKY_BITS != 0),
            count(|key| key.scene),
            asked,
            keys.len() - asked,
            modules,
            self.again_split,
            self.again_view
        )
    }

    /// The report of [`UBER_SET`]: the keys sent to the thread, made before their first draw, drawn first with a
    /// pipeline of the frame thread (late, or a key that the thread could not make), and not drawn in the session,
    /// by their place from 1 among the keys sent (the table's place, unless the frame has one sample).
    pub(crate) fn uber_set_report(&self) -> String {
        let not_drawn: Vec<String> = self
            .uber_set
            .iter()
            .enumerate()
            .filter(|(_, key)| !self.uber_drawn.contains(key))
            .map(|(index, _)| (index + 1).to_string())
            .collect();
        format!(
            "uber set: {} keys sent, {} made before their first draw, {} drawn first on the frame thread, {} not drawn{}{}",
            self.uber_set.len(),
            self.uber_ahead,
            self.uber_late,
            not_drawn.len(),
            if not_drawn.is_empty() { "" } else { ": " },
            not_drawn.join(" ")
        )
    }
}

impl Drop for StageShaders {
    /// The end of the session writes the list once more, when it changed.
    fn drop(&mut self) {
        self.save();
    }
}

/// The list: `MGE_XE_PIPELINE_LIST` when set, else [`list_name`] in `%LOCALAPPDATA%\MGE XE`.
fn list_path(world_hash: u64) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MGE_XE_PIPELINE_LIST") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("LOCALAPPDATA").map(|folder| PathBuf::from(folder).join("MGE XE").join(list_name(world_hash)))
}

/// The list's file name, with the list's format and the hash of the world shader: two installs,
/// two texture tiers or two builds with another format of the key do not write over each other's
/// list.
fn list_name(world_hash: u64) -> String {
    format!("numidium-stage-pipelines-{LIST_FORMAT}-{world_hash:016x}.txt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug_lines::view_id;
    use bytemuck::Zeroable;

    fn world() -> String {
        crate::request::world_shader_source(&crate::extension::extension_shaders(None))
    }

    fn key(count: u8, alpha_test: u8) -> StageKey {
        let mut key = StageKey {
            count,
            alpha_test,
            shade: 1 | 128,
            lit: 1,
            normal: 1,
            ..Default::default()
        };
        // Stage 0: MODULATE of the texture and the diffuse; stage 1: ADD of the texture and
        // the current colour (an environment map after the base map).
        key.color[0] = [4, 2, 0, 1];
        key.alpha[0] = [4, 2, 0];
        if count > 1 {
            key.color[1] = [7, 2, 1, 1];
            key.alpha[1] = [1, 1, 0];
        }
        key
    }

    #[test]
    fn each_key_gives_valid_wgsl() {
        let world = world();
        for (count, test) in [(0, 0), (1, 0), (1, 5), (2, 0), (6, 7)] {
            let source = fragment_source(&world, &key(count, test)).expect("the rewrite finds its parts");
            crate::assert_wgsl("stage shader", &source);
        }
    }

    #[test]
    fn each_set_of_facts_gives_valid_wgsl() {
        let world = world();
        for (shade, lit, normal, scene, adds) in [
            (0, 0, 0, false, false),
            (SHADE_BITS as u16, 1, 1, true, true),
            (1 | 32 | 128, 1, 0, false, true),
            (2 | 16 | 512, 0, 1, true, false),
            (crate::CLOUDS_HOOK_BIT as u16, 1, 0, false, false),
        ] {
            let key = StageKey {
                shade,
                lit,
                normal,
                scene,
                adds,
                ..key(1, 0)
            };
            let source = fragment_source(&world, &key).expect("the rewrite finds its parts");
            crate::assert_wgsl("stage shader", &source);
        }
    }

    #[test]
    fn the_facts_of_the_key_are_constants() {
        let key = StageKey {
            shade: 1 | 32,
            lit: 1,
            normal: 0,
            scene: false,
            ..key(1, 0)
        };
        let source = fragment_source(&world(), &key).unwrap();
        assert!(source.contains("fn from_scene() -> bool {\n    return false;\n}\n"));
        assert!(!source.contains("light_range.w & 64u"));
        assert!(!source.contains("draws[record].flags.x"));
        assert!(!source.contains("draws[record].flags.y"));
        let masked = format!("((draws[record].counts.w & {}u) | 33u)", !SHADE_BITS);
        assert!(source.contains(&masked));
        assert_eq!(
            source.matches("draws[record].counts.w").count(),
            source.matches(&masked).count(),
            "a read of counts.w without the key's bits"
        );
    }

    #[test]
    fn a_field_read_is_replaced_and_a_longer_name_is_not() {
        let text = "a = d.flags.x; b = d.flags.xy; c = d.flags.x_2; e = d.flags.x\n";
        assert_eq!(
            replace_field(text, "d.flags.x", "1u").as_deref(),
            Some("a = 1u; b = d.flags.xy; c = d.flags.x_2; e = 1u\n")
        );
        assert_eq!(replace_field("d.flags.xy", "d.flags.x", "1u"), None);
    }

    #[test]
    fn the_key_takes_the_facts_of_the_record() {
        let mut data = DrawData::zeroed();
        data.counts[3] = 1 | 4 | 32 | 2048;
        data.flags = [1, 1, 0, 0];
        data.light_range[3] = 4 | 1 | crate::ADDS_BIT;
        let key = StageKey::of(&data).unwrap();
        assert_eq!(
            (key.shade, key.lit, key.normal, key.scene, key.adds, key.flat, key.plane),
            (1 | 4 | 32, 1, 1, true, true, false, false)
        );
        // A bit outside SHADE_BITS (2048) does not split a key.
        let mut other = data;
        other.counts[3] = 1 | 4 | 32;
        assert_eq!(StageKey::of(&other), Some(key));
    }

    #[test]
    fn the_flat_and_the_plane_are_facts_of_the_key() {
        let mut data = DrawData::zeroed();
        data.counts[3] = 1;
        let plain = StageKey::of(&data).unwrap();
        data.light_range[3] = 16;
        let flat = StageKey::of(&data).unwrap();
        assert!(flat.flat && !plain.flat);
        data.light_range[3] = 0;
        data.clip_plane = [0.0, 0.0, 1.0, -5.0];
        let plane = StageKey::of(&data).unwrap();
        assert!(plane.plane && !plain.plane);
        // A draw with the scene's values takes the scene's plane, which is 0: no plane.
        data.light_range[3] = 4;
        assert!(!StageKey::of(&data).unwrap().plane);
    }

    #[test]
    fn the_flat_the_plane_and_the_sky_are_constants_of_the_text() {
        let world = world();
        for (flat, bit) in [(false, 0), (true, 16)] {
            let source = fragment_source(&world, &StageKey { flat, ..key(1, 0) }).unwrap();
            assert!(!source.contains("light_range.w & 16u"));
            assert!(
                source.contains(&format!("    if ({bit}u) != 0u {{\n        inp.color = inp.color_flat;")),
                "{flat}"
            );
            // The vertex stage's test of a draw with no lights takes the key's flat bit too.
            assert!(
                source.contains(&format!("(draw_counts_w() & 32768u) != 0u && ({bit}u) == 0u")),
                "{flat}"
            );
        }
        let without = fragment_source(&world, &key(1, 0)).unwrap();
        assert!(!without.contains("any(clip_plane != vec4f(0.0))"));
        assert!(without.contains("== 0u && false && dot(inp.world_position, clip_plane.xyz)"));
        let with = fragment_source(
            &world,
            &StageKey {
                plane: true,
                ..key(1, 0)
            },
        )
        .unwrap();
        assert!(with.contains("== 0u && any(clip_plane != vec4f(0.0)) && dot("));
        for source in [&without, &with] {
            crate::assert_wgsl("stage shader", source);
        }
        let all = StageKey {
            flat: true,
            plane: true,
            shade: SHADE_BITS as u16,
            ..key(2, 0)
        };
        crate::assert_wgsl("stage shader", &fragment_source(&world, &all).unwrap());
        // The scene's part of the bits has no sky bit, in each key.
        for source in [&without, &with] {
            let masked = format!("(scene.counts[k >> 2u][k & 3u] & {}u)", !SKY_BITS);
            assert_eq!(source.matches(&masked).count(), 1);
            assert_eq!(source.matches("scene.counts[").count(), 1);
        }
    }

    /// No capture of the play set has a flat-shaded draw or the shadow of a moon. So the text is
    /// their test: the text of a key with the bit and of the same key without it differ only in
    /// the bit's constants, and nothing else of the text changes.
    #[test]
    fn the_flat_bit_and_the_moon_bit_change_only_their_constants() {
        let world = world();

        // The flat bit: each read of it is the constant 0u or 16u (the start of `fs`, the
        // vertex stage's and the extension's tests of a draw with no lights).
        let smooth = fragment_source(&world, &key(1, 0)).unwrap();
        let flat = fragment_source(&world, &StageKey { flat: true, ..key(1, 0) }).unwrap();
        assert_eq!(smooth.lines().count(), flat.lines().count());
        let mut changed = 0;
        for (without, with) in smooth.lines().zip(flat.lines()) {
            if without != with {
                assert_eq!(without.replace("(0u)", "(16u)"), with);
                changed += 1;
            }
        }
        assert!(changed >= 2, "{changed} lines changed");

        // The shadow of a moon, bit 8 of the shade: the key's bits in each read of `counts.w`.
        let base = key(1, 0);
        let moon = StageKey {
            shade: base.shade | 8,
            ..base
        };
        let without = fragment_source(&world, &base).unwrap();
        let with = fragment_source(&world, &moon).unwrap();
        let (from, to) = (format!("| {}u)", base.shade), format!("| {}u)", moon.shade));
        assert!(without.contains(&from));
        assert_eq!(without.replace(&from, &to), with);
        assert!(with.contains("(draw_counts_w() & 1036u) != 0u"));
        for source in [&smooth, &flat, &with] {
            crate::assert_wgsl("stage shader", source);
        }
    }

    #[test]
    fn each_write_of_a_plane_is_known() {
        // The plane is a fact of the stage key, taken from the record. A scene's plane must stay
        // 0 (a scene's draw has no plane in its key), and a record that takes a plane after its
        // key must take its key again. So each write of a plane in the crate is one of these.
        // When a new write fails here, do not only add it to the list: a write to a record takes
        // the key again right after it (`key.stages = stage_shaders.id_with_plane(..)`, as the two
        // here do), and a scene's plane stays 0, or `StageKey::of` must read the scene's plane
        // for a scene's draw first. A key with no plane for a record with one draws the far side.
        let known = [
            ("player/draws.rs", "data.clip_plane = plane_of_draw(plane, draw.origin);"),
            ("replay.rs", "data.clip_plane = plane_of_draw(plane, kept.origin);"),
            ("player/items.rs", "clip_plane: Some(plane),"),
            ("retained.rs", "clip_plane: [0.0; 4],"),
        ];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut folders = vec![root.clone()];
        let mut found = Vec::new();
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let name = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
                let text = std::fs::read_to_string(&path).unwrap();
                // The writes outside the test modules (rustfmt ends a module with "}" alone).
                let mut in_tests = false;
                let mut last = "";
                for line in text.lines() {
                    if line.starts_with("mod ") && last == "#[cfg(test)]" {
                        in_tests = true;
                    }
                    let code = line.trim();
                    let write = code.contains("clip_plane =") && !code.contains("clip_plane ==")
                        || code.contains("clip_plane[")
                        || code.starts_with("clip_plane:");
                    if !in_tests && write {
                        found.push((name.clone(), code.to_string()));
                    }
                    if in_tests && line == "}" {
                        in_tests = false;
                    }
                    last = line;
                }
            }
        }
        found.sort();
        let mut expected: Vec<(String, String)> = known.iter().map(|&(f, l)| (f.to_string(), l.to_string())).collect();
        expected.sort();
        assert_eq!(found, expected);
    }

    /// A switch of the world text keeps each stage key at its id, so that an id the retained table kept
    /// still names its key and the rewrite finds it (live run G3 of 2026-11-16: "the stage key of the id: 0 in
    /// the text, 1 wanted" after the switch). The null: stage shaders made new for the text lose the id.
    #[test]
    fn a_switch_of_the_world_text_keeps_the_ids_of_the_stage_keys() {
        let mut data = DrawData::zeroed();
        data.counts[3] = 1 | 128;
        data.flags = [1, 1, 0, 0];
        let mut shaders = StageShaders::new(true, &world());
        let id = shaders.id_of(&data);
        assert_ne!(id, 0);
        shaders.past_cap = 3;
        shaders.no_key = 2;

        let switched_text = format!("{}\n// switched\n", world());
        let mut switched = shaders.switched(&switched_text);
        assert!(switched.source_of((id, 0)).is_ok());
        assert_eq!(switched.id_of(&data), id);
        assert!(StageShaders::new(true, &switched_text).source_of((id, 0)).is_err());

        // The counts of the key table go with the keys: the shapes that got id 0 do not ask again.
        assert_eq!((switched.past_cap, switched.no_key), (3, 2));

        // A key that is new after the switch takes the next id.
        data.light_range[3] = crate::ADDS_BIT;
        assert_eq!(switched.id_of(&data), id + 1);
    }

    #[test]
    fn the_add_bit_is_a_constant_of_the_key() {
        // A draw that adds and one that does not take two keys, and each key's `fogged` has the
        // test as a constant: the code of a draw that does not add has no branch on the bit (W287).
        let mut data = DrawData::zeroed();
        data.counts[3] = 1 | 128;
        data.flags = [1, 1, 0, 0];
        let other = StageKey::of(&data).unwrap();
        data.light_range[3] = crate::ADDS_BIT;
        let adding = StageKey::of(&data).unwrap();
        assert_ne!(other, adding);

        let world = world();
        for (key, constant) in [(other, "if false {"), (adding, "if true {")] {
            let source = fragment_source(&world, &key).unwrap();
            let fogged = &source[source.find("fn fogged(").unwrap()..];
            let fogged = &fogged[..fogged.find("\n}\n").unwrap()];
            assert!(fogged.contains(constant), "{fogged}");
            assert!(!source.contains("light_range.w & 64u"));
        }
    }

    #[test]
    fn no_lights_is_a_fact_of_the_key() {
        let mut data = DrawData::zeroed();
        data.counts[3] = 1 | 16 | 128 | 512;
        data.flags = [1, 0, 1, 0];
        let lit = StageKey::of(&data).unwrap();
        data.counts[3] |= crate::NO_LIGHTS_BIT;
        let without_lights = StageKey::of(&data).unwrap();
        assert_ne!(lit, without_lights);
        assert_eq!(u32::from(without_lights.shade), 1 | 16 | 128 | 512 | crate::NO_LIGHTS_BIT);
    }

    #[test]
    fn the_corner_light_is_a_fact_of_the_key() {
        let mut data = DrawData::zeroed();
        data.counts[3] = 1 | 16 | 128 | 512;
        data.flags = [1, 0, 1, 0];
        let per_pixel = StageKey::of(&data).unwrap();
        data.counts[3] |= crate::CORNER_LIGHT_BIT;
        let corners = StageKey::of(&data).unwrap();
        assert_ne!(per_pixel, corners);
        assert_eq!(u32::from(corners.shade), 1 | 16 | 128 | 512 | crate::CORNER_LIGHT_BIT);
    }

    #[test]
    fn the_rewrite_leaves_no_loop_and_no_unused_read() {
        let source = fragment_source(&world(), &key(2, 0)).unwrap();
        assert!(!source.contains("for (var i = 0u; i < draws[record].counts.y; i++)"));
        assert!(source.contains("    texels[2] = vec4f(1.0);"));
        assert!(!source.contains("draws[record].more.z != 0u && !compare"));
        // Stage 0 is no bump stage: stage 1 reads its texture with no bump code.
        assert!(source.contains("    texels[1] = textureSample(textures[draws[record].stages[1].slots.x], samplers[draws[record].stages[1].slots.y], final_coordinate(1u, inp.coordinate1));\n"));
        assert!(!source.contains("    texels[1] = bump_light("));
    }

    #[test]
    fn a_stage_after_a_bump_stage_keeps_the_bump_code() {
        let world = world();
        for op in [22, 23] {
            let mut bump = key(3, 0);
            bump.color[1] = [op, 2, 1, 1];
            bump.color[2] = [4, 2, 0, 1];
            let source = fragment_source(&world, &bump).unwrap();
            crate::assert_wgsl("stage shader", &source);
            assert!(source.contains("    texels[2] = bump_light(1u, texels[1], textureSample("));
            assert!(source.contains(" + bump_move(1u, texels[1])));\n"));
            assert!(!source.contains("    texels[1] = bump_light("));
        }
    }

    /// The lines after the loop are free: a line between the end mark and the alpha test, the place of a
    /// hook's call, does not move the range, and each kind of key gets its text with the line in it.
    #[test]
    fn a_line_after_the_end_mark_leaves_the_loop_where_it_is() {
        let world = world();
        let planted = world.replacen(STAGE_LOOP_END, &format!("{STAGE_LOOP_END}    current = current * 1.0;\n"), 1);
        assert_ne!(planted, world);
        assert_eq!(stage_loop_range(&planted), stage_loop_range(&world));
        assert_eq!(stage_rewrite_refusals(&planted), Vec::<String>::new());
        for (_, key) in kinds() {
            let source = fragment_source(&planted, &key).unwrap();
            assert_eq!(source.matches("    current = current * 1.0;\n").count(), 1);
        }
    }

    /// Each mark of the loop is in the crate's text one time, and neither holds the text that the join
    /// rewrites in an extension's text.
    #[test]
    fn the_marks_of_the_stage_loop_are_in_the_shader_one_time() {
        let shader = include_str!("shader.wgsl");
        assert_eq!(shader.matches(STAGE_LOOP_BEGIN).count(), 1);
        assert_eq!(shader.matches(STAGE_LOOP_END).count(), 1);
        for mark in [
            STAGE_LOOP_BEGIN,
            STAGE_LOOP_END,
            crate::request::EXTENSION_BEGIN,
            crate::request::EXTENSION_END,
        ] {
            assert!(!mark.contains("draw."), "{mark}");
        }
        let world = world();
        assert_eq!(world.matches(crate::request::EXTENSION_BEGIN).count(), 1);
        assert_eq!(world.matches(crate::request::EXTENSION_END).count(), 1);
    }

    /// A mark that is gone, or there two times, is named with its count; no key gets a text.
    #[test]
    fn a_broken_mark_of_the_stage_loop_is_named() {
        let world = world();
        let no_end = world.replacen(STAGE_LOOP_END, "", 1);
        let refusal = fragment_source(&no_end, &key(1, 0)).unwrap_err();
        assert_eq!((refusal.part.as_str(), refusal.found), ("the end mark of the stage loop", 0));
        let two_begins = world.replacen(STAGE_LOOP_END, &format!("{STAGE_LOOP_END}{STAGE_LOOP_BEGIN}\n"), 1);
        let refusal = fragment_source(&two_begins, &key(1, 0)).unwrap_err();
        assert_eq!(
            (refusal.part.as_str(), refusal.found),
            ("the begin mark of the stage loop", 2)
        );
        assert_eq!(
            refusal.to_string(),
            "the begin mark of the stage loop: 2 in the text, 1 wanted"
        );
        let refusals = stage_rewrite_refusals(&no_end);
        assert_eq!(refusals.len(), kinds().len());
        assert_eq!(
            refusals[0],
            "a plain key: the end mark of the stage loop: 0 in the text, 1 wanted"
        );
    }

    /// The text of an extension or of a mod's file can hold a second copy of each part that the rewrite needs
    /// one time: the rewrite searches those parts in the crate's own text only, so each kind of key still gets
    /// its text, and the copies stay as the mod wrote them. The null: the same text with no boundary line gives
    /// no text for any kind.
    #[test]
    fn a_second_copy_of_a_part_in_the_extensions_text_changes_nothing() {
        let copies = [
            "    // STAGE LOOP BEGIN\n",
            "    // STAGE LOOP END\n",
            "    for (var i = 0u; i < draws[record].counts.y; i++) {\n",
            "    if draws[record].more.z != 0u && !compare(draws[record].more.z, floor(current.a * 255.0 + 0.5), f32(draws[record].more.w)) {\n        discard;\n    }\n",
            "        if draws[record].more.z != 0u && !compare(draws[record].more.z, floor(cloud.a * 255.0 + 0.5), f32(draws[record].more.w)) {\n            discard;\n        }\n",
            "fn from_scene() -> bool {\n    return (draws[record].light_range.w & 4u) != 0u;\n}\n",
            "(draws[record].light_range.w & 64u) != 0u\n",
            "any(clip_plane != vec4f(0.0))\n",
            "scene.counts[k >> 2u][k & 3u]\n",
            "    texels[1] = bump_light(\n",
            "// EXTENSION TEXT BEGIN: the crate's own text ends here.\n",
        ];
        // The mod's text here is not code that compiles: the rewrite works on the text and does not read WGSL.
        let mod_text: String = copies.concat();
        let own = crate::extension::extension_shaders(None);
        let world = crate::request::world_shader_source(&format!("{own}{mod_text}"));
        assert_eq!(stage_rewrite_refusals(&world), Vec::<String>::new());
        let source = fragment_source(&world, &key(2, 5)).unwrap();
        for copy in &copies[..copies.len() - 1] {
            assert!(source.contains(copy), "the copy is as the mod wrote it: {copy}");
        }

        // The null: with no boundary line the whole text is searched, and every kind is refused.
        let no_boundary = world.replacen(crate::request::EXTENSION_BEGIN, "", 1);
        let refusals = stage_rewrite_refusals(&no_boundary);
        assert_eq!(refusals.len(), kinds().len(), "{refusals:?}");
        assert!(refusals[0].ends_with("2 in the text, 1 wanted"), "{}", refusals[0]);
    }

    /// The rewrite makes a text for each kind of key on the crate's own text, in both forms of the world
    /// shader.
    #[test]
    fn the_rewrite_makes_a_text_for_each_kind_of_key() {
        let world = world();
        assert_eq!(stage_rewrite_refusals(&world), Vec::<String>::new());
        assert_eq!(
            stage_rewrite_refusals(&crate::tier_c_shader_source(&world)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_stage_loop_is_the_code_that_the_steps_copy() {
        let world = world();
        let (start, end) = stage_loop_range(&world).expect("the loop of fs");
        assert_eq!(
            without_comments(&world[start..end]),
            STAGE_LOOP,
            "fs changed its stage loop: change the steps of fragment_source and STAGE_LOOP with it"
        );
        // A new line in the loop body is refused.
        let planted = world.replacen(
            "        var alpha = current;\n",
            "        var alpha = current;\n        current = current * 0.5;\n",
            1,
        );
        assert_eq!(
            fragment_source(&planted, &key(2, 0)).unwrap_err().wanted,
            "its code is not the code that the steps copy"
        );
        // A new comment is not a change of the code.
        let commented = world.replacen(
            "        var alpha = current;\n",
            "        var alpha = current;\n        // A note.\n",
            1,
        );
        assert!(fragment_source(&commented, &key(2, 0)).is_ok());
    }

    #[test]
    fn fs_reads_the_texels_only_in_the_reads_and_the_stage_loop() {
        let shader = include_str!("shader.wgsl");
        let fs_start = shader.find("@fragment\nfn fs(").expect("fs");
        let fs = &shader[fs_start..fs_start + shader[fs_start..].find("\n}\n").expect("the end of fs")];
        let (start, end) = stage_loop_range(fs).expect("the loop of fs");
        let outside = format!("{}{}", &fs[..start], &fs[end..]);
        let uses: Vec<&str> = outside.lines().filter(|line| line.contains("texels[")).collect();
        let reads: Vec<String> = (0..STAGES).map(|stage| format!("    texels[{stage}] = ")).collect();
        assert_eq!(
            uses.len(),
            STAGES,
            "fs uses the texels outside the reads and the loop: {uses:?}"
        );
        for (line, read) in uses.iter().zip(&reads) {
            assert!(
                line.starts_with(read.as_str()),
                "a use of the texels that is not a read: {line}"
            );
        }
    }

    #[test]
    fn the_tier_c_text_rewrites_too() {
        let world = crate::request::tier_c_shader_source(&world());
        assert!(fragment_source(&world, &key(2, 0)).is_ok());
    }

    #[test]
    fn a_key_and_a_pipeline_go_through_the_list() {
        let stage = StageKey {
            scene: true,
            adds: true,
            ..key(2, 5)
        };
        assert_eq!(StageKey::from_words(&stage.words()), Some(stage));
        let own = StageKey {
            flat: true,
            plane: true,
            shade: 1 | 8,
            ..key(2, 5)
        };
        assert_eq!(StageKey::from_words(&own.words()), Some(own));
        // A bit outside SHADE_BITS is not a key of this build.
        let mut words = stage.words();
        words[2] |= 2048;
        assert_eq!(StageKey::from_words(&words), None);
        // The add bit is a flag: another value is not a key of this build.
        let mut words = stage.words();
        words[6] = 2;
        assert_eq!(StageKey::from_words(&words), None);
        // A draw with the scene's values has no plane.
        let mut words = stage.words();
        words[8] = 1;
        assert_eq!(StageKey::from_words(&words), None);
        // A line of format 4 (7 facts) is not a key of this build.
        assert_eq!(StageKey::from_words(&stage.words()[2..]), None);
        let pipeline = PipelineKey {
            fvf: 0x152,
            stride: 24,
            topology: 4,
            blend: [1, 5, 6],
            depth: [1, 0, 4],
            cull: 1,
            fill: 3,
            stencil: [0; 7],
            color_write: 7,
            samples: 1,
            stages: 0,
            debug: 0,
        };
        let mut words = pipeline_words(&pipeline);
        words.push(0);
        assert_eq!(pipeline_from_words(&words), Some(pipeline));
    }

    #[test]
    fn the_list_of_another_format_is_another_file() {
        assert_eq!(
            list_name(0x1234),
            format!("numidium-stage-pipelines-{LIST_FORMAT}-0000000000001234.txt")
        );
        // The name of the list of the builds before the format was in the name.
        assert_ne!(list_name(0x1234), "numidium-stage-pipelines-0000000000001234.txt");
    }

    #[test]
    fn unused_stages_do_not_split_a_key() {
        let mut a = DrawData::zeroed();
        a.counts[1] = 1;
        a.stages[0].color = [4, 2, 0, 1];
        let mut b = a;
        b.stages[3].color = [7, 2, 1, 0];
        assert_eq!(StageKey::of(&a), StageKey::of(&b));
    }

    fn pipeline_key(stages: u32, debug: u32) -> PipelineKey {
        PipelineKey {
            fvf: 0x152,
            stride: 24,
            topology: 4,
            blend: [1, 5, 6],
            depth: [1, 0, 4],
            cull: 1,
            fill: 3,
            stencil: [0; 7],
            color_write: 7,
            samples: 1,
            stages,
            debug,
        }
    }

    /// The list keeps no variant: a key of a variant reads back from its words as the key of
    /// normal play.
    #[test]
    fn the_list_words_keep_no_variant() {
        let mut words = pipeline_words(&pipeline_key(0, view_id::LIGHTS));
        words.push(0);
        assert_eq!(pipeline_from_words(&words), Some(pipeline_key(0, 0)));
    }

    /// A stage module is kept by its stage key and its variant: the first pipeline of a variant
    /// sends the variant's text, though the thread has the module of normal play of that key.
    #[test]
    fn a_stage_module_is_kept_by_its_key_and_its_variant() {
        let mut shaders = StageShaders::new(true, &world());
        let (jobs, received) = mpsc::channel();
        shaders.jobs = Some(jobs);
        let id = shaders.intern(key(1, 0));
        shaders.set_variant(view_id::LIGHTS);
        assert!(shaders.send(pipeline_key(id, 0)));
        assert!(shaders.send(pipeline_key(id, view_id::LIGHTS)));
        assert!(shaders.send(PipelineKey {
            cull: 2,
            ..pipeline_key(id, view_id::LIGHTS)
        }));
        let texts: Vec<Option<String>> = received.try_iter().map(|job| job.source).collect();
        assert_eq!(texts.len(), 3);
        let normal = texts[0].as_deref().expect("the text of normal play");
        let variant = texts[1].as_deref().expect("the text of the variant");
        assert!(normal.contains("const DEBUG_VIEW: u32 = 0u;"));
        assert!(variant.contains("const DEBUG_VIEW: u32 = 1u;"));
        crate::assert_wgsl("stage shader of a variant", variant);
        // The second pipeline of the variant takes the module that the thread has.
        assert_eq!(texts[2], None);
    }

    /// A key whose rewrite fails is named with the part that refused, and its draws count as draws after a
    /// failed rewrite. A live session has no fault of a replay for it; the same counts in a replay are a fault,
    /// in one line with the key and each count.
    #[test]
    fn a_failed_rewrite_is_counted_with_its_key_and_its_draws() {
        // A world text with no end mark of the stage loop: no key gets a text.
        let broken = world().replacen(STAGE_LOOP_END, "", 1);
        let mut shaders = StageShaders::new(true, &broken);
        let (jobs, _received) = mpsc::channel();
        shaders.jobs = Some(jobs);
        shaders.sync = false;
        let id = shaders.intern(key(1, 0));
        let pipeline = pipeline_key(id, 0);
        // As `pipeline` does for a key whose send fails, and then for each of its draws.
        assert!(!shaders.send(pipeline));
        shaders.note_failed_pipeline(pipeline);
        shaders.count_uber_draw(&pipeline);
        shaders.count_uber_draw(&pipeline);
        // A second pipeline of the same key adds no second line and no second key.
        let other = PipelineKey { cull: 2, ..pipeline };
        assert!(!shaders.send(other));
        shaders.note_failed_pipeline(other);
        shaders.count_uber_draw(&other);

        let refusal = "the end mark of the stage loop: 0 in the text, 1 wanted";
        assert_eq!(shaders.refused.iter().collect::<Vec<_>>(), [(&id, &refusal.to_string())]);
        assert!(shaders.failed_pipelines.is_empty());
        assert_eq!(
            (shaders.draws_refused, shaders.draws_failed, shaders.draws_not_made),
            (3, 0, 0)
        );
        let counts = format!(
            "1 rewrites failed (stage key {id}: {refusal}); 0 pipelines failed; draws on the uber-shader with a stage key: 3 after a failed rewrite, 0 after a failed pipeline, 0 with a pipeline not made; 0 draws past the cap, 0 draws with no key"
        );
        assert!(shaders.report().ends_with(&counts), "{}", shaders.report());
        assert_eq!(shaders.replay_fault(), None, "a live session has no fault of a replay");

        shaders.sync = true;
        let fault = shaders.replay_fault().expect("a replay with a failed rewrite");
        assert_eq!(
            fault,
            format!(
                "stage shaders: a fault of the replay: 1 rewrites failed (stage key {id}: {refusal}); 0 pipelines failed; the uber-shader drew 3 draws that have a stage key (3 after a failed rewrite, 0 after a failed pipeline, 0 with a pipeline not made); 0 draws past the cap of {MAX_KEYS} keys"
            )
        );
    }

    /// A pipeline that the thread could not make is a failed pipeline, apart from a failed rewrite; a draw of
    /// a key that waits for the thread is a draw with a pipeline not made. A replay with no failure and no such
    /// draw has no fault, and a draw past the cap is one.
    #[test]
    fn a_failed_pipeline_and_a_pipeline_not_made_are_counted_apart() {
        let mut shaders = StageShaders::new(true, &world());
        let (jobs, _received) = mpsc::channel();
        shaders.jobs = Some(jobs);
        let (done_sender, done) = mpsc::channel();
        shaders.done = Some(done);
        shaders.sync = false;
        let id = shaders.intern(key(1, 0));
        let pipeline = pipeline_key(id, 0);
        assert!(shaders.send(pipeline));
        shaders.count_uber_draw(&pipeline);
        done_sender
            .send(Done {
                key: pipeline,
                pipeline: None,
            })
            .unwrap();
        let mut pipelines = rustc_hash::FxHashMap::default();
        shaders.receive(&mut pipelines);
        shaders.count_uber_draw(&pipeline);

        assert!(shaders.refused.is_empty());
        assert_eq!(shaders.failed_pipelines.iter().collect::<Vec<_>>(), [&id]);
        assert_eq!(
            (shaders.draws_refused, shaders.draws_failed, shaders.draws_not_made),
            (0, 1, 1)
        );
        assert!(shaders
            .report()
            .contains(&format!("0 rewrites failed; 1 pipelines failed (stage keys {id}); ")));
        assert!(shaders.report().contains("(made on a thread, with no list: a cold start)"));
        assert_eq!(shaders.replay_fault(), None);
        shaders.sync = true;
        assert!(shaders.replay_fault().unwrap().contains("1 pipelines failed (stage keys 1)"));

        let mut clean = StageShaders::new(true, &world());
        assert_eq!(clean.replay_fault(), None);
        assert!(clean.report().contains("(made at the first use)"));
        clean.past_cap = 3;
        let fault = clean.replay_fault().expect("a replay past the cap");
        assert!(fault.ends_with("3 draws past the cap of 256 keys"), "{fault}");
    }

    /// A change of the variant drops what the stage shaders keep of the old one, and a job of
    /// the old variant that comes back later is discarded.
    #[test]
    fn a_change_of_the_variant_drops_the_old_one() {
        let mut shaders = StageShaders::new(true, &world());
        let (jobs, _received) = mpsc::channel();
        shaders.jobs = Some(jobs);
        let (done_sender, done) = mpsc::channel();
        shaders.done = Some(done);
        shaders.sync = false;
        let id = shaders.intern(key(1, 0));
        let lights = pipeline_key(id, view_id::LIGHTS);
        let layers = pipeline_key(id, view_id::SHADOW_LAYERS);

        shaders.set_variant(view_id::LIGHTS);
        assert!(shaders.send(lights));
        shaders.failed.insert(PipelineKey { cull: 2, ..lights });
        shaders.set_variant(view_id::SHADOW_LAYERS);
        assert!(shaders.pending.is_empty() && shaders.failed.is_empty() && shaders.sent_modules.is_empty());
        assert_eq!(shaders.world_text(view_id::LIGHTS), None);
        assert!(shaders.world_text(view_id::SHADOW_LAYERS).is_some());

        // The job of the dropped variant fails late: it is not a failed pipeline of the session.
        // A failed job of the variant of the frames is.
        done_sender
            .send(Done {
                key: lights,
                pipeline: None,
            })
            .unwrap();
        done_sender
            .send(Done {
                key: layers,
                pipeline: None,
            })
            .unwrap();
        let mut pipelines = rustc_hash::FxHashMap::default();
        shaders.receive(&mut pipelines);
        assert!(!shaders.failed.contains(&lights));
        assert!(shaders.failed.contains(&layers));
        assert_eq!(shaders.failed_pipelines.iter().collect::<Vec<_>>(), [&id]);

        shaders.set_variant(0);
        assert!(shaders.failed.is_empty());
        assert_eq!(shaders.world_text(view_id::SHADOW_LAYERS), None);
    }

    /// A change of the key's layout breaks the words of the list: each key of the uber set must come back from its
    /// words, with the frame's samples of a multisampled frame and of a frame with one sample.
    #[test]
    fn each_key_of_the_uber_set_comes_back_from_its_words() {
        for frame_samples in [4, 1] {
            for key in UBER_SET {
                let samples = if key.samples == FRAME_SAMPLES {
                    frame_samples
                } else {
                    key.samples
                };
                let key = PipelineKey { samples, ..key };
                let mut words = pipeline_words(&key);
                words.push(0);
                assert_eq!(pipeline_from_words(&words), Some(key));
            }
        }
    }

    /// A shaders of a live session with a channel of its own in place of the thread.
    fn live_shaders() -> (StageShaders, mpsc::Receiver<Job>) {
        let mut shaders = StageShaders::new(true, &world());
        let (jobs, job_receiver) = mpsc::channel::<Job>();
        shaders.jobs = Some(jobs);
        shaders.sync = false;
        (shaders, job_receiver)
    }

    #[test]
    fn the_uber_set_goes_to_the_thread_in_its_order_with_the_module_first() {
        let (mut shaders, job_receiver) = live_shaders();
        shaders.send_uber_set(4);
        let jobs: Vec<Job> = job_receiver.try_iter().collect();
        assert_eq!(jobs.len(), UBER_SET.len());
        assert_eq!(jobs[0].source.as_deref(), Some(shaders.world_source.as_str()));
        assert!(jobs[1..].iter().all(|job| job.source.is_none()));
        for (job, key) in jobs.iter().zip(UBER_SET) {
            let samples = if key.samples == FRAME_SAMPLES { 4 } else { key.samples };
            assert_eq!(job.key, PipelineKey { samples, ..key });
            assert_eq!((job.key.stages, job.key.debug), (0, 0));
        }
        assert_eq!(shaders.pending.len(), UBER_SET.len());
        assert!(shaders.sent_modules.contains(&(0, 0)));
    }

    #[test]
    fn with_a_list_the_uber_set_is_not_sent() {
        let (mut shaders, job_receiver) = live_shaders();
        shaders.queued.push_back((UBER_SET[0], key(1, 0)));
        shaders.send_uber_set(4);
        assert_eq!(job_receiver.try_iter().count(), 0);
        assert!(shaders.uber_set.is_empty() && shaders.pending.is_empty());
    }

    #[test]
    fn with_one_sample_a_key_of_the_frame_that_equals_a_key_of_a_texture_goes_once() {
        let (mut shaders, job_receiver) = live_shaders();
        shaders.send_uber_set(1);
        let keys: Vec<PipelineKey> = job_receiver.try_iter().map(|job| job.key).collect();
        let resolved: HashSet<PipelineKey> = UBER_SET.iter().map(|key| PipelineKey { samples: 1, ..*key }).collect();
        assert_eq!(keys.len(), resolved.len());
        assert_eq!(keys.iter().copied().collect::<HashSet<_>>(), resolved);
        assert!(keys.iter().all(|key| key.samples == 1));
    }

    #[test]
    fn a_key_of_the_set_drawn_before_the_thread_gives_it_back_counts_late_once() {
        let (mut shaders, _job_receiver) = live_shaders();
        shaders.send_uber_set(4);
        let first = shaders.uber_set[0];
        let outside = PipelineKey { fvf: 2, ..first };
        assert!(shaders.uber_pipeline(outside).is_none());
        assert!(shaders.uber_pipeline(first).is_none());
        assert!(shaders.uber_pipeline(first).is_none());
        assert_eq!((shaders.uber_ahead, shaders.uber_late), (0, 1));

        // The thread's late copy is dropped: the frame made the key.
        let (done_sender, done) = mpsc::channel::<Done>();
        shaders.done = Some(done);
        done_sender
            .send(Done {
                key: first,
                pipeline: None,
            })
            .unwrap();
        let mut pipelines = rustc_hash::FxHashMap::default();
        shaders.receive(&mut pipelines);
        assert!(!shaders.pending.contains(&first));
        assert!(shaders.uber_ready.is_empty());
        assert!(pipelines.is_empty());
        assert!(shaders.made.is_empty(), "the list keeps no key of the uber set");

        let report = shaders.uber_set_report();
        assert!(
            report.starts_with(&format!(
                "uber set: {} keys sent, 0 made before their first draw, 1 drawn first on the frame thread, {} not drawn: 2 3 ",
                UBER_SET.len(),
                UBER_SET.len() - 1
            )),
            "{report}"
        );
    }
}
