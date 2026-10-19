//! The engine's own lists of what moves: every mobile actor loaded (the world controller's
//! list) and the player's two references, the active projectiles and the moving props,
//! each with the scene node of its reference. Design D6 (2026-10-19): step 1 measured how
//! much of the table walk's per-frame set lies under these roots; step 2 made the roots
//! the per-frame set of the movers. Read once a walk run from the world controller, with
//! MWSE's layouts (`TES3WorldController.h`, `TES3MobManager.h`, `TES3MobilePlayer.h`,
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
/// `ProcessManager::mobilePlayer`.
const MOBILE_PLAYER: usize = 0x0;
/// `MobilePlayer::firstPersonReference` (`TES3MobilePlayer.h`).
const FIRST_PERSON_REFERENCE: usize = 0x660;
/// `WorldController::allMobileActors` (a pointer to an iterated list of mobile actors).
const ALL_MOBILE_ACTORS: usize = 0x33C;
/// `MobileObject::actorFlags` and its `ActiveInSimulation` bit (`TES3MobileObject.h`).
const ACTOR_FLAGS: usize = 0x10;
const ACTIVE_IN_SIMULATION: u32 = 0x4;
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
    /// Counts up each time the set of roots differs from the run before (1 at the first
    /// run): an answer about a chain computed under one generation holds while it lasts.
    pub generation: u64,
    last_roots: HashSet<usize>,
    /// The roots the lists lost at the last change (empty while they do not change): the
    /// shapes under them get one work pass, since their marks were suppressed up to the
    /// moment the root left (an actor that left the simulation at the end of a run kept
    /// 38 armour parts a frame stale for 25 frames, the audit of 13:12, 2026-10-19).
    pub removed: Vec<usize>,
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
        // The player has two references, the body and the first-person arms (MWSE's
        // `tes3.player` and `tes3.player1stPerson`), and the mobile's reference points at
        // one or the other by the frame: both nodes are listed, always (2026-10-19: with
        // the mobile's reference alone the arms' root was listed every other frame, and
        // the arms' bones went stale on the frames between).
        let mobile_player: usize = rd(process_manager, MOBILE_PLAYER);
        if mobile_player != 0 {
            for node in [node_of_mobile(mobile_player), node_of_reference(rd(mobile_player, FIRST_PERSON_REFERENCE))] {
                if node != 0 && r.roots.insert(node) {
                    r.actors += 1;
                }
            }
        }
    }
    // Every mobile actor loaded (`WorldController::allMobileActors`), not the process
    // manager's planners: the planners are the actors within the AI distance, and at that
    // distance's edge two actors swapped in and out on alternate frames (a guard and a fish
    // at the hold, 2026-10-19), which changed the set every frame and threw away every
    // cached answer. A loaded actor that is not simulated does not move, so listing it
    // costs one bound test a frame and loses nothing.
    // Only an actor in the simulation (the engine's own gate, set by `MobManager::addMob`
    // once the actor is built): an actor the cell worker is still building has a scene
    // node under construction, and the walk of it from the main thread crashed the worker
    // in the clone of a skin instance (13:02, 2026-10-19).
    let all_actors: usize = rd(world_controller, ALL_MOBILE_ACTORS);
    if all_actors != 0 {
        let mut node: usize = rd(all_actors, LIST_HEAD);
        let mut seen = 0;
        while node != 0 && seen < MOST_LIST_NODES {
            let mobile: usize = rd(node, NODE_DATA);
            if mobile != 0 && rd::<u32>(mobile, ACTOR_FLAGS) & ACTIVE_IN_SIMULATION != 0 {
                let root = node_of_mobile(mobile);
                if root != 0 && r.roots.insert(root) {
                    r.actors += 1;
                }
            }
            node = rd(node, NODE_NEXT);
            seen += 1;
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
    r.removed.clear();
    if r.generation == 0 || r.roots != r.last_roots {
        r.generation += 1;
        r.removed.extend(r.last_roots.iter().filter(|root| !r.roots.contains(root)));
        r.last_roots.clone_from(&r.roots);
    }
}

/// True when a node of the chain is one of the frame's roots. The chain runs from the shape
/// up to the world's root, and a listed root (a reference's node) is near its top: the
/// scan starts there.
pub fn under_root(chain: &[(usize, u8)]) -> bool {
    let r = roots();
    !r.roots.is_empty() && chain.iter().rev().any(|&(node, _)| r.roots.contains(&node))
}

/// Design D6, step 2: the lists as the per-frame set of the movers (`NIRENDER_MOB_SET=0`
/// turns it off; the A/B switch). The rule (the owner, 2026-10-19): the renderer keeps only
/// what the helper has been told, and asks the engine for everything else at the frame it
/// needs it. So a listed root in the view has its subtree walked in the engine's own tree,
/// and nothing is kept per root but whether the helper was told its shapes are hidden.
pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("NIRENDER_MOB_SET").map_or(true, |v| v != "0"))
}

/// What the helper was told about a root's shapes: `seen` once the root was handled, and
/// whether it was in the view then (out of the view, its shapes were hidden).
#[derive(Default, Clone, Copy)]
pub struct RootState {
    pub seen: bool,
    pub in_view: bool,
}

static mut STATES: Option<std::collections::HashMap<usize, RootState>> = None;

fn states() -> &'static mut std::collections::HashMap<usize, RootState> {
    // SAFETY: the plugin's tables live on the main thread, as the walk does.
    unsafe { (*core::ptr::addr_of_mut!(STATES)).get_or_insert_with(Default::default) }
}

pub fn state(root: usize) -> &'static mut RootState {
    states().entry(root).or_default()
}

/// Forgets the states of the roots the lists no longer hold: a root that comes back is
/// handled as new (its shapes hidden once if it is out of the view).
pub fn prune_states() {
    let r = roots();
    states().retain(|root, _| r.roots.contains(root));
}

/// The objects below a node in the engine's own tree, as `NiNode::Display` reaches them:
/// each child that is a node is entered, each other child is given to `f`. The depth is
/// capped at 32.
pub unsafe fn subtree(node: usize, f: &mut dyn FnMut(usize)) {
    subtree_at(node, f, 0);
}

unsafe fn subtree_at(node: usize, f: &mut dyn FnMut(usize), depth: u32) {
    use crate::gen::layouts::NiNode as N;
    if depth > 32 {
        return;
    }
    let count = rd::<u32>(node, N::kChildren_endIndex) as usize;
    let storage: usize = rd(node, N::kChildren_storage);
    if storage == 0 {
        return;
    }
    for i in 0..count.min(4096) {
        let child: usize = rd(storage, i * 4);
        if child == 0 {
            continue;
        }
        if is_node(child) {
            // A node without renderable geometry below it (the engine's flag, which its
            // update pass keeps and `NiNode::Display` tests) is not entered: an actor's
            // skeleton is about a hundred such nodes, and the walk of them cost 0.3 ms a
            // frame at the hold (2026-10-19).
            if rd::<u16>(child, crate::gen::layouts::NiAVObject::flags) & NODE_HAS_GEOMETRY == 0 {
                continue;
            }
            subtree_at(child, f, depth + 1);
        } else {
            f(child);
        }
    }
}

/// Bit 3 of `NiAVObject::flags` on a node: the subtree holds renderable geometry.
const NODE_HAS_GEOMETRY: u16 = 0x8;

/// The marks not made at the intake because the shape is under a listed root
/// (`instance_stream::note_world_data`): the root serves it. About 2500 raw marks a frame
/// at the hold were the out-of-view actors' shapes (2026-10-19). The engine's own entry
/// for the update of a subtree (`NiAVObject::Update`, 0x6EB000) would have told it with
/// one test per actor, but MWSE hooks that function at its entry, so it stays game code.
pub static MARKS_SUPPRESSED: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

static mut NODE_CLASSES: Option<std::collections::HashMap<usize, bool>> = None;

/// Whether the object is a node (`NiNode` or a class under it). The answer is kept by the
/// object's run-time type pointer, which is static per class: one virtual call and one
/// look-up per child, and the name compare along the class chain once per class (the
/// compare for every child cost 0.65 ms a frame at the hold, 2026-10-19).
unsafe fn is_node(object: usize) -> bool {
    let rtti = crate::ports::ni::get_rtti(object);
    let classes = (*core::ptr::addr_of_mut!(NODE_CLASSES)).get_or_insert_with(Default::default);
    if let Some(&answer) = classes.get(&rtti) {
        return answer;
    }
    let answer = crate::live_walk::is_kind(object, b"NiNode");
    classes.insert(rtti, answer);
    answer
}
