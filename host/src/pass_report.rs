//! The pass report: which parts of a frame ran, and which did not run and why (W341 step A).
//!
//! A part notes itself where it runs: [`note_other_pass`] when it opens a pass of its own,
//! [`note_ran`] when it works inside the game's pass or makes a copy. A pass inside a part
//! ([`note_inner_pass`]) is counted by name in the census only. Where it does not run,
//! [`note_skipped`] gives a fixed reason ([`Skip`]). The key of a part is its name and the kind
//! of the view it runs in (`in_view`). Where a GPU stamp exists for the part, its name is the
//! stamp's name: one name for the count and for the time.
//!
//! At the end of a frame (`end_frame`) the frame's set of parts is compared with the set of
//! the frame before. A set that differs and was not seen before prints one block; a set seen
//! before is only counted. A part that an earlier frame noted, and that has no note in this
//! frame, in a view of a kind that this frame has, shows "not run, reason unknown": its skip
//! is decided in a place that cannot name it, and that place is on the list of step B. A part
//! that runs only when a record of the stream asks for it ([`on_request`]: a map tile, the
//! depth of the first person) is not a skip in a frame without that record; in a frame where
//! its call came and it noted neither a run nor a skip, it is "reason unknown" too. The
//! totals print at the exit (`exit_lines`).
//!
//! Nothing reads the report to choose what to draw. The calls add to counts in memory; no draw,
//! pipeline or GPU state depends on them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use crate::ViewKind;

/// Why a part did not run: a fixed text.
pub type Skip = &'static str;

/// The names of the view kinds, in the order of [`ViewKind`].
const VIEW_NAMES: [&str; 3] = ["main view", "picture", "other screen"];

/// A part: its name and the kind of its view.
type Part = (&'static str, u8);

/// What a part did in a frame. A part can have more than one in a frame: it ran in one view and
/// not in another of the same kind.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Outcome {
    Ran,
    Skipped(Skip),
    Unknown,
}

type Entry = (Part, Outcome);

/// The state of the report. A struct of its own, so that a test can run it without the
/// process-wide one.
#[derive(Default)]
struct Report {
    /// The kind of the view that the notes are in now.
    view: u8,
    /// The view kinds of this frame, as bits.
    views: u8,
    frame: BTreeSet<Entry>,
    /// Every part that a frame noted.
    seen: BTreeSet<Part>,
    /// The parts that run only when a record asks for them ([`on_request`]).
    requested: BTreeSet<&'static str>,
    /// The parts on request whose call came in this frame.
    asked: BTreeSet<Part>,
    last: Vec<Entry>,
    /// The frames of each distinct set.
    sets: BTreeMap<Vec<Entry>, u64>,
    /// The frames of each entry.
    totals: BTreeMap<Entry, u64>,
    /// The passes of each name since the start.
    passes: BTreeMap<&'static str, u64>,
    frames: u64,
    /// The frames whose set differed from the frame before and was seen before: not printed.
    repeats: u64,
}

impl Report {
    const fn new() -> Report {
        Report {
            view: 0,
            views: 0,
            frame: BTreeSet::new(),
            seen: BTreeSet::new(),
            requested: BTreeSet::new(),
            asked: BTreeSet::new(),
            last: Vec::new(),
            sets: BTreeMap::new(),
            totals: BTreeMap::new(),
            passes: BTreeMap::new(),
            frames: 0,
            repeats: 0,
        }
    }

    fn note(&mut self, name: &'static str, outcome: Outcome) {
        self.frame.insert(((name, self.view), outcome));
    }

    /// The call of the parts `names` on request came: in this frame they run or say why not.
    fn ask(&mut self, names: &[&'static str]) {
        self.requested.extend(names);
        let view = self.view;
        self.asked.extend(names.iter().map(|name| (*name, view)));
    }

    /// The notes from now are in a view of `kind`, and the frame has a view of that kind.
    fn enter(&mut self, kind: ViewKind) {
        self.view = kind as u8;
        self.views |= 1 << self.view;
    }

    /// Ends the frame. Returns the block to print, or nothing in a frame like the one before.
    fn end_frame(&mut self) -> Option<Vec<String>> {
        let noted: BTreeSet<Part> = self.frame.iter().map(|(part, _)| *part).collect();
        // A part on request is expected only when its call came; the others in each view of
        // their kind that the frame has.
        let asked = std::mem::take(&mut self.asked);
        for &part in self.seen.iter().chain(&asked) {
            let expected = if self.requested.contains(part.0) {
                asked.contains(&part)
            } else {
                self.views & (1 << part.1) != 0
            };
            if expected && !noted.contains(&part) {
                self.frame.insert((part, Outcome::Unknown));
            }
        }
        self.seen.extend(noted);
        let set: Vec<Entry> = std::mem::take(&mut self.frame).into_iter().collect();
        self.views = 0;
        self.view = 0;
        self.frames += 1;
        for entry in &set {
            *self.totals.entry(*entry).or_default() += 1;
        }
        let frames_of_set = self.sets.entry(set.clone()).or_default();
        *frames_of_set += 1;
        let first_time = *frames_of_set == 1;
        if set == self.last {
            return None;
        }
        let block = if !first_time {
            self.repeats += 1;
            None
        } else if self.last.is_empty() {
            Some(self.block_whole(&set))
        } else {
            Some(self.block_change(&set))
        };
        self.last = set;
        block
    }

    fn block_whole(&self, set: &[Entry]) -> Vec<String> {
        let mut lines = vec![format!("pass report, frame {}: {} parts", self.frames, set.len())];
        lines.extend(set.iter().map(|entry| format!("  {}", entry_text(entry))));
        lines
    }

    fn block_change(&self, set: &[Entry]) -> Vec<String> {
        let mut lines = vec![format!(
            "pass report, frame {}: a set of parts not seen before ({} distinct sets)",
            self.frames,
            self.sets.len()
        )];
        let gone = self.last.iter().filter(|entry| !set.contains(entry));
        let new = set.iter().filter(|entry| !self.last.contains(entry));
        lines.extend(gone.map(|entry| format!("  - {}", entry_text(entry))));
        lines.extend(new.map(|entry| format!("  + {}", entry_text(entry))));
        lines
    }

    fn exit_lines(&self) -> Vec<String> {
        let unknown = self.totals.keys().filter(|(_, outcome)| *outcome == Outcome::Unknown).count();
        let mut lines = vec![format!(
            "pass report: {} frames, {} distinct sets of parts, {} changes to a set seen before (not printed); {} parts not run with the reason unknown",
            self.frames,
            self.sets.len(),
            self.repeats,
            unknown
        )];
        lines.extend(
            self.totals
                .iter()
                .map(|(entry, frames)| format!("  {frames:7} frames  {}", entry_text(entry))),
        );
        lines
    }
}

fn entry_text(((name, view), outcome): &Entry) -> String {
    let view = VIEW_NAMES[*view as usize];
    match outcome {
        Outcome::Ran => format!("ran: {name} ({view})"),
        Outcome::Skipped(reason) => format!("not run: {name} ({view}): {reason}"),
        Outcome::Unknown => format!("not run, reason unknown: {name} ({view})"),
    }
}

static REPORT: Mutex<Report> = Mutex::new(Report::new());

fn report() -> std::sync::MutexGuard<'static, Report> {
    REPORT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The part `name` opened a pass of its own: it ran, and the census counts the pass by name.
pub fn note_other_pass(name: &'static str) {
    let mut report = report();
    *report.passes.entry(name).or_default() += 1;
    report.note(name, Outcome::Ran);
}

/// A pass `name` opened inside a part that notes itself (a blur of the shadow map, a shader of
/// the post chain): the census counts it by name; the report has no part of that name, so a
/// frame without the pass is not a skip.
pub fn note_inner_pass(name: &'static str) {
    *report().passes.entry(name).or_default() += 1;
}

/// The part `name` ran without a pass of its own: inside the game's pass, or a copy.
pub fn note_ran(name: &'static str) {
    report().note(name, Outcome::Ran);
}

/// The part `name` did not run, for `reason`.
pub fn note_skipped(name: &'static str, reason: Skip) {
    report().note(name, Outcome::Skipped(reason));
}

/// The parts `names` run only when a record of the stream asks for them, and this call is
/// theirs: in this frame they note a run or a skip, or show "reason unknown". A frame without
/// this call is not a skip of theirs.
pub fn on_request(names: &[&'static str]) {
    report().ask(names);
}

/// Notes `result` of a part that answers why it did not run: a run for `Ok`, a skip for `Err`.
/// Returns the value of `Ok`, for the caller's branch.
pub fn note_result<T>(name: &'static str, result: Result<T, Skip>) -> Option<T> {
    match result {
        Ok(value) => {
            note_ran(name);
            Some(value)
        }
        Err(reason) => {
            note_skipped(name, reason);
            None
        }
    }
}

/// The frame has a main view: its world started (the extension's frame call). A frame without
/// one (a loading screen, a menu without the world) shows no part of the main view as unknown,
/// although the passes of the window note themselves in it.
pub(crate) fn main_view() {
    report().enter(ViewKind::Main);
}

/// The notes are in a view of `kind` until the guard drops; then in the main view again.
pub(crate) fn in_view(kind: ViewKind) -> ViewGuard {
    report().enter(kind);
    ViewGuard(())
}

/// The guard of `in_view`.
#[must_use]
pub struct ViewGuard(());

impl Drop for ViewGuard {
    fn drop(&mut self) {
        report().view = ViewKind::Main as u8;
    }
}

/// Ends the frame of the report, and prints its block when its set of parts is new.
pub(crate) fn end_frame() {
    let block = report().end_frame();
    for line in block.into_iter().flatten() {
        println!("{line}");
    }
}

/// The passes of each name since the start, for the census of a mark.
pub(crate) fn passes() -> BTreeMap<&'static str, u64> {
    report().passes.clone()
}

/// The lines of the report at the exit: the frames of each part and outcome.
pub(crate) fn exit_lines() -> Vec<String> {
    report().exit_lines()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of the world: the main view starts, then `notes`.
    fn frame(report: &mut Report, notes: &[(&'static str, Option<Skip>)]) -> Option<Vec<String>> {
        report.enter(ViewKind::Main);
        for (name, skip) in notes {
            report.note(name, skip.map_or(Outcome::Ran, Outcome::Skipped));
        }
        report.end_frame()
    }

    fn has_unknown(report: &Report) -> bool {
        report.last.iter().any(|(_, outcome)| *outcome == Outcome::Unknown)
    }

    #[test]
    fn a_steady_frame_prints_nothing() {
        let mut report = Report::new();
        let first = frame(&mut report, &[("sky", None), ("post", None)]).unwrap();
        assert_eq!(first[0], "pass report, frame 1: 2 parts");
        assert_eq!(frame(&mut report, &[("sky", None), ("post", None)]), None);
    }

    #[test]
    fn a_change_prints_once_for_each_distinct_set() {
        let mut report = Report::new();
        frame(&mut report, &[("sky", None), ("post", None)]);
        let change = frame(&mut report, &[("sky", None), ("post", Some("no environment"))]).unwrap();
        assert!(change.contains(&"  - ran: post (main view)".to_owned()), "{change:?}");
        assert!(
            change.contains(&"  + not run: post (main view): no environment".to_owned()),
            "{change:?}"
        );
        // A menu over the world alternates the two sets: each is printed once, then counted.
        assert!(frame(&mut report, &[("sky", None), ("post", None)]).is_none());
        assert!(frame(&mut report, &[("sky", None), ("post", Some("no environment"))]).is_none());
        assert_eq!(report.repeats, 2);
        assert_eq!(report.sets.len(), 2);
    }

    #[test]
    fn a_part_without_a_note_shows_reason_unknown() {
        let mut report = Report::new();
        frame(&mut report, &[("sky", None), ("water", None)]);
        let change = frame(&mut report, &[("sky", None)]).unwrap();
        assert!(
            change.contains(&"  + not run, reason unknown: water (main view)".to_owned()),
            "{change:?}"
        );
    }

    #[test]
    fn a_frame_without_the_world_has_no_unknown_part() {
        let mut report = Report::new();
        frame(&mut report, &[("sky", None), ("overlay", None)]);
        // A loading screen: the window's passes note themselves, the world does not start.
        report.note("overlay", Outcome::Ran);
        assert!(report.end_frame().is_some());
        assert!(!has_unknown(&report), "{:?}", report.last);
    }

    #[test]
    fn a_part_on_request_is_not_unknown_without_its_record() {
        let mut report = Report::new();
        report.enter(ViewKind::Main);
        report.ask(&["map tile copy"]);
        frame(&mut report, &[("sky", None), ("map tile copy", None)]);
        frame(&mut report, &[("sky", None)]);
        assert!(!has_unknown(&report), "{:?}", report.last);
    }

    #[test]
    fn a_part_on_request_whose_call_noted_nothing_is_unknown() {
        let mut report = Report::new();
        report.enter(ViewKind::Main);
        report.ask(&["map tile copy"]);
        let block = frame(&mut report, &[("sky", None)]).unwrap();
        assert!(
            block.contains(&"  not run, reason unknown: map tile copy (main view)".to_owned()),
            "{block:?}"
        );
    }

    #[test]
    fn the_view_kind_is_part_of_the_key() {
        let mut report = Report::new();
        report.enter(ViewKind::Main);
        report.note("terrain", Outcome::Ran);
        report.enter(ViewKind::Picture);
        report.note("terrain", Outcome::Skipped("no slot"));
        report.view = ViewKind::Main as u8;
        let block = report.end_frame().unwrap();
        assert!(block.contains(&"  ran: terrain (main view)".to_owned()), "{block:?}");
        assert!(
            block.contains(&"  not run: terrain (picture): no slot".to_owned()),
            "{block:?}"
        );
        // The next frame has no picture: the part of the picture is not unknown.
        frame(&mut report, &[("terrain", None)]);
        assert!(!has_unknown(&report), "{:?}", report.last);
    }
}
