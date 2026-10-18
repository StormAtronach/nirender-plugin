//! The destruction of a scene object deferred to the main thread (the lifetime rules of the
//! cell worker audit, 2026-10-18; the second step after `deferred_unload`).
//!
//! With every `RecordsHandler::unloadCell` deferred (`deferred_unload`), the engine's cell
//! worker still destroyed scene objects: 23000 geometry destructors in one Narsis crossing
//! glide, 64 of them shapes the table drew (runs 20261018-203207-cross-census and
//! 20261018-203908-cross-notes). They come from node destructors on the worker
//! (`NiNode::dtor` detaching and releasing its children) below `Reference::getVisualNode` and
//! other loading paths. The detach and the free are one operation on the worker, so no event
//! can quarantine the shapes before the free, and the main thread's walks can read a freed
//! shape between the free and the next drain of the worker's events.
//!
//! Here the first slot of each scene-object class's virtual table, the deleting destructor
//! (`deleting_dtor(this, flag)`), points at `detour`. On the main thread the detour calls the
//! class's destructor at once. On another thread, when the call frees the object (`flag & 1`),
//! it queues the object and returns: the object stays allocated with no owner (its reference
//! count is 0 and nothing holds it), and the main thread runs the destructor at the next drain
//! of the worker's events (`run`). Then every destruction of a scene object is on the main
//! thread, the ports see it, and no object the plugin models is freed while a frame reads it.
//! A destructor that does not free (`flag & 1 == 0`, an object inside another) runs where it is
//! called.
//!
//! The classes: every concrete class under `NiAVObject` in Morrowind.exe (the nodes, the
//! geometry, the lights, the texture effect, the camera, the screen polygon), by MWSE's table of
//! virtual tables (`SharedSE/NIVirtualTableDefines.Morrowind.h`). The slot is checked against
//! three known destructors before any write; a mismatch leaves every slot alone.
//!
//! `NIRENDER_DEFER_DESTROY=0` turns it off (the A/B).

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::mem::rd;

/// The virtual tables of the concrete scene-object classes (MWSE's `NI::VirtualTableAddress`).
const CLASSES: [(&str, usize); 26] = [
    ("NiNode", 0x74F9C8),
    ("NiBSAnimationNode", 0x750D68),
    ("NiBSParticleNode", 0x751130),
    ("BSMirroredNode", 0x74768C),
    ("NiBillboardNode", 0x746B00),
    ("NiBSPNode", 0x74FF30),
    ("NiSwitchNode", 0x750080),
    ("NiLODNode", 0x7502C0),
    ("NiFltAnimationNode", 0x750130),
    ("NiSortAdjustNode", 0x750580),
    ("AvoidNode", 0x74F370),
    ("RootCollisionNode", 0x74A6CC),
    ("NiCollisionSwitch", 0x74F418),
    ("NiCamera", 0x74FAA8),
    ("NiTriShape", 0x7508B0),
    ("NiTriStrips", 0x750A00),
    ("NiParticles", 0x74FCE8),
    ("NiAutoNormalParticles", 0x74FDE8),
    ("NiRotatingParticles", 0x7504A0),
    ("NiLines", 0x750220),
    ("NiScreenPolygon", 0x74FA74),
    ("NiAmbientLight", 0x74FB90),
    ("NiDirectionalLight", 0x74FFE0),
    ("NiPointLight", 0x749D40),
    ("NiSpotLight", 0x7506A8),
    ("NiTextureEffect", 0x7507C0),
];
/// Three slots whose destructor the IDA database names (`NiNode::deleting_dtor`,
/// `NiTriShape::deleting_dtor`, `NiGeometry::deleting_dtor` of the abstract table): the check
/// before any write.
const KNOWN: [(usize, usize); 3] = [(0x74F9C8, 0x6C82A0), (0x7508B0, 0x6E4E60), (0x750EB8, 0x6F3D40)];

type Dtor = unsafe extern "thiscall" fn(usize, u32) -> u32;

/// The original destructor of each patched virtual table, by the table's address.
static ORIGINALS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
/// The objects whose destruction waits for the main thread: the object and its destructor.
static QUEUE: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
static QUEUED: AtomicU64 = AtomicU64::new(0);
static RUN: AtomicU64 = AtomicU64::new(0);
static DIRECT_OFF_THREAD: AtomicU64 = AtomicU64::new(0);
static TICKS: AtomicI64 = AtomicI64::new(0);
static LONGEST: AtomicI64 = AtomicI64::new(0);

fn original_of(vtable: usize) -> Option<usize> {
    ORIGINALS.lock().ok().and_then(|o| o.iter().find(|&&(v, _)| v == vtable).map(|&(_, d)| d))
}

unsafe extern "thiscall" fn detour(this: usize, flag: u32) -> u32 {
    let vtable: usize = rd(this, 0);
    let Some(original) = original_of(vtable) else {
        // A class this module did not patch reached the detour through a copied slot: the
        // object's own table has the destructor at its first slot, so nothing can be called.
        crate::log!("deferred destroy: an object 0x{this:x} with the unknown virtual table 0x{vtable:x} reached the detour; its destructor did not run");
        return 0;
    };
    let dtor: Dtor = core::mem::transmute(original);
    if crate::shadow::on_main_thread() || flag & 1 == 0 {
        if !crate::shadow::on_main_thread() {
            DIRECT_OFF_THREAD.fetch_add(1, Ordering::Relaxed);
        }
        return dtor(this, flag);
    }
    if let Ok(mut queue) = QUEUE.lock() {
        queue.push((this, original));
        QUEUED.fetch_add(1, Ordering::Relaxed);
    }
    0
}

pub unsafe fn install() {
    if std::env::var("NIRENDER_DEFER_DESTROY").is_ok_and(|v| v == "0") {
        crate::log!("deferred destroy: off (NIRENDER_DEFER_DESTROY=0); the cell worker destroys scene objects on its own thread");
        return;
    }
    for (vtable, dtor) in KNOWN {
        let slot: usize = rd(vtable, 0);
        if slot != dtor {
            crate::log!("deferred destroy: the first slot of the virtual table 0x{vtable:x} is 0x{slot:x}, not the destructor 0x{dtor:x} the database names; no slot is changed");
            return;
        }
    }
    let detour_at = detour as *const () as usize;
    let mut originals = Vec::with_capacity(CLASSES.len());
    for (name, vtable) in CLASSES {
        let slot: usize = rd(vtable, 0);
        if slot == detour_at {
            continue;
        }
        if !crate::win::patch(vtable, &(detour_at as u32).to_le_bytes()) {
            crate::log!("deferred destroy: the virtual table of {name} (0x{vtable:x}) could not be written");
            continue;
        }
        originals.push((vtable, slot));
    }
    let count = originals.len();
    if let Ok(mut o) = ORIGINALS.lock() {
        *o = originals;
    }
    crate::log!("deferred destroy: the deleting destructors of {count} scene-object classes run on the main thread when another thread calls them");
}

/// The main thread runs the queued destructors. At each drain of the worker's events.
pub unsafe fn run() {
    if !crate::shadow::on_main_thread() {
        return;
    }
    let batch = match QUEUE.lock() {
        Ok(mut queue) => {
            if queue.is_empty() {
                return;
            }
            core::mem::take(&mut *queue)
        }
        Err(_) => return,
    };
    let start = crate::capture::ticks();
    for (this, original) in batch {
        let dtor: Dtor = core::mem::transmute(original);
        dtor(this, 1);
        if RUN.fetch_add(1, Ordering::Relaxed) == 0 {
            crate::log!("deferred destroy: the first scene object destroyed on the main thread for the cell worker");
        }
    }
    let ticks = crate::capture::ticks() - start;
    TICKS.fetch_add(ticks, Ordering::Relaxed);
    LONGEST.fetch_max(ticks, Ordering::Relaxed);
}

fn ms(ticks: i64) -> f64 {
    ticks as f64 * 1000.0 / crate::capture::ticks_per_second() as f64
}

pub fn report() -> String {
    format!(
        "deferred destroy: {} scene objects queued by another thread, {} destroyed on the main thread ({:.3} ms in all, the longest drain {:.3} ms), {} destroyed off the main thread without a free, {} waiting",
        QUEUED.load(Ordering::Relaxed),
        RUN.load(Ordering::Relaxed),
        ms(TICKS.load(Ordering::Relaxed)),
        ms(LONGEST.load(Ordering::Relaxed)),
        DIRECT_OFF_THREAD.load(Ordering::Relaxed),
        QUEUE.lock().map_or(0, |q| q.len())
    )
}
