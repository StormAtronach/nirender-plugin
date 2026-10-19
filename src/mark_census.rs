//! Design D7, step 1 (2026-10-19): the census of the marks by their source in the engine.
//! Every retained shape's world update (the moved marks' intake, before D6's filter, one
//! count per distinct shape a frame) and every item of the per-frame set is classified by
//! the shape's chain: under a listed root (D6), under the visual effects root, under the
//! projectile root (the projectiles and the blood splashes), under an animation-managed node
//! (the Managed bit of a `BSAnimationNode` listed by an animation manager), under the land,
//! under a reference's node that no list names, or under none of these. The shares say which
//! engine list would take which marks, and the size of the residue the per-shape marks must
//! keep. `NIRENDER_MARK_CENSUS=1` turns it on: a chain scan per distinct moved shape a frame,
//! off by default. The offsets: MWSE's headers and the survey of 2026-10-19 (engine facts
//! R5c); the visual effects root is at +0xC of its manager, not the header's +0x8.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::engine::WORLD_CONTROLLER;
use crate::mem::rd;

const DATA_HANDLER: usize = 0x7C67E0;
/// `WorldController::vfxManager`, then `VFXManager::worldVFXRoot`.
const VFX_MANAGER: usize = 0x74;
const VFX_ROOT: usize = 0xC;
/// `WorldController::mobManager`, `MobManager::projectileManager`,
/// `ProjectileManager::worldProjectileRoot`.
const MOB_MANAGER: usize = 0x5C;
const PROJECTILE_MANAGER: usize = 0x28;
const PROJECTILE_ROOT: usize = 0x14;
/// `DataHandler::worldLandscapeRoot`.
const LANDSCAPE_ROOT: usize = 0x94;
/// `BSAnimationNode::animFlags` and its Managed bit (set by `AddManagedNode`).
const ANIM_FLAGS: usize = crate::gen::layouts::BSAnimationNode::animFlags;
const ANIM_MANAGED: u8 = 0x04;

pub const SOURCES: usize = 7;
pub const NAMES: [&str; SOURCES] = [
    "a listed root (D6)",
    "the visual effects root",
    "the projectile root",
    "an animation-managed node",
    "the land",
    "a reference's node that no list names",
    "no reference's node",
];
const LISTED: usize = 0;
const VFX: usize = 1;
const PROJECTILE: usize = 2;
const MANAGED: usize = 3;
const LAND: usize = 4;
const REFERENCE: usize = 5;
const NONE: usize = 6;

static MOVED: [AtomicU64; SOURCES] = [const { AtomicU64::new(0) }; SOURCES];
static DYNAMIC: [AtomicU64; SOURCES] = [const { AtomicU64::new(0) }; SOURCES];
static RUNS: AtomicU64 = AtomicU64::new(0);
/// The roots of the frame: the visual effects root, the projectile root, the land's root.
static mut ROOTS: [usize; 3] = [0; 3];
/// Whether an object's run-time type is `BSAnimationNode` or a class under it, by the type
/// pointer (one name compare per class, as `mob_set::is_node`).
static mut ANIM_CLASSES: Option<HashMap<usize, bool>> = None;

pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var("NIRENDER_MARK_CENSUS").is_ok_and(|v| v == "1");
        if on {
            crate::log!("mark census: the moved marks and the per-frame set are classified by their source in the engine (NIRENDER_MARK_CENSUS=1; D7 step 1)");
        }
        on
    })
}

/// Reads the roots of this run.
pub unsafe fn refresh() {
    let roots = &mut *core::ptr::addr_of_mut!(ROOTS);
    *roots = [0; 3];
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    if world_controller != 0 {
        let vfx_manager: usize = rd(world_controller, VFX_MANAGER);
        if vfx_manager != 0 {
            roots[0] = rd(vfx_manager, VFX_ROOT);
        }
        let mob_manager: usize = rd(world_controller, MOB_MANAGER);
        if mob_manager != 0 {
            let projectile_manager: usize = rd(mob_manager, PROJECTILE_MANAGER);
            if projectile_manager != 0 {
                roots[1] = rd(projectile_manager, PROJECTILE_ROOT);
            }
        }
    }
    let data_handler: usize = rd(DATA_HANDLER, 0);
    if data_handler != 0 {
        roots[2] = rd(data_handler, LANDSCAPE_ROOT);
    }
    RUNS.fetch_add(1, Ordering::Relaxed);
}

unsafe fn is_animation_node(object: usize) -> bool {
    let rtti = crate::ports::ni::get_rtti(object);
    let classes = (*core::ptr::addr_of_mut!(ANIM_CLASSES)).get_or_insert_with(Default::default);
    if let Some(&answer) = classes.get(&rtti) {
        return answer;
    }
    // The engine's class name is "NiBSAnimationNode" (MWSE's virtual table list); the first
    // census compared "BSAnimationNode" and counted no managed node at the hold (17:12).
    let answer = crate::live_walk::is_kind(object, b"NiBSAnimationNode");
    classes.insert(rtti, answer);
    answer
}

/// The source of a shape with this chain (from the shape's parent up) and this reference's
/// node (0 for none). The order of the tests is the order of the lists that would take the
/// shape: the listed roots first, then the three roots, then a managed node, then the
/// reference. The roots are compared as pointers; a node is read (its type, its flags) only
/// when `alive` says the table has a shape below it, the proof of life the node rules and
/// the hierarchical gate use. A chain can name a node the engine freed until the tree marks
/// rebuild it: the first build read such a node at the save's load and crashed (17:07,
/// 2026-10-19).
pub unsafe fn classify(chain: &[(usize, u8)], ref_root: usize, alive: &dyn Fn(usize) -> bool) -> usize {
    if crate::mob_set::under_root(chain) {
        return LISTED;
    }
    let roots = *core::ptr::addr_of!(ROOTS);
    let mut managed = false;
    for &(node, _) in chain {
        if node == roots[0] && node != 0 {
            return VFX;
        }
        if node == roots[1] && node != 0 {
            return PROJECTILE;
        }
        if node == roots[2] && node != 0 {
            return LAND;
        }
        if !managed && alive(node) && is_animation_node(node) && rd::<u8>(node, ANIM_FLAGS) & ANIM_MANAGED != 0 {
            managed = true;
        }
    }
    if managed {
        return MANAGED;
    }
    if ref_root != 0 {
        REFERENCE
    } else {
        NONE
    }
}

pub fn count_moved(source: usize) {
    MOVED[source].fetch_add(1, Ordering::Relaxed);
}

pub fn count_dynamic(source: usize) {
    DYNAMIC[source].fetch_add(1, Ordering::Relaxed);
}

pub fn report() -> String {
    if !on() {
        return String::from("mark census (D7 step 1): off");
    }
    let runs = RUNS.load(Ordering::Relaxed).max(1) as f64;
    let moved: Vec<String> = (0..SOURCES).map(|i| format!("{} {:.1}", NAMES[i], MOVED[i].load(Ordering::Relaxed) as f64 / runs)).collect();
    let dynamic: Vec<String> = (0..SOURCES).map(|i| format!("{} {:.1}", NAMES[i], DYNAMIC[i].load(Ordering::Relaxed) as f64 / runs)).collect();
    format!(
        "mark census (D7 step 1): runs {}; moved shapes a run by source: {}; the per-frame set a run by source: {}",
        RUNS.load(Ordering::Relaxed),
        moved.join(", "),
        dynamic.join(", ")
    )
}
