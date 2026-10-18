//! The lights of a draw as records of their own, for the retained pipeline (step R6): a
//! `LIGHT` record for a light the first time a draw has it and when its values change,
//! keyed by the index of the light, and the keys of the lights of a draw in its shape
//! record. A light with each draw was 108 bytes; its key is 4.

use crate::gen::layouts::{NiDX8LightManager as LM, NiDX8LightManager_LightEntry as LE, NiDX8Renderer as R, NiLight as L};
use crate::mem::rd;

/// `TListItem`: data, next.
const ITEM_DATA: usize = 0;
const ITEM_NEXT: usize = 4;
/// The most lights of a draw, as the state packet has.
pub const MAX_LIGHTS: usize = 64;

/// The values that were sent for each light, by its index: the indexes of the lights of
/// the game are small numbers, and a draw looks up each of its lights.
static mut VERSIONS: Vec<Option<[u32; 26]>> = Vec::new();
/// An index above this is not a light of the game: it is not sent.
const MOST_INDEX: usize = 1 << 20;
/// The keys of the lights of the draw in progress. Only the render thread uses it.
static mut KEYS: [u32; MAX_LIGHTS] = [0; MAX_LIGHTS];
static mut RECORDS: u64 = 0;

unsafe fn versions() -> &'static mut Vec<Option<[u32; 26]>> {
    &mut *core::ptr::addr_of_mut!(VERSIONS)
}

/// Forgets the lights: the next draw with each sends it again. For the start of a capture
/// file, and for the frames of a file that held.
pub unsafe fn reset() {
    versions().clear();
}

pub unsafe fn report() -> String {
    format!("lights: {} records, {} lights known", *core::ptr::addr_of!(RECORDS), versions().iter().filter(|v| v.is_some()).count())
}

/// Notes one light by its key and its device values: sends a record when they are new or
/// changed. For a light of a shape the engine does not display (`instance_stream`). True
/// when a record went.
pub unsafe fn note_values(key: u32, values: &[u32; 26]) -> bool {
    if key as usize >= MOST_INDEX {
        return false;
    }
    let versions = versions();
    if versions.len() <= key as usize {
        versions.resize(key as usize + 1, None);
    }
    if versions[key as usize] != Some(*values) {
        crate::capture::light(key, values);
        versions[key as usize] = Some(*values);
        RECORDS += 1;
        return true;
    }
    false
}

/// The words of the colours in the device values (`D3DLIGHT8`: diffuse, specular, ambient).
pub const COLOUR_WORDS: core::ops::Range<usize> = 1..13;

/// A light that gives no light (destroyed, off, or too dim): its record again with zero
/// colours, when one was sent.
pub unsafe fn darken(key: u32) {
    let Some(Some(values)) = versions().get(key as usize).copied() else { return };
    let mut dark = values;
    dark[COLOUR_WORDS].fill(0);
    note_values(key, &dark);
}

/// Notes the lights of the light manager of the renderer, as the state of the draw has
/// them: sends the ones that are new or changed, and returns the keys of all of them.
pub unsafe fn note(renderer: usize) -> &'static [u32] {
    let keys = &mut *core::ptr::addr_of_mut!(KEYS);
    let mut count = 0;
    let manager: usize = rd(renderer, R::lightManager);
    let mut item: usize = rd(manager, LM::deviceLights_head);
    while item != 0 && count < MAX_LIGHTS {
        let light: usize = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        if light == 0 {
            continue;
        }
        let key: u32 = rd(light, L::index);
        let entry = crate::ports::hash_map::find_value(manager + LM::lightEntries, key);
        if entry == 0 || key as usize >= MOST_INDEX {
            continue;
        }
        let values: [u32; 26] = rd(entry, LE::d3dLight);
        let versions = versions();
        if versions.len() <= key as usize {
            versions.resize(key as usize + 1, None);
        }
        if versions[key as usize] != Some(values) {
            crate::capture::light(key, &values);
            versions[key as usize] = Some(values);
            RECORDS += 1;
        }
        keys[count] = key;
        count += 1;
    }
    &keys[..count]
}
