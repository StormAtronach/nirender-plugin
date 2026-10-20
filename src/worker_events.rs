//! The events of the engine's cell worker (plan C26 and C27, 2026-10-18). The engine loads and
//! unloads cells on a second thread (engine facts LD3, LD4): it attaches, detaches, updates,
//! destroys and purges there. A port runs its Rust code on the main thread only: on another
//! thread it calls the engine and nothing else (`shadow::dispatch_inner`), so the plugin's side
//! effects of those calls were lost, and its model of the scene drifted from the engine's. One
//! result was the crash of the manual review (2026-10-18 17:16): the live walk called a
//! virtual function of a shape the worker had destroyed.
//!
//! Here a port's `any_thread` hook writes an event when the call comes from another thread:
//! the call's kind and arguments, and what must be read while the object still exists (the
//! index of a light). It touches no table of the plugin. The main thread drains the queue at
//! fixed points, before it reads the pointers it keeps (the start of each scene, the start of
//! the table walk, the end of the frame), and applies each event as the port does on the main
//! thread. Applying an event reads nothing of the object: it can be freed by then. A block
//! freed and used again before the drain makes the plugin forget or mark the new object too:
//! it is found again at its next update.
//!
//! The events (the census of C27; the other ports seen on the worker do only the engine's own
//! work):
//! - `NiGeometry::dtor`: the shape is forgotten (the table, the live walk).
//! - `NiNode::AttachChild`, `DetachChild`, `DetachChildAt`, `SetChildAt`: the tree mark of the
//!   table walk, which builds the chains below the node again.
//! - `NiDX8Renderer::PurgeEffect`: the light leaves the light set.
//! - `NiDX8SourceTextureData::dtor`: counted only. On the main thread the destructor makes
//!   every instance note its handles again (a new generation, and a full pass of the table
//!   walk), for a purge that gives a live texture new data. On the worker the data goes with
//!   its texture, which a live shape would hold a reference to, so no live instance names it
//!   [I]; a new generation for each texture of an unloaded cell made a frame hitch at each
//!   crossing (the owner, 2026-10-18).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::gen::layouts::NiLight as L;
use crate::mem::rd;

#[derive(Clone, Copy)]
enum Event {
    /// A geometry was destroyed.
    Destroyed(usize),
    /// A node's children changed: the node, and the child attached (0 for a detach).
    Tree(usize, usize),
    /// An effect was purged from the renderer: the effect and, for a light, its index.
    EffectPurged(usize, u32),
    /// A source texture's renderer data was destroyed (counted only: see the module).
    TextureFreed,
}

const KINDS: usize = 4;
const KIND_NAMES: [&str; KINDS] = ["shapes destroyed", "tree changes", "effects purged", "textures freed"];

impl Event {
    fn kind(self) -> usize {
        match self {
            Event::Destroyed(_) => 0,
            Event::Tree(..) => 1,
            Event::EffectPurged(..) => 2,
            Event::TextureFreed => 3,
        }
    }
}

static QUEUE: Mutex<Vec<Event>> = Mutex::new(Vec::new());
static WAITING: AtomicBool = AtomicBool::new(false);
static QUEUED: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];
static APPLIED: [AtomicU64; KINDS] = [const { AtomicU64::new(0) }; KINDS];
/// Of the shapes the worker destroyed: those the table had as instances, and those in the
/// live walk's list, when the main thread applied the event. A count above 0 means the worker
/// frees objects the plugin models, and the window between the free and the drain is real.
static DESTROYED_INSTANCES: AtomicU64 = AtomicU64::new(0);
static DESTROYED_LISTED: AtomicU64 = AtomicU64::new(0);
/// Notes on the first worker-side destructors of shapes that carry the table's mark (the
/// shapes the helper draws): the shape's name, its ancestors' names, and the engine addresses
/// found on the worker's stack (a scan: the engine omits frame pointers, so the frame chain
/// gives nothing). The audit's fact that the worker frees only through
/// `RecordsHandler::unloadCell` was contradicted by 22952 worker-side destructors in one
/// glide with every `unloadCell` deferred, 30 of them table instances (2026-10-18, runs
/// 20261018-202328-cross-rules and 20261018-203207-cross-census).
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

const FLAGS: usize = 0x14;
const PARENT: usize = 0x18;
const NAME: usize = 0x8;

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
    let mut addresses = Vec::new();
    for i in 0..192 {
        let word: usize = rd(esp, i * 4);
        if (0x401000..0x7A0000).contains(&word) && !addresses.contains(&word) {
            addresses.push(word);
        }
    }
    let addresses: Vec<String> = addresses.iter().map(|a| format!("{a:#x}")).collect();
    format!("shape 0x{shape:x} '{}' under {}; stack words in the engine: {}", names[0], names[1..].join(" < "), addresses.join(" "))
}

/// `AttachChild(this, child, firstAvailable)`.
unsafe fn on_attach_child(args: &[usize]) {
    if let [node, child, ..] = *args {
        push(Event::Tree(node, child));
    }
}

/// `DetachChild(this, out, child)` and `DetachChildAt(this, out, index)`.
unsafe fn on_detach(args: &[usize]) {
    if let Some(&node) = args.first() {
        push(Event::Tree(node, 0));
    }
}

/// `SetChildAt(this, out, index, child)`.
unsafe fn on_set_child_at(args: &[usize]) {
    if let [node, _, _, child, ..] = *args {
        push(Event::Tree(node, child));
    }
}

/// `PurgeEffect(this, effect)`: the effect still exists here; its index is read now. For an
/// effect that is not a light the index is never used (it is in no light set).
unsafe fn on_purge_effect(args: &[usize]) {
    if crate::shadow::on_main_thread() {
        return;
    }
    if let [_, effect, ..] = *args {
        if effect != 0 {
            push(Event::EffectPurged(effect, rd(effect, L::index)));
        }
    }
}

unsafe fn on_texture_data_dtor(_args: &[usize]) {
    push(Event::TextureFreed);
}

/// The hooks and the names of their ports.
const HOOKS: [(&str, unsafe fn(&[usize])); 7] = [
    ("NiGeometry::dtor", on_geometry_dtor),
    ("NiNode::AttachChild", on_attach_child),
    ("NiNode::DetachChild", on_detach),
    ("NiNode::DetachChildAt", on_detach),
    ("NiNode::SetChildAt", on_set_child_at),
    ("NiDX8Renderer::PurgeEffect", on_purge_effect),
    ("NiDX8SourceTextureData::dtor", on_texture_data_dtor),
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
                if crate::instance_stream::forget(shape) {
                    DESTROYED_INSTANCES.fetch_add(1, Ordering::Relaxed);
                }
                if crate::live_walk::forget(shape) {
                    DESTROYED_LISTED.fetch_add(1, Ordering::Relaxed);
                }
            }
            Event::Tree(node, child) => crate::instance_stream::note_tree(node, child),
            Event::EffectPurged(effect, index) => crate::light_set::purge_with_index(effect, index),
            Event::TextureFreed => {}
        }
        if APPLIED[event.kind()].fetch_add(1, Ordering::Relaxed) == 0 {
            crate::log!("worker events: the first of the {} applied on the main thread", KIND_NAMES[event.kind()]);
        }
    }
    log_notes();
}

/// Writes the notes on the worker's destructors of marked shapes to the log as they come (at
/// most `NOTES_KEPT`).
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
        "worker events (C26, C27): {}; of the shapes destroyed, {} were instances of the table and {} were in the live walk's list",
        parts.join("; "),
        DESTROYED_INSTANCES.load(Ordering::Relaxed),
        DESTROYED_LISTED.load(Ordering::Relaxed)
    )
}
