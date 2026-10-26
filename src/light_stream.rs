//! The lights of a draw as records of their own, for the retained pipeline: a `LIGHT`
//! record for a light the first time a draw has it and when its values change, keyed by the
//! index of the light, and the keys of the lights of a draw in its shape record.

use crate::checked::Open;
use crate::main_thread::{MainCell, MainThread};
use crate::gen::layouts::{NiDX8LightManager as LM, NiDX8LightManager_LightEntry as LE, NiDX8Renderer as R, NiLight as L};
use crate::mem::rd;

/// `TListItem`: data, next.
const ITEM_DATA: usize = 0;
const ITEM_NEXT: usize = 4;
/// The most lights of a draw, as the state packet has.
pub const MAX_LIGHTS: usize = 64;

/// The values that were sent for each light, by its index: the indexes of the lights of
/// the game are small numbers, and a draw looks up each of its lights. The device values,
/// then the radius of the game.
type Sent = ([u32; 26], u32);
static VERSIONS: MainCell<Vec<Option<Sent>>> = MainCell::new(Vec::new());
/// An index above this is not a light of the game: it is not sent.
const MOST_INDEX: usize = 1 << 20;
/// The keys of the lights of the draw in progress. Only the render thread uses it.
static KEYS: MainCell<[u32; MAX_LIGHTS]> = MainCell::new([0; MAX_LIGHTS]);
static RECORDS: MainCell<u64> = MainCell::new(0);
static GONE: MainCell<u64> = MainCell::new(0);
/// The radius of each light by its key, as the engine passed it to its attach pass (MWSE's
/// event of a light placed, `world_lights`); 0 when none came. The light's specular colour
/// holds the attach radius, which a module that owns the light attach radius makes larger
/// than the light: MGE XE makes it the end of its fade and 160 more.
static RADII: MainCell<Vec<f32>> = MainCell::new(Vec::new());

/// Notes the radius that the engine gave a light at its placement.
pub fn note_radius(t: &MainThread, key: u32, radius: f32) {
    if key as usize >= MOST_INDEX {
        return;
    }
    let mut radii = RADII.get(t);
    if radii.len() <= key as usize {
        radii.resize(key as usize + 1, 0.0);
    }
    radii[key as usize] = radius;
}

/// The radius of a light from its placement, when one came.
pub fn radius_of(t: &MainThread, key: u32) -> Option<f32> {
    RADII.get(t).get(key as usize).copied().filter(|&radius| radius > 0.0)
}

#[track_caller]
fn versions<'a>(t: &'a MainThread) -> Open<'a, Vec<Option<Sent>>> {
    VERSIONS.get(t)
}

/// Forgets the lights: the next draw with each sends it again. For the start of a capture
/// file, and for the frames of a file that held.
pub unsafe fn reset(t: &MainThread) {
    versions(t).clear();
    crate::world_lights::resend();
}

pub unsafe fn report(t: &MainThread) -> String {
    format!(
        "lights: {} records, {} gone records, {} lights known",
        *RECORDS.get(t),
        *GONE.get(t),
        versions(t).iter().filter(|v| v.is_some()).count()
    )
}

/// The light with this key was destroyed (`NiDX8LightManager::RemoveLight`): the engine gives
/// the key to the next light it makes. Sends a `LIGHT_GONE` record when a `LIGHT` record went
/// for the key, so that the next light with the key goes as new.
pub unsafe fn forget(t: &MainThread, key: u32) {
    if let Some(radius) = RADII.get(t).get_mut(key as usize) {
        *radius = 0.0;
    }
    let mut versions = versions(t);
    let Some(sent) = versions.get_mut(key as usize) else { return };
    if sent.take().is_some() {
        crate::capture::light_gone(key);
        *GONE.get(t) += 1;
    }
}

/// Notes one light by its key, its device values and the radius of the game: sends a record
/// when they are new or changed. For a light of a shape the engine does not display
/// (`instance_stream`). True when a record went.
pub unsafe fn note_values(t: &MainThread, key: u32, values: &[u32; 26], radius: f32) -> bool {
    if key as usize >= MOST_INDEX {
        return false;
    }
    let mut versions = versions(t);
    if versions.len() <= key as usize {
        versions.resize(key as usize + 1, None);
    }
    let sent = (*values, radius.to_bits());
    if versions[key as usize] != Some(sent) {
        crate::capture::light(key, values, radius);
        versions[key as usize] = Some(sent);
        *RECORDS.get(t) += 1;
        return true;
    }
    false
}

/// The words of the colours in the device values (`D3DLIGHT8`: diffuse, specular, ambient).
pub const COLOUR_WORDS: core::ops::Range<usize> = 1..13;


/// Notes the lights of the light manager of the renderer, as the state of the draw has
/// them: sends the ones that are new or changed, and returns the keys of all of them.
#[track_caller]
pub unsafe fn note<'a>(t: &'a MainThread, renderer: usize) -> Open<'a, [u32]> {
    let mut keys = KEYS.get(t);
    let mut count = 0;
    let manager: usize = rd(renderer, R::lightManager);
    // The list and the map under the manager's own lock, as the engine's `updateLights` walks
    // them.
    let _lock = crate::sync::engine_lock(manager + LM::criticalSection);
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
        note_values(t, key, &values, crate::link_client::point_light_radius(light));
        keys[count] = key;
        count += 1;
    }
    keys.map(|keys| &mut keys[..count])
}
