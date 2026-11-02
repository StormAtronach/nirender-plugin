//! The texture effects under a billboard that turns to the camera.
//!
//! `NiBillboardNode::Display` 0x6D4040 turns the node (`RotateToCamera` 0x6D3C00) and updates its
//! children, so the projection of an effect under it (`NiTextureEffect::UpdateProjection`
//! 0x6E37C0) changes with the camera and with the moment of the walk. The plugin sends what does
//! not: the world of the node above the first turning billboard, the local transform of each node
//! from it down to the effect, and the model projection (`capture::effect_chain`). None of these
//! holds a turn, so a read at any moment gives the same words. The helper makes the projection for
//! the camera of each frame (`effect_turn` of the helper), and the material record names the chain
//! in place of the projection: a move of the camera makes no new material.
//!
//! `NIRENDER_EFFECT_TURN_CHECK=1` sends the engine's words after the billboard's display at the
//! end of each world scene (`send_checks`), and the helper compares them with its own.

use crate::checked::Open;
use crate::gen::layouts::{NiAVObject as AV, NiTextureEffect as TE};
use crate::main_thread::{MainCell, MainThread};
use crate::mem::{rd, wr};
use crate::ptr_hash::HashMap;

/// The run-time type of `NiBillboardNode`.
const RTTI_BILLBOARD_NODE: usize = 0x7DE0A0;
/// The most nodes above an effect that the walk looks at, as `instance_stream` for a shape.
const MAX_DEPTH: usize = 32;
/// The kind of a link that does not turn: a plain node, a billboard of the mode 3, the effect.
const NO_TURN: u32 = 3;
/// The flag that `NiNode::Display` 0x6C9190 sets: after it, the billboard has turned.
const DISPLAYED: u16 = 0x10;
/// The frames an effect stays in the table without a display of a shape it lights. Its address
/// can be another object's by then.
const KEEP_FRAMES: u32 = 4096;
/// The words of a `NiTransform`: the rotation, the translation, the scale.
const IDENTITY_TRANSFORM: [f32; 13] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

#[derive(Default)]
struct Chain {
    /// The frame of the last read.
    frame: u32,
    /// The words of the record; empty for an effect with no turning billboard above it.
    words: Vec<u32>,
    /// The turning billboards above the effect, for the check.
    billboards: Vec<usize>,
    /// The helper has the record of these words.
    sent: bool,
}

static CHAINS: MainCell<Option<HashMap<usize, Chain>>> = MainCell::new(None);
static FRAME: MainCell<u32> = MainCell::new(1);

#[track_caller]
fn chains<'a>(t: &'a MainThread) -> Open<'a, HashMap<usize, Chain>> {
    CHAINS.get(t).map(|chains| chains.get_or_insert_with(HashMap::default))
}

/// The billboard's mode (flags bits 5 and 6) when it turns to the camera at its display: mode 3
/// has no turn (`RotateToCamera` 0x6D3C00).
fn turn_mode(flags: u16) -> Option<u32> {
    let mode = u32::from(flags >> 5) & 3;
    (mode != NO_TURN).then_some(mode)
}

/// The mode of the node, when it is a billboard that turns.
unsafe fn node_turn(node: usize) -> Option<u32> {
    if crate::ports::ni::is_kind_of(node, RTTI_BILLBOARD_NODE) {
        turn_mode(rd(node, AV::flags))
    } else {
        None
    }
}

/// The words of the local transform of a node. A node always has its rotation
/// (`NiAVObject::UpdateWorldData` 0x6FB350 reads it with no test).
unsafe fn local_words(node: usize, out: &mut Vec<u32>) {
    out.extend_from_slice(&rd::<[u32; 9]>(rd::<usize>(node, AV::localTransform_pLocalRotate), 0));
    out.extend_from_slice(&rd::<[u32; 3]>(node, AV::localTransform_localTranslate));
    out.push(rd(node, AV::localTransform_localScale));
}

/// Reads the chain of `effect` into `chain`: its words stay empty for an effect with no turning
/// billboard above it.
unsafe fn read_chain(effect: usize, chain: &mut Chain) -> Vec<u32> {
    chain.billboards.clear();
    let mut first = 0;
    let mut node = effect;
    for _ in 0..MAX_DEPTH {
        if node == 0 {
            break;
        }
        if node_turn(node).is_some() {
            first = node;
            chain.billboards.push(node);
        }
        node = rd(node, AV::parentNode);
    }
    if first == 0 {
        return Vec::new();
    }
    let mut nodes = Vec::new();
    let mut node = effect;
    loop {
        nodes.push(node);
        if node == first {
            break;
        }
        node = rd(node, AV::parentNode);
    }
    let mut words = vec![effect as u32, rd(effect, TE::coordMode), nodes.len() as u32];
    let above: usize = rd(first, AV::parentNode);
    if above != 0 {
        words.extend_from_slice(&rd::<[u32; 13]>(above, AV::worldTransform));
    } else {
        words.extend(IDENTITY_TRANSFORM.map(f32::to_bits));
    }
    for &node in nodes.iter().rev() {
        words.push(node_turn(node).unwrap_or(NO_TURN));
        local_words(node, &mut words);
    }
    words.extend_from_slice(&rd::<[u32; 9]>(effect, TE::modelProjMat));
    words.extend_from_slice(&rd::<[u32; 3]>(effect, TE::modelProjTrans));
    words
}

/// True when a turning billboard is above the effect: the helper makes its projection. Reads the
/// chain at most once a frame, and sends it when it changed.
///
/// # Safety
///
/// `effect` points at a live `NiTextureEffect` with live nodes above it.
pub unsafe fn note(t: &MainThread, effect: usize) -> bool {
    let frame = *FRAME.get(t);
    let mut table = chains(t);
    let chain = table.entry(effect).or_default();
    if chain.frame != frame {
        chain.frame = frame;
        let words = read_chain(effect, chain);
        if words != chain.words {
            chain.words = words;
            chain.sent = false;
        }
    }
    if !chain.words.is_empty() && !chain.sent {
        chain.sent = true;
        crate::capture::effect_chain(&chain.words);
    }
    !chain.words.is_empty()
}

/// Forgets the records that were sent: for a capture file from the stream.
pub fn reset(t: &MainThread) {
    for chain in chains(t).values_mut() {
        chain.sent = false;
    }
}

/// The end of a frame.
pub fn frame_tick(t: &MainThread) {
    let frame = {
        let mut frame = FRAME.get(t);
        *frame = frame.wrapping_add(1).max(1);
        *frame
    };
    if frame.is_multiple_of(KEEP_FRAMES) {
        chains(t).retain(|_, chain| frame.wrapping_sub(chain.frame) < KEEP_FRAMES);
    }
}

fn check_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| crate::switches::on("NIRENDER_EFFECT_TURN_CHECK").unwrap_or(false))
}

/// The test switch, at the end of a world scene: the engine's projection of each effect read in
/// this frame whose billboards have all turned in this frame. A billboard out of the view keeps
/// an older turn, and the flag tells the two apart: it is cleared here, and only a display sets
/// it again. [I] The check is for a place with no other walk before the world's in a frame (the
/// test level): the local map's walk would set the flag with another camera.
///
/// # Safety
///
/// The effects and the billboards read in this frame are live.
pub unsafe fn send_checks(t: &MainThread) {
    if !check_on() {
        return;
    }
    let frame = *FRAME.get(t);
    let table = chains(t);
    let turning = || table.iter().filter(|(_, chain)| chain.frame == frame && !chain.words.is_empty());
    for (&effect, chain) in turning() {
        if chain.billboards.iter().all(|&node| rd::<u16>(node, AV::flags) & DISPLAYED != 0) {
            let mut words = vec![effect as u32];
            words.extend_from_slice(&rd::<[u32; 9]>(effect, TE::worldProjMat));
            words.extend_from_slice(&rd::<[u32; 3]>(effect, TE::worldProjTrans));
            crate::capture::effect_turn_check(&words);
        }
    }
    for (_, chain) in turning() {
        for &node in &chain.billboards {
            wr(node, AV::flags, rd::<u16>(node, AV::flags) & !DISPLAYED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_billboard_turns_in_every_mode_but_the_third() {
        // The flags of the test level's billboards: mode 0, 1, 2 and 2, with the displayed bit.
        for (flags, mode) in [(0x0A, 0), (0x2A, 1), (0x4A, 2), (0x5A, 2)] {
            assert_eq!(turn_mode(flags), Some(mode), "{flags:#x}");
        }
        // Mode 3 turns nothing, with any other bit.
        assert_eq!(turn_mode(0x60), None);
        assert_eq!(turn_mode(0x7F), None);
    }
}
