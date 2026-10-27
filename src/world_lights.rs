//! The lights of the world: the point lights that the engine placed in the world and has not
//! taken out, from MWSE's events (`NATIVE_EVENT_LIGHT_PLACED`, `NATIVE_EVENT_LIGHT_REMOVED`).
//! At the start of each world scene the main thread sends the values of each light that
//! changed (`light_stream::note_values`) and, when the set changed, the keys of the lights that
//! are on (`capture::world_lights`). In its world mode the helper lights a pixel with these
//! lights, in place of the lights that the engine chose for each draw.
//!
//! A light counts only while it is in the active cells (`in_active_cells`): the engine does not
//! take the lights of the exterior out when the player goes into an interior.
//!
//! The removal can come on the engine's cell worker (the destruction of a reference of an
//! unloaded cell). The set is under a lock, and the listener takes the light out at once. The
//! main thread reads a light only under the lock, so it never reads a light after its removal.
//! The destruction of a light (`NiDX8LightManager::RemoveLight`, `forget_destroyed`) takes it out
//! too.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::gen::layouts::NiLight as L;
use crate::main_thread::MainThread;
use crate::mem::rd;
use crate::mwse_native::{LightPlacedV1, LightRemovedV1};

/// The state of a light of the set: the revision of the values sent last (0: none), whether
/// the light was on and in the active cells at the last send, and the cell of its last
/// placement (0: none came with a cell).
#[derive(Clone, Copy, Default)]
struct Sent {
    revision: u32,
    on: bool,
    cell: usize,
}

/// `TES3::DataHandler` (the pointer at 0x7C67E0); a cell's flags, bit 1 an interior.
const DATA_HANDLER: usize = 0x7C67E0;
const CURRENT_INTERIOR_CELL: usize = crate::gen::layouts::DataHandler::currentInteriorCell;
const CELL_FLAGS: usize = crate::gen::layouts::Cell::cellFlags;
/// For `describe`.
const CELL_NAME: usize = crate::gen::layouts::Cell::name;
const CELL_INTERIOR: u32 = 1;

/// Whether a light placed in `cell` is in the active cells. A change from an exterior to an
/// interior detaches the exterior's scene without taking its lights out
/// (`DataHandler::cellChangeToInterior` 0x489050 resets only the lights of an interior it
/// leaves), so in an interior only the lights placed in it count. A light placed only on other
/// references (`game_updateLightHelper2`, no cell) counts in an exterior.
unsafe fn in_active_cells(cell: usize, interior: usize) -> bool {
    if interior != 0 {
        return cell == interior;
    }
    cell == 0 || rd::<u32>(cell, CELL_FLAGS) & CELL_INTERIOR == 0
}

struct WorldSet {
    /// The lights by their address.
    lights: HashMap<usize, Sent>,
    /// The set changed since the keys went.
    changed: bool,
}

static SET: Mutex<Option<WorldSet>> = Mutex::new(None);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static PLACED: AtomicU64 = AtomicU64::new(0);
static REMOVED: AtomicU64 = AtomicU64::new(0);
/// The lights destroyed while in the set: the engine destroyed them without a removal.
static DESTROYED_IN_SET: AtomicU64 = AtomicU64::new(0);
static KEY_RECORDS: AtomicU64 = AtomicU64::new(0);
/// The lights on and in the active cells at the last record of the keys.
static LAST_KEYS: AtomicU64 = AtomicU64::new(0);
static MOST: AtomicU64 = AtomicU64::new(0);

fn with_set<R>(f: impl FnOnce(&mut WorldSet) -> R) -> Option<R> {
    let mut guard = SET.lock().ok()?;
    Some(f(guard.get_or_insert_with(|| WorldSet { lights: HashMap::new(), changed: true })))
}

/// Listens to MWSE's two events. The removal first: without it a light of an unloaded cell
/// would stay in the set. A refusal leaves the plugin without the lights of the world: the
/// helper's world mode then has no lights.
pub unsafe fn install() {
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    use crate::mwse_native::{subscribe, NATIVE_EVENT_LIGHT_PLACED, NATIVE_EVENT_LIGHT_REMOVED};
    if let Err(error) = subscribe(NATIVE_EVENT_LIGHT_REMOVED, removed) {
        crate::log!("world lights: {error}: no lights of the world");
        return;
    }
    if let Err(error) = subscribe(NATIVE_EVENT_LIGHT_PLACED, placed) {
        crate::log!("world lights: {error}: no lights of the world");
        return;
    }
    INSTALLED.store(true, Ordering::Relaxed);
    crate::log!("world lights: MWSE reports each light placed in the world and each light removed");
}

/// MWSE_NATIVE_EVENT_LIGHT_PLACED, on the main thread.
unsafe extern "C" fn placed(_user: *mut c_void, packet: *const c_void) {
    let packet = &*(packet as *const LightPlacedV1);
    if packet.light == 0 {
        return;
    }
    PLACED.fetch_add(1, Ordering::Relaxed);
    // The radius the engine passed, before a module's light attach radius changes it.
    if let Some(t) = MainThread::here() {
        crate::light_stream::note_radius(&t, rd(packet.light as usize, L::index), packet.radius as f32);
    }
    with_set(|set| {
        // A new light is off until a send finds it on: that send gives the keys again.
        let sent = set.lights.entry(packet.light as usize).or_default();
        if packet.cell != 0 {
            sent.cell = packet.cell as usize;
        }
        MOST.fetch_max(set.lights.len() as u64, Ordering::Relaxed);
    });
}

/// MWSE_NATIVE_EVENT_LIGHT_REMOVED, on the main thread or on the cell worker.
unsafe extern "C" fn removed(_user: *mut c_void, packet: *const c_void) {
    let packet = &*(packet as *const LightRemovedV1);
    REMOVED.fetch_add(1, Ordering::Relaxed);
    with_set(|set| {
        if set.lights.remove(&(packet.light as usize)).is_some() {
            set.changed = true;
        }
    });
}

/// A light is destroyed (`NiDX8LightManager::RemoveLight`, on any thread): it leaves the set if
/// the engine did not remove it first.
pub fn forget_destroyed(light: usize) {
    with_set(|set| {
        if set.lights.remove(&light).is_some() {
            set.changed = true;
            DESTROYED_IN_SET.fetch_add(1, Ordering::Relaxed);
        }
    });
}

/// The next send gives every light and the keys again: for the start of a capture file.
pub fn resend() {
    with_set(|set| {
        // The cell of a light stays: only a placement gives it.
        for sent in set.lights.values_mut() {
            sent.revision = 0;
            sent.on = false;
        }
        set.changed = true;
    });
}

/// At the start of a world scene: the values of each light of the set that changed, then the
/// keys of the lights that are on when the set changed.
pub unsafe fn send(t: &MainThread) {
    if !INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    // The lights are read under the lock; the records go after it.
    let mut values: Vec<(u32, [u32; 26], f32)> = Vec::new();
    let data_handler: usize = rd(DATA_HANDLER, 0);
    let interior: usize = if data_handler != 0 { rd(data_handler, CURRENT_INTERIOR_CELL) } else { 0 };
    let keys = with_set(|set| {
        let mut changed = core::mem::replace(&mut set.changed, false);
        for (&light, sent) in set.lights.iter_mut() {
            let on = rd::<u8>(light, L::enabled) != 0 && in_active_cells(sent.cell, interior);
            if on != sent.on {
                sent.on = on;
                changed = true;
            }
            let revision: u32 = rd(light, L::revisionID);
            if on && revision != sent.revision {
                sent.revision = revision;
                values.push((rd(light, L::index), crate::ports::light_manager::d3d_light_of(light), crate::link_client::point_light_radius(light)));
            }
        }
        changed.then(|| set.lights.iter().filter(|(_, sent)| sent.on).map(|(&light, _)| rd::<u32>(light, L::index)).collect::<Vec<u32>>())
    })
    .flatten();
    for (key, values, radius) in &values {
        crate::light_stream::note_values(t, *key, values, *radius);
    }
    if let Some(keys) = keys {
        crate::capture::world_lights(&keys);
        KEY_RECORDS.fetch_add(1, Ordering::Relaxed);
        LAST_KEYS.store(keys.len() as u64, Ordering::Relaxed);
    }
}

/// One line for each light of the set, for a check in a live session
/// (`nirender.worldLights()`): the key, the position, the cell of the last placement with a
/// cell, whether the last send had it on.
pub unsafe fn describe() -> Vec<String> {
    let name_of = |cell: usize| -> String {
        if cell == 0 {
            return String::from("-");
        }
        let at: usize = rd(cell, CELL_NAME);
        if at == 0 {
            return format!("0x{cell:x}");
        }
        core::ffi::CStr::from_ptr(at as *const core::ffi::c_char).to_string_lossy().into_owned()
    };
    with_set(|set| {
        set.lights
            .iter()
            .map(|(&light, sent)| {
                let p: [f32; 3] = rd(light, L::worldTransform_translation);
                format!("{} {:.0} {:.0} {:.0} [{}] {}", rd::<u32>(light, L::index), p[0], p[1], p[2], name_of(sent.cell), if sent.on { "on" } else { "off" })
            })
            .collect()
    })
    .unwrap_or_default()
}

/// The report line.
pub fn report() -> String {
    let now = with_set(|set| set.lights.len()).unwrap_or(0);
    format!(
        "world lights: {} placed events, {} removed events, {} lights now (at most {}), {} destroyed without a removal, {} records of the keys, {} lights in the last",
        PLACED.load(Ordering::Relaxed),
        REMOVED.load(Ordering::Relaxed),
        now,
        MOST.load(Ordering::Relaxed),
        DESTROYED_IN_SET.load(Ordering::Relaxed),
        KEY_RECORDS.load(Ordering::Relaxed),
        LAST_KEYS.load(Ordering::Relaxed)
    )
}
