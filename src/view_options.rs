//! The options of the views (D82 stage 3b, section 3.4e of `d82-view-per-render.md`). A mod
//! asks for parts of MGE XE in a view that it draws itself with `mge.camera.setViewOptions`;
//! MWSE's call reaches `d3d8.dll` (`MGEAPIv4::viewOptionsSet`), which calls
//! [`nirender_view_options`]. The plugin keeps the parts by the address of the key's object: a
//! `NiRenderedTexture` for a view into that texture, a `NiCamera` for that camera into the back
//! buffer. The options last until changed, cleared, or until the object is freed: the
//! destructors of both classes drop them ([`install`]). They do not end with the view's slot.
//!
//! The helper learns the parts of a view from the frame event `link::EVENT_VIEW_OPTIONS`: the
//! plugin writes it when a scene of a view opens (`view_slots`) and the parts differ from those
//! that the helper has for the slot ([`Sent`]), so at the change only.

use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::hook::{self, Cleanup};
use crate::view_slots::{Click, MAIN_VIEW, VIEW_SLOTS};

/// The key is a `NiRenderedTexture`.
pub const KIND_RENDERED_TEXTURE: u32 = 1;
/// The key is a `NiCamera` other than the world camera.
pub const KIND_CAMERA: u32 = 2;
/// The bits of the parts that the API knows (`MGEApi.h`, `ViewPart`).
pub const PARTS_ALL: u32 = (1 << 6) - 1;

/// 0x6DC5A0 `NiRenderedTexture::dtor`, and 0x6CC510 `NiCamera::dtor`. Both begin with
/// `push -1; push <SEH handler>`: seven bytes of whole instructions, no relative target.
const RENDERED_TEXTURE_DTOR: usize = 0x6DC5A0;
const RENDERED_TEXTURE_DTOR_HEAD: [u8; 7] = [0x6A, 0xFF, 0x68, 0x38, 0x38, 0x73, 0x00];
const CAMERA_DTOR: usize = 0x6CC510;
const CAMERA_DTOR_HEAD: [u8; 7] = [0x6A, 0xFF, 0x68, 0x80, 0x2C, 0x73, 0x00];

/// The parts of the keys that have options.
#[derive(Default)]
struct Store {
    by_key: HashMap<(u32, usize), u32>,
    /// The parts that the helper has for each slot.
    sent: [u32; VIEW_SLOTS],
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with_store<T>(use_store: impl FnOnce(&mut Store) -> T) -> T {
    let mut guard = STORE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    use_store(guard.get_or_insert_with(Store::default))
}

/// Sets the parts of a view, or clears them (`parts` is [`None`]). False for a key that the
/// plugin refuses: no object, or an unknown kind.
pub fn set(kind: u32, key: usize, parts: Option<u32>) -> bool {
    if key == 0 || !matches!(kind, KIND_RENDERED_TEXTURE | KIND_CAMERA) {
        return false;
    }
    with_store(|store| match parts {
        Some(parts) => {
            store.by_key.insert((kind, key), parts & PARTS_ALL);
        }
        None => {
            store.by_key.remove(&(kind, key));
        }
    });
    true
}

/// The parts of a view, or [`None`] when the key has none.
pub fn get(kind: u32, key: usize) -> Option<u32> {
    with_store(|store| store.by_key.get(&(kind, key)).copied())
}

/// The object of a key is freed: its options go.
pub fn forget(kind: u32, key: usize) {
    with_store(|store| {
        store.by_key.remove(&(kind, key));
    });
}

/// The parts for the view of a scene: those of its texture, or of its camera for a view into the
/// back buffer. The main view has none.
fn parts_of(click: &Click) -> u32 {
    if click.world_camera && click.target.is_none() {
        return 0;
    }
    let (kind, key) = match click.target {
        Some(target) => (KIND_RENDERED_TEXTURE, target),
        None => (KIND_CAMERA, click.camera),
    };
    get(kind, key).unwrap_or(0)
}

/// A scene of the view in `slot` opens. Returns the parts to write as `EVENT_VIEW_OPTIONS` when
/// they differ from those that the helper has for the slot.
pub fn scene(slot: u8, click: &Click) -> Option<u32> {
    if slot == MAIN_VIEW || slot as usize >= VIEW_SLOTS {
        return None;
    }
    let parts = parts_of(click);
    with_store(|store| {
        let sent = &mut store.sent[slot as usize];
        (*sent != parts).then(|| {
            *sent = parts;
            parts
        })
    })
}

/// The slot retired: the helper drops its parts with the slot.
pub fn retire(slot: u8) {
    with_store(|store| {
        if let Some(sent) = store.sent.get_mut(slot as usize) {
            *sent = 0;
        }
    });
}

/// The call of `d3d8.dll` for `MGEAPIv4::viewOptionsSet`. `clear` is non-zero to clear the
/// options of the key. Returns 1 when the key is accepted.
#[no_mangle]
pub extern "C" fn nirender_view_options(kind: u32, key: usize, parts: u32, clear: u32) -> u32 {
    set(kind, key, (clear == 0).then_some(parts)) as u32
}

static RENDERED_TEXTURE_DTOR_CALL: AtomicUsize = AtomicUsize::new(0);
static CAMERA_DTOR_CALL: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn rendered_texture_dtor(this: usize) {
    forget(KIND_RENDERED_TEXTURE, this);
    crate::engine::thiscall0(RENDERED_TEXTURE_DTOR_CALL.load(Ordering::Relaxed), this);
}

unsafe extern "C" fn camera_dtor(this: usize) {
    forget(KIND_CAMERA, this);
    crate::engine::thiscall0(CAMERA_DTOR_CALL.load(Ordering::Relaxed), this);
}

/// Puts the hooks on the destructors of `NiRenderedTexture` and `NiCamera`. Without them an
/// address that the engine reuses would give its new object the old options.
///
/// # Safety
///
/// Called once, on the main thread, at the plugin's start: the entry bytes are checked against
/// the heads above, and the other threads do not run these functions at that time.
pub unsafe fn install() {
    static INSTALLED: crate::sync::InstallState = crate::sync::InstallState::new();
    if !INSTALLED.begin() {
        return;
    }
    let hooks = [
        ("NiRenderedTexture::dtor", RENDERED_TEXTURE_DTOR, &RENDERED_TEXTURE_DTOR_HEAD, rendered_texture_dtor as unsafe extern "C" fn(usize), &RENDERED_TEXTURE_DTOR_CALL),
        ("NiCamera::dtor", CAMERA_DTOR, &CAMERA_DTOR_HEAD, camera_dtor as unsafe extern "C" fn(usize), &CAMERA_DTOR_CALL),
    ];
    for (name, address, head, detour, call) in hooks {
        match hook::install_raw(address, head, &[], detour as usize, 0, Cleanup::Callee) {
            Ok(original) => call.store(original, Ordering::Relaxed),
            Err(error) => {
                crate::log!("view options: {name}: {error}: options of a freed object stay until its address is used again");
                INSTALLED.finish(false);
                return;
            }
        }
    }
    INSTALLED.finish(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(target: Option<usize>, camera: usize, world_camera: bool) -> Click {
        Click { kind: crate::link::SCENE_OTHER, camera, root: 1, target, world_camera }
    }

    /// The keys are separate for the two kinds; a freed object loses its options; a refused key
    /// stores nothing. The addresses are far from those of other tests.
    #[test]
    fn options_follow_their_key() {
        assert!(!set(KIND_RENDERED_TEXTURE, 0, Some(1)));
        assert!(!set(9, 0x1000, Some(1)));
        assert!(set(KIND_RENDERED_TEXTURE, 0x7100_0000, Some(0b101)));
        assert_eq!(get(KIND_RENDERED_TEXTURE, 0x7100_0000), Some(0b101));
        assert_eq!(get(KIND_CAMERA, 0x7100_0000), None);
        assert!(set(KIND_RENDERED_TEXTURE, 0x7100_0000, Some(u32::MAX)));
        assert_eq!(get(KIND_RENDERED_TEXTURE, 0x7100_0000), Some(PARTS_ALL));
        forget(KIND_RENDERED_TEXTURE, 0x7100_0000);
        assert_eq!(get(KIND_RENDERED_TEXTURE, 0x7100_0000), None);
        assert!(set(KIND_CAMERA, 0x7100_0010, Some(1)));
        assert!(set(KIND_CAMERA, 0x7100_0010, None));
        assert_eq!(get(KIND_CAMERA, 0x7100_0010), None);
    }

    /// A scene of a view writes the parts at the change only; the world camera's view never has
    /// any; a retired slot starts again.
    #[test]
    fn a_scene_reports_the_parts_at_the_change_only() {
        set(KIND_RENDERED_TEXTURE, 0x7200_0000, Some(0b11));
        let texture = click(Some(0x7200_0000), 0x7200_0100, false);
        assert_eq!(scene(20, &texture), Some(0b11));
        assert_eq!(scene(20, &texture), None);
        set(KIND_RENDERED_TEXTURE, 0x7200_0000, Some(0b1));
        assert_eq!(scene(20, &texture), Some(0b1));
        set(KIND_RENDERED_TEXTURE, 0x7200_0000, None);
        assert_eq!(scene(20, &texture), Some(0));
        assert_eq!(scene(20, &texture), None);
        set(KIND_CAMERA, 0x7200_0200, Some(0b10));
        let screen = click(None, 0x7200_0200, false);
        assert_eq!(scene(21, &screen), Some(0b10));
        retire(21);
        assert_eq!(scene(21, &screen), Some(0b10));
        // The main view: its slot, or the world camera into the back buffer.
        assert_eq!(scene(MAIN_VIEW, &screen), None);
        set(KIND_CAMERA, 0x7200_0300, Some(1));
        assert_eq!(scene(22, &click(None, 0x7200_0300, true)), None);
        // The parts of a texture view are those of the texture, not of its camera.
        set(KIND_CAMERA, 0x7200_0400, Some(0b100));
        assert_eq!(scene(23, &click(Some(0x7200_0500), 0x7200_0400, false)), None);
    }
}
