//! The readers of the display flag of the engine (plan F8, part 1): before each of them
//! runs, the flag it reads is set on the nodes the policy selects (`crate::animation`), and
//! the engine function then runs as it is. The three readers (engine-rules R15, entry A of
//! the journal): `BSAnimationManager::Update` gives a managed node with the flag the full
//! update, and a node without it the controllers only when it is beyond the far plane;
//! `BSAnimationNode::UpdateControllers` runs the controllers only with its own displayed
//! bit (or AlwaysUpdate). The third reader, `Reference::animIdleUpdate` (the pose of an
//! actor only with the flag on its scene node), cannot be a port: MWSE patches it (a call
//! into MWSE.dll at +0x64) and the census locks such a port and refuses the renderer
//! (2026-10-17). The actors get their flag another way before the walk goes (plan F8,
//! part 4). The stencil shadow manager keeps its rule as it is (an actor within 256
//! units, or the flag).

use crate::gen::layouts::NiAVObject as AV;
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
    rust = animation_manager_update);

/// The flag on each managed node the policy selects, then the engine's update.
pub unsafe fn animation_manager_update(this: usize, time: f32, camera: usize) {
    crate::shadow::touch(&animation_manager_update_port::STATE);
    if crate::animation::active() {
        let count = rd::<i32>(this, MANAGED_FILLED).max(0) as usize;
        let storage: usize = rd(this, MANAGED_STORAGE);
        for i in 0..count {
            let node: usize = rd(storage, i * 4);
            if node != 0 && crate::animation::displayed(node) {
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
    rust = animation_node_update_controllers);

/// The displayed bit of the animation node when the policy selects it, then the engine's
/// update of its controllers.
pub unsafe fn animation_node_update_controllers(this: usize, time: f32) {
    crate::shadow::touch(&animation_node_update_controllers_port::STATE);
    if crate::animation::active() && crate::animation::displayed(this) {
        wr::<u8>(this, ANIM_FLAGS, rd::<u8>(this, ANIM_FLAGS) | ANIM_DISPLAYED);
    }
    animation_node_update_controllers_port::engine(this, time);
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &animation_manager_update_port::STATE,
    &animation_node_update_controllers_port::STATE,
];
