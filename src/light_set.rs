//! The lights of the retained instances (plan 8e): the set of the lights that the effect
//! states of the table's instances hold, kept by events. Each frame the table builds the
//! values of each light of the set and sends a record when they changed
//! (`instance_stream::note_light_once`): the values decide, not the revision of the light,
//! because not every writer raises the revision (MWSE's Lua bindings do not).
//!
//! The set follows the instances. When an instance counts a new effect state, it counts up
//! the lights of the state's list and keeps them; when it leaves the state or goes, it counts
//! them down. An effect state does not change after it is built: `NiNode::PushLocalEffects`
//! 0x6C8F00 adds lights only to a state it made, and the downward pass clones a state that
//! changes. The states do not keep their lights alive: a destroyed light leaves the set at
//! once (`purge`, from the port of `NiDX8Renderer::PurgeEffect`, which `NiLight::dtor`
//! 0x6F99F0 calls), and its record gets zero colours. The engine gives a new light the index
//! (the record key) of a destroyed one, and the allocator can give it the same address: each
//! entry has a serial number, and a count down with another serial does nothing.

use crate::gen::layouts::{NiDynamicEffectState as DES, NiDynamicEffectState_InternalList as DL, NiLight as L};
use crate::mem::rd;
use crate::ptr_hash::HashMap;

/// A light of the set: its serial number and the counts of the instances that hold it.
struct Entry {
    serial: u32,
    count: u32,
}

static mut SET: Option<HashMap<usize, Entry>> = None;
static mut NEXT_SERIAL: u32 = 1;
/// The counts up and down, the lights that entered and left the set, and the lights purged.
static mut COUNTS: [u64; 5] = [0; 5];

unsafe fn set() -> &'static mut HashMap<usize, Entry> {
    (*core::ptr::addr_of_mut!(SET)).get_or_insert_with(HashMap::default)
}

/// The lights of an effect state's list, without the ambient lights (the light manager does
/// not turn them on for a draw), into `out` with their serials.
pub unsafe fn count(state: usize, out: &mut Vec<(usize, u32)>) {
    out.clear();
    if state == 0 {
        return;
    }
    let set = set();
    let counts = &mut *core::ptr::addr_of_mut!(COUNTS);
    let mut iter: usize = rd(state, DES::lights);
    while iter != 0 {
        let light: usize = rd(iter, DL::effect);
        iter = rd(iter, DL::next);
        if light == 0 || crate::ports::light_manager::light_get_type(light) == 0 {
            continue;
        }
        let entry = set.entry(light).or_insert_with(|| {
            counts[2] += 1;
            let serial = NEXT_SERIAL;
            NEXT_SERIAL = NEXT_SERIAL.wrapping_add(1).max(1);
            Entry { serial, count: 0 }
        });
        entry.count += 1;
        counts[0] += 1;
        out.push((light, entry.serial));
    }
}

/// The lights that `count` gave, counted down. A light that left the set (destroyed) or whose
/// address has a new light is not counted down.
pub unsafe fn uncount(lights: &[(usize, u32)]) {
    let set = set();
    let counts = &mut *core::ptr::addr_of_mut!(COUNTS);
    for &(light, serial) in lights {
        let Some(entry) = set.get_mut(&light) else { continue };
        if entry.serial != serial {
            continue;
        }
        counts[1] += 1;
        entry.count -= 1;
        if entry.count == 0 {
            set.remove(&light);
            counts[3] += 1;
        }
    }
}

/// A light is destroyed: it leaves the set, and its record gets zero colours (an instance
/// that still has its key until its next work gets no light from it).
pub unsafe fn purge(light: usize) {
    if set().remove(&light).is_some() {
        (*core::ptr::addr_of_mut!(COUNTS))[4] += 1;
        crate::light_stream::darken(rd(light, L::index));
    }
}

/// The lights of the set.
pub unsafe fn lights() -> impl Iterator<Item = usize> {
    set().keys().copied()
}

pub unsafe fn len() -> usize {
    set().len()
}

pub unsafe fn report() -> String {
    let c = *core::ptr::addr_of!(COUNTS);
    format!("light set (plan 8e): {} lights; counts up {}, down {}; entered {}, left {}, destroyed {}", len(), c[0], c[1], c[2], c[3], c[4])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::wr;

    /// The slot of `GetEffectType` in the light's vtable (`ports::light_manager`).
    const GET_TYPE: usize = 0x94;

    extern "thiscall" fn type_ambient(_light: usize) -> u32 {
        0
    }
    extern "thiscall" fn type_point(_light: usize) -> u32 {
        2
    }

    fn leak(size: usize) -> usize {
        Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr() as usize
    }

    unsafe fn light(get_type: extern "thiscall" fn(usize) -> u32, index: u32) -> usize {
        let vtable = leak(0x98);
        wr(vtable, GET_TYPE, get_type as *const () as usize);
        let light = leak(0xF0);
        wr(light, 0, vtable);
        wr::<u32>(light, L::index, index);
        light
    }

    unsafe fn state(lights: &[usize]) -> usize {
        let state = leak(DES::SIZE);
        let mut next = 0usize;
        for &light in lights.iter().rev() {
            let link = leak(DL::SIZE);
            wr(link, DL::next, next);
            wr(link, DL::effect, light);
            next = link;
        }
        wr(state, DES::lights, next);
        state
    }

    #[test]
    fn the_set_follows_the_counts_and_a_destroyed_light() {
        unsafe {
            let (a, b, ambient) = (light(type_point, 7), light(type_point, 8), light(type_ambient, 9));
            let one = state(&[a, ambient, b]);
            let two = state(&[b]);
            let (mut first, mut second) = (Vec::new(), Vec::new());
            count(one, &mut first);
            count(two, &mut second);
            // The ambient light is not in the set.
            assert_eq!(first.iter().map(|&(l, _)| l).collect::<Vec<_>>(), vec![a, b]);
            assert_eq!(len(), 2);
            // b is held twice: one count down keeps it.
            uncount(&first);
            assert_eq!(lights().collect::<Vec<_>>(), vec![b]);
            // b is destroyed while `second` holds it, and a new light at its address enters
            // the set: the old count down does not touch the new entry.
            purge(b);
            assert_eq!(len(), 0);
            let mut third = Vec::new();
            count(two, &mut third);
            assert_ne!(third[0].1, second[0].1);
            uncount(&second);
            assert_eq!(len(), 1);
            uncount(&third);
            assert_eq!(len(), 0);
        }
    }
}
