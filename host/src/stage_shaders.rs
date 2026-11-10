//! A fragment shader for each stage state of the draws (W293, option E2 of W279). The world
//! shader's `fs` interprets the Direct3D stages of a draw for each pixel: it loads the codes
//! of each stage from the draw's record and switches on them. A stage key holds the codes that
//! `fs` switches on (the stage count, the colour and alpha operations and their arguments, the
//! alpha test function). For each key the helper makes a fragment shader from the world
//! shader's own text, with those codes as constants and the reads of the stages past the count
//! left out. The arithmetic is the same; the switches and the record loads go.
//!
//! The world shader stays as the uber-shader: it draws a key until that key's pipeline is made,
//! when a pipeline fails, past the cap on the keys, and with `MGE_XE_STAGE_SHADERS=0`. It is
//! also the reference of the look test (W293).
//!
//! A live helper makes the pipelines on a thread of its own, so a new key costs a few frames
//! of the uber-shader and not a hitch [I: wgpu's own locks can still hold the frame]. A
//! replay makes them at once, so that its frames are the same from the first. The pipelines
//! of a live session go to a list in the user's folder (`%LOCALAPPDATA%\MGE XE`, never the
//! game folder), one file for each world shader. The next start makes them in its first
//! frames, behind the keys of first use, and the list's keys take at most half of the cap. A
//! list that cannot be read, that a build with another world shader wrote, or that cannot be
//! written, leaves each key to its first use, with a line in the log.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc;

use crate::data::{DrawData, PipelineKey};

/// The most stage keys of a session; a draw of a key past them takes the uber-shader.
pub(crate) const MAX_KEYS: usize = 256;
const STAGES: usize = 6;
/// The first line of the list, then the hash of the world shader's text.
const LIST_HEADER: &str = "numidium stage pipelines 1";
/// The frames between two writes of a changed list.
const SAVE_FRAMES: u32 = 600;
/// The list's pipelines go to the thread while it has fewer jobs than this, so that a key of
/// first use waits behind at most this many of them.
const LIST_BATCH: usize = 4;
/// The stage keys that the list may take; the rest of the cap is for the keys of first use.
const LIST_KEYS: usize = MAX_KEYS / 2;

/// The codes that `fs` switches on for a draw. Only the stages below `count` are kept; the
/// rest are 0, so that two draws that differ only in an unused stage share a key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(crate) struct StageKey {
    count: u8,
    /// The colour operation and its arguments 1, 2 and 0 of each stage.
    color: [[u8; 4]; STAGES],
    /// The alpha operation and its arguments 1 and 2 of each stage.
    alpha: [[u8; 3]; STAGES],
    /// The alpha test function, 0 for none.
    alpha_test: u8,
}

impl StageKey {
    /// The key of a draw's record, or None when a code does not fit (the uber-shader draws it).
    pub(crate) fn of(data: &DrawData) -> Option<StageKey> {
        let count = data.counts[1];
        if count as usize > STAGES {
            return None;
        }
        let small = |value: u32| u8::try_from(value).ok();
        let mut key = StageKey { count: count as u8, alpha_test: small(data.more[2])?, ..Default::default() };
        for stage in 0..count as usize {
            let s = &data.stages[stage];
            for part in 0..4 {
                key.color[stage][part] = small(s.color[part])?;
            }
            for part in 0..3 {
                key.alpha[stage][part] = small(s.alpha[part])?;
            }
        }
        Some(key)
    }

    /// The key as a line of numbers for the list.
    fn words(&self) -> Vec<u32> {
        let mut words = vec![self.count as u32, self.alpha_test as u32];
        for stage in 0..STAGES {
            words.extend(self.color[stage].iter().map(|&v| v as u32));
            words.extend(self.alpha[stage].iter().map(|&v| v as u32));
        }
        words
    }

    fn from_words(words: &[u32]) -> Option<StageKey> {
        if words.len() != 2 + STAGES * 7 {
            return None;
        }
        let small = |value: u32| u8::try_from(value).ok();
        let mut key = StageKey { count: small(words[0])?, alpha_test: small(words[1])?, ..Default::default() };
        if key.count as usize > STAGES {
            return None;
        }
        for stage in 0..STAGES {
            let at = 2 + stage * 7;
            for part in 0..4 {
                key.color[stage][part] = small(words[at + part])?;
            }
            for part in 0..3 {
                key.alpha[stage][part] = small(words[at + 4 + part])?;
            }
        }
        Some(key)
    }
}

/// The world shader's `fs` for one key, made from the world shader's text: the reads of the
/// stages at and past the count give white, the stage loop becomes one step for each stage with
/// its codes as constants, and the alpha test has its function as a constant. None when the
/// text does not have the parts that this rewrite expects (then the uber-shader draws the key).
pub(crate) fn fragment_source(world: &str, key: &StageKey) -> Option<String> {
    let mut source = world.to_string();
    // The reads: each stage's line starts so; stage 0's read is kept (a draw with no stage has no
    // use for it, and the compiler drops it).
    for stage in (key.count as usize).max(1)..STAGES {
        let start = format!("    texels[{stage}] = bump_light(");
        let at = source.find(&start)?;
        if source[at + 1..].contains(&start) {
            return None;
        }
        let end = at + source[at..].find('\n')?;
        source.replace_range(at..end, &format!("    texels[{stage}] = vec4f(1.0);"));
    }

    // The stage loop: its code must be the code that the steps copy.
    let (start, end) = stage_loop_range(&source)?;
    if without_comments(&source[start..end]) != STAGE_LOOP {
        return None;
    }
    let mut steps = String::new();
    for stage in 0..key.count as usize {
        let [op, a1, a2, a0] = key.color[stage];
        let [alpha_op, alpha1, alpha2] = key.alpha[stage];
        steps.push_str(&format!("    {{\n        let texel = texels[{stage}];\n"));
        steps.push_str(&format!(
            "        let color = operation({op}u, argument({a1}u, diffuse, current, texel), argument({a2}u, diffuse, current, texel), argument({a0}u, diffuse, current, texel), diffuse, current, texel);\n"
        ));
        steps.push_str("        var alpha = current;\n");
        // As the loop: a bump stage (22, 23) and the alpha operation 1 keep the current alpha.
        if alpha_op != 1 && op != 22 && op != 23 {
            steps.push_str(&format!(
                "        alpha = operation({alpha_op}u, argument({alpha1}u, diffuse, current, texel), argument({alpha2}u, diffuse, current, texel), vec4f(0.0), diffuse, current, texel);\n"
            ));
        }
        steps.push_str("        current = vec4f(color.rgb, alpha.a);\n");
        steps.push_str("        if (draw_counts_w() & 128u) == 0u {\n            current = clamp(current, vec4f(0.0), vec4f(1.0));\n        }\n    }\n");
    }
    source.replace_range(start..end, &steps);

    // The alpha test.
    let test = "    if draws[record].more.z != 0u && !compare(draws[record].more.z, floor(current.a * 255.0 + 0.5), f32(draws[record].more.w)) {\n        discard;\n    }\n";
    if source.matches(test).count() != 1 {
        return None;
    }
    let fixed_test = match key.alpha_test {
        0 => String::new(),
        function => format!("    if !compare({function}u, floor(current.a * 255.0 + 0.5), f32(draws[record].more.w)) {{\n        discard;\n    }}\n"),
    };
    Some(source.replacen(test, &fixed_test, 1))
}

/// The code of the stage loop of the world shader's `fs` without its comments, as the steps of
/// [`fragment_source`] copy it. An edit of the loop's code no longer matches: the rewrite then
/// refuses the text (the uber-shader draws every key), and the test of each key fails until the
/// steps follow the edit.
const STAGE_LOOP: &str = r#"    for (var i = 0u; i < draws[record].counts.y; i++) {
        let stage = draws[record].stages[i];
        let texel = texels[i];
        let color = operation(stage.color.x, argument(stage.color.y, diffuse, current, texel),
            argument(stage.color.z, diffuse, current, texel), argument(stage.color.w, diffuse, current, texel), diffuse, current, texel);
        var alpha = current;
        if stage.alpha.x != 1u && stage.color.x != 22u && stage.color.x != 23u {
            alpha = operation(stage.alpha.x, argument(stage.alpha.y, diffuse, current, texel),
                argument(stage.alpha.z, diffuse, current, texel), vec4f(0.0), diffuse, current, texel);
        }
        current = vec4f(color.rgb, alpha.a);
        if (draw_counts_w() & 128u) == 0u {
            current = clamp(current, vec4f(0.0), vec4f(1.0));
        }
    }
"#;

/// Where the stage loop of `fs` is in the world shader's text: from its head to its closing
/// line, before the alpha test.
fn stage_loop_range(source: &str) -> Option<(usize, usize)> {
    let head = "    for (var i = 0u; i < draws[record].counts.y; i++) {\n";
    if source.matches(head).count() != 1 {
        return None;
    }
    let start = source.find(head)?;
    let end = start + source[start..].find("\n    }\n\n    if draws[record].more.z != 0u")? + "\n    }\n".len();
    Some((start, end))
}

/// The lines of a text that are not comments, each with its line end.
fn without_comments(text: &str) -> String {
    text.lines().filter(|line| !line.trim_start().starts_with("//")).flat_map(|line| [line, "\n"]).collect()
}

/// The hash of a text, for the list's check that it was written for this world shader.
fn text_hash(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn pipeline_words(key: &PipelineKey) -> Vec<u32> {
    let mut words = vec![key.fvf, key.stride, key.topology];
    words.extend(key.blend);
    words.extend(key.depth);
    words.push(key.cull);
    words.push(key.fill);
    words.extend(key.stencil);
    words.push(key.color_write);
    words.push(key.samples);
    words
}

fn pipeline_from_words(words: &[u32]) -> Option<PipelineKey> {
    if words.len() != 21 {
        return None;
    }
    Some(PipelineKey {
        fvf: words[0],
        stride: words[1],
        topology: words[2],
        blend: words[3..6].try_into().ok()?,
        depth: words[6..9].try_into().ok()?,
        cull: words[9],
        fill: words[10],
        stencil: words[11..18].try_into().ok()?,
        color_write: words[18],
        samples: words[19],
        // Word 20 is the place of the stage key's id, which the list does not keep.
        stages: 0,
    })
}

/// A pipeline to make on the thread: its key, and the fragment module's text when the thread
/// has no module of its stage key yet.
struct Job {
    key: PipelineKey,
    source: Option<String>,
}

struct Done {
    key: PipelineKey,
    pipeline: Option<wgpu::RenderPipeline>,
}

/// The stage keys of the session and their pipelines that are not made yet.
pub(crate) struct StageShaders {
    pub(crate) on: bool,
    /// A replay makes a key's pipeline at its first use, on the frame's thread.
    sync: bool,
    world_source: String,
    world_hash: u64,
    ids: HashMap<StageKey, u32>,
    /// The keys by id; id 0 is the uber-shader.
    keys: Vec<StageKey>,
    /// The fragment modules of a replay, by id; None for a key whose module failed.
    modules: HashMap<u32, Option<wgpu::ShaderModule>>,
    /// The ids whose module the thread has.
    sent_modules: HashSet<u32>,
    /// The ids whose text the rewrite cannot make.
    failed_sources: HashSet<u32>,
    pending: HashSet<PipelineKey>,
    failed: HashSet<PipelineKey>,
    jobs: Option<mpsc::Sender<Job>>,
    done: Option<mpsc::Receiver<Done>>,
    list: Option<PathBuf>,
    /// The pipelines of the list not yet sent to the thread (`stages` 0), with their stage keys.
    queued: VecDeque<(PipelineKey, StageKey)>,
    /// The pipelines made in this session, with their stage keys, for the list.
    made: HashSet<(PipelineKey, StageKey)>,
    changed: bool,
    frames_since_save: u32,
    // Counts for the report.
    pub(crate) made_count: u32,
    pub(crate) from_list: u32,
    /// The list's pipelines left out: their stage key is past the list's share of the cap.
    pub(crate) list_left_out: u32,
    pub(crate) failed_count: u32,
    pub(crate) uber_draws: u64,
    pub(crate) past_cap: u64,
}

impl StageShaders {
    /// The stage shaders of a session, made at their first use (a replay) until [`StageShaders::start`].
    pub(crate) fn new(on: bool, world_source: &str) -> StageShaders {
        StageShaders {
            on,
            sync: true,
            world_source: world_source.to_string(),
            world_hash: text_hash(world_source),
            ids: HashMap::new(),
            keys: vec![StageKey::default()],
            modules: HashMap::new(),
            sent_modules: HashSet::new(),
            failed_sources: HashSet::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            jobs: None,
            done: None,
            list: None,
            queued: VecDeque::new(),
            made: HashSet::new(),
            changed: false,
            frames_since_save: 0,
            made_count: 0,
            from_list: 0,
            list_left_out: 0,
            failed_count: 0,
            uber_draws: 0,
            past_cap: 0,
        }
    }

    /// The id of a draw's stage key, 0 (the uber-shader) when the feature is off, the key does
    /// not fit, or the cap is reached.
    pub(crate) fn id_of(&mut self, data: &DrawData) -> u32 {
        if !self.on {
            return 0;
        }
        let Some(key) = StageKey::of(data) else { return 0 };
        self.intern(key)
    }

    fn intern(&mut self, key: StageKey) -> u32 {
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        if self.keys.len() > MAX_KEYS {
            self.past_cap += 1;
            return 0;
        }
        let id = self.keys.len() as u32;
        self.keys.push(key);
        self.ids.insert(key, id);
        id
    }

    fn source_of(&self, id: u32) -> Option<String> {
        fragment_source(&self.world_source, self.keys.get(id as usize)?)
    }

    /// Starts the thread that makes the pipelines of a live session, and gives it the pipelines
    /// of the list.
    pub(crate) fn start(&mut self, maker: crate::renderer::PipelineMaker) {
        if !self.on {
            return;
        }
        let (jobs, job_receiver) = mpsc::channel::<Job>();
        let (done_sender, done) = mpsc::channel::<Done>();
        let spawned = std::thread::Builder::new().name("stage pipelines".into()).spawn(move || {
            let mut modules: HashMap<u32, Option<wgpu::ShaderModule>> = HashMap::new();
            while let Ok(job) = job_receiver.recv() {
                if let Some(source) = job.source {
                    modules.insert(job.key.stages, maker.module(&source));
                }
                let pipeline = modules.get(&job.key.stages).and_then(|module| module.as_ref()).and_then(|module| maker.make(job.key, module));
                if done_sender.send(Done { key: job.key, pipeline }).is_err() {
                    break;
                }
            }
        });
        if spawned.is_err() {
            println!("stage shaders: the thread of the pipelines cannot start; the uber-shader draws every key");
            self.on = false;
            return;
        }
        self.jobs = Some(jobs);
        self.done = Some(done);
        self.sync = false;
        self.list = list_path(self.world_hash);
        self.load_list();
    }

    fn load_list(&mut self) {
        let Some(path) = self.list.clone() else {
            println!("stage shaders: no folder for the list (LOCALAPPDATA); each key is made at its first use");
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => {
                println!("stage shaders: no list at {}; each key is made at its first use", path.display());
                return;
            }
        };
        let mut lines = text.lines();
        let header = format!("{LIST_HEADER} {:016x}", self.world_hash);
        if lines.next() != Some(header.as_str()) {
            println!("stage shaders: the list at {} is of another world shader or format; it is not used", path.display());
            return;
        }
        let mut dropped = 0;
        for line in lines {
            let words: Option<Vec<u32>> = line.split_whitespace().map(|word| word.parse().ok()).collect();
            let parsed = words.as_deref().filter(|words| words.len() == 21 + 2 + STAGES * 7).and_then(|words| Some((pipeline_from_words(&words[..21])?, StageKey::from_words(&words[21..])?)));
            let Some(entry) = parsed else {
                dropped += 1;
                continue;
            };
            self.queued.push_back(entry);
        }
        if dropped != 0 {
            println!("stage shaders: {dropped} lines of the list at {} not read", path.display());
        }
        println!("stage shaders: {} pipelines of the list at {} are made behind the keys of first use", self.queued.len(), path.display());
    }

    /// Sends the list's pipelines to the thread while it has fewer than [`LIST_BATCH`] jobs. A
    /// pipeline whose stage key would pass [`LIST_KEYS`] is left out, and the next save drops it.
    fn send_queued(&mut self) {
        while self.pending.len() < LIST_BATCH {
            let Some((pipeline, stage)) = self.queued.pop_front() else { return };
            if self.made.contains(&(pipeline, stage)) {
                continue;
            }
            let id = match self.ids.get(&stage) {
                Some(&id) => id,
                None if self.keys.len() <= LIST_KEYS => self.intern(stage),
                None => {
                    self.list_left_out += 1;
                    continue;
                }
            };
            let key = PipelineKey { stages: id, ..pipeline };
            if id != 0 && !self.pending.contains(&key) && self.send(key) {
                self.from_list += 1;
            }
        }
    }

    /// Sends a pipeline to the thread; false when the thread is gone.
    fn send(&mut self, key: PipelineKey) -> bool {
        // The module's text goes with the first pipeline of its stage key; a key whose text the
        // rewrite cannot make is not sent.
        let source = if self.sent_modules.contains(&key.stages) {
            None
        } else if self.failed_sources.contains(&key.stages) {
            return false;
        } else {
            match self.source_of(key.stages) {
                Some(source) => Some(source),
                None => {
                    self.failed_sources.insert(key.stages);
                    println!("stage shaders: the rewrite of stage key {} failed; the uber-shader draws it", key.stages);
                    return false;
                }
            }
        };
        let Some(jobs) = &self.jobs else { return false };
        if jobs.send(Job { key, source }).is_err() {
            return false;
        }
        self.sent_modules.insert(key.stages);
        self.pending.insert(key);
        true
    }

    /// Takes the pipelines that the thread made.
    pub(crate) fn receive(&mut self, pipelines: &mut rustc_hash::FxHashMap<PipelineKey, std::rc::Rc<wgpu::RenderPipeline>>) {
        let Some(done) = &self.done else { return };
        while let Ok(Done { key, pipeline }) = done.try_recv() {
            self.pending.remove(&key);
            match pipeline {
                Some(pipeline) => {
                    pipelines.insert(key, std::rc::Rc::new(pipeline));
                    self.made_count += 1;
                    if let Some(stage) = self.keys.get(key.stages as usize) {
                        if self.made.insert((PipelineKey { stages: 0, ..key }, *stage)) {
                            self.changed = true;
                        }
                    }
                }
                None => {
                    self.failed.insert(key);
                    self.failed_count += 1;
                    println!("stage shaders: the pipeline of stage key {} failed; the uber-shader draws it", key.stages);
                }
            }
        }
        self.send_queued();
    }

    /// The pipeline of a key with a stage key, or None: then the caller draws with the uber-shader.
    pub(crate) fn pipeline(&mut self, key: PipelineKey, maker: &crate::renderer::PipelineMaker, pipelines: &mut rustc_hash::FxHashMap<PipelineKey, std::rc::Rc<wgpu::RenderPipeline>>) -> Option<std::rc::Rc<wgpu::RenderPipeline>> {
        if let Some(pipeline) = pipelines.get(&key) {
            return Some(pipeline.clone());
        }
        if self.failed.contains(&key) {
            self.uber_draws += 1;
            return None;
        }
        if self.sync {
            if !self.modules.contains_key(&key.stages) {
                let module = self.source_of(key.stages).and_then(|source| maker.module(&source));
                self.modules.insert(key.stages, module);
            }
            let pipeline = self.modules.get(&key.stages).and_then(|module| module.as_ref()).and_then(|module| maker.make(key, module));
            let Some(pipeline) = pipeline else {
                self.failed.insert(key);
                self.failed_count += 1;
                println!("stage shaders: the pipeline of stage key {} failed; the uber-shader draws it", key.stages);
                return None;
            };
            let pipeline = std::rc::Rc::new(pipeline);
            pipelines.insert(key, pipeline.clone());
            self.made_count += 1;
            return Some(pipeline);
        }
        if !self.pending.contains(&key) && !self.send(key) {
            self.failed.insert(key);
            self.failed_count += 1;
        }
        self.uber_draws += 1;
        None
    }

    /// After a frame: writes the list when it changed, at most every [`SAVE_FRAMES`] frames.
    pub(crate) fn after_frame(&mut self) {
        self.frames_since_save += 1;
        if self.changed && self.frames_since_save >= SAVE_FRAMES {
            self.save();
        }
    }

    pub(crate) fn save(&mut self) {
        self.frames_since_save = 0;
        if !self.changed {
            return;
        }
        self.changed = false;
        let Some(path) = &self.list else { return };
        let mut text = format!("{LIST_HEADER} {:016x}\n", self.world_hash);
        let entries: HashSet<&(PipelineKey, StageKey)> = self.made.iter().chain(self.queued.iter()).collect();
        for (pipeline, stage) in entries {
            let words: Vec<String> = pipeline_words(pipeline).into_iter().chain([0]).chain(stage.words()).map(|w| w.to_string()).collect();
            text.push_str(&words.join(" "));
            text.push('\n');
        }
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(path, text));
        if let Err(error) = written {
            println!("stage shaders: the list at {} cannot be written ({error}); the next start makes each key at its first use", path.display());
            self.list = None;
        }
    }

    pub(crate) fn report(&self) -> String {
        format!(
            "stage shaders: {} ({}), {} stage keys (cap {MAX_KEYS}, {} draws past it), {} pipelines made ({} sent from the list, {} of the list left out past its share of the cap), {} failed, {} draws on the uber-shader while their pipeline was not made",
            if self.on { "on" } else { "off" },
            if self.sync { "made at the first use" } else { "made on a thread" },
            self.keys.len() - 1,
            self.past_cap,
            self.made_count,
            self.from_list,
            self.list_left_out,
            self.failed_count,
            self.uber_draws
        )
    }
}

impl Drop for StageShaders {
    /// The end of the session writes the list once more, when it changed.
    fn drop(&mut self) {
        self.save();
    }
}

/// The list: `MGE_XE_PIPELINE_LIST` when set, else
/// `%LOCALAPPDATA%\MGE XE\numidium-stage-pipelines-<hash>.txt` with the hash of the world
/// shader, so that two installs or two texture tiers do not write over each other's list.
fn list_path(world_hash: u64) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MGE_XE_PIPELINE_LIST") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("LOCALAPPDATA").map(|folder| PathBuf::from(folder).join("MGE XE").join(format!("numidium-stage-pipelines-{world_hash:016x}.txt")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn world() -> String {
        crate::request::world_shader_source(&crate::extension::extension_shaders(None))
    }

    fn key(count: u8, alpha_test: u8) -> StageKey {
        let mut key = StageKey { count, alpha_test, ..Default::default() };
        // Stage 0: MODULATE of the texture and the diffuse; stage 1: ADD of the texture and
        // the current colour (an environment map after the base map).
        key.color[0] = [4, 2, 0, 1];
        key.alpha[0] = [4, 2, 0];
        if count > 1 {
            key.color[1] = [7, 2, 1, 1];
            key.alpha[1] = [1, 1, 0];
        }
        key
    }

    #[test]
    fn each_key_gives_valid_wgsl() {
        let world = world();
        for (count, test) in [(0, 0), (1, 0), (1, 5), (2, 0), (6, 7)] {
            let source = fragment_source(&world, &key(count, test)).expect("the rewrite finds its parts");
            crate::assert_wgsl("stage shader", &source);
        }
    }

    #[test]
    fn the_rewrite_leaves_no_loop_and_no_unused_read() {
        let source = fragment_source(&world(), &key(2, 0)).unwrap();
        assert!(!source.contains("for (var i = 0u; i < draws[record].counts.y; i++)"));
        assert!(source.contains("    texels[2] = vec4f(1.0);"));
        assert!(source.contains("    texels[1] = bump_light("));
        assert!(!source.contains("draws[record].more.z != 0u && !compare"));
    }

    #[test]
    fn the_stage_loop_is_the_code_that_the_steps_copy() {
        let world = world();
        let (start, end) = stage_loop_range(&world).expect("the loop of fs");
        assert_eq!(without_comments(&world[start..end]), STAGE_LOOP, "fs changed its stage loop: change the steps of fragment_source and STAGE_LOOP with it");
        // A new line in the loop body is refused.
        let planted = world.replacen("        var alpha = current;\n", "        var alpha = current;\n        current = current * 0.5;\n", 1);
        assert!(fragment_source(&planted, &key(2, 0)).is_none());
        // A new comment is not a change of the code.
        let commented = world.replacen("        var alpha = current;\n", "        var alpha = current;\n        // A note.\n", 1);
        assert!(fragment_source(&commented, &key(2, 0)).is_some());
    }

    #[test]
    fn fs_reads_the_texels_only_in_the_reads_and_the_stage_loop() {
        let shader = include_str!("shader.wgsl");
        let fs_start = shader.find("@fragment\nfn fs(").expect("fs");
        let fs = &shader[fs_start..fs_start + shader[fs_start..].find("\n}\n").expect("the end of fs")];
        let (start, end) = stage_loop_range(fs).expect("the loop of fs");
        let outside = format!("{}{}", &fs[..start], &fs[end..]);
        let uses: Vec<&str> = outside.lines().filter(|line| line.contains("texels[")).collect();
        let reads: Vec<String> = (0..STAGES).map(|stage| format!("    texels[{stage}] = ")).collect();
        assert_eq!(uses.len(), STAGES, "fs uses the texels outside the reads and the loop: {uses:?}");
        for (line, read) in uses.iter().zip(&reads) {
            assert!(line.starts_with(read.as_str()), "a use of the texels that is not a read: {line}");
        }
    }

    #[test]
    fn the_tier_c_text_rewrites_too() {
        let world = crate::request::tier_c_shader_source(&world());
        assert!(fragment_source(&world, &key(2, 0)).is_some());
    }

    #[test]
    fn a_key_and_a_pipeline_go_through_the_list() {
        let stage = key(2, 5);
        assert_eq!(StageKey::from_words(&stage.words()), Some(stage));
        let pipeline = PipelineKey { fvf: 0x152, stride: 24, topology: 4, blend: [1, 5, 6], depth: [1, 0, 4], cull: 1, fill: 3, stencil: [0; 7], color_write: 7, samples: 1, stages: 0 };
        let mut words = pipeline_words(&pipeline);
        words.push(0);
        assert_eq!(pipeline_from_words(&words), Some(pipeline));
    }

    #[test]
    fn unused_stages_do_not_split_a_key() {
        let mut a = DrawData::zeroed();
        a.counts[1] = 1;
        a.stages[0].color = [4, 2, 0, 1];
        let mut b = a;
        b.stages[3].color = [7, 2, 1, 0];
        assert_eq!(StageKey::of(&a), StageKey::of(&b));
    }
}
