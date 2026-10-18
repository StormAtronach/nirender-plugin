//! A test instrument: a kept engine pointer is checked by its virtual table before its first
//! read in a walk (`NIRENDER_CHECK_VTABLES=1`; off by default). The check is one read of the
//! object's first word and a match against the virtual tables of the scene-object classes
//! (MWSE's `NIVirtualTableDefines.Morrowind.h`, as `deferred_destroy` uses them). It cannot
//! see a freed block (the read itself is the fault, which `freed_fill` makes certain); it sees
//! a block that holds another object now, and names it. A mismatch is counted and the first
//! ones go to the log; the caller skips the item.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::mem::rd;

static ON: AtomicU8 = AtomicU8::new(2);
static CHECKED: AtomicU64 = AtomicU64::new(0);
static WRONG: AtomicU64 = AtomicU64::new(0);
const LOGGED_AT_MOST: u64 = 16;

#[inline]
pub fn on() -> bool {
    match ON.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let on = std::env::var("NIRENDER_CHECK_VTABLES").is_ok_and(|v| v == "1");
            ON.store(on as u8, Ordering::Relaxed);
            if on {
                crate::log!("pointer check: every kept shape, node and light is checked by its virtual table before a walk reads it (NIRENDER_CHECK_VTABLES=1)");
            }
            on
        }
    }
}

/// A geometry: NiTriShape, NiTriStrips, NiParticles, NiAutoNormalParticles,
/// NiRotatingParticles, NiLines.
fn is_shape(vtable: usize) -> bool {
    matches!(vtable, 0x7508B0 | 0x750A00 | 0x74FCE8 | 0x74FDE8 | 0x7504A0 | 0x750220)
}

/// A node of any kind the engine makes (NiNode and its subclasses, the collision nodes).
fn is_node(vtable: usize) -> bool {
    matches!(vtable, 0x74F9C8 | 0x750D68 | 0x751130 | 0x74768C | 0x746B00 | 0x74FF30 | 0x750080 | 0x7502C0 | 0x750130 | 0x750580 | 0x74F370 | 0x74A6CC | 0x74F418)
}

/// A light or a texture effect.
fn is_light(vtable: usize) -> bool {
    matches!(vtable, 0x74FB90 | 0x74FFE0 | 0x749D40 | 0x7506A8 | 0x7507C0)
}

unsafe fn check(object: usize, ok: fn(usize) -> bool, kind: &str, place: &str) -> bool {
    if !on() || object == 0 {
        return true;
    }
    CHECKED.fetch_add(1, Ordering::Relaxed);
    let vtable: usize = rd(object, 0);
    if ok(vtable) {
        return true;
    }
    let n = WRONG.fetch_add(1, Ordering::Relaxed);
    if n < LOGGED_AT_MOST {
        crate::log!("pointer check: the kept {kind} 0x{object:x} read in {place} has the virtual table 0x{vtable:x}, not a {kind}'s; skipped");
    }
    false
}

/// True when the kept shape pointer still holds a geometry.
pub unsafe fn shape(object: usize, place: &str) -> bool {
    check(object, is_shape, "shape", place)
}

/// True when the kept node pointer still holds a node.
pub unsafe fn node(object: usize, place: &str) -> bool {
    check(object, is_node, "node", place)
}

/// True when the kept light pointer still holds a light or a texture effect.
pub unsafe fn light(object: usize, place: &str) -> bool {
    check(object, is_light, "light", place)
}

pub fn report() -> String {
    if !on() {
        return String::from("pointer check: off");
    }
    format!("pointer check: {} kept pointers checked, {} held another object", CHECKED.load(Ordering::Relaxed), WRONG.load(Ordering::Relaxed))
}
