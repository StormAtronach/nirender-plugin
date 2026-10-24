//! NetImmerse idioms that the game code has inline and that several ports replicate.
//!
//! The compiler put this code inline at each place of use, thus there is no game function to
//! call. A function here makes the same reads, writes and virtual calls in the same order
//! [I: transcribed from the inline game code; there is no single function to compare with].
//! The forms that call a game function (`NiObject::release` at 0x404630, `NiPointer::set` at
//! 0x471B00 and the others) are in `engine`. Do not use one form in place of the other: a
//! port does what the game code does at that place.


use crate::engine::{vcall0, vcall1, NI_REF_OBJECT_COUNT};
use crate::gen::layouts::{InterfaceNiObject, NiRTTI, NiRefObject};
use crate::mem::{rd, wr};

/// `NiRefObject::refCount`: the offset of the reference count in each [`NiRefObject`].
pub const REF_COUNT: usize = NiRefObject::refCount;
/// The vtable slot of the deleting destructor of each [`NiRefObject`].
pub const SLOT_DELETING_DTOR: usize = InterfaceNiObject::deleting_dtor;
/// The vtable slot of `GetRTTI` of each `NiObject`.
pub const SLOT_GET_RTTI: usize = InterfaceNiObject::GetRTTI;
/// The pointer in a `NiRTTI` to the `NiRTTI` of the base class.
pub const RTTI_BASE: usize = NiRTTI::baseRTTI;

/// The slots of the map array of a `NiTexturingProperty` (`maps_storage`), by map type; the
/// decals start at [`MAP_DECAL_0`].
pub const MAP_DARK: usize = 1;
pub const MAP_DETAIL: usize = 2;
pub const MAP_GLOSS: usize = 3;
pub const MAP_GLOW: usize = 4;
pub const MAP_BUMP: usize = 5;
pub const MAP_DECAL_0: usize = 6;

/// The map of a slot of the map array `maps` of a `NiTexturingProperty`, or 0.
#[inline(always)]
pub unsafe fn texturing_map(maps: usize, slot: usize) -> usize {
    rd(maps, slot * core::mem::size_of::<u32>())
}

/// Calls the virtual deleting destructor of `object`, slot 0x00, with the argument 1. The
/// argument 1 is the flag "free the memory".
#[inline(always)]
pub unsafe fn delete_object(object: usize) {
    vcall1(object, SLOT_DELETING_DTOR, 1);
}

/// Adds one reference to `object`. The game code has this inline as `inc dword ptr [object+4]`.
/// `object` is not 0.
#[inline(always)]
pub unsafe fn add_reference(object: usize) {
    wr(object, REF_COUNT, rd::<u32>(object, REF_COUNT).wrapping_add(1));
}

/// The inline release of one reference. It subtracts 1 from the count and writes the count.
/// When the count is 0 it calls the deleting destructor with the argument 1. `object` is not
/// 0. This is not a call of `NiObject::release` (0x404630).
#[inline(always)]
pub unsafe fn release(object: usize) {
    let count = rd::<u32>(object, REF_COUNT).wrapping_sub(1);
    wr(object, REF_COUNT, count);
    if count == 0 {
        delete_object(object);
    }
}

/// The inline release with the test for a null object before it. It does nothing when
/// `object` is 0.
#[inline(always)]
pub unsafe fn release_if_set(object: usize) {
    if object != 0 {
        release(object);
    }
}

/// The inline release of the object that the pointer at `slot` holds. Then it writes 0 to
/// `slot`. It does nothing when the pointer is 0, also no write.
#[inline(always)]
pub unsafe fn release_and_clear(slot: usize) {
    let object: usize = rd(slot, 0);
    if object != 0 {
        release(object);
        wr::<usize>(slot, 0, 0);
    }
}

/// The inline assignment to the `NiPointer` at `slot`. When the pointer holds `object`
/// already, it does nothing. If not, it releases the old object inline, writes `object` to
/// `slot`, then adds one reference to `object`. This is not a call of `NiPointer::set`
/// (0x471B00), `NiPointer::operator=` (0x6EA9C0) or `NiPointer::assign` (0x712850).
#[inline(always)]
pub unsafe fn pointer_assign(slot: usize, object: usize) {
    let old: usize = rd(slot, 0);
    if old != object {
        if old != 0 {
            release(old);
        }
        wr(slot, 0, object);
        if object != 0 {
            add_reference(object);
        }
    }
}

/// `GetRTTI` of a `NiObject` through its vtable, slot 0x04. Returns the address of the
/// `NiRTTI`.
#[inline(always)]
pub unsafe fn get_rtti(object: usize) -> usize {
    vcall0(object, SLOT_GET_RTTI) as usize
}

/// The inline `NiIsKindOf`. It makes one `GetRTTI` call, then it follows the base pointers.
/// The result is true when the run-time type of `object` is `rtti` or comes from `rtti`.
#[inline(always)]
pub unsafe fn is_kind_of(object: usize, rtti: usize) -> bool {
    let mut at = get_rtti(object);
    while at != 0 {
        if at == rtti {
            return true;
        }
        at = rd(at, RTTI_BASE);
    }
    false
}

/// Adds `delta` to `global_count_of_NiRefObject` (0x7DECE0). The inline constructor of a
/// [`NiRefObject`] adds 1. The inline destructor adds -1.
#[inline(always)]
pub unsafe fn ref_object_count_add(delta: i32) {
    wr(NI_REF_OBJECT_COUNT, 0, rd::<i32>(NI_REF_OBJECT_COUNT, 0).wrapping_add(delta));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        /// The object and the flags of each call of the fake deleting destructor.
        static DELETED: RefCell<Vec<(usize, u32)>> = const { RefCell::new(Vec::new()) };
    }

    unsafe extern "thiscall" fn deleting_dtor(this: usize, flags: u32) -> u32 {
        DELETED.with(|deleted| deleted.borrow_mut().push((this, flags)));
        0
    }

    /// [`GetRTTI`] of the fake object. The object has the address of its [`NiRTTI`] at +8.
    unsafe extern "thiscall" fn fake_get_rtti(this: usize) -> u32 {
        rd(this, 8)
    }

    fn vtable() -> [usize; 2] {
        [deleting_dtor as *const () as usize, fake_get_rtti as *const () as usize]
    }

    fn deleted() -> Vec<(usize, u32)> {
        DELETED.with(|deleted| core::mem::take(&mut *deleted.borrow_mut()))
    }

    #[test]
    fn release_deletes_at_zero_only() {
        let vtable = vtable();
        // vtable, count
        let mut object = [vtable.as_ptr() as usize, 2];
        let o = object.as_mut_ptr() as usize;
        unsafe {
            release(o);
            assert_eq!(rd::<u32>(o, REF_COUNT), 1);
            assert_eq!(deleted(), []);
            release(o);
            assert_eq!(rd::<u32>(o, REF_COUNT), 0);
            // The flag "free the memory".
            assert_eq!(deleted(), [(o, 1)]);
            release_if_set(0);
            assert_eq!(deleted(), []);
            add_reference(o);
            add_reference(o);
            release_if_set(o);
            assert_eq!(rd::<u32>(o, REF_COUNT), 1);
            delete_object(o);
            assert_eq!(deleted(), [(o, 1)]);
        }
    }

    #[test]
    fn release_and_clear_writes_the_slot_only_when_it_held_an_object() {
        let vtable = vtable();
        let mut object = [vtable.as_ptr() as usize, 1];
        let o = object.as_mut_ptr() as usize;
        let mut slot = [o];
        let s = slot.as_mut_ptr() as usize;
        unsafe {
            release_and_clear(s);
            assert_eq!(rd::<usize>(s, 0), 0);
            assert_eq!(deleted(), [(o, 1)]);
            release_and_clear(s);
            assert_eq!(deleted(), []);
        }
    }

    #[test]
    fn pointer_assign_counts_references() {
        let vtable = vtable();
        let mut first = [vtable.as_ptr() as usize, 5];
        let mut second = [vtable.as_ptr() as usize, 1];
        let (a, b) = (first.as_mut_ptr() as usize, second.as_mut_ptr() as usize);
        let mut slot = [0usize];
        let s = slot.as_mut_ptr() as usize;
        unsafe {
            pointer_assign(s, a);
            assert_eq!((rd::<usize>(s, 0), rd::<u32>(a, REF_COUNT)), (a, 6));
            // The same object again. Nothing changes.
            pointer_assign(s, a);
            assert_eq!(rd::<u32>(a, REF_COUNT), 6);
            pointer_assign(s, b);
            assert_eq!(rd::<usize>(s, 0), b);
            assert_eq!((rd::<u32>(a, REF_COUNT), rd::<u32>(b, REF_COUNT)), (5, 2));
            pointer_assign(s, 0);
            assert_eq!((rd::<usize>(s, 0), rd::<u32>(b, REF_COUNT)), (0, 1));
            assert_eq!(deleted(), []);
            // The slot has the last reference of the old object.
            wr::<u32>(a, REF_COUNT, 1);
            wr(s, 0, a);
            pointer_assign(s, b);
            assert_eq!(deleted(), [(a, 1)]);
            assert_eq!((rd::<usize>(s, 0), rd::<u32>(b, REF_COUNT)), (b, 2));
        }
    }

    #[test]
    fn is_kind_of_follows_the_base_types() {
        let vtable = vtable();
        // A NiRTTI is a name and the NiRTTI of the base class.
        let root = [0usize, 0];
        let middle = [0usize, root.as_ptr() as usize];
        let leaf = [0usize, middle.as_ptr() as usize];
        let other = [0usize, root.as_ptr() as usize];
        let object = [vtable.as_ptr() as usize, 1, leaf.as_ptr() as usize];
        let o = object.as_ptr() as usize;
        unsafe {
            assert_eq!(get_rtti(o), leaf.as_ptr() as usize);
            assert!(is_kind_of(o, leaf.as_ptr() as usize));
            assert!(is_kind_of(o, middle.as_ptr() as usize));
            assert!(is_kind_of(o, root.as_ptr() as usize));
            assert!(!is_kind_of(o, other.as_ptr() as usize));
        }
    }
}
