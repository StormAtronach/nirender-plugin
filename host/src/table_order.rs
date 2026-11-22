//! The order of the table's unsorted draws: the engine's order, from the shown list (W535, the
//! owner's D511), with the groups of the draws whose order cannot show (D513) and the units of
//! the shapes with more than one pass (D522). The design: the Architect's pages
//! w535-draw-order-root-2026-11-16.md (5.1, 5.2) and w535-reserve-passes-2026-11-16.md (2).
//!
//! The engine draws an unsorted shape at its display. The shown list holds the instances in
//! the order of their first display, so the list's order is the engine's order. An instance is
//! FREE of the order when no draw before or after it can change its pixels but at a tie of the
//! depth ([`Facts::bound_kind`]); an instance with more than one pass is free as a UNIT when its
//! passes have the facts of [`unit_fail`]. Every other instance is BOUND: it draws at its place
//! in the list, its passes in a row. A run of free instances and units that are neighbours in
//! the list is a GROUP; a bound instance closes it. Inside a group the draws go by the number
//! of their pass, then by bucket (in the order of each bucket's first draw of the group), then
//! in the list's order, so that the draws of one bucket join into one call.
//!
//! [`TableOrder::Engine`] is the engine's order with no group: the option of D528, and the
//! measure of the grouping in the merge step's stage "table order".

use std::collections::HashMap;

/// The values of `MGE_XE_TABLE_ORDER`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum TableOrder {
    /// The engine's order with the groups and the units: the default.
    #[default]
    Grouped,
    /// The engine's order with no group: every unsorted draw at its place in the list.
    Engine,
}

impl TableOrder {
    pub(crate) const NAMES: [&'static str; 2] = ["grouped", "engine"];

    /// The order of a value of the switch; unset, or a value that is not known, is the default.
    pub(crate) fn of(name: Option<&str>) -> TableOrder {
        match name {
            Some("engine") => TableOrder::Engine,
            _ => TableOrder::Grouped,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    /// The route line of a run: the order that the helper uses.
    pub(crate) fn route_line(self) -> String {
        format!("table order: {}", self.name())
    }
}

/// The facts of one pass (one slot) that the rule reads, from the pipeline key of its bucket.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PassFacts {
    /// The key's blend word 0 is not 0.
    pub(crate) blend: bool,
    /// The key's depth words: the enable (always 1), the write, the function (8, "always",
    /// with the test off).
    pub(crate) depth: [u32; 3],
    /// The key's stencil word 0 is not 0.
    pub(crate) stencil: bool,
    /// The key's colour write: red 1, green 2, blue 4.
    pub(crate) colours: u32,
}

impl PassFacts {
    pub(crate) fn of(key: &crate::PipelineKey) -> PassFacts {
        PassFacts {
            blend: key.blend[0] != 0,
            depth: key.depth,
            stencil: key.stencil[0] != 0,
            colours: key.color_write,
        }
    }
}

/// The kinds of a bound instance, in the order of the test: the first fact that fails. An
/// instance with more than one pass is "passes" whatever its passes blend (the landscape's
/// shapes with texture layers); "blend" is an instance of one pass whose pipeline blends (a
/// blend that the engine does not sort).
pub(crate) const KINDS: [&str; 6] = [
    "passes",
    "blend",
    "no depth write",
    "test off or function",
    "stencil",
    "colour",
];
/// The index of "blend" in [`KINDS`].
pub(crate) const KIND_BLEND: usize = 1;

/// The lines of the unit's rule that an instance with more than one pass can fail, in the
/// order of the test.
pub(crate) const UNIT_FAILS: [&str; 6] = [
    "first pass not free",
    "first pass's alpha test",
    "a later pass's depth test or function",
    "a later pass's stencil",
    "a later pass's colour write",
    "over 255 passes",
];

/// The facts of an instance: the number of its slots, and what its passes have.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Facts {
    /// The slots of the instance in the table (its passes), in view or not.
    pub(crate) slots: usize,
    /// A pass whose pipeline blends.
    pub(crate) blend: bool,
    /// A pass with no depth write.
    pub(crate) no_write: bool,
    /// A pass whose depth function is not "less" (2) or "less or equal" (4).
    pub(crate) function: bool,
    pub(crate) stencil: bool,
    /// A pass that does not write red, green and blue.
    pub(crate) colours: bool,
}

impl Facts {
    /// The facts of an instance with `slots` slots from its passes, in one pass over them.
    pub(crate) fn of(slots: usize, passes: impl Iterator<Item = PassFacts>) -> Facts {
        let mut facts = Facts {
            slots,
            ..Facts::default()
        };
        for pass in passes {
            facts.blend |= pass.blend;
            facts.no_write |= pass.depth[1] == 0;
            facts.function |= !matches!(pass.depth[2], 2 | 4);
            facts.stencil |= pass.stencil;
            facts.colours |= pass.colours & 7 != 7;
        }
        facts
    }

    /// [`None`] for an instance that is free of the order: one pass, no blend, the depth write
    /// on, the depth test on with "less" or "less or equal", no stencil, and red, green and
    /// blue written. Else the index in [`KINDS`] of the first fact that fails. An alpha test
    /// does not bind: a pixel that fails it writes no colour and no depth.
    pub(crate) fn bound_kind(&self) -> Option<usize> {
        if self.slots != 1 {
            Some(0)
        } else if self.blend {
            Some(KIND_BLEND)
        } else if self.no_write {
            Some(2)
        } else if self.function {
            Some(3)
        } else if self.stencil {
            Some(4)
        } else if self.colours {
            Some(5)
        } else {
            None
        }
    }
}

/// The unit's rule. [`None`] when an instance with more than one pass is a unit; else the
/// index in [`UNIT_FAILS`] of the first line that fails. `passes` are ALL the passes of the
/// instance in the order of its slots; `alpha_test` is the alpha test of the instance's
/// material record.
/// - The first pass is free by the six facts, and has no alpha test: with one it leaves holes
///   in its depth, and a later pass would draw there over what the other order has not drawn
///   yet.
/// - Every later pass has the depth test on with "less or equal" (4): it draws on its own
///   first pass and nowhere behind it. Its stencil is off and it writes red, green and blue.
///   Its blend, its depth write and its alpha test are free.
pub(crate) fn unit_fail(slots: usize, alpha_test: bool, passes: impl Iterator<Item = PassFacts>) -> Option<usize> {
    if slots > 255 {
        return Some(5);
    }
    let mut fail: Option<usize> = None;
    for (number, pass) in passes.enumerate() {
        let line = if number == 0 {
            if Facts::of(1, core::iter::once(pass)).bound_kind().is_some() {
                Some(0)
            } else if alpha_test {
                Some(1)
            } else {
                None
            }
        } else if pass.depth[2] != 4 {
            Some(2)
        } else if pass.stencil {
            Some(3)
        } else if pass.colours & 7 != 7 {
            Some(4)
        } else {
            None
        };
        fail = match (fail, line) {
            (Some(before), Some(now)) => Some(before.min(now)),
            (before, now) => before.or(now),
        };
    }
    fail
}

/// What [`keys`] asks about an instance.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Instance {
    /// [`Facts::bound_kind`].
    pub(crate) kind: Option<usize>,
    /// For an instance with more than one pass: [`unit_fail`]. [`None`] for an instance of one
    /// pass.
    pub(crate) unit: Option<Option<usize>>,
}

impl Instance {
    /// The answer for an instance from all its passes in the order of its slots.
    pub(crate) fn of(alpha_test: bool, passes: impl Iterator<Item = PassFacts> + Clone) -> Instance {
        let slots = passes.clone().count();
        Instance {
            kind: Facts::of(slots, passes.clone()).bound_kind(),
            unit: (slots > 1).then(|| unit_fail(slots, alpha_test, passes)),
        }
    }
}

/// The counts of one frame's order.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameOrder {
    /// The instances with a draw in the order: [`keys`] asks the facts of each one.
    pub(crate) instances: u64,
    pub(crate) groups: u64,
    /// The bound instances by [`KINDS`].
    pub(crate) bound: [u64; 6],
    /// The runs of bound instances of the kind "blend" that are neighbours in the list and
    /// have one bucket: the most calls that those blends can need.
    pub(crate) blend_runs: u64,
    /// The instances with more than one pass that are units.
    pub(crate) units: u64,
    /// The instances with more than one pass that stay bound, by [`UNIT_FAILS`].
    pub(crate) unit_fails: [u64; 6],
}

/// The first number of a group or of a bound draw in the high word of a key.
pub(crate) const FIRST_NUMBER: u64 = 16;
/// The place of the pass's number in the low word of a free entry's key: above the sequence
/// of the bucket's first entry.
const PASS_SHIFT: u32 = 24;

/// The lists of [`keys`], kept from frame to frame.
#[derive(Default)]
pub(crate) struct Buffers {
    /// The id of the instance and the bucket of each entry, in the list's order.
    pub(crate) entries: Vec<(u32, u32)>,
    /// The new key and `within` of each entry.
    pub(crate) keys: Vec<(u64, u32)>,
    /// The sequence of the first entry of a pass number and a bucket in the open group.
    pub(crate) first_of_bucket: HashMap<u64, u32>,
}

/// Writes into `keys` the key and `within` of each entry. `entries` are the entries of the
/// order in the list's order, each with its instance's id and its bucket; the entries of one
/// instance are neighbours. `facts` gives, for an instance, what [`Instance`] holds. `pass_of`
/// gives the pass's number of an entry (from 0): the place of its slot in its instance; it is
/// asked for the entries of a unit only. `first_of_bucket` is a list for the function's own
/// use.
///
/// The sort then compares the key, the bucket, `within` and the slot. So:
/// - a free entry (of a free instance or of a unit) has the number of its group in the high
///   word; in the low word the number of its pass, then the rank of its bucket among the
///   entries of that pass number in the group: the sequence of the first of them. Inside a
///   bucket `within`, the entry's own sequence, keeps the list's order.
/// - a bound entry, and every entry of [`TableOrder::Engine`], has its own sequence in the low
///   word, so the index of its bucket is never compared: the passes of an instance stay in
///   the order of its slots.
pub(crate) fn keys(
    order: TableOrder,
    entries: &[(u32, u32)],
    keys: &mut Vec<(u64, u32)>,
    first_of_bucket: &mut HashMap<u64, u32>,
    mut facts: impl FnMut(u32) -> Instance,
    pass_of: impl Fn(usize) -> u32,
) -> FrameOrder {
    debug_assert!(entries.len() < 1 << PASS_SHIFT);
    keys.clear();
    first_of_bucket.clear();
    let grouped = order == TableOrder::Grouped;
    let mut frame = FrameOrder::default();
    let mut number = FIRST_NUMBER - 1;
    let mut open = false;
    // The bucket of the last instance, when that instance was a bound blend of one entry.
    let mut blend_before: Option<u32> = None;
    let mut at = 0;
    while at < entries.len() {
        let id = entries[at].0;
        let mut end = at + 1;
        while end < entries.len() && entries[end].0 == id {
            end += 1;
        }
        let instance = facts(id);
        frame.instances += 1;
        let is_unit = instance.unit == Some(None);
        match instance.unit {
            Some(None) => frame.units += 1,
            Some(Some(line)) => frame.unit_fails[line] += 1,
            None => {}
        }
        // A unit is free of the order as one piece: it has no bound kind.
        let kind = if is_unit { None } else { instance.kind };
        if let Some(kind) = kind {
            frame.bound[kind] += 1;
        }
        let blend_now = (kind == Some(KIND_BLEND)).then_some(entries[at].1);
        if blend_now.is_some() && blend_now != blend_before {
            frame.blend_runs += 1;
        }
        blend_before = blend_now;
        if grouped {
            match kind {
                None if !open => {
                    number += 1;
                    open = true;
                    first_of_bucket.clear();
                    frame.groups += 1;
                }
                None => {}
                Some(_) => {
                    number += 1;
                    open = false;
                }
            }
        }
        for (index, entry) in entries.iter().enumerate().take(end).skip(at) {
            let sequence = index as u32;
            let key = if !grouped {
                FIRST_NUMBER << 32 | sequence as u64
            } else if kind.is_none() {
                let pass = if is_unit { pass_of(index).min(255) } else { 0 };
                let first = *first_of_bucket
                    .entry((pass as u64) << 32 | entry.1 as u64)
                    .or_insert(sequence);
                number << 32 | (pass as u64) << PASS_SHIFT | first as u64
            } else {
                number << 32 | sequence as u64
            };
            keys.push((key, sequence));
        }
        at = end;
    }
    frame
}

/// The counts of the order over the frames of a session, for the report's line.
#[derive(Default)]
pub struct OrderCounts {
    pub(crate) frames: u64,
    pub(crate) entries: u64,
    pub(crate) instances: u64,
    pub(crate) groups: u64,
    pub(crate) most_groups: u64,
    pub(crate) bound: [u64; 6],
    pub(crate) most_bound: u64,
    pub(crate) blend_runs: u64,
    pub(crate) units: u64,
    pub(crate) unit_fails: [u64; 6],
}

impl OrderCounts {
    pub(crate) fn add(&mut self, entries: u64, frame: &FrameOrder) {
        self.frames += 1;
        self.entries += entries;
        self.instances += frame.instances;
        self.groups += frame.groups;
        self.most_groups = self.most_groups.max(frame.groups);
        for (sum, count) in self.bound.iter_mut().zip(frame.bound) {
            *sum += count;
        }
        self.most_bound = self.most_bound.max(frame.bound.iter().sum());
        self.blend_runs += frame.blend_runs;
        self.units += frame.units;
        for (sum, count) in self.unit_fails.iter_mut().zip(frame.unit_fails) {
            *sum += count;
        }
    }

    /// The line of the report's end: the order's counts of a frame of the main view, as means.
    pub(crate) fn line(&self, order: TableOrder) -> String {
        let frames = self.frames.max(1) as f64;
        let mean = |names: &[&str], counts: [u64; 6]| -> String {
            let parts: Vec<String> = names
                .iter()
                .zip(counts)
                .map(|(name, count)| format!("{name} {:.2}", count as f64 / frames))
                .collect();
            parts.join(", ")
        };
        format!(
            "table order {}: {} frames of the main view; a frame as a mean: {:.1} unsorted draws of {:.1} instances, {:.1} groups (the most {}), {:.1} bound instances (the most {}) by kind: {}; the bound blends stand in {:.2} runs of neighbours with one bucket; {:.2} units; instances with more than one pass that stay bound, by the first line that fails: {}",
            order.name(),
            self.frames,
            self.entries as f64 / frames,
            self.instances as f64 / frames,
            self.groups as f64 / frames,
            self.most_groups,
            self.bound.iter().sum::<u64>() as f64 / frames,
            self.most_bound,
            mean(&KINDS, self.bound),
            self.blend_runs as f64 / frames,
            self.units as f64 / frames,
            mean(&UNIT_FAILS, self.unit_fails)
        )
    }
}

/// The digest of the order of a frame's unsorted table draws of the main view, for a replay:
/// FNV-1a, 64 bits, over the draws in the order in which they go to the GPU. Each draw gives 8
/// bytes: the id of its instance and the number of its pass in the instance, two u32, little
/// endian. Two replays of one capture with the two values of the switch have equal counts;
/// equal hashes say that the grouping moved no draw of the frame (the merge step's stage
/// "table order" reads the line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Digest {
    pub(crate) hash: u64,
    pub(crate) draws: u64,
}

impl Default for Digest {
    fn default() -> Self {
        Digest {
            hash: Digest::OFFSET,
            draws: 0,
        }
    }
}

impl Digest {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    /// One more draw of the frame.
    pub(crate) fn add(&mut self, id: u32, pass: u32) {
        for byte in id.to_le_bytes().into_iter().chain(pass.to_le_bytes()) {
            self.hash = (self.hash ^ byte as u64).wrapping_mul(Digest::PRIME);
        }
        self.draws += 1;
    }

    /// The line of the report's end for the frame `frame` of a replay (the number that
    /// `--frame` and `--frames-at` take).
    pub(crate) fn line(&self, frame: usize) -> String {
        format!("table order digest: frame {frame} {:016x} {} draws", self.hash, self.draws)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FREE: PassFacts = PassFacts {
        blend: false,
        depth: [1, 1, 4],
        stencil: false,
        colours: 7,
    };

    fn kind(slots: usize, passes: &[PassFacts]) -> Option<usize> {
        Facts::of(slots, passes.iter().copied()).bound_kind()
    }

    fn plain_instance(kind: Option<usize>) -> Instance {
        Instance { kind, unit: None }
    }

    /// The entries sorted as `collect_order` sorts them (the key, the bucket, `within`, the
    /// slot), as the list of their slots. The slot of an entry is its index in `entries`.
    /// The instances of `bound` have the kind 2 (no depth write).
    fn sorted(order: TableOrder, entries: &[(u32, u32)], bound: &[u32]) -> (Vec<u32>, FrameOrder) {
        sorted_with(order, entries, &[], |id| plain_instance(bound.contains(&id).then_some(2)))
    }

    /// As [`sorted`], with the answer for each instance and the pass's number of each entry
    /// (`passes`; an entry that the list does not have is of the pass 0).
    fn sorted_with(
        order: TableOrder,
        entries: &[(u32, u32)],
        passes: &[u32],
        instance: impl Fn(u32) -> Instance,
    ) -> (Vec<u32>, FrameOrder) {
        let (mut keys, mut first) = (Vec::new(), HashMap::new());
        let frame = super::keys(order, entries, &mut keys, &mut first, &instance, |index| {
            passes.get(index).copied().unwrap_or(0)
        });
        let mut all: Vec<(u64, u32, u32, u32)> = keys
            .iter()
            .zip(entries)
            .enumerate()
            .map(|(slot, (&(key, within), &(_, bucket)))| (key, bucket, within, slot as u32))
            .collect();
        all.sort_unstable();
        (all.iter().map(|entry| entry.3).collect(), frame)
    }

    /// An instance of two passes that is a unit when `units` has its id, bound as "passes"
    /// with the failing line 2 when `fails` has it; every other instance is free.
    fn layered(units: &'static [u32], fails: &'static [u32]) -> impl Fn(u32) -> Instance {
        move |id| {
            if units.contains(&id) {
                Instance {
                    kind: Some(0),
                    unit: Some(None),
                }
            } else if fails.contains(&id) {
                Instance {
                    kind: Some(0),
                    unit: Some(Some(2)),
                }
            } else {
                plain_instance(None)
            }
        }
    }

    /// The six facts: each one alone binds; an instance with all of them is free, also with
    /// the function "less". The first fact that fails names the kind: the passes before the
    /// blend, so that a landscape shape with a layer that blends is "passes".
    #[test]
    fn an_instance_is_free_only_with_all_six_facts() {
        let less = PassFacts {
            depth: [1, 1, 2],
            ..FREE
        };
        let no_write = PassFacts {
            depth: [1, 0, 4],
            ..FREE
        };
        let test_off = PassFacts {
            depth: [1, 1, 8],
            ..FREE
        };
        let never = PassFacts {
            depth: [1, 1, 1],
            ..FREE
        };
        assert_eq!(kind(1, &[FREE]), None);
        assert_eq!(kind(1, &[less]), None);
        assert_eq!(kind(2, &[FREE, FREE]), Some(0));
        assert_eq!(
            kind(2, &[FREE]),
            Some(0),
            "the slots of the instance count, not the entries in view"
        );
        assert_eq!(kind(1, &[PassFacts { blend: true, ..FREE }]), Some(KIND_BLEND));
        assert_eq!(kind(1, &[no_write]), Some(2));
        assert_eq!(kind(1, &[test_off]), Some(3));
        assert_eq!(kind(1, &[never]), Some(3), "never");
        assert_eq!(kind(1, &[PassFacts { stencil: true, ..FREE }]), Some(4));
        assert_eq!(kind(1, &[PassFacts { colours: 3, ..FREE }]), Some(5));
        assert_eq!(
            kind(1, &[PassFacts { colours: 15, ..FREE }]),
            None,
            "the alpha's bit is no fact"
        );
        // A base pass and a layer that blends: "passes", not "blend".
        assert_eq!(kind(2, &[FREE, PassFacts { blend: true, ..FREE }]), Some(0));
        assert_eq!(KINDS[0], "passes");
        assert_eq!(KINDS[KIND_BLEND], "blend");
        // A blend of one pass with no depth write is "blend": the blend comes first.
        let blend_no_write = PassFacts {
            blend: true,
            depth: [1, 0, 8],
            ..FREE
        };
        assert_eq!(kind(1, &[blend_no_write]), Some(KIND_BLEND));
    }

    /// The stop 1-29 of the tour in small: free boxes, bound bones, free boxes. The bones
    /// stand between the two groups, and no free draw crosses them. The null: with one group
    /// for all free draws (the order before W535) the later box would come before the bones.
    #[test]
    fn a_bound_instance_stands_between_two_groups() {
        // ids 1, 2 free in bucket 5; ids 3, 4 bound in bucket 1; id 6 free in bucket 5.
        let entries = [(1, 5), (2, 5), (3, 1), (4, 1), (6, 5)];
        let (order, frame) = sorted(TableOrder::Grouped, &entries, &[3, 4]);
        assert_eq!(order, [0, 1, 2, 3, 4]);
        assert_eq!((frame.groups, frame.bound[2]), (2, 2));
        // The null: the same entries with all free draws in one group, by bucket, put the
        // free entry 4 before the bound entries 2 and 3.
        let mut one_group: Vec<(u64, u32, u32, u32)> = entries
            .iter()
            .enumerate()
            .map(|(slot, &(id, bucket))| (if [3, 4].contains(&id) { 2 } else { 0 }, bucket, slot as u32, slot as u32))
            .collect();
        one_group.sort_unstable();
        assert_eq!(one_group.iter().map(|entry| entry.3).collect::<Vec<_>>(), [0, 1, 4, 2, 3]);
    }

    /// Inside a group the buckets go in the order of each bucket's first instance of the
    /// group, not of the buckets' indexes, and the instances of a bucket in the list's order.
    #[test]
    fn a_group_orders_its_buckets_by_their_first_instance() {
        // Bucket 9 comes first in the list, bucket 3 second; they mix.
        let entries = [(1, 9), (2, 3), (3, 9), (4, 3), (5, 9)];
        let (order, frame) = sorted(TableOrder::Grouped, &entries, &[]);
        assert_eq!(
            order,
            [0, 2, 4, 1, 3],
            "bucket 9 (first in the list), then bucket 3; each in the list's order"
        );
        assert_eq!(frame.groups, 1);
        // The same buckets after a bound draw are a new group with its own ranks.
        let entries = [(1, 9), (2, 3), (7, 0), (3, 3), (4, 9)];
        let (order, frame) = sorted(TableOrder::Grouped, &entries, &[7]);
        assert_eq!(order, [0, 1, 2, 3, 4]);
        assert_eq!(frame.groups, 2);
    }

    /// The passes of a bound instance stay in the order of its slots, also when the second
    /// pass has the bucket of the lower index; two bound neighbours draw in the list's order.
    #[test]
    fn the_passes_of_a_bound_instance_keep_the_order_of_its_slots() {
        // Instance 1: two passes, buckets 8 then 2. Instance 2: bound, bucket 0.
        let entries = [(1, 8), (1, 2), (2, 0), (3, 4)];
        let (order, frame) = sorted(TableOrder::Grouped, &entries, &[1, 2]);
        assert_eq!(order, [0, 1, 2, 3]);
        assert_eq!((frame.groups, frame.bound[2]), (1, 2));
        assert_eq!(frame.instances, 3, "the four entries are of three instances");
    }

    /// The report's line gives the means of a frame: the draws, the instances that `keys`
    /// asked, the groups and the bound instances.
    #[test]
    fn the_counts_line_names_the_draws_and_the_instances_of_a_frame() {
        let entries = [(1, 8), (1, 2), (2, 0), (3, 4)];
        let (_, frame) = sorted(TableOrder::Grouped, &entries, &[1, 2]);
        let mut counts = OrderCounts::default();
        counts.add(entries.len() as u64, &frame);
        counts.add(entries.len() as u64, &frame);
        let line = counts.line(TableOrder::Grouped);
        assert!(
            line.starts_with(
                "table order grouped: 2 frames of the main view; a frame as a mean: 4.0 unsorted draws of 3.0 instances, 1.0 groups (the most 1), 2.0 bound instances (the most 2) by kind: "
            ),
            "{line}"
        );
    }

    /// The engine's order with no group keeps every entry at its place in the list, whatever
    /// the instances are, and still counts the bound instances and the units.
    #[test]
    fn the_engine_order_keeps_the_list_and_has_no_group() {
        let mixed = [(1, 9), (2, 3), (3, 9), (4, 8), (4, 2)];
        let (order, frame) = sorted(TableOrder::Engine, &mixed, &[4]);
        assert_eq!(order, [0, 1, 2, 3, 4]);
        assert_eq!((frame.groups, frame.bound[2]), (0, 1));
        // The same list grouped moves entry 2 (bucket 9) before entry 1 (bucket 3).
        assert_eq!(sorted(TableOrder::Grouped, &mixed, &[4]).0, [0, 2, 1, 3, 4]);
        // Units change nothing in the engine's order.
        let entries = [(1, 8), (1, 2), (2, 8), (3, 8), (3, 2)];
        let (order, frame) = sorted_with(TableOrder::Engine, &entries, &[0, 1, 0, 0, 1], layered(&[1, 3], &[]));
        assert_eq!(order, [0, 1, 2, 3, 4]);
        assert_eq!((frame.groups, frame.units), (0, 2));
    }

    /// The bound blends that are neighbours in the list and have one bucket count as one run
    /// (a tree's leaves); another bucket, or a draw of another kind between them, starts a
    /// new run.
    #[test]
    fn the_bound_blends_of_one_bucket_in_a_row_are_one_run() {
        // Ids 1 to 3: blends of bucket 7; id 4: a blend of bucket 8; id 5 free; id 6: bucket 7.
        let entries = [(1, 7), (2, 7), (3, 7), (4, 8), (5, 2), (6, 7)];
        let (_, frame) = sorted_with(TableOrder::Grouped, &entries, &[], |id| {
            plain_instance((id != 5).then_some(KIND_BLEND))
        });
        assert_eq!((frame.bound[KIND_BLEND], frame.blend_runs, frame.groups), (5, 3, 1));
    }

    /// The unit's rule, each line with a case that fails it, and the first line that fails
    /// names the count.
    #[test]
    fn a_unit_needs_every_line_of_the_rule() {
        let layer = PassFacts {
            blend: true,
            depth: [1, 0, 4],
            ..FREE
        };
        let layer_that_writes = PassFacts {
            depth: [1, 1, 4],
            ..layer
        };
        let less = PassFacts {
            depth: [1, 1, 2],
            ..FREE
        };
        let no_write = PassFacts {
            depth: [1, 0, 4],
            ..FREE
        };
        assert_eq!(
            unit_fail(2, false, [FREE, layer].into_iter()),
            None,
            "the land: a base pass and a layer that blends"
        );
        assert_eq!(unit_fail(3, false, [FREE, layer, layer_that_writes].into_iter()), None);
        // The first pass: free by the six facts, also with "less"; no alpha test.
        assert_eq!(unit_fail(2, false, [less, layer].into_iter()), None);
        assert_eq!(
            unit_fail(2, false, [PassFacts { blend: true, ..FREE }, layer].into_iter()),
            Some(0)
        );
        assert_eq!(unit_fail(2, false, [no_write, layer].into_iter()), Some(0));
        assert_eq!(unit_fail(2, true, [FREE, layer].into_iter()), Some(1));
        // A later pass: "less or equal" only; "less", "always" (the test off) and "equal" fail.
        for function in [2, 8, 3] {
            let later = PassFacts {
                depth: [1, 0, function],
                ..layer
            };
            assert_eq!(unit_fail(2, false, [FREE, later].into_iter()), Some(2));
        }
        assert_eq!(
            unit_fail(2, false, [FREE, PassFacts { stencil: true, ..layer }].into_iter()),
            Some(3)
        );
        assert_eq!(
            unit_fail(2, false, [FREE, PassFacts { colours: 5, ..layer }].into_iter()),
            Some(4)
        );
        assert_eq!(unit_fail(256, false, [FREE, layer].into_iter()), Some(5));
        // The first line that fails, whatever the order of the passes that fail.
        let bad_depth = PassFacts {
            depth: [1, 0, 2],
            ..layer
        };
        let bad_colour = PassFacts { colours: 5, ..layer };
        assert_eq!(unit_fail(3, false, [FREE, bad_colour, bad_depth].into_iter()), Some(2));
        assert_eq!(unit_fail(3, true, [FREE, bad_colour, bad_depth].into_iter()), Some(1));
        assert_eq!(UNIT_FAILS.len(), FrameOrder::default().unit_fails.len());
    }

    /// The answer for an instance from its passes: one pass has no unit answer; more than one
    /// has the kind "passes" and the unit's answer.
    #[test]
    fn an_instance_gets_its_kind_and_its_unit_answer_from_all_its_passes() {
        let layer = PassFacts {
            blend: true,
            depth: [1, 0, 4],
            ..FREE
        };
        let one = Instance::of(false, [FREE].into_iter());
        assert_eq!((one.kind, one.unit), (None, None));
        let land = Instance::of(false, [FREE, layer].into_iter());
        assert_eq!((land.kind, land.unit), (Some(0), Some(None)));
        let with_alpha_test = Instance::of(true, [FREE, layer].into_iter());
        assert_eq!((with_alpha_test.kind, with_alpha_test.unit), (Some(0), Some(Some(1))));
        let leaf = Instance::of(true, [PassFacts { blend: true, ..FREE }].into_iter());
        assert_eq!((leaf.kind, leaf.unit), (Some(KIND_BLEND), None));
    }

    /// In a group with units all first passes come before all second passes; inside a pass
    /// number the buckets go by their first entry and the list's order; a free instance of
    /// one pass is among the first passes.
    #[test]
    fn a_group_draws_the_first_passes_of_its_units_before_their_later_passes() {
        // Unit 1: buckets 8 then 2. Instance 2: free, bucket 8. Unit 3: buckets 8 then 2.
        let entries = [(1, 8), (1, 2), (2, 8), (3, 8), (3, 2)];
        let passes = [0, 1, 0, 0, 1];
        let (order, frame) = sorted_with(TableOrder::Grouped, &entries, &passes, layered(&[1, 3], &[]));
        assert_eq!(order, [0, 2, 3, 1, 4]);
        assert_eq!((frame.groups, frame.units, frame.bound[0]), (1, 2, 0));
    }

    /// A unit does not cross a bound draw, and an instance with more than one pass that
    /// fails a line stays bound and is counted by that line.
    #[test]
    fn a_unit_does_not_cross_a_bound_draw() {
        // Unit 1 (buckets 8, 2), the bound instance 7 (two passes, fails the line 2), unit 3.
        let entries = [(1, 8), (1, 2), (7, 8), (7, 2), (3, 8), (3, 2)];
        let passes = [0, 1, 0, 1, 0, 1];
        let (order, frame) = sorted_with(TableOrder::Grouped, &entries, &passes, layered(&[1, 3], &[7]));
        assert_eq!(order, [0, 1, 2, 3, 4, 5]);
        assert_eq!((frame.groups, frame.units, frame.bound[0], frame.unit_fails[2]), (2, 2, 1, 1));
    }

    /// The pass's number is the place of the slot in its instance, not the place of the
    /// entry among the instance's entries in view: a unit whose first pass is not in the
    /// order still draws its second pass after the first passes of the group.
    #[test]
    fn the_pass_number_is_the_slots_place_in_its_instance() {
        // Unit 1 has only its second pass in view (bucket 2); instance 2 is free (bucket 8).
        let entries = [(1, 2), (2, 8)];
        let (order, _) = sorted_with(TableOrder::Grouped, &entries, &[1, 0], layered(&[1], &[]));
        assert_eq!(
            order,
            [1, 0],
            "the second pass of the unit comes after the first pass of the other"
        );
        // Counted as the entry's place in view (0), it would come first.
        let (order, _) = sorted_with(TableOrder::Grouped, &entries, &[0, 0], layered(&[1], &[]));
        assert_eq!(order, [0, 1]);
    }

    /// A second call makes the lists anew, and every key has a number from [`FIRST_NUMBER`].
    #[test]
    fn a_second_call_makes_the_lists_anew() {
        let entries: Vec<(u32, u32)> = (0..40).map(|id| (id, id % 3)).collect();
        let (mut keys, mut first) = (Vec::new(), HashMap::new());
        for order in [TableOrder::Grouped, TableOrder::Engine] {
            super::keys(
                order,
                &entries,
                &mut keys,
                &mut first,
                |id| Instance {
                    kind: (id % 2 == 0).then_some(0),
                    unit: (id % 2 == 0).then_some((id % 4 == 0).then_some(0)),
                },
                |index| index as u32 % 3,
            );
            assert!(keys.iter().all(|&(key, _)| key >> 32 >= FIRST_NUMBER), "{order:?}");
            assert_eq!(keys.len(), entries.len(), "{order:?}");
        }
    }

    /// The switch has two values; unset, or a value that is not known, is "grouped", and the
    /// route line names the order that the helper uses.
    #[test]
    fn the_switch_has_two_values_and_the_route_line_names_the_one_in_use() {
        assert_eq!(TableOrder::of(None), TableOrder::Grouped);
        assert_eq!(TableOrder::of(Some("grouped")), TableOrder::Grouped);
        assert_eq!(TableOrder::of(Some("engine")), TableOrder::Engine);
        assert_eq!(
            TableOrder::of(Some("Engine")),
            TableOrder::Grouped,
            "a value that is not known"
        );
        assert_eq!(TableOrder::default(), TableOrder::Grouped);
        assert_eq!(TableOrder::Grouped.route_line(), "table order: grouped");
        assert_eq!(TableOrder::Engine.route_line(), "table order: engine");
        assert_eq!(TableOrder::of(Some("today")).route_line(), "table order: grouped");
        for (index, name) in TableOrder::NAMES.iter().enumerate() {
            assert_eq!(TableOrder::of(Some(name)).name(), TableOrder::NAMES[index]);
        }
    }

    /// The digest is FNV-1a, 64 bits, over 8 bytes a draw. The fixed vector: no draw gives the
    /// offset basis 0xcbf29ce484222325; the one draw (id 1, pass 0) gives the bytes 01 00 00 00
    /// 00 00 00 00, hashed here step by step with the prime 0x100000001b3.
    #[test]
    fn the_digest_is_fnv_1a_over_the_id_and_the_pass_of_each_draw() {
        let empty = Digest::default();
        assert_eq!((empty.hash, empty.draws), (0xcbf2_9ce4_8422_2325, 0));
        let mut by_hand = 0xcbf2_9ce4_8422_2325u64;
        for byte in [1u8, 0, 0, 0, 0, 0, 0, 0] {
            by_hand = (by_hand ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut one = Digest::default();
        one.add(1, 0);
        assert_eq!((one.hash, one.draws), (by_hand, 1));
        // The id's bytes come first, little endian: (0x0102, 0) is the bytes 02 01 00 ...
        let mut bytes = 0xcbf2_9ce4_8422_2325u64;
        for byte in [2u8, 1, 0, 0, 3, 0, 0, 0] {
            bytes = (bytes ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut two = Digest::default();
        two.add(0x0102, 3);
        assert_eq!(two.hash, bytes);
    }

    /// The same draws in another order give another hash and the same count; the same list
    /// gives the same hash; a pass number is a part of the draw.
    #[test]
    fn the_digest_shows_a_draw_that_moved() {
        let of = |draws: &[(u32, u32)]| {
            let mut digest = Digest::default();
            for &(id, pass) in draws {
                digest.add(id, pass);
            }
            digest
        };
        let list = [(10, 0), (11, 0), (11, 1), (12, 0)];
        let moved = [(10, 0), (11, 0), (12, 0), (11, 1)];
        assert_eq!(of(&list), of(&list));
        assert_ne!(of(&list).hash, of(&moved).hash);
        assert_eq!(of(&list).draws, of(&moved).draws);
        assert_ne!(of(&[(11, 1)]).hash, of(&[(11, 0)]).hash);
    }

    /// The digest's line, as the merge step's stage reads it.
    #[test]
    fn the_digest_line_has_the_frame_the_hash_and_the_count() {
        let digest = Digest {
            hash: 0x00ab_cdef_0123_4567,
            draws: 3120,
        };
        assert_eq!(digest.line(29), "table order digest: frame 29 00abcdef01234567 3120 draws");
        assert_eq!(
            Digest::default().line(0),
            "table order digest: frame 0 cbf29ce484222325 0 draws"
        );
    }
}
