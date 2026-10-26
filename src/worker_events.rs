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
//! main thread. Applying an event reads nothing of the object: it can be freed by then. A
//! block freed and used again before the drain makes the plugin forget the new object too: it
//! is found again at its next draw.
//!
//! The events:
//! - `NiGeometry::dtor`: the shape leaves the table.
//! - `NiDX8SourceTextureData::dtor`: counted only. On the main thread the destructor makes
//!   every instance note its handles again (a new generation). On the worker it does not:
//!   the data goes with its texture, so no live instance names it (an inference). A new
//!   generation for each texture of an unloaded cell makes a frame hitch.
//! - `NiDX8LightManager::RemoveLight`: the light's key is free again (`light_stream::forget`).
//! - `NiRenderer_static::RemoveGeometryData`: the geometry data goes from the helper
//!   (`geometry_stream::gone`). Its one caller is the data's destructor. The hook reads the
//!   unique id while the data is alive; on the main thread it sends the record at once.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::mem::rd;

#[derive(Clone, Copy)]
enum Event {
    /// A geometry was destroyed.
    Destroyed(usize),
    /// A source texture's renderer data was destroyed (counted only: see the module).
    TextureFreed,
    /// A light was destroyed: its index.
    LightGone(u32),
    /// A geometry data was destroyed: its address and its unique id.
    GeometryGone(u32, u32),
}

const KINDS: usize = 4;
const KIND_NAMES: [&str; KINDS] = ["shapes destroyed", "textures freed", "lights destroyed", "geometry data destroyed"];

impl Event {
    fn kind(self) -> usize {
        match self {
            Event::Destroyed(_) => 0,
            Event::TextureFreed => 1,
            Event::LightGone(_) => 2,
            Event::GeometryGone(..) => 3,
        }
    }
}

static QUEUE: Mutex<Vec<Event>> = Mutex::new(Vec::new());
static WAITING: AtomicBool = AtomicBool::new(false);
static QUEUED: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];
static APPLIED: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];
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
    if let Ok(mut queue) = QUEUE.lock() {
        queue.push(event);
        QUEUED[event.kind()].fetch_add(1, Ordering::Relaxed);
        WAITING.store(true, Ordering::Release);
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
    core::ffi::CStr::from_ptr(at as *const core::ffi::c_char).to_string_lossy().into_owned()
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
    format!("shape 0x{shape:x} '{}' under {}; stack words in the engine: {}", names[0], names[1..].join(" < "), addresses.join(" "))
}

unsafe fn on_texture_data_dtor(_args: &[usize]) {
    push(Event::TextureFreed);
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
        port.after.store(crate::ports::geometry::worker_removal_ends as *const () as usize, Ordering::Relaxed);
    }
    if missing.is_empty() {
        crate::log!("worker events: {} ports write their events from the engine's cell worker", HOOKS.len());
    } else {
        crate::log!("worker events: no port for {}; their events from the cell worker are lost", missing.join(", "));
    }
}

/// The main thread applies the events written since the last drain, in their order.
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
            core::mem::take(&mut *queue)
        }
        Err(_) => return,
    };
    for &event in &events {
        match event {
            Event::Destroyed(shape) => {
                if crate::instance_stream::forget(t, shape) {
                    DESTROYED_INSTANCES.fetch_add(1, Ordering::Relaxed);
                }
            }
            Event::TextureFreed => {}
            Event::LightGone(key) => crate::light_stream::forget(t, key),
            Event::GeometryGone(data, unique) => crate::geometry_stream::gone(t, data, unique),
        }
        if APPLIED[event.kind()].fetch_add(1, Ordering::Relaxed) == 0 {
            crate::log!("worker events: the first of the {} applied on the main thread", KIND_NAMES[event.kind()]);
        }
    }
    log_notes();
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
        .map(|k| format!("{} {} queued, {} applied", KIND_NAMES[k], QUEUED[k].load(Ordering::Relaxed), APPLIED[k].load(Ordering::Relaxed)))
        .collect();
    format!(
        "worker events (C26, C27): {}; of the shapes destroyed, {} were instances of the table",
        parts.join("; "),
        DESTROYED_INSTANCES.load(Ordering::Relaxed)
    )
}
