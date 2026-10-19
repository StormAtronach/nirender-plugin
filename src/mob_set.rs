//! The engine's own lists of what moves: the mob manager's active actors (the process
//! manager's planners and the player), the active projectiles and the moving props, each
//! with the scene node of its reference. Design D6, step 1 (2026-10-19): a measurement of
//! how much of the table walk's per-frame set lies under these roots, before the lists
//! become the per-frame set. Read once a walk run from the world controller, with MWSE's
//! layouts (`TES3WorldController.h`, `TES3MobManager.h`, `TES3AIData.h`,
//! `TES3MobileObject.h`, `TES3Reference.h`, `NIIteratedList.h`). Every pointer is guarded:
//! the main menu has no mob manager, a mobile can have no reference, a reference no node.

use crate::engine::WORLD_CONTROLLER;
use crate::mem::rd;
use std::collections::HashSet;

/// `WorldController::mobManager`.
const MOB_MANAGER: usize = 0x5C;
/// `MobManager::processManager`, `::projectileManager`, `::listMovingProps`.
const PROCESS_MANAGER: usize = 0x24;
const PROJECTILE_MANAGER: usize = 0x28;
const MOVING_PROPS: usize = 0x30;
/// `ProcessManager::mobilePlayer`, `::plannerCount`, `::planners[500]`.
const MOBILE_PLAYER: usize = 0x0;
const PLANNER_COUNT: usize = 0x58;
const PLANNERS: usize = 0x5C;
const MOST_PLANNERS: usize = 500;
/// `AIPlanner::mobileActor`.
const PLANNER_ACTOR: usize = 0x4;
/// `MobileObject::reference`.
const MOBILE_REFERENCE: usize = 0x14;
/// `Reference::sceneNode` (a pointer the reference owns).
const REFERENCE_SCENE_NODE: usize = 0x10;
/// `ProjectileManager::activeProjectiles`.
const ACTIVE_PROJECTILES: usize = 0x0;
/// `NI::IteratedList`: the head at +0x8; a node's next at +0x4, its data at +0x8.
const LIST_HEAD: usize = 0x8;
const NODE_NEXT: usize = 0x4;
const NODE_DATA: usize = 0x8;
/// A list longer than this is taken as corrupt and left.
const MOST_LIST_NODES: usize = 4096;

/// The roots of the frame: the scene nodes of the active actors, the projectiles and the
/// moving props, and how many of each the lists held.
#[derive(Default)]
pub struct MobRoots {
    pub roots: HashSet<usize>,
    pub actors: u32,
    pub projectiles: u32,
    pub props: u32,
}

static mut ROOTS: Option<MobRoots> = None;

pub fn roots() -> &'static mut MobRoots {
    // SAFETY: the plugin's tables live on the main thread, as the walk does.
    unsafe { (*core::ptr::addr_of_mut!(ROOTS)).get_or_insert_with(MobRoots::default) }
}

/// The scene node of a mobile's reference, or 0.
unsafe fn node_of_mobile(mobile: usize) -> usize {
    if mobile == 0 {
        return 0;
    }
    node_of_reference(rd(mobile, MOBILE_REFERENCE))
}

unsafe fn node_of_reference(reference: usize) -> usize {
    if reference == 0 {
        return 0;
    }
    rd(reference, REFERENCE_SCENE_NODE)
}

/// Reads the lists again for this run of the walk.
pub unsafe fn refresh() {
    let r = roots();
    r.roots.clear();
    r.actors = 0;
    r.projectiles = 0;
    r.props = 0;
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    if world_controller == 0 {
        return;
    }
    let mob_manager: usize = rd(world_controller, MOB_MANAGER);
    if mob_manager == 0 {
        return;
    }
    let process_manager: usize = rd(mob_manager, PROCESS_MANAGER);
    if process_manager != 0 {
        let player = node_of_mobile(rd(process_manager, MOBILE_PLAYER));
        if player != 0 {
            r.roots.insert(player);
            r.actors += 1;
        }
        let count = (rd::<u32>(process_manager, PLANNER_COUNT) as usize).min(MOST_PLANNERS);
        for i in 0..count {
            let planner: usize = rd(process_manager, PLANNERS + i * 4);
            if planner == 0 {
                continue;
            }
            let node = node_of_mobile(rd(planner, PLANNER_ACTOR));
            if node != 0 {
                r.roots.insert(node);
                r.actors += 1;
            }
        }
    }
    let projectile_manager: usize = rd(mob_manager, PROJECTILE_MANAGER);
    if projectile_manager != 0 {
        let mut node: usize = rd(projectile_manager + ACTIVE_PROJECTILES, LIST_HEAD);
        let mut seen = 0;
        while node != 0 && seen < MOST_LIST_NODES {
            let root = node_of_mobile(rd(node, NODE_DATA));
            if root != 0 {
                r.roots.insert(root);
                r.projectiles += 1;
            }
            node = rd(node, NODE_NEXT);
            seen += 1;
        }
    }
    let mut node: usize = rd(mob_manager + MOVING_PROPS, LIST_HEAD);
    let mut seen = 0;
    while node != 0 && seen < MOST_LIST_NODES {
        let root = node_of_reference(rd(node, NODE_DATA));
        if root != 0 {
            r.roots.insert(root);
            r.props += 1;
        }
        node = rd(node, NODE_NEXT);
        seen += 1;
    }
}

/// True when a node of the chain is one of the frame's roots.
pub fn under_root(chain: &[(usize, u8)]) -> bool {
    let r = roots();
    !r.roots.is_empty() && chain.iter().any(|&(node, _)| r.roots.contains(&node))
}
