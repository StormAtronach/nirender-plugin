//! The readers of the engine's display flag. Before each reader runs, the port sets the flag
//! it reads on the nodes that the policy selects ([`crate::animation`]). Then the engine
//! function runs as it is.
//!
//! `BSAnimationManager::Update` gives a managed node with the flag the full update. A node
//! without it gets the controllers only when it is beyond the far plane.
//! `BSAnimationNode::UpdateControllers` and `BSAnimationNode::UpdateDownwardPass` run the
//! controllers only with the node's own displayed bit (or AlwaysUpdate). Only a keyframe or
//! visibility controller in the subtree sets AlwaysUpdate; an animated texture effect does not.
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

unsafe fn flag_node(node: usize) {
    wr::<u16>(node, AV::flags, rd::<u16>(node, AV::flags) | NODE_DISPLAYED);
}

port_thiscall!(animation_manager_update_port, 0x6EE690, "BSAnimationManager::Update",
    fn(this, time: f32, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = animation_manager_update);

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
            if node != 0 && crate::animation::displayed(t, node) {
                flag_node(node);
            }
        }
    }
    animation_manager_update_port::engine(this, time, camera);
}

port_thiscall!(animation_node_update_controllers_port, 0x6F2180, "BSAnimationNode::UpdateControllers",
    fn(this, time: f32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = animation_node_update_controllers);

/// The displayed bit of the animation node when the policy selects it, then the engine's
/// update of its controllers.
///
/// # Safety
///
/// `this` points at a live animation node: the function writes its flags byte (`ANIM_FLAGS`). The
/// port is installed (the function calls its engine function).
pub unsafe fn animation_node_update_controllers(t: &MainThread, this: usize, time: f32) {
    crate::shadow::touch(&animation_node_update_controllers_port::STATE);
    if crate::animation::active(t) && crate::animation::displayed(t, this) {
        wr::<u8>(this, ANIM_FLAGS, rd::<u8>(this, ANIM_FLAGS) | ANIM_DISPLAYED);
    }
    animation_node_update_controllers_port::engine(this, time);
}

port_thiscall!(animation_node_update_downward_pass_port, 0x6F2060, "BSAnimationNode::UpdateDownwardPass",
    fn(this, time: f32, update_controllers: u8, update_children: u8) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = animation_node_update_downward_pass);

/// The displayed bit of the animation node when the policy selects it, then the engine's
/// downward pass, which runs the controllers only with that bit (or AlwaysUpdate).
///
/// # Safety
///
/// `this` points at a live animation node: the function writes its flags byte (`ANIM_FLAGS`). The
/// port is installed (the function calls its engine function).
pub unsafe fn animation_node_update_downward_pass(t: &MainThread, this: usize, time: f32, update_controllers: u8, update_children: u8) {
    crate::shadow::touch(&animation_node_update_downward_pass_port::STATE);
    if update_controllers != 0 && crate::animation::active(t) && crate::animation::displayed(t, this) {
        wr::<u8>(this, ANIM_FLAGS, rd::<u8>(this, ANIM_FLAGS) | ANIM_DISPLAYED);
    }
    animation_node_update_downward_pass_port::engine(this, time, update_controllers, update_children);
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &animation_manager_update_port::STATE,
    &animation_node_update_controllers_port::STATE,
    &animation_node_update_downward_pass_port::STATE,
];
