//! The events of the engine's cell worker. The engine loads and unloads cells on a second
//! thread: it attaches, detaches, updates, destroys and purges there. A port runs its Rust
//! code on the main thread only: on another thread it calls the engine and nothing else
//! (`shadow::dispatch_inner`). This module brings those calls to the plugin's model of the
//! scene.
//!
//! A port's `any_thread` hook writes an event when the call comes from another thread: the
//! call's kind and arguments. It touches no table of the plugin. The main thread drains the
//! queue at fixed points, before it reads the pointers it keeps (the start of each scene, a
//! change of the mode, the end of the frame). It applies each event as the port does on the
//! main thread. Applying an event reads nothing of the object: it can be freed by then.
//!
//! The same address is never the same object (lifetime rule 4, W589). The worker can free an
//! object and the engine can give its address, or a light's index, to a new object before the
//! drain. So a keyed lookup of the main thread applies the pending destroys of its key first
//! ([`settle`]): the new object is then new, and the later drain does not forget it. The keys
//! are a shape's address (`instance_stream::seen`, `consider`) and a light's index
//! (`light_stream::note_values`). A texture's renderer data has its own form: its mark takes the
//! pending free of its address out of the queue (`ports::textures::mark_unreadable`). Geometry
//! data needs none: its record carries the engine's unique id.
//!
//! The plugin reads through a kept engine address only with a proof of life in the same call,
//! or after a destroy event removed every freed one (lifetime rule 3, W589):
//! - `world_lights`, the lights: a light leaves the set at MWSE's LIGHT_REMOVED and at
//!   `RemoveLight` (the hook below), under the set's lock, before the free.
//! - `world_lights`, the light's cell: the engine frees a cell only in
//!   `RecordsHandler::clearAllRecords`, at the game's start (`loadESMPsEverything`) and at its
//!   exit (`DataHandler::dtor`) [IDA, Morrowind2.i64 0x4B8741]. A cell lives through the play.
//! - `camera_space`, the roots of the eye: found in the live camera's parent chain in the same
//!   call (`copied_root_above`).
//! - `effect_chain`: none. The draw that notes a chain notes the effect's projection, its stage
//!   words and its billboards' flags as values, and the check at the scene's end reads those.
//!
//! A new store of an engine address needs its line in `known_stores.txt` (the test
//! `every_store_of_an_address_is_known`).
//!
//! The events:
//! - `NiGeometry::dtor`: the shape leaves the table.
//! - `NiDX8SourceTextureData::dtor`: the mark of the unreadable texture goes
//!   (`ports::textures::forget_unreadable`). On the main thread the destructor also makes
//!   every instance note its handles again (a new generation). On the worker it does not:
//!   the data goes with its texture, so no live instance names it (an inference). A new
//!   generation for each texture of an unloaded cell makes a frame hitch.
//! - `NiDX8LightManager::RemoveLight`: the light's key is free again (`light_stream::forget`).
//! - `NiRenderer_static::RemoveGeometryData`: the geometry data goes from the helper
//!   (`geometry_stream::gone`). Its one caller is the data's destructor. The hook reads the
//!   unique id while the data is alive; on the main thread it sends the record at once.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::main_thread::MainThread;
use crate::mem::rd;
use crate::ptr_hash::{HashMap, PtrBuild};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Event {
    /// A geometry was destroyed.
    Destroyed(usize),
    /// A source texture's renderer data was destroyed: its address.
    TextureFreed(usize),
    /// A light was destroyed: its index.
    LightGone(u32),
    /// A geometry data was destroyed: its address and its unique id.
    GeometryGone(u32, u32),
}

const KINDS: usize = 4;
const KIND_NAMES: [&str; KINDS] = [
    "shapes destroyed",
    "textures freed",
    "lights destroyed",
    "geometry data destroyed",
];

/// The key of a lookup on the main thread that [`settle`] settles.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    /// A shape's address (`NiGeometry`).
    Shape(usize),
    /// A light's index (`NiLight::index`).
    Light(u32),
}

impl Event {
    fn kind(self) -> usize {
        match self {
            Event::Destroyed(_) => 0,
            Event::TextureFreed(_) => 1,
            Event::LightGone(_) => 2,
            Event::GeometryGone(..) => 3,
        }
    }

    /// The key that a lookup settles, for the kinds that have one.
    fn key(self) -> Option<Key> {
        match self {
            Event::Destroyed(shape) => Some(Key::Shape(shape)),
            Event::LightGone(index) => Some(Key::Light(index)),
            Event::TextureFreed(_) | Event::GeometryGone(..) => None,
        }
    }
}

/// The events in their order, and the number of events of each key that waits.
struct Queue {
    events: Vec<Event>,
    keys: HashMap<Key, u32>,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue {
    events: Vec::new(),
    keys: HashMap::with_hasher(PtrBuild::new()),
});
static WAITING: AtomicBool = AtomicBool::new(false);
/// The number of events with a key in the queue: 0 lets [`settle`] return at once.
static KEYED: AtomicUsize = AtomicUsize::new(0);
static QUEUED: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];
static APPLIED: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];
/// Of the applied events, those that a lookup of their key applied before the drain.
static SETTLED: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];
/// Of the shapes the worker destroyed, those the table had as instances when the main thread
/// applied the event. A count above 0 means the worker frees objects the plugin models, and
/// the window between the free and the drain is real.
static DESTROYED_INSTANCES: AtomicU64 = AtomicU64::new(0);
/// Notes on the first worker-side destructors of shapes that carry the table's mark (the
/// shapes the helper draws): the shape's name, its ancestors' names, and the engine addresses
/// found on the worker's stack (a scan: the engine omits frame pointers, so the frame chain
/// gives nothing). The worker frees shapes outside `RecordsHandler::unloadCell` too.
const NOTES_KEPT: usize = 16;
static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static NOTES_LOGGED: AtomicU64 = AtomicU64::new(0);

/// Writes an event when the call came from another thread (on the main thread the port's own
/// code does the work at once).
fn push(event: Event) {
    if crate::shadow::on_main_thread() {
        return;
    }
    enqueue(event);
}

/// A destroy of a shape or a light from the cell worker, for a test on the main thread.
#[cfg(test)]
pub(crate) fn queue_for_test(key: Key) {
    enqueue(match key {
        Key::Shape(shape) => Event::Destroyed(shape),
        Key::Light(index) => Event::LightGone(index),
    });
}

/// True when a destroy of `key` waits in the queue.
#[cfg(test)]
pub(crate) fn waits_for_test(key: Key) -> bool {
    let queue = QUEUE.lock().unwrap();
    queue.keys.contains_key(&key) || queue.events.iter().any(|event| event.key() == Some(key))
}

fn enqueue(event: Event) {
    if let Ok(mut queue) = QUEUE.lock() {
        queue.events.push(event);
        // The key and the count under the lock: a lookup that reads the count after the free
        // of the object finds its key.
        if let Some(key) = event.key() {
            *queue.keys.entry(key).or_insert(0) += 1;
            KEYED.fetch_add(1, Ordering::Release);
        }
        QUEUED[event.kind()].fetch_add(1, Ordering::Relaxed);
        WAITING.store(true, Ordering::Release);
    }
}

/// Takes the frees of renderer data at `data` out of the queue: the address is in use again.
pub fn drop_texture_freed(data: usize) {
    if let Ok(mut queue) = QUEUE.lock() {
        let before = queue.events.len();
        queue
            .events
            .retain(|event| !matches!(event, Event::TextureFreed(freed) if *freed == data));
        let dropped = before - queue.events.len();
        if dropped != 0 {
            QUEUED[Event::TextureFreed(data).kind()].fetch_sub(dropped as u64, Ordering::Relaxed);
        }
    }
}

/// A lookup of `key` on the main thread: applies the destroys of `key` that wait in the queue,
/// in their order, before the lookup. Then an object at the address or the index of a freed one
/// is new, and the later drain does not forget it (lifetime rule 4, W589). With no keyed event in
/// the queue, the function returns at once.
///
/// The hook pushes an event in the object's destructor, before the free, so a new object at its
/// address or index comes after the event, and a lookup of the new object finds the key.
///
/// # Safety
///
/// As [`drain`]: the events only make the plugin forget. `t` is the proof of the main thread.
/// The caller holds no table that the applied events open: a shape's event opens
/// `instance_stream`'s tables, a light's event `light_stream`'s lists.
pub unsafe fn settle(t: &MainThread, key: Key) {
    if KEYED.load(Ordering::Acquire) == 0 {
        return;
    }
    let settled = {
        let Ok(mut queue) = QUEUE.lock() else { return };
        if queue.keys.remove(&key).is_none() {
            return;
        }
        let mut settled = Vec::new();
        queue.events.retain(|&event| {
            if event.key() == Some(key) {
                settled.push(event);
                return false;
            }
            true
        });
        KEYED.fetch_sub(settled.len(), Ordering::Relaxed);
        settled
    };
    for event in settled {
        apply(t, event);
        SETTLED[event.kind()].fetch_add(1, Ordering::Relaxed);
    }
}

// The `any_thread` hooks, with the arguments of each port's call.

unsafe fn on_geometry_dtor(args: &[usize]) {
    if crate::shadow::on_main_thread() {
        return;
    }
    if let Some(&shape) = args.first() {
        push(Event::Destroyed(shape));
        // The shape is alive here (its destructor runs): its mark, name and ancestors can be
        // read. Only the marked ones are noted.
        if rd::<u16>(shape, FLAGS) & crate::instance_stream::RETAINED_BIT != 0 {
            if let Ok(mut notes) = NOTES.try_lock() {
                if notes.len() < NOTES_KEPT {
                    notes.push(note_of(shape));
                }
            }
        }
    }
}

const FLAGS: usize = crate::gen::layouts::NiAVObject::flags;
const PARENT: usize = crate::gen::layouts::NiAVObject::parentNode;
const NAME: usize = crate::gen::layouts::NiObjectNET::name;

unsafe fn name_of(object: usize) -> String {
    let at: usize = rd(object, NAME);
    if at == 0 {
        return String::from("-");
    }
    core::ffi::CStr::from_ptr(at as *const core::ffi::c_char)
        .to_string_lossy()
        .into_owned()
}

/// The shape's name, its ancestors' names from the parent up (at most 8), and the engine
/// addresses among the 192 words above the stack pointer.
unsafe fn note_of(shape: usize) -> String {
    let mut names = vec![name_of(shape)];
    let mut node: usize = rd(shape, PARENT);
    while node != 0 && names.len() < 9 {
        names.push(name_of(node));
        node = rd(node, PARENT);
    }
    let esp: usize;
    core::arch::asm!("mov {}, esp", out(reg) esp, options(nomem, nostack, preserves_flags));
    // The top of this thread's stack (the thread information block, `fs:[4]`): the scan
    // must stop there, or it reads past the worker's stack and the worker crashes.
    let top: usize;
    core::arch::asm!("mov {}, fs:[4]", out(reg) top, options(nostack, readonly, preserves_flags));
    let words = top.saturating_sub(esp) / 4;
    let mut addresses = Vec::new();
    for i in 0..192.min(words) {
        let word: usize = rd(esp, i * 4);
        if (0x401000..0x7A0000).contains(&word) && !addresses.contains(&word) {
            addresses.push(word);
        }
    }
    let addresses: Vec<String> = addresses.iter().map(|a| format!("{a:#x}")).collect();
    format!(
        "shape 0x{shape:x} '{}' under {}; stack words in the engine: {}",
        names[0],
        names[1..].join(" < "),
        addresses.join(" ")
    )
}

unsafe fn on_texture_data_dtor(args: &[usize]) {
    let Some(&data) = args.first() else { return };
    push(Event::TextureFreed(data));
}

/// `NiDX8LightManager::RemoveLight(this, light)`: the light's destructor calls it, so the
/// light is alive here.
unsafe fn on_remove_light(args: &[usize]) {
    let Some(&light) = args.get(1) else { return };
    // On any thread: the set of the lights of the world never keeps a destroyed light.
    crate::world_lights::forget_destroyed(light);
    if crate::shadow::on_main_thread() {
        return;
    }
    push(Event::LightGone(rd(light, crate::gen::layouts::NiLight::index)));
}

/// `NiRenderer_static::RemoveGeometryData(data)`: the data's destructor calls it, so the data
/// is alive here.
unsafe fn on_remove_geometry_data(args: &[usize]) {
    let Some(&data) = args.first() else { return };
    if !crate::shadow::on_main_thread() {
        crate::ports::geometry::worker_removal_begins();
    }
    if data == 0 {
        return;
    }
    let unique: u32 = rd(data, crate::gen::layouts::NiGeometryData::uniqueID);
    match crate::main_thread::MainThread::here() {
        Some(t) => crate::geometry_stream::gone(&t, data as u32, unique),
        None => push(Event::GeometryGone(data as u32, unique)),
    }
}

/// The hooks and the names of their ports.
const HOOKS: [(&str, unsafe fn(&[usize])); 4] = [
    ("NiGeometry::dtor", on_geometry_dtor),
    ("NiDX8SourceTextureData::dtor", on_texture_data_dtor),
    ("NiDX8LightManager::RemoveLight", on_remove_light),
    ("NiRenderer_static::RemoveGeometryData", on_remove_geometry_data),
];

/// Puts the hooks on their ports.
pub fn install() {
    let mut missing = Vec::new();
    for (name, hook) in HOOKS {
        match crate::ports::all().find(|port| port.name == name) {
            Some(port) => port.any_thread.store(hook as usize, Ordering::Relaxed),
            None => missing.push(name),
        }
    }
    // The end of a removal of geometry data on the cell worker, for the crossings of the
    // geometry cache (`ports::geometry`).
    if let Some(port) = crate::ports::all().find(|port| port.name == "NiRenderer_static::RemoveGeometryData") {
        port.after_off_thread.store(
            crate::ports::geometry::worker_removal_ends as *const () as usize,
            Ordering::Relaxed,
        );
    }
    if missing.is_empty() {
        crate::log!(
            "worker events: {} ports write their events from the engine's cell worker",
            HOOKS.len()
        );
    } else {
        crate::log!(
            "worker events: no port for {}; their events from the cell worker are lost",
            missing.join(", ")
        );
    }
}

/// The main thread applies the events written since the last drain, in their order.
///
/// # Safety
///
/// No condition: the function returns on a thread other than the main thread. The events name
/// shapes, lights and geometry data that the worker threads saw go, which the function only
/// forgets.
pub unsafe fn drain() {
    if !crate::shadow::on_main_thread() {
        return;
    }
    // The check above: this is the main thread, and the events open its table.
    let t = &crate::main_thread::MainThread::claim();
    // The textures freed on any thread since the last drain take their materials with them.
    crate::material_stream::drain_freed_textures(t);
    if !WAITING.load(Ordering::Acquire) {
        return;
    }
    let events = match QUEUE.lock() {
        Ok(mut queue) => {
            WAITING.store(false, Ordering::Relaxed);
            queue.keys.clear();
            KEYED.store(0, Ordering::Relaxed);
            core::mem::take(&mut queue.events)
        }
        Err(_) => return,
    };
    for &event in &events {
        apply(t, event);
    }
    log_notes();
}

/// Applies one event as the port does on the main thread.
unsafe fn apply(t: &MainThread, event: Event) {
    match event {
        Event::Destroyed(shape) => {
            if crate::instance_stream::forget(t, shape) {
                DESTROYED_INSTANCES.fetch_add(1, Ordering::Relaxed);
            }
        }
        Event::TextureFreed(data) => crate::ports::textures::forget_unreadable(data),
        Event::LightGone(key) => crate::light_stream::forget(t, key),
        Event::GeometryGone(data, unique) => crate::geometry_stream::gone(t, data, unique),
    }
    if APPLIED[event.kind()].fetch_add(1, Ordering::Relaxed) == 0 {
        crate::log!(
            "worker events: the first of the {} applied on the main thread",
            KIND_NAMES[event.kind()]
        );
    }
}

/// Writes the notes on the worker's destructors of marked shapes to the log as they come (at
/// most [`NOTES_KEPT`]).
unsafe fn log_notes() {
    let Ok(mut notes) = NOTES.try_lock() else { return };
    for note in notes.drain(..) {
        let n = NOTES_LOGGED.fetch_add(1, Ordering::Relaxed);
        if n < NOTES_KEPT as u64 {
            crate::log!("worker events: marked shape destroyed on the cell worker ({n}): {note}");
        }
    }
}

pub fn report() -> String {
    let parts: Vec<String> = (0..KINDS)
        .map(|k| {
            format!(
                "{} {} queued, {} applied",
                KIND_NAMES[k],
                QUEUED[k].load(Ordering::Relaxed),
                APPLIED[k].load(Ordering::Relaxed)
            )
        })
        .collect();
    format!(
        "worker events (C26, C27): {}; of the shapes destroyed, {} were instances of the table; applied at a lookup of their key before the drain (W589): shapes {}, lights {}",
        parts.join("; "),
        DESTROYED_INSTANCES.load(Ordering::Relaxed),
        SETTLED[Event::Destroyed(0).kind()].load(Ordering::Relaxed),
        SETTLED[Event::LightGone(0).kind()].load(Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::textures::{forget_unreadable, is_unreadable, mark_unreadable};

    /// The end of a removal of the cell worker is the `after_off_thread` hook of
    /// RemoveGeometryData. Its `after` hook runs on the main thread only, where no removal of
    /// the worker ends.
    #[test]
    fn the_end_of_a_removal_is_the_hook_of_the_other_threads() {
        install();
        let port = crate::ports::all()
            .find(|port| port.name == "NiRenderer_static::RemoveGeometryData")
            .expect("the port");
        let removal_ends = crate::ports::geometry::worker_removal_ends as *const () as usize;
        assert_eq!(port.after_off_thread.load(Ordering::Relaxed), removal_ends);
        assert_eq!(port.after.load(Ordering::Relaxed), 0);
    }

    /// The hook and the call of the drain's arm, one by one. The arm in `drain` itself has no
    /// test: `drain` applies every kind of event, and the others need the tables of the game.
    #[test]
    fn a_freed_address_no_longer_counts_as_unreadable() {
        let data = 0x7E57_0082;
        mark_unreadable(data);
        assert!(is_unreadable(data));
        unsafe { on_texture_data_dtor(&[data]) };
        // The hook keeps to the queue: the mark stays until the main thread applies the event.
        assert!(is_unreadable(data));
        let mut queue = QUEUE.lock().unwrap();
        let at = queue
            .events
            .iter()
            .position(|event| matches!(event, Event::TextureFreed(freed) if *freed == data));
        queue.events.remove(at.expect("the hook writes the address into the queue"));
        drop(queue);
        forget_unreadable(data);
        assert!(!is_unreadable(data));
    }

    #[test]
    fn a_new_mark_drops_the_older_free_of_its_address() {
        let (data, other) = (0x7E57_0086, 0x7E57_008A);
        unsafe {
            on_texture_data_dtor(&[data]);
            on_texture_data_dtor(&[other]);
        }
        mark_unreadable(data);
        let waiting = |address: usize| {
            QUEUE
                .lock()
                .unwrap()
                .events
                .iter()
                .any(|event| matches!(event, Event::TextureFreed(freed) if *freed == address))
        };
        assert!(!waiting(data), "the free from before the mark stays in the queue");
        assert!(waiting(other), "the free of another address went too");
        assert!(is_unreadable(data));
        drop_texture_freed(other);
        forget_unreadable(data);
    }

    /// The name of a module static on `line` whose type holds addresses in a map, a set or a list.
    /// A static inside a block (a `thread_local!` of a test) has an indent and is not one.
    fn store_on(line: &str) -> Option<&str> {
        let rest = ["static ", "pub static ", "pub(crate) static "]
            .iter()
            .find_map(|start| line.strip_prefix(start))?;
        let (name, kind) = rest.split_once(':')?;
        let kind = kind.split('=').next().unwrap_or("");
        ["HashMap<usize", "HashSet<usize", "Vec<usize"]
            .iter()
            .any(|pattern| kind.contains(pattern))
            .then_some(name.trim())
    }

    /// The stores of the `.rs` files below `dir`, as "path NAME" with the path from `src`.
    fn stores_below(src: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stores_below(src, &path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let file = path.strip_prefix(src).unwrap().to_string_lossy().replace('\\', "/");
                for line in std::fs::read_to_string(&path).unwrap().lines() {
                    if let Some(name) = store_on(line) {
                        out.push(format!("{file} {name}"));
                    }
                }
            }
        }
    }

    /// The stores without a line, and the lines without a store, of `known` (the text of
    /// `known_stores.txt`).
    fn unknown_stores(found: &[String], known: &str) -> (Vec<String>, Vec<String>) {
        let lines: Vec<&str> = known
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .map(|line| line.split(':').next().unwrap_or("").trim())
            .collect();
        let missing = found
            .iter()
            .filter(|store| !lines.contains(&store.as_str()))
            .cloned()
            .collect();
        let stale = lines
            .iter()
            .filter(|line| !found.iter().any(|store| store == *line))
            .map(|line| line.to_string())
            .collect();
        (missing, stale)
    }

    /// Lifetime rule 3 (W589): each module static that holds addresses has its line in
    /// `known_stores.txt`, "key only" or its proof of life, and each line has its static.
    #[test]
    fn every_store_of_an_address_is_known() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        stores_below(&src, &src, &mut found);
        let known = std::fs::read_to_string(src.join("known_stores.txt")).unwrap();
        let (missing, stale) = unknown_stores(&found, &known);
        assert!(
            missing.is_empty(),
            "stores of addresses without a line in src/known_stores.txt: {missing:?}"
        );
        assert!(
            stale.is_empty(),
            "lines of src/known_stores.txt without their store: {stale:?}"
        );
        // A new store fails until it has its line.
        found.push("new.rs NEW".to_owned());
        assert_eq!(unknown_stores(&found, &known).0, ["new.rs NEW"]);
    }

    #[test]
    fn a_store_is_a_module_static_of_addresses() {
        assert_eq!(
            store_on("static X: MainCell<Vec<usize>> = MainCell::new(Vec::new());"),
            Some("X")
        );
        assert_eq!(
            store_on("pub(crate) static Y: LockCell<Option<HashMap<usize, u32>>> = x;"),
            Some("Y")
        );
        assert_eq!(
            store_on("    static Z: RefCell<Vec<usize>> = x;"),
            None,
            "a static in a block"
        );
        assert_eq!(store_on("static W: MainCell<Vec<u32>> = x;"), None);
        assert_eq!(
            store_on("static V: MainCell<u32> = f::<Vec<usize>>();"),
            None,
            "the value is not the type"
        );
    }

    /// A lookup settles only its own key: a destroy of another key waits for the drain, and a
    /// lookup with nothing waiting changes nothing (W589).
    #[test]
    fn a_lookup_settles_only_its_own_key() {
        let t = crate::main_thread::MainThread::for_test();
        let (waiting, looked_up) = (Key::Shape(0x7E57_0100), Key::Shape(0x7E57_0104));
        queue_for_test(waiting);
        let settled_before = SETTLED[0].load(Ordering::Relaxed);
        unsafe { settle(&t, looked_up) };
        assert!(waits_for_test(waiting));
        assert_eq!(SETTLED[0].load(Ordering::Relaxed), settled_before);
        unsafe { settle(&t, waiting) };
        assert!(!waits_for_test(waiting));
        assert!(SETTLED[0].load(Ordering::Relaxed) > settled_before);
    }

    /// The mark of the old texture is still in the list when its free waits in the queue.
    #[test]
    fn a_second_mark_on_an_address_needs_one_forget() {
        let data = 0x7E57_008E;
        mark_unreadable(data);
        unsafe { on_texture_data_dtor(&[data]) };
        mark_unreadable(data);
        forget_unreadable(data);
        assert!(!is_unreadable(data), "the address stays marked with no texture");
    }
}
