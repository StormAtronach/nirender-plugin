//! The CPU time of each view group of a frame (D82 stage 0): the parts of the main view that
//! draw the table, and each view of its own. Each group has its time, the frames that had it,
//! its worst frame, and the places drawn in it. The sums of a frame close at the start of the
//! next frame, so a flush in the middle of a frame (a pixel read) counts in its own frame.

/// The times of one view group over the run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ViewTotal {
    pub(crate) name: &'static str,
    /// The time in all frames, and in the worst frame, in ticks (`ticks`).
    pub(crate) ticks: u64,
    pub(crate) worst: u64,
    pub(crate) frames: u32,
    /// The number of the group's first frame and of its worst frame (the helper's frame count),
    /// so that the log says whether the worst frame is one after a load.
    pub(crate) first_frame: u32,
    pub(crate) worst_frame: u32,
    /// The places drawn: the displays of a view of its own; 0 for a part of the main view.
    pub(crate) places: u64,
}

/// The view groups of the run, and the sums of the frame in progress.
#[derive(Debug, Default)]
pub(crate) struct ViewTimes {
    /// The number of the frame in progress, and its time and places for each name.
    frame: u32,
    open: Vec<(&'static str, u64, u64)>,
    totals: Vec<ViewTotal>,
}

impl ViewTimes {
    /// The start of the frame `frame`: the sums of the last frame go into the totals.
    pub(crate) fn frame_start(&mut self, frame: u32) {
        fold(&mut self.totals, self.frame, &self.open);
        self.open.clear();
        self.frame = frame;
    }

    /// Adds the time and the places of one item of a view group to the frame in progress.
    pub(crate) fn add(&mut self, name: &'static str, ticks: u64, places: u64) {
        match self.open.iter_mut().find(|(known, ..)| *known == name) {
            Some((_, time, count)) => {
                *time += ticks;
                *count += places;
            }
            None => self.open.push((name, ticks, places)),
        }
    }

    /// The totals with the frame in progress, in the order of each name's first frame.
    pub(crate) fn totals(&self) -> Vec<ViewTotal> {
        let mut totals = self.totals.clone();
        fold(&mut totals, self.frame, &self.open);
        totals
    }
}

/// Adds the sums of the frame `number` to the totals.
fn fold(totals: &mut Vec<ViewTotal>, number: u32, frame: &[(&'static str, u64, u64)]) {
    for &(name, ticks, places) in frame {
        match totals.iter_mut().find(|total| total.name == name) {
            Some(total) => {
                total.ticks += ticks;
                if ticks > total.worst {
                    total.worst = ticks;
                    total.worst_frame = number;
                }
                total.frames += 1;
                total.places += places;
            }
            None => totals.push(ViewTotal {
                name,
                ticks,
                worst: ticks,
                frames: 1,
                first_frame: number,
                worst_frame: number,
                places,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(
        name: &'static str,
        ticks: u64,
        worst: u64,
        frames: u32,
        first_frame: u32,
        worst_frame: u32,
        places: u64,
    ) -> ViewTotal {
        ViewTotal {
            name,
            ticks,
            worst,
            frames,
            first_frame,
            worst_frame,
            places,
        }
    }

    #[test]
    fn the_items_of_a_group_in_one_frame_are_one_frame_of_it() {
        let mut times = ViewTimes::default();
        times.frame_start(7);
        times.add("view 1", 10, 3);
        times.add("main", 5, 0);
        times.add("view 1", 20, 4);
        let totals = times.totals();
        assert_eq!(totals.len(), 2);
        assert_eq!(totals[0], total("view 1", 30, 30, 1, 7, 7, 7));
        assert_eq!(totals[1], total("main", 5, 5, 1, 7, 7, 0));
    }

    #[test]
    fn the_worst_frame_is_the_sum_of_a_frame_and_not_of_an_item() {
        let mut times = ViewTimes::default();
        times.frame_start(1);
        times.add("view 1", 10, 1);
        times.add("view 1", 10, 1);
        times.frame_start(2);
        times.add("view 1", 15, 1);
        times.frame_start(3);
        let totals = times.totals();
        assert_eq!(totals, vec![total("view 1", 35, 20, 2, 1, 1, 3)]);
    }

    /// The worst frame keeps its number; a later frame with the same time does not take it.
    #[test]
    fn the_worst_frame_has_its_number() {
        let mut times = ViewTimes::default();
        for (frame, ticks) in [(10, 4), (11, 9), (12, 3), (13, 9)] {
            times.frame_start(frame);
            times.add("main", ticks, 0);
        }
        times.frame_start(14);
        assert_eq!(times.totals(), vec![total("main", 25, 9, 4, 10, 11, 0)]);
    }

    #[test]
    fn a_frame_without_a_group_does_not_count_for_it() {
        let mut times = ViewTimes::default();
        times.add("view 1", 10, 1);
        times.frame_start(1);
        times.frame_start(2);
        times.add("main", 1, 0);
        let totals = times.totals();
        assert_eq!(totals[0].frames, 1);
        assert_eq!(totals[1].frames, 1);
        assert_eq!(totals[1].first_frame, 2);
    }
}
