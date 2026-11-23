//! The readers of the engine's display flag. Before each reader runs, the port sets the flag
//! it reads on the nodes that the policy selects ([`crate::animation`]). Then the engine
//! function runs as it is.
//!
//! `BSAnimationManager::Update` gives a managed node with the flag the full update. A node
//! without it gets the controllers only when it is beyond the far plane.
//! `BSAnimationNode::UpdateControllers` and `BSAnimationNode::UpdateDownwardPass` run the
//! controllers only with the node's own displayed bit (or AlwaysUpdate). Only a keyframe or
//! visibility controller in the subtree sets AlwaysUpdate; an animated texture effect does not.
//! The two node readers read and clear the bit only through a gate (`node_reader_reads`); the port
//! compares with the engine's walk and writes the bit only in a call that reads it (W97).
//!
//! `Reference::animIdleUpdate` also reads the flag, but it cannot be a port: MWSE patches it
//! (a call into MWSE.dll at +0x64). The actors get their flag another way.

use crate::gen::layouts::NiAVObject as AV;
use crate::main_thread::MainThread;
use crate::mem::{rd, wr};

use super::scene::{ANIM_DISPLAYED, ANIM_FLAGS, NODE_DISPLAYED};

/// `BSAnimationManager::managedNodes`, a `TArray` at 0xB0: the storage at +4 and the filled
/// count at +0x10, as the engine's loop reads them.
const MANAGED_STORAGE: usize = 0xB0 + 0x4;
const MANAGED_FILLED: usize = 0xB0 + 0x10;

/// `global_BSAnimationNode_inUpdateCall`: 1 while `BSAnimationManager::Update` runs.
const IN_UPDATE_CALL: usize = 0x7DE9B8;
/// The bits of `animFlags` that the gate of the two node readers reads.
const ANIM_FIRST_TIME: u8 = 0x02;
const ANIM_MANAGED: u8 = 0x04;

/// True when the node reader reads its displayed bit in this call, and clears it: the gate of
/// `UpdateControllers` (0x6F2191-0x6F2199) and of `UpdateDownwardPass` (0x6F212A-0x6F2132), the
/// managed bit equal to the update in progress. The downward pass reads it only with
/// `update_controllers` and not the first time (its first-time path clears the bit at 0x6F210F
/// with no read). The port compares with the walk and writes only then (W97, verdict 743): a bit
/// that the port wrote in a call with no read stays, and the next call would count it as the
/// walk's.
pub(crate) fn node_reader_reads(anim_flags: u8, in_update_call: u8, downward: bool, update_controllers: bool) -> bool {
    if downward && (anim_flags & ANIM_FIRST_TIME != 0 || !update_controllers) {
        return false;
    }
    u8::from(anim_flags & ANIM_MANAGED != 0) == in_update_call
}

unsafe fn flag_node(node: usize) {
    wr::<u16>(node, AV::flags, rd::<u16>(node, AV::flags) | NODE_DISPLAYED);
}

port_thiscall!(
    animation_manager_update_port,
    0x6EE690,
    "BSAnimationManager::Update",
    fn(this, time: f32, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = animation_manager_update
);

/// The flag on each managed node the policy selects, then the engine's update.
///
/// # Safety
///
/// `this` points at a live animation manager whose array of managed nodes (the count at
/// `MANAGED_FILLED`, the storage at `MANAGED_STORAGE`) holds live nodes or 0; `camera` is the
/// engine's argument, passed on. The port is installed (the function calls its engine function).
pub unsafe fn animation_manager_update(t: &MainThread, this: usize, time: f32, camera: usize) {
    crate::shadow::touch(&animation_manager_update_port::STATE);
    if crate::animation::active(t) {
        let count = rd::<i32>(this, MANAGED_FILLED).max(0) as usize;
        let storage: usize = rd(this, MANAGED_STORAGE);
        for i in 0..count {
            let node: usize = rd(storage, i * 4);
            if node == 0 {
                continue;
            }
            // The flag before the write: the engine's walk's own answer (W97).
            let walk = rd::<u16>(node, AV::flags) & NODE_DISPLAYED != 0;
            if crate::animation::displayed_by_walk(t, node, crate::animation::Reader::Manager, walk) {
                flag_node(node);
            }
        }
    }
    animation_manager_update_port::engine(this, time, camera);
}

port_thiscall!(
    animation_node_update_controllers_port,
    0x6F2180,
    "BSAnimationNode::UpdateControllers",
    fn(this, time: f32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = animation_node_update_controllers
);

/// The displayed bit of the animation node when the policy selects it, then the engine's
/// update of its controllers.
///
/// # Safety
///
/// `this` points at a live animation node: the function writes its flags byte (`ANIM_FLAGS`). The
/// port is installed (the function calls its engine function).
pub unsafe fn animation_node_update_controllers(t: &MainThread, this: usize, time: f32) {
    crate::shadow::touch(&animation_node_update_controllers_port::STATE);
    let flags = rd::<u8>(this, ANIM_FLAGS);
    if crate::animation::active(t) && node_reader_reads(flags, rd(IN_UPDATE_CALL, 0), false, true) {
        let walk = flags & ANIM_DISPLAYED != 0;
        if crate::animation::displayed_by_walk(t, this, crate::animation::Reader::Controllers, walk) {
            wr::<u8>(this, ANIM_FLAGS, flags | ANIM_DISPLAYED);
        }
    }
    animation_node_update_controllers_port::engine(this, time);
}

port_thiscall!(
    animation_node_update_downward_pass_port,
    0x6F2060,
    "BSAnimationNode::UpdateDownwardPass",
    fn(this, time: f32, update_controllers: u8, update_children: u8) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = animation_node_update_downward_pass
);

/// The displayed bit of the animation node when the policy selects it, then the engine's
/// downward pass, which runs the controllers only with that bit (or AlwaysUpdate).
///
/// # Safety
///
/// `this` points at a live animation node: the function writes its flags byte (`ANIM_FLAGS`). The
/// port is installed (the function calls its engine function).
pub unsafe fn animation_node_update_downward_pass(
    t: &MainThread,
    this: usize,
    time: f32,
    update_controllers: u8,
    update_children: u8,
) {
    crate::shadow::touch(&animation_node_update_downward_pass_port::STATE);
    let flags = rd::<u8>(this, ANIM_FLAGS);
    if crate::animation::active(t) && node_reader_reads(flags, rd(IN_UPDATE_CALL, 0), true, update_controllers != 0) {
        let walk = flags & ANIM_DISPLAYED != 0;
        if crate::animation::displayed_by_walk(t, this, crate::animation::Reader::DownwardPass, walk) {
            wr::<u8>(this, ANIM_FLAGS, flags | ANIM_DISPLAYED);
        }
    }
    animation_node_update_downward_pass_port::engine(this, time, update_controllers, update_children);
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &animation_manager_update_port::STATE,
    &animation_node_update_controllers_port::STATE,
    &animation_node_update_downward_pass_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::{node_reader_reads, ANIM_FIRST_TIME, ANIM_MANAGED};

    /// The gate of the two node readers (W97, verdict 743): the read follows the managed bit and
    /// the update in progress; the downward pass also needs the controllers' update and not the
    /// first time.
    #[test]
    fn a_node_reader_reads_only_through_its_gate() {
        for downward in [false, true] {
            assert!(node_reader_reads(0, 0, downward, true), "a node outside the manager's update");
            assert!(
                node_reader_reads(ANIM_MANAGED, 1, downward, true),
                "a managed node in the update"
            );
            assert!(
                !node_reader_reads(ANIM_MANAGED, 0, downward, true),
                "a managed node outside the update"
            );
            assert!(!node_reader_reads(0, 1, downward, true), "a node not managed in the update");
        }
        assert!(
            node_reader_reads(0, 0, false, false),
            "UpdateControllers has no controllers' argument"
        );
        assert!(
            !node_reader_reads(0, 0, true, false),
            "the downward pass without the controllers' update"
        );
        assert!(
            !node_reader_reads(ANIM_FIRST_TIME, 0, true, true),
            "the first time clears with no read"
        );
    }
}
