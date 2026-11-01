//! A summary of the state of the engine and of the plugin, by part: for the test that a capture
//! leaves the game as it found it (`notes/designs-2026-10/harmless-capture-design.md`, section 5).
//! `NIRENDER_STATE_SUMMARY=1` turns it on.
//!
//! `nirender.stateSummary()` asks for a summary and returns the last one that is done, with its
//! number. The summary is made at the next Present: its walk of the engine's texture list and of
//! the renderer's maps needs the cell worker stalled, and a stall under a Lua call can wait for
//! a worker that waits for the Lua lock. For the same reason a test asks in a paused menu, not
//! during a load: a Lua teleport draws frames of a load under its Lua call [I]. The test takes two
//! summaries with a capture between them; a control takes two with no capture between them. Two
//! calls with no Present between them give the same summary: compare the numbers too.
//!
//! A table that a reset emptied fills again with equal items at the next draws, so the parts'
//! notes count the records sent: only they show a capture's second send. Not covered:
//! `light_noted` of the instance table, frames that move by themselves.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::main_thread::{MainCell, MainThread};

/// The first word of the items of the tables that share a part with another table.
pub const MORPH_ITEM: u32 = 0xFFFF_0001;
pub const GROUPS_ITEM: u32 = 0xFFFF_0002;
pub const BY_TEXTURE_ITEM: u32 = 0xFFFF_0003;

/// One part: the number of its items and a hash of them that does not depend on their order
/// (the tables are hash maps). The note is for the reader: the counts that say what differs.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Part {
    pub count: u32,
    pub hash: u64,
    pub note: String,
}

impl Part {
    /// Adds an item, by its words.
    pub fn add(&mut self, words: &[u32]) {
        self.count += 1;
        let mut item: u64 = 0xCBF2_9CE4_8422_2325;
        for &word in words {
            item = (item ^ word as u64).wrapping_mul(0x0000_0100_0000_01B3);
        }
        self.hash = self.hash.wrapping_add(item);
    }

    fn line(&self, name: &str) -> String {
        let note = if self.note.is_empty() { String::new() } else { format!(", {}", self.note) };
        format!("{name}: {} items, hash {:016x}{note}\n", self.count, self.hash)
    }
}

static ASKED: AtomicBool = AtomicBool::new(false);
/// The last summary that is done, and its number.
static LAST: MainCell<(Option<String>, u32)> = MainCell::new((None, 0));

/// Asks for a summary at the next Present, and gives the last one that is done. [`None`] without
/// the switch, or before the first one is done.
pub fn ask(t: &MainThread) -> Option<(String, u32)> {
    if crate::switches::on("NIRENDER_STATE_SUMMARY") != Some(true) {
        return None;
    }
    ASKED.store(true, Ordering::Release);
    let last = LAST.get(t);
    last.0.clone().map(|text| (text, last.1))
}

/// Makes a summary that was asked for. Called by `link_client::after_present`, as the purge of a
/// capture: on the main thread, without the device lock.
///
/// # Safety
///
/// The plugin runs in Morrowind.exe, and the caller holds no lock that the cell worker can need.
pub unsafe fn run_pending() {
    if !ASKED.load(Ordering::Acquire) {
        return;
    }
    let Some(t) = MainThread::here() else { return };
    if crate::sync::device_lock_held_here() {
        return;
    }
    ASKED.store(false, Ordering::Release);
    let ((source_textures, rendered_textures), buffers) = crate::capture::with_worker_stalled(|| (crate::ports::textures::state_part(), crate::ports::geometry::state_part(crate::engine::renderer())));
    let (instances, mode, generation) = crate::instance_stream::state_part(&t);
    let text = [
        instances.line("instances"),
        crate::geometry_stream::state_part(&t).line("geometry"),
        crate::material_stream::state_part(&t).line("materials"),
        crate::light_stream::state_part(&t).line("lights"),
        source_textures.line("source textures"),
        rendered_textures.line("rendered textures"),
        buffers.line("buffers"),
        format!("mode: {mode:?}\ngeneration: {generation}\n"),
    ]
    .concat();
    let mut last = LAST.get(&t);
    last.1 += 1;
    crate::log!("state summary {}:\n{text}", last.1);
    last.0 = Some(text);
}

#[cfg(test)]
mod tests {
    use super::Part;

    fn part(items: &[&[u32]]) -> Part {
        let mut part = Part::default();
        for item in items {
            part.add(item);
        }
        part
    }

    #[test]
    fn the_order_of_the_items_does_not_count() {
        assert_eq!(part(&[&[1, 2], &[3, 4]]), part(&[&[3, 4], &[1, 2]]));
    }

    #[test]
    fn a_change_in_an_item_shows() {
        let before = part(&[&[1, 1, 1], &[2, 1, 1]]);
        assert_ne!(before.hash, part(&[&[1, 0, 1], &[2, 1, 1]]).hash);
        assert_ne!(before.hash, part(&[&[1, 1, 1]]).hash);
        // The same words in other items.
        assert_ne!(part(&[&[1, 2], &[3, 4]]).hash, part(&[&[1, 4], &[3, 2]]).hash);
    }

    #[test]
    fn a_line_names_the_part_and_its_note() {
        let mut p = part(&[&[1]]);
        p.note = "sent 1".to_string();
        let line = p.line("instances");
        assert!(line.starts_with("instances: 1 items, hash "), "{line}");
        assert!(line.ends_with(", sent 1\n"), "{line}");
    }
}
