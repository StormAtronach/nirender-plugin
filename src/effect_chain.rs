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
use crate::gen::layouts::{NiAVObject as AV, NiNode as NODE, NiRTTI, NiSwitchNode as SWITCH, NiTextureEffect as TE};
use crate::main_thread::{MainCell, MainThread};
use crate::mem::{rd, wr};
use crate::ptr_hash::HashMap;

/// The run-time type of `NiBillboardNode`.
const RTTI_BILLBOARD_NODE: usize = 0x7DE0A0;
/// The kind of a link that does not turn: a plain node, a billboard of the mode 3, the effect.
const NO_TURN: u32 = 3;
/// The flag that `NiNode::Display` 0x6C9190 sets: after it, the billboard has turned.
const DISPLAYED: u16 = 0x10;
/// The flag of a node with geometry under it (`NiNode::IsVisualObject` 0x445A60).
const CONTAINS_GEOMETRY: u16 = 0x08;
/// The flag of a hidden node (`NiAVObject::GetAppCulled` 0x442B30).
const APP_CULLED: u16 = 0x01;
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

/// True when a parent that has its display displays this child. `NiAVObject::CullShow` 0x6EB480
/// skips a hidden node. A node with no geometry under it does not display its children
/// (`NiNode::Display` 0x6C9190 tests the flag that `NiNode::UpdateWorldBound` 0x6C8C90 keeps). A
/// switch node displays only the child at its index (`NiSwitchNode::Display` 0x6D8320). So a
/// billboard without its display never turns.
fn child_displayed(flags: u16, parent_flags: Option<u16>, selected: bool) -> bool {
    flags & APP_CULLED == 0 && parent_flags.is_none_or(|parent| parent & CONTAINS_GEOMETRY != 0) && selected
}

/// True when the node is a switch node, or a level of detail or an animation node on one.
unsafe fn is_switch(node: usize) -> bool {
    let mut rtti = crate::ports::ni::get_rtti(node);
    while rtti != 0 {
        let name: usize = rd(rtti, NiRTTI::name);
        if name != 0 && core::ffi::CStr::from_ptr(name as *const core::ffi::c_char).to_bytes() == b"NiSwitchNode" {
            return true;
        }
        rtti = rd(rtti, NiRTTI::baseRTTI);
    }
    false
}

/// True when a switch parent selects this child, and for any other parent.
unsafe fn selected_by(parent: usize, node: usize) -> bool {
    if !is_switch(parent) {
        return true;
    }
    let index: i32 = rd(parent, SWITCH::switchIndex);
    index >= 0 && rd::<usize>(rd::<usize>(parent, NODE::kChildren_storage), index as usize * 4) == node
}

/// The words of the local transform of a node. A node always has its rotation
/// (`NiAVObject::UpdateWorldData` 0x6FB350 reads it with no test).
unsafe fn local_words(node: usize, out: &mut Vec<u32>) {
    out.extend_from_slice(&rd::<[u32; 9]>(rd::<usize>(node, AV::localTransform_pLocalRotate), 0));
    out.extend_from_slice(&rd::<[u32; 3]>(node, AV::localTransform_localTranslate));
    out.push(rd(node, AV::localTransform_localScale));
}

/// True when a turning billboard with its display is above the effect, with no table: for the
/// report of the shapes that the table does not take.
///
/// # Safety
///
/// `effect` points at a live `NiTextureEffect` with live nodes above it.
pub unsafe fn turning_above(effect: usize) -> bool {
    let nodes = nodes_up(effect);
    !turns_down(&nodes).iter().all(|turn| turn.is_none())
}

/// The nodes from the effect up to the root.
unsafe fn nodes_up(effect: usize) -> Vec<usize> {
    let mut nodes = Vec::new();
    let mut node = effect;
    while node != 0 {
        nodes.push(node);
        node = rd(node, AV::parentNode);
    }
    nodes
}

/// For each node of [`nodes_up`], the mode of its turn: a billboard that turns, when it and the
/// nodes above it have their displays.
unsafe fn turns_down(nodes: &[usize]) -> Vec<Option<u32>> {
    let mut turns = vec![None; nodes.len()];
    let mut shown = true;
    for at in (0..nodes.len()).rev() {
        let node = nodes[at];
        let parent = nodes.get(at + 1).copied();
        let selected = parent.is_none_or(|parent| selected_by(parent, node));
        shown = shown && child_displayed(rd(node, AV::flags), parent.map(|parent| rd(parent, AV::flags)), selected);
        if shown && crate::ports::ni::is_kind_of(node, RTTI_BILLBOARD_NODE) {
            turns[at] = turn_mode(rd(node, AV::flags));
        }
    }
    turns
}

/// Reads the chain of `effect` into `chain`: its words stay empty for an effect with no turning
/// billboard above it.
unsafe fn read_chain(t: &MainThread, effect: usize, chain: &mut Chain) -> Vec<u32> {
    chain.billboards.clear();
    let nodes = nodes_up(effect);
    let turns = turns_down(&nodes);
    let Some(first) = turns.iter().rposition(|turn| turn.is_some()) else { return Vec::new() };
    chain.billboards.extend(nodes.iter().zip(&turns).filter(|(_, turn)| turn.is_some()).map(|(&node, _)| node));

    let mut words = vec![effect as u32, rd(effect, TE::coordMode), first as u32 + 1];
    let above = nodes.get(first + 1).copied();
    let exact = match above {
        Some(above) => {
            let stored: [f32; 3] = rd(above, AV::worldTransform_translation);
            words.extend_from_slice(&rd::<[u32; 13]>(above, AV::worldTransform));
            crate::camera_space::exact_translation(t, above, 0).unwrap_or(stored.map(f64::from))
        }
        None => {
            words.extend(IDENTITY_TRANSFORM.map(f32::to_bits));
            [0.0; 3]
        }
    };
    for value in exact {
        words.push(value.to_bits() as u32);
        words.push((value.to_bits() >> 32) as u32);
    }
    for at in (0..=first).rev() {
        words.push(turns[at].unwrap_or(NO_TURN));
        local_words(nodes[at], &mut words);
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
        let words = read_chain(t, effect, chain);
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
        chains(t).retain(|&effect, chain| {
            let keep = frame.wrapping_sub(chain.frame) < KEEP_FRAMES;
            if !keep && chain.sent {
                // SAFETY: no engine memory is read.
                unsafe { crate::capture::effect_chain_gone(effect as u32) };
            }
            keep
        });
    }
}

fn check_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| crate::switches::on("NIRENDER_EFFECT_TURN_CHECK").unwrap_or(false))
}

/// The test switch, at the end of a world scene: the engine's projection of each effect read in
/// this frame whose billboards have all turned in this frame, and the place of the camera as the
/// engine stores it. A billboard out of the view keeps an older turn, and the flag tells the two
/// apart: it is cleared here, and only a display sets it again. [I] The check is for a place with
/// no other walk before the world's in a frame (the test level): the local map's walk would set
/// the flag with another camera.
///
/// # Safety
///
/// The effects and the billboards read in this frame are live, and `camera` is the live camera of
/// the world scene.
pub unsafe fn send_checks(t: &MainThread, camera: usize) {
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
            words.extend_from_slice(&rd::<[u32; 3]>(camera, AV::worldTransform_translation));
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

    /// The flags of the test level's `turn_two`, read in the game: the node between the two
    /// billboards has 0x02, so the second billboard (0x42) has no display and no turn.
    #[test]
    fn a_billboard_under_a_node_with_no_geometry_does_not_turn() {
        assert!(!child_displayed(0x42, Some(0x02), true));
        assert!(child_displayed(0x32, Some(0x1A), true));
        assert!(child_displayed(0x32, None, true));
    }

    /// `NiAVObject::CullShow` 0x6EB480 and `NiSwitchNode::Display` 0x6D8320.
    #[test]
    fn a_hidden_node_and_a_child_not_selected_have_no_display() {
        assert!(!child_displayed(0x33, Some(0x1A), true));
        assert!(!child_displayed(0x32, Some(0x1A), false));
    }
}
