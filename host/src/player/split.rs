//! The split of a flush in the middle of a frame (D82 stage 2). The items of the back buffer stay
//! queued until the frame's end, because the plugin closes view 0 (its hides) at the swap. A run
//! into a rendered texture after them goes ahead only when that changes nothing it draws or
//! samples, and nothing that a held item draws or samples:
//!
//! - (i) every item of the run is of a kind that reads no target of the frame: a target, a clear,
//!   a viewport, a draw, a screen draw, the table's places (`WalkPlaces`, `ViewPlaces`,
//!   `ViewSorted`) or a view's table (`ViewTable`). The sorted blends can copy the frame's depth (`merge_late_depth`), and any
//!   other kind, a new one too, stays in order;
//! - (ii) no held item reads or writes the texture that the run writes, and no held item writes a
//!   texture that the run reads.
//!
//! A run that fails stays held, in order, and the runs after it test against it too. The
//! operation of the flush (a read or a write of a texture) is one more run: when a held item
//! must come before it, the flush runs every item.
//!
//! The extension's items that get the replay of the world's draws (the world's start and end,
//! a replaced class, a held draw in place) read every texture that a draw or a slot of the table
//! binds. A texture run that goes ahead of the world's start gets that start's rebase and effect
//! turn first, as in the order of the stream.

use crate::*;

/// A viewport item's rectangle and depth range.
pub(crate) type Viewport = ([f32; 4], [f32; 2]);

/// The rendered textures that an item samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Reads {
    /// The item may sample any rendered texture: it is not known which.
    pub(crate) any: bool,
    pub(crate) handles: Vec<u32>,
}

impl Reads {
    pub(crate) fn of(handles: impl IntoIterator<Item = u32>) -> Reads {
        Reads { any: false, handles: handles.into_iter().collect() }
    }

    fn add(&mut self, other: &Reads) {
        self.any |= other.any;
        for &handle in &other.handles {
            if !self.handles.contains(&handle) {
                self.handles.push(handle);
            }
        }
    }

    fn has(&self, handle: u32) -> bool {
        self.any || self.handles.contains(&handle)
    }

    /// One of the textures is in `writes`.
    fn meets(&self, writes: &[u32]) -> bool {
        if self.any {
            return !writes.is_empty();
        }
        self.handles.iter().any(|handle| writes.contains(handle))
    }
}

/// What the split knows of one queued item.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ItemFacts {
    /// `Item::Target`: the target of the items after it.
    pub(crate) target: Option<Option<u32>>,
    /// `Item::Viewport`.
    pub(crate) viewport: Option<Viewport>,
    /// The kind is on the list of (i).
    pub(crate) movable: bool,
    /// `Item::WorldStart`.
    pub(crate) world_start: bool,
    pub(crate) reads: Reads,
}

/// The textures that the replay of the world's draws can sample: those that the draws `draws`
/// and the table's slots (`pinned`) bind, of those that `known` keeps.
pub(crate) fn replay_reads<'a>(draws: impl Iterator<Item = &'a [u32; STAGES]>, pinned: impl Iterator<Item = u32>, known: impl Fn(&u32) -> bool) -> Reads {
    let mut handles: Vec<u32> = draws.flat_map(|handles| handles.iter().copied()).chain(pinned).filter(|handle| known(handle)).collect();
    handles.sort_unstable();
    handles.dedup();
    Reads::of(handles)
}

/// What the operation of the flush does with the rendered textures, after the items that run.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Touch {
    pub(crate) reads: Vec<u32>,
    pub(crate) writes: Vec<u32>,
}

/// One step of a list of items to run: a queued item by its index, or a target or a viewport
/// that puts back the state that the item had in the order of the stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Step {
    Item(usize),
    Target(Option<u32>),
    Viewport(Viewport),
}

/// The plan of a flush: the steps to run now, and the steps that stay queued.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Split {
    pub(crate) now: Vec<Step>,
    pub(crate) later: Vec<Step>,
    /// The texture runs that went ahead of a held item, those held for (i), those held for (ii).
    pub(crate) runs_ahead: u32,
    pub(crate) held_for_kind: u32,
    pub(crate) held_for_textures: u32,
    /// A held item had to come before the operation of the flush: every item runs now.
    pub(crate) held_for_operation: bool,
    /// The held world start that a texture run went ahead of: its rebase and effect turn come
    /// before the items that run now.
    pub(crate) world_start_passed: Option<usize>,
}

/// The target and the viewport of the pass.
type State = (Option<u32>, Option<Viewport>);

fn apply(state: &mut State, facts: &ItemFacts) {
    if let Some(target) = facts.target {
        state.0 = target;
    }
    if let Some(viewport) = facts.viewport {
        state.1 = Some(viewport);
    }
}

/// Puts the item `index` into `steps`, after the target and the viewport that it had in the order
/// of the stream when `state` (the state that the steps leave) has others.
fn put(steps: &mut Vec<Step>, state: &mut State, index: usize, facts: &ItemFacts, at: State) {
    if facts.target.is_none() && state.0 != at.0 {
        steps.push(Step::Target(at.0));
        state.0 = at.0;
    }
    if let (None, Some(viewport)) = (facts.viewport, at.1.filter(|_| state.1 != at.1)) {
        steps.push(Step::Viewport(viewport));
        state.1 = Some(viewport);
    }
    steps.push(Step::Item(index));
    apply(state, facts);
}

/// The plan of a flush of the items `facts`, with the target and the viewport `start` before the
/// first of them, and the operation `touch` after those that run.
pub(crate) fn plan(facts: &[ItemFacts], start: State, touch: &Touch) -> Split {
    let mut split = Split::default();
    // The state before each item, in the order of the stream, and after the last.
    let mut at = Vec::with_capacity(facts.len());
    let mut state = start;
    for item in facts {
        at.push(state);
        apply(&mut state, item);
    }
    let end = state;

    let mut now = vec![true; facts.len()];
    let mut holding = false;
    let mut held_world_start = None;
    let (mut held_reads, mut held_writes) = (Reads::default(), Vec::<u32>::new());
    let mut first = 0;
    while first < facts.len() {
        // A run: a target item and the items up to the next target item, or the first items.
        let target = facts[first].target.unwrap_or(at[first].0);
        let run = first..(first + 1 + facts[first + 1..].iter().take_while(|item| item.target.is_none()).count());
        first = run.end;
        let mut reads = Reads::default();
        for item in &facts[run.clone()] {
            reads.add(&item.reads);
        }
        let ahead = match target {
            // Before the first item of the back buffer every run is in order.
            Some(_) if !holding => continue,
            None => false,
            Some(_) if !facts[run.clone()].iter().all(|item| item.movable) => {
                split.held_for_kind += 1;
                false
            }
            Some(texture) if held_reads.has(texture) || held_writes.contains(&texture) || reads.meets(&held_writes) => {
                split.held_for_textures += 1;
                false
            }
            Some(_) => true,
        };
        if ahead {
            split.runs_ahead += 1;
            split.world_start_passed = split.world_start_passed.or(held_world_start);
            continue;
        }
        holding = true;
        held_world_start = held_world_start.or_else(|| run.clone().find(|&index| facts[index].world_start));
        now[run.clone()].fill(false);
        held_reads.add(&reads);
        if let Some(texture) = target.filter(|texture| !held_writes.contains(texture)) {
            held_writes.push(texture);
        }
    }

    let waits = touch.writes.iter().any(|&texture| held_reads.has(texture) || held_writes.contains(&texture)) || touch.reads.iter().any(|texture| held_writes.contains(texture));
    if waits {
        split.held_for_operation = true;
        now.fill(true);
    }
    if !holding || waits {
        split.now = (0..facts.len()).map(Step::Item).collect();
        (split.runs_ahead, split.held_for_kind, split.held_for_textures) = (0, 0, 0);
        split.world_start_passed = None;
        return split;
    }

    let mut state = start;
    for index in (0..facts.len()).filter(|&index| now[index]) {
        put(&mut split.now, &mut state, index, &facts[index], at[index]);
    }
    for index in (0..facts.len()).filter(|&index| !now[index]) {
        put(&mut split.later, &mut state, index, &facts[index], at[index]);
    }
    // The items that come after the flush continue from the state of the stream.
    if state.0 != end.0 {
        split.later.push(Step::Target(end.0));
    }
    if let Some(viewport) = end.1.filter(|_| state.1 != end.1) {
        split.later.push(Step::Viewport(viewport));
    }
    split
}

impl Player {
    /// Runs the queued items that the split lets go now ([`plan()`]), and gives back those that
    /// stay queued, with the target and the viewport they had in the stream.
    pub(crate) fn run_items_split(&mut self, touch: &Touch) -> Vec<Item> {
        let items = std::mem::take(&mut self.items);
        let (mut table, mut replay) = (None, None);
        let facts: Vec<ItemFacts> = items.iter().map(|item| self.item_facts(item, &items, touch, &mut table, &mut replay)).collect();
        let split = plan(&facts, (self.pass_target, self.viewport), touch);
        // A second split past the same held world start finds it prepared already.
        if let Some(Item::WorldStart(facts)) = split.world_start_passed.filter(|_| !self.world_prepared).map(|index| &items[index]) {
            self.prepare_world(facts);
            self.world_prepared = true;
        }

        let counts = &mut self.renderer.counts;
        counts.flush_splits += 1;
        counts.flush_runs_ahead += split.runs_ahead;
        counts.flush_runs_held_kind += split.held_for_kind;
        counts.flush_runs_held_textures += split.held_for_textures;
        counts.flush_operation_waits += u32::from(split.held_for_operation);

        let mut items: Vec<Option<Item>> = items.into_iter().map(Some).collect();
        let mut take = |steps: &[Step]| -> Vec<Item> {
            steps
                .iter()
                .filter_map(|&step| match step {
                    Step::Item(index) => items[index].take(),
                    Step::Target(target) => Some(Item::Target(target)),
                    Step::Viewport(viewport) => Some(Item::Viewport(viewport)),
                })
                .collect()
        };
        self.items = take(&split.now);
        let held = take(&split.later);
        self.run_items();
        held
    }

    /// The facts of the queued item `item` of `items` for the split. Its reads are of the
    /// rendered textures and the textures that the operation writes. `table` keeps those of the
    /// table's slots and `replay` those of the replay of the world's draws, each made at the
    /// first item that needs it.
    fn item_facts(&self, item: &Item, items: &[Item], touch: &Touch, table: &mut Option<Reads>, replay: &mut Option<Reads>) -> ItemFacts {
        let targets = &self.renderer.targets;
        let known = |handle: &u32| *handle != 0 && (targets.contains_key(handle) || touch.writes.contains(handle));
        let mut table_reads = || table.get_or_insert_with(|| replay_reads(core::iter::empty(), self.retained.pinned_textures(), known)).clone();
        // The world's draws of the frame so far (those queued are among them) and those queued.
        let mut replayed = || {
            replay
                .get_or_insert_with(|| {
                    let queued = items.iter().filter_map(|item| if let Item::Draw(draw) = item { Some(&draw.texture_handles) } else { None });
                    let world = self.world_draws.iter().map(|draw| &draw.texture_handles);
                    replay_reads(queued.chain(world), self.retained.pinned_textures(), known)
                })
                .clone()
        };
        let movable = |reads: Reads| ItemFacts { movable: true, reads, ..Default::default() };
        let fixed = |reads: Reads| ItemFacts { movable: false, reads, ..Default::default() };
        match item {
            Item::Target(target) => ItemFacts { target: Some(*target), movable: true, ..Default::default() },
            Item::Viewport(viewport) => ItemFacts { viewport: Some(*viewport), movable: true, ..Default::default() },
            Item::Clear(..) => movable(Reads::default()),
            Item::Draw(draw) => movable(Reads::of(draw.texture_handles.iter().copied().filter(known))),
            Item::Screen(screen) => movable(Reads::of(Some(screen.texture).filter(known))),
            Item::WalkPlaces(_) | Item::ViewPlaces(..) | Item::ViewSorted(_) | Item::ViewTable(_) => movable(table_reads()),
            // The sorted blends can copy the frame's depth (`merge_late_depth`).
            Item::SortedBlends(_) => fixed(table_reads()),
            // The extension's items with the replay of the world's draws (extension.rs:614-692);
            // a held draw also samples its own maps.
            Item::HeldHere(held, _) => {
                let mut reads = replayed();
                let maps = targets.iter().filter(|(_, target)| held.maps.iter().any(|map| Rc::ptr_eq(&map.texture, &target.view))).map(|(handle, _)| *handle);
                reads.add(&Reads::of(maps));
                fixed(reads)
            }
            Item::WorldStart(_) => ItemFacts { world_start: true, ..fixed(replayed()) },
            // The table's main view in a world begun earlier: the extension holds its shapes
            // there (`retained_hold`), as at the world's start.
            Item::WorldEnd(_) | Item::Replace(..) | Item::MainView(_) => fixed(replayed()),
            // These get no replay (extension.rs:707): they read the frame's targets and the
            // extension's own textures.
            // The parts of the full tier read the extension's own textures, and the replay is not
            // passed to them in a way that reads the frame's targets.
            Item::Post(_) | Item::FirstPersonDepth(_) | Item::TierStart(..) | Item::TierEnd(..) | Item::TierPost(..) => fixed(Reads::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: Viewport = ([0.0, 0.0, 1920.0, 1080.0], [0.0, 1.0]);
    const SMALL: Viewport = ([0.0, 0.0, 512.0, 512.0], [0.0, 1.0]);

    fn target(handle: Option<u32>) -> ItemFacts {
        ItemFacts { target: Some(handle), movable: true, ..Default::default() }
    }

    fn viewport(viewport: Viewport) -> ItemFacts {
        ItemFacts { viewport: Some(viewport), movable: true, ..Default::default() }
    }

    fn draw(reads: &[u32]) -> ItemFacts {
        ItemFacts { movable: true, reads: Reads::of(reads.iter().copied()), ..Default::default() }
    }

    fn fixed(reads: &[u32]) -> ItemFacts {
        ItemFacts { movable: false, ..draw(reads) }
    }

    fn items(steps: &[Step]) -> Vec<usize> {
        steps.iter().filter_map(|step| if let Step::Item(index) = step { Some(*index) } else { None }).collect()
    }

    #[test]
    fn a_texture_run_goes_ahead_of_the_back_buffer_items() {
        // The back buffer, a texture run, the back buffer again, then the flush.
        let facts = [draw(&[]), draw(&[]), target(Some(5)), viewport(SMALL), draw(&[]), target(None), viewport(FULL), draw(&[])];
        let split = plan(&facts, (None, Some(FULL)), &Touch::default());
        assert_eq!(split.now, [Step::Item(2), Step::Item(3), Step::Item(4)]);
        // The held items start in the back buffer with its viewport, and the stream goes on in the
        // state it had.
        assert_eq!(split.later, [Step::Target(None), Step::Viewport(FULL), Step::Item(0), Step::Item(1), Step::Viewport(SMALL), Step::Item(5), Step::Item(6), Step::Item(7)]);
        assert_eq!((split.runs_ahead, split.held_for_kind, split.held_for_textures, split.held_for_operation), (1, 0, 0, false));
    }

    #[test]
    fn the_items_before_the_first_back_buffer_item_run_in_order() {
        let facts = [target(Some(3)), draw(&[]), target(None), draw(&[]), target(Some(4)), draw(&[])];
        let split = plan(&facts, (None, None), &Touch::default());
        assert_eq!(items(&split.now), [0, 1, 4, 5]);
        assert_eq!(items(&split.later), [2, 3]);
        // The texture run 4 is the target of the stream at the flush: the items after the flush
        // go there.
        assert_eq!(split.later.last(), Some(&Step::Target(Some(4))));
    }

    #[test]
    fn a_frame_with_no_back_buffer_item_runs_whole() {
        let facts = [target(Some(3)), draw(&[]), target(Some(4)), draw(&[3])];
        let split = plan(&facts, (Some(9), None), &Touch::default());
        assert_eq!(split.now, (0..4).map(Step::Item).collect::<Vec<_>>());
        assert!(split.later.is_empty());
    }

    #[test]
    fn a_run_with_a_kind_off_the_list_stays_held() {
        let facts = [draw(&[]), target(Some(5)), fixed(&[]), target(Some(6)), draw(&[])];
        let split = plan(&facts, (None, None), &Touch::default());
        assert_eq!(items(&split.now), [3, 4]);
        assert_eq!(items(&split.later), [0, 1, 2]);
        assert_eq!((split.runs_ahead, split.held_for_kind), (1, 1));
    }

    #[test]
    fn a_run_that_writes_a_texture_a_held_item_samples_stays_held() {
        let facts = [draw(&[5]), target(Some(5)), draw(&[])];
        let split = plan(&facts, (None, None), &Touch::default());
        assert!(items(&split.now).is_empty());
        assert_eq!(items(&split.later), [0, 1, 2]);
        assert_eq!(split.held_for_textures, 1);
    }

    #[test]
    fn a_run_that_samples_a_texture_a_held_run_writes_stays_held() {
        // The run into 7 is held for its kind; the run into 8 samples 7 and stays after it.
        let facts = [draw(&[]), target(Some(7)), fixed(&[]), target(Some(8)), draw(&[7]), target(Some(9)), draw(&[])];
        let split = plan(&facts, (None, None), &Touch::default());
        assert_eq!(items(&split.now), [5, 6]);
        assert_eq!(items(&split.later), [0, 1, 2, 3, 4]);
        assert_eq!((split.runs_ahead, split.held_for_kind, split.held_for_textures), (1, 1, 1));
    }

    #[test]
    fn a_held_item_that_may_sample_any_texture_holds_every_run_after_it() {
        let any = ItemFacts { reads: Reads { any: true, handles: Vec::new() }, ..fixed(&[]) };
        let facts = [any, target(Some(5)), draw(&[])];
        let split = plan(&facts, (None, None), &Touch::default());
        assert!(items(&split.now).is_empty());
        assert_eq!(split.held_for_textures, 1);
    }

    #[test]
    fn an_operation_that_a_held_item_must_come_before_runs_every_item() {
        let facts = [draw(&[5]), target(Some(6)), draw(&[])];
        // The game writes the pixels of 5, which a held draw samples.
        let split = plan(&facts, (None, None), &Touch { reads: Vec::new(), writes: vec![5] });
        assert_eq!(split.now, (0..3).map(Step::Item).collect::<Vec<_>>());
        assert!(split.later.is_empty());
        assert!(split.held_for_operation);
        // A read of 6 can go: no held item writes 6.
        let split = plan(&facts, (None, None), &Touch { reads: vec![6], writes: Vec::new() });
        assert_eq!(items(&split.now), [1, 2]);
        assert!(!split.held_for_operation);
    }

    #[test]
    fn the_replay_reads_the_textures_the_draws_and_slots_bind_and_not_the_others() {
        let mut draws = [[0; STAGES]; 2];
        (draws[0][0], draws[0][1], draws[1][0]) = (5, 40, 6);
        // 5, 6 and 7 are rendered textures; 40 is a texture of the game; the map target 8 is
        // bound by nothing.
        let known = |handle: &u32| matches!(handle, 5..=8);
        let reads = replay_reads(draws.iter(), [7, 7, 40].into_iter(), known);
        assert_eq!(reads, Reads::of([5, 6, 7]));
        // A held world end with those reads holds the run into 6, and the run into the map
        // target 8 goes ahead.
        let facts = [fixed(&[5, 6, 7]), target(Some(6)), draw(&[]), target(Some(8)), draw(&[])];
        let split = plan(&facts, (None, None), &Touch { reads: vec![8], writes: Vec::new() });
        assert_eq!(items(&split.now), [3, 4]);
        assert_eq!(split.held_for_textures, 1);
    }

    #[test]
    fn a_run_that_goes_ahead_of_a_held_world_start_names_it() {
        let start = ItemFacts { world_start: true, ..fixed(&[]) };
        let facts = [draw(&[]), start.clone(), target(Some(5)), draw(&[])];
        assert_eq!(plan(&facts, (None, None), &Touch::default()).world_start_passed, Some(1));
        // A run before the world start goes in order: it passes nothing.
        let facts = [target(Some(5)), draw(&[]), target(None), start.clone()];
        assert_eq!(plan(&facts, (None, None), &Touch::default()).world_start_passed, None);
        // A run held after it passes nothing either.
        let facts = [start, target(Some(5)), fixed(&[])];
        assert_eq!(plan(&facts, (None, None), &Touch::default()).world_start_passed, None);
    }

    #[test]
    fn a_read_of_a_texture_that_a_held_run_writes_runs_every_item() {
        let facts = [draw(&[]), target(Some(5)), fixed(&[])];
        let split = plan(&facts, (None, None), &Touch { reads: vec![5], writes: Vec::new() });
        assert!(split.held_for_operation);
        assert!(split.later.is_empty());
    }
}
