//! The count of the engine's effect stage cache (W255), a diagnostic that is off by default
//! (`NIRENDER_EFFECT_CACHE_COUNT=1`).
//!
//! The engine keeps one entry: `NiDX8ConfigurableTexturePipeline::field_B04` holds the
//! `NiDynamicEffectState` of the last call of `setTexturePipeline` (0x6B5870). A call with the
//! same pointer keeps the stages of the call before it. `nextFrame` (0x6B9E00, from
//! `BeginPaint` 0x6AC520) and `selectAndLinkPass` (0x6B9C40, when the pass in use changes)
//! clear the entry. The shape stream makes the stage words of the environment map and the fog
//! map again for each draw (`shape_stream::effect_words`). This count copies the engine's
//! entry over the streamed draws and counts, for each of the two stages, the draws whose words
//! differ from the words that the entry keeps. A difference is put in a class by its cause: an
//! input of the effect (its switch, texture, coordinate mode, world projection or the stages
//! before it), the camera of the pipeline, or neither. The resets are counted by their kind,
//! and the calls of `BeginPaint` in each frame.
//!
//! The retained draws (W620): the engine does not draw a retained shape, so its entry does
//! not see them. The count takes each of them at the place where the engine would draw it:
//! at its display, or in the flush of the main sorted queue (`ports::scene`). Their counts are
//! apart from the streamed ones. A second count compares, for each effect of a retained draw,
//! the table's copy of its words (`instance_stream::watched_effect_words`), from which the
//! helper makes the stage, with the words read at the draw. An effect with a turning chain
//! counts apart: the helper makes its projection. The count only reads: it changes no watch,
//! notes no chain and sends nothing.

use std::sync::OnceLock;

use crate::gen::layouts::{
    NiDX8ConfigurableTexturePipeline as CTP, NiDX8Renderer as R, NiDynamicEffectState as DES, NiTextureEffect as TE,
};
use crate::main_thread::{MainCell, MainThread};
use crate::mem::rd;
use crate::ports::texture_pass_b::{effect_stage_words, effect_stages_before, EFFECT_STAGE_WORDS};

/// The switch of the count.
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| crate::switches::on("NIRENDER_EFFECT_CACHE_COUNT").unwrap_or(false))
}

/// The two stages that the shape stream sends: the environment map, then the fog map.
pub const STAGES: [&str; 2] = ["environment map", "fog map"];

/// The words of the pipeline's camera that the coordinate callbacks read: the 3 x 3 matrix
/// and the place from `worldRightX`, then `field_310`.
pub const CAMERA_WORDS: usize = 13;

/// The inputs of one texture effect that its stage words come from. The default is no effect.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StageInput {
    pub effect: usize,
    pub enabled: u8,
    pub texture: usize,
    pub mode: u32,
    /// The world projection: the 3 x 3 matrix, then the translation.
    pub projection: [u32; 12],
    /// The stages before this one in the pass (the projected lights and shadows).
    pub before: u32,
}

/// Where a draw comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// A draw of the shape stream (`shape_stream::record`).
    Streamed,
    /// A draw of the retained table, at the place of the engine's draw.
    Retained,
}

/// What the count takes from one draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Draw {
    pub source: Source,
    pub effect_state: usize,
    /// The pass that `selectAndLinkPass` takes for the draw.
    pub pass: usize,
    pub camera: [u32; CAMERA_WORDS],
    pub inputs: [StageInput; 2],
    pub words: [[u32; EFFECT_STAGE_WORDS]; 2],
}

/// The counts of one stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StageCount {
    /// The draws with this effect.
    pub draws: u64,
    /// Those of them with the effect state of the kept entry and no reset between.
    pub hits: u64,
    /// The hits whose words differ from the kept words, by an input of the effect.
    pub by_input: u64,
    /// The hits whose words differ, with the same inputs and another camera.
    pub by_camera: u64,
    /// The hits whose words differ with the same inputs and the same camera.
    pub other: u64,
}

/// The second count of one stage over the retained draws: the table's copy of the effect's
/// words against the words at the draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CopyCount {
    /// The retained draws with this effect and a projection in the copy.
    pub draws: u64,
    /// Those of them whose copy differs from the words at the draw.
    pub differ: u64,
    /// The retained draws with this effect and a turning chain in the copy.
    pub turning: u64,
    /// Those of them whose texture or modes differ (the projection is the helper's).
    pub turning_differ: u64,
    /// The retained draws with this effect that the table does not watch.
    pub not_watched: u64,
}

impl CopyCount {
    /// One retained draw: `copy` is the table's copy, `now` the words at the draw.
    pub fn take(
        &mut self,
        copy: Option<[u32; crate::material_stream::EFFECT_SIGNATURE_WORDS]>,
        now: [u32; crate::material_stream::EFFECT_SIGNATURE_WORDS],
    ) {
        let Some(copy) = copy else {
            self.not_watched += 1;
            return;
        };
        if crate::material_stream::effect_signature_turns(&copy) {
            self.turning += 1;
            self.turning_differ += u64::from(copy[..4] != now[..4]);
        } else {
            self.draws += 1;
            self.differ += u64::from(copy != now);
        }
    }
}

/// The count: the copy of the engine's entry, the counts of each stage by the source of the
/// draw, the counts of the table's copy, the resets by their kind and the calls of
/// `BeginPaint` in each frame.
#[derive(Clone, Debug)]
pub struct Count {
    kept: Option<Draw>,
    last_pass: Option<usize>,
    /// The streamed draws.
    pub draws: u64,
    /// The counts of the streamed draws.
    pub stages: [StageCount; 2],
    /// The retained draws.
    pub retained_draws: u64,
    /// Those of them that the count takes at their display, though the engine would draw them
    /// in an accumulator other than the main one, at a place that the count does not know.
    pub retained_order_unknown: u64,
    /// The counts of the retained draws.
    pub retained_stages: [StageCount; 2],
    /// The counts of the table's copy, for each stage.
    pub copies: [CopyCount; 2],
    pub resets_begin_paint: u64,
    pub resets_pass: u64,
    paints: u32,
    pub frames: u64,
    /// The frames with 0, 1, 2, 3, and 4 or more calls of `BeginPaint`.
    pub paint_table: [u64; 5],
    pub paint_most: u32,
}

impl Count {
    pub const fn new() -> Count {
        const NO_STAGE: StageCount = StageCount {
            draws: 0,
            hits: 0,
            by_input: 0,
            by_camera: 0,
            other: 0,
        };
        const NO_COPY: CopyCount = CopyCount {
            draws: 0,
            differ: 0,
            turning: 0,
            turning_differ: 0,
            not_watched: 0,
        };
        Count {
            kept: None,
            last_pass: None,
            draws: 0,
            stages: [NO_STAGE; 2],
            retained_draws: 0,
            retained_order_unknown: 0,
            retained_stages: [NO_STAGE; 2],
            copies: [NO_COPY; 2],
            resets_begin_paint: 0,
            resets_pass: 0,
            paints: 0,
            frames: 0,
            paint_table: [0; 5],
            paint_most: 0,
        }
    }

    /// A call of `BeginPaint`: `nextFrame` clears the entry.
    pub fn begin_paint(&mut self) {
        self.paints += 1;
        self.resets_begin_paint += 1;
        self.kept = None;
    }

    /// The end of a frame: the calls of `BeginPaint` in it go into the table.
    pub fn end_frame(&mut self) {
        self.frames += 1;
        self.paint_table[self.paints.min(4) as usize] += 1;
        self.paint_most = self.paint_most.max(self.paints);
        self.paints = 0;
    }

    /// A draw, streamed or retained, in the order of the engine's draws.
    pub fn draw(&mut self, d: Draw) {
        let stages = match d.source {
            Source::Streamed => {
                self.draws += 1;
                &mut self.stages
            }
            Source::Retained => {
                self.retained_draws += 1;
                &mut self.retained_stages
            }
        };
        // `selectAndLinkPass` clears the entry when the pass in use changes.
        if self.last_pass != Some(d.pass) {
            if self.last_pass.is_some() {
                self.resets_pass += 1;
                self.kept = None;
            }
            self.last_pass = Some(d.pass);
        }
        match self.kept {
            Some(k) if k.effect_state == d.effect_state => {
                for (i, stage) in stages.iter_mut().enumerate() {
                    if d.inputs[i].effect == 0 && k.inputs[i].effect == 0 {
                        continue;
                    }
                    stage.draws += 1;
                    stage.hits += 1;
                    if k.words[i] != d.words[i] {
                        if k.inputs[i] != d.inputs[i] {
                            stage.by_input += 1;
                        } else if k.camera != d.camera {
                            stage.by_camera += 1;
                        } else {
                            stage.other += 1;
                        }
                    }
                }
            }
            _ => {
                for (i, stage) in stages.iter_mut().enumerate() {
                    if d.inputs[i].effect != 0 {
                        stage.draws += 1;
                    }
                }
                self.kept = Some(d);
            }
        }
    }

    /// The line of the log.
    pub fn line(&self) -> String {
        let stages = |counts: &[StageCount; 2]| -> String {
            let parts: Vec<String> = STAGES
                .iter()
                .zip(counts)
                .map(|(name, s)| {
                    format!(
                        "{name}: {} draws, {} hits, differ: {} by an effect input, {} by the camera, {} other",
                        s.draws, s.hits, s.by_input, s.by_camera, s.other
                    )
                })
                .collect();
            parts.join("; ")
        };
        let copies: Vec<String> = STAGES
            .iter()
            .zip(&self.copies)
            .map(|(name, c)| {
                format!(
                    "{name}: {} draws, {} differ; turning chain {} draws, {} differ in the texture or the modes; {} not watched",
                    c.draws, c.differ, c.turning, c.turning_differ, c.not_watched
                )
            })
            .collect();
        let t = &self.paint_table;
        format!(
            "effect cache count (W255, W620): {} streamed draws; {}; {} retained draws ({} at a place not known); {}; \
             the table's copy at the retained draws: {}; resets: {} at BeginPaint, {} at a pass change; \
             BeginPaint in each of {} frames: 0 x {}, 1 x {}, 2 x {}, 3 x {}, 4+ x {}, most {}",
            self.draws,
            stages(&self.stages),
            self.retained_draws,
            self.retained_order_unknown,
            stages(&self.retained_stages),
            copies.join("; "),
            self.resets_begin_paint,
            self.resets_pass,
            self.frames,
            t[0],
            t[1],
            t[2],
            t[3],
            t[4],
            self.paint_most
        )
    }
}

impl Default for Count {
    fn default() -> Count {
        Count::new()
    }
}

static COUNT: MainCell<Count> = MainCell::new(Count::new());

/// The frames between two lines of the log.
pub const LINE_FRAMES: u64 = 600;

/// The inputs of the effect `effect` (0 for none), with `before` stages before it.
///
/// # Safety
///
/// `effect` is 0 or a live `NiTextureEffect`.
unsafe fn input_of(effect: usize, before: u32) -> StageInput {
    if effect == 0 {
        return StageInput::default();
    }
    StageInput {
        effect,
        enabled: rd(effect, TE::enabled),
        texture: rd(effect, TE::sourceTexture),
        mode: rd(effect, TE::coordMode),
        projection: rd(effect, TE::worldProjMat),
        before,
    }
}

/// What the count takes from the draw in progress.
///
/// # Safety
///
/// The main thread. `renderer` points at the live `NiDX8Renderer` of the draw in progress, as
/// for `shape_stream::effect_words`, with its pipeline and its current property state.
unsafe fn read_draw(renderer: usize) -> Draw {
    read_draw_of(
        Source::Streamed,
        renderer,
        rd(renderer, R::currentEffectState),
        rd(renderer, R::currentPropertyState),
    )
}

/// What the count takes from a draw with the effect state and the property state given.
///
/// # Safety
///
/// The main thread. `renderer` points at the live `NiDX8Renderer` with its pipeline and the
/// camera of the scene; `effect_state` and `property_state` are 0 or live.
unsafe fn read_draw_of(source: Source, renderer: usize, effect_state: usize, property_state: usize) -> Draw {
    let pipeline: usize = rd(renderer, R::configurableTexturePipeline);
    let pass = if pipeline != 0 && property_state != 0 {
        crate::ports::texture_pipeline::pass_for_draw(pipeline, property_state)
    } else {
        0
    };
    let mut camera = [0u32; CAMERA_WORDS];
    if pipeline != 0 {
        camera[..12].copy_from_slice(&rd::<[u32; 12]>(pipeline, CTP::worldRightX));
        camera[12] = rd(pipeline, CTP::field_310);
    }
    let mut inputs = [StageInput::default(); 2];
    let mut words = [[0u32; EFFECT_STAGE_WORDS]; 2];
    if effect_state != 0 {
        let effects = [
            rd::<usize>(effect_state, DES::environmentMap),
            rd::<usize>(effect_state, DES::fogMap),
        ];
        if effects != [0, 0] {
            let before = effect_stages_before(renderer, effect_state);
            for i in 0..2 {
                inputs[i] = input_of(effects[i], before[i]);
                words[i] = effect_stage_words(renderer, effects[i], before[i]);
            }
        }
    }
    Draw {
        source,
        effect_state,
        pass,
        camera,
        inputs,
        words,
    }
}

/// Counts a draw of the retained shape `shape` when the switch is on, at the place where the
/// engine would draw it. `order_known` is false when the engine would draw it at a place that
/// the count does not know (an accumulator other than the main one): the count takes it at its
/// display. Only reads: the table's watch and the effect chains stay as they are.
///
/// # Safety
///
/// The main thread, in a scene of the engine, with the tables of `instance_stream` closed.
/// `shape` points at a live `NiGeometry` whose states are 0 or live.
pub unsafe fn note_retained_draw(t: &MainThread, shape: usize, order_known: bool) {
    use crate::gen::layouts::NiGeometry as G;
    if !on() {
        return;
    }
    let renderer = crate::engine::renderer();
    if renderer == 0 {
        return;
    }
    let draw = read_draw_of(
        Source::Retained,
        renderer,
        rd(shape, G::spEffectState),
        rd(shape, G::spPropertyState),
    );
    let mut copies = [None; 2];
    for (i, input) in draw.inputs.iter().enumerate() {
        if input.effect != 0 {
            let copy = crate::instance_stream::watched_effect_words(t, input.effect);
            copies[i] = Some((copy, crate::material_stream::effect_signature_read(renderer, input.effect)));
        }
    }
    let mut count = COUNT.get(t);
    count.draw(draw);
    count.retained_order_unknown += u64::from(!order_known);
    for (i, taken) in copies.into_iter().enumerate() {
        if let Some((copy, now)) = taken {
            count.copies[i].take(copy, now);
        }
    }
}

/// Counts the draw in progress when the switch is on.
///
/// # Safety
///
/// The main thread. `renderer` points at the live `NiDX8Renderer` of the draw in progress, as
/// for `shape_stream::effect_words`, with its pipeline and its current property state.
pub unsafe fn note_draw(t: &MainThread, renderer: usize) {
    if on() {
        COUNT.get(t).draw(read_draw(renderer));
    }
}

/// Counts a call of `BeginPaint` when the switch is on.
pub fn note_begin_paint(t: &MainThread) {
    if on() {
        COUNT.get(t).begin_paint();
    }
}

/// Counts the end of a frame when the switch is on, with a line in the log every
/// [`LINE_FRAMES`] frames.
pub fn note_end_frame(t: &MainThread) {
    if !on() {
        return;
    }
    let mut count = COUNT.get(t);
    count.end_frame();
    if count.frames.is_multiple_of(LINE_FRAMES) {
        crate::log!("{}", count.line());
    }
}

/// The line of the log, or `None` when the switch is off.
pub fn report(t: &MainThread) -> Option<String> {
    on().then(|| COUNT.get(t).line())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(effect_state: usize, pass: usize) -> Draw {
        let mut d = Draw {
            source: Source::Streamed,
            effect_state,
            pass,
            camera: [0; CAMERA_WORDS],
            inputs: [StageInput::default(); 2],
            words: [[0; EFFECT_STAGE_WORDS]; 2],
        };
        d.inputs[0].effect = 0x100;
        d.inputs[0].enabled = 1;
        d.words[0][0] = 7;
        d
    }

    #[test]
    fn a_miss_then_a_hit_with_the_same_words() {
        let mut c = Count::new();
        c.draw(draw(1, 9));
        c.draw(draw(1, 9));
        assert_eq!(
            c.stages[0],
            StageCount {
                draws: 2,
                hits: 1,
                ..StageCount::default()
            }
        );
        assert_eq!(c.stages[1], StageCount::default());
    }

    #[test]
    fn another_effect_state_is_a_miss() {
        let mut c = Count::new();
        c.draw(draw(1, 9));
        let mut d = draw(2, 9);
        d.words[0][0] = 8;
        c.draw(d);
        assert_eq!(
            c.stages[0],
            StageCount {
                draws: 2,
                ..StageCount::default()
            }
        );
    }

    #[test]
    fn begin_paint_clears_the_entry() {
        let mut c = Count::new();
        c.draw(draw(1, 9));
        c.begin_paint();
        let mut d = draw(1, 9);
        d.inputs[0].enabled = 0;
        d.words[0] = [0; EFFECT_STAGE_WORDS];
        c.draw(d);
        assert_eq!(c.stages[0].hits, 0);
        assert_eq!((c.resets_begin_paint, c.resets_pass), (1, 0));
    }

    #[test]
    fn a_pass_change_clears_the_entry_and_the_first_pass_is_no_change() {
        let mut c = Count::new();
        c.draw(draw(1, 9));
        c.draw(draw(1, 10));
        assert_eq!(c.stages[0].hits, 0);
        c.draw(draw(1, 10));
        assert_eq!(c.stages[0].hits, 1);
        assert_eq!((c.resets_begin_paint, c.resets_pass), (0, 1));
    }

    #[test]
    fn each_input_of_the_effect_is_class_a() {
        let changes: [fn(&mut StageInput); 6] = [
            |i| i.enabled = 0,
            |i| i.texture = 5,
            |i| i.mode = 3,
            |i| i.projection[4] = 1,
            |i| i.before = 2,
            |i| i.effect = 0x200,
        ];
        for change in changes {
            let mut c = Count::new();
            c.draw(draw(1, 9));
            let mut d = draw(1, 9);
            change(&mut d.inputs[0]);
            d.words[0][1] = 1;
            c.draw(d);
            assert_eq!(
                c.stages[0],
                StageCount {
                    draws: 2,
                    hits: 1,
                    by_input: 1,
                    ..StageCount::default()
                }
            );
        }
    }

    #[test]
    fn the_camera_alone_is_class_b_and_neither_is_other() {
        let mut c = Count::new();
        c.draw(draw(1, 9));
        let mut d = draw(1, 9);
        d.camera[3] = 1;
        d.words[0][5] = 1;
        c.draw(d);
        let mut e = draw(1, 9);
        e.words[0][6] = 1;
        c.draw(e);
        assert_eq!(
            c.stages[0],
            StageCount {
                draws: 3,
                hits: 2,
                by_camera: 1,
                other: 1,
                ..StageCount::default()
            }
        );
    }

    #[test]
    fn the_stages_count_apart() {
        let mut c = Count::new();
        let mut d = draw(1, 9);
        d.inputs[1].effect = 0x300;
        c.draw(d);
        let mut e = d;
        e.words[1][2] = 4;
        e.inputs[1].texture = 6;
        c.draw(e);
        assert_eq!(
            c.stages[0],
            StageCount {
                draws: 2,
                hits: 1,
                ..StageCount::default()
            }
        );
        assert_eq!(
            c.stages[1],
            StageCount {
                draws: 2,
                hits: 1,
                by_input: 1,
                ..StageCount::default()
            }
        );
        let mut none = draw(2, 9);
        none.inputs[0] = StageInput::default();
        c.draw(none);
        assert_eq!(c.stages[0].draws, 2);
        assert_eq!(c.draws, 3);
    }

    #[test]
    fn the_begin_paint_table() {
        let mut c = Count::new();
        c.end_frame();
        c.begin_paint();
        c.end_frame();
        for _ in 0..2 {
            c.begin_paint();
        }
        c.end_frame();
        for _ in 0..6 {
            c.begin_paint();
        }
        c.end_frame();
        assert_eq!(c.paint_table, [1, 1, 1, 0, 1]);
        assert_eq!((c.frames, c.paint_most, c.resets_begin_paint), (4, 6, 9));
    }

    #[test]
    fn the_retained_draws_count_apart_and_share_the_entry() {
        let mut c = Count::new();
        c.draw(draw(1, 9));
        let mut d = draw(1, 9);
        d.source = Source::Retained;
        d.camera[0] = 1;
        d.words[0][3] = 1;
        c.draw(d);
        assert_eq!(
            c.stages[0],
            StageCount {
                draws: 1,
                ..StageCount::default()
            }
        );
        assert_eq!(
            c.retained_stages[0],
            StageCount {
                draws: 1,
                hits: 1,
                by_camera: 1,
                ..StageCount::default()
            }
        );
        assert_eq!((c.draws, c.retained_draws), (1, 1));
    }

    #[test]
    fn the_copy_count_takes_the_turning_chains_apart() {
        use crate::material_stream::EFFECT_SIGNATURE_WORDS as N;
        let plain = {
            let mut w = [0u32; N];
            w[0] = 0x500;
            w[4] = 1;
            w
        };
        let turning = {
            let mut w = plain;
            w[4] = 0x700;
            w[N - 1] = 0x8000_0000;
            w
        };
        let mut c = CopyCount::default();
        c.take(None, plain);
        c.take(Some(plain), plain);
        let mut moved = plain;
        moved[5] = 2;
        c.take(Some(plain), moved);
        // The projection of a turning chain is the helper's: only the texture and the modes count.
        c.take(Some(turning), plain);
        let mut other_texture = plain;
        other_texture[0] = 0x600;
        c.take(Some(turning), other_texture);
        assert_eq!(
            c,
            CopyCount {
                draws: 2,
                differ: 1,
                turning: 2,
                turning_differ: 1,
                not_watched: 1,
            }
        );
    }

    #[test]
    fn the_line_names_both_stages_and_both_resets() {
        let line = Count::new().line();
        for part in [
            "environment map:",
            "fog map:",
            "at BeginPaint",
            "at a pass change",
            "4+ x 0",
            "0 retained draws (0 at a place not known)",
            "the table's copy at the retained draws",
            "turning chain 0 draws",
        ] {
            assert!(line.contains(part), "{line}");
        }
    }

    #[test]
    fn the_switch_is_off_by_default() {
        if std::env::var("NIRENDER_EFFECT_CACHE_COUNT").is_err() {
            assert!(!on());
            let t = MainThread::for_test();
            assert!(report(&t).is_none());
        }
    }
}
