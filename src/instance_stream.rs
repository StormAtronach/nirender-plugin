//! The retained instances of the plugin. A shape of the safe class gets an `INSTANCE`
//! record once: its handles, its world transform with an exact translation, and the
//! colours of its material. The helper then draws the instance from its table. A change of
//! the transform, the colours or the handles sends the `INSTANCE` record again with the
//! same id. The destructor of the geometry sends `INSTANCE_DELETE`.
//!
//! What the table takes today (`reject_reason`; the rest stays on the engine's path until its
//! work is done, charter 1): a shape of the world scene of a retained class
//! (`class_retained`), with no depth test that depends on an earlier draw and no rejected node
//! kind above it (`REJECTED_KINDS`).
//! A controller on a node above moves the instance in place. A controller on the shape or on
//! one of its properties makes it a live shape: its display runs the controllers' `OnPreDisplay` and
//! notes the geometry and the material each time. A blended shape is retained: the helper
//! sorts the blends of its table back to front. The class of a shape is decided once and
//! kept until the shape is deleted.
//!
//! The modes ([`Mode`]):
//! - `off`: no instance.
//! - `dual`: every record as before, plus the instance records. The shape record is the
//!   twin of an instance draw, and the helper draws one of the two (`MGE_XE_RETAINED`).
//! - `gpu`: the helper draws the instance from its table and culls it with its bound. The
//!   plugin sends no draw, only the changes of the lights (`INSTANCE_LIGHTS`) and of the
//!   visibility (`INSTANCE_STATE`).
//!
//! `NIRENDER_RETAINED` sets the mode at the start (default `gpu`); `nirender.retained(mode)`
//! sets it from Lua.
//!
//! In mode gpu the engine does not display a retained shape. The shape carries
//! [`RETAINED_BIT`], and the ports of its `Display` call `skipped_display`, which does the
//! checks of a draw and returns. The engine's traversal still reaches the shape, so the
//! engine's draws stay the measure of its visibility. `NIRENDER_SKIP=0` keeps the engine's
//! display.
//!
//! The handles of an instance get a check every `CHECK_FRAMES` frames, and at its next draw
//! after a device texture goes away (`textures_changed`, `reset`). The transform and the
//! colours get a check at each draw.
//!
//! An `INSTANCE` record carries two translations: the stored one of the world transform
//! (32-bit floats) and the exact one (`camera_space::exact_translation`, 64-bit). The helper
//! takes the exact one for a record with the flag `RELATIVE`, else the stored one. A parent
//! move smaller than one float step at the shape's position is not seen.
//!
//! The tables are not synchronized: all functions run on the main thread, except
//! `queue_change`.

// The tables are keyed by engine pointers: the pointer hasher, not SipHash (`ptr_hash`).
use crate::ptr_hash::{HashMap, HashSet};

use crate::capture;
use crate::engine::{thiscall1, vcall0};
use crate::gen::layouts::{InterfaceNiTimeController, NiAVObject, NiGeometry, NiProperty, NiPropertyState, NiRTTI};
use crate::gen::layouts::{NiCamera as C, NiDX8LightManager_LightEntry as E, NiDynamicEffectState as DES, NiDynamicEffectState_InternalList as DL, NiLight as L, NiPropertyState as PS, NiTimeController as TC, NiZBufferProperty as ZP};
use crate::link::{CLASS_LANDSCAPE, CLASS_OBJECT, CLASS_SKY_DOME, CLASS_SKY_OTHER, CLASS_UNKNOWN, CLASS_WEATHER_PARTICLES, SCENE_CHARACTER_PREVIEW, SCENE_LOCAL_MAP, SCENE_OTHER, SCENE_RACE_MENU_HEAD, SCENE_WATER, SCENE_WATER_REFLECTION, SCENE_WORLD};
use crate::checked::Open;
use crate::main_thread::{MainCell, MainThread};
use crate::view_slots::{Click, Closed, Opening, ViewKey, ViewTable, VIEW_SLOTS};
use crate::mem::{rd, wr};

const CONTROLLERS: usize = NiAVObject::controllers;
const PARENT: usize = NiAVObject::parentNode;
const WORLD: usize = NiAVObject::worldTransform;
const WORLD_TRANSLATION: usize = NiAVObject::worldTransform_translation;
/// The centre and the radius.
const WORLD_BOUND: usize = NiAVObject::worldBound;
/// `NiBound::WhichSide(plane)`: 0 across the plane, 1 in front, 2 behind (out of the view).
const WHICH_SIDE: usize = 0x6F2660;
/// The flag of an instance record of the mode gpu: the helper draws it from its table.
const BLOCK: u32 = 0x100;
/// `NiAVObject::flags` bit 12: the plugin's mark on a shape the helper draws from its table.
pub const RETAINED_BIT: u16 = 0x1000;
const FLAGS: usize = NiAVObject::flags;
const GEOMETRY_PROPERTY_STATE: usize = NiGeometry::spPropertyState;
const GEOMETRY_EFFECT_STATE: usize = NiGeometry::spEffectState;
const GEOMETRY_MODEL_DATA: usize = NiGeometry::spModelData;
const GEOMETRY_SKIN: usize = NiGeometry::skinInstance;
/// The words of a `NiTransform`: the rotation, the translation (from word 9), the scale.
const TRANSFORM_WORDS: usize = 13;
const MOST_LIGHTS: usize = 64;
/// The property pointers of a `NiPropertyState`: alpha first, 12 of them.
const STATE_FIRST: usize = NiPropertyState::propAlpha;
const STATE_COUNT: usize = 12;
/// A u16; bit 0 of the alpha property is the blend.
const PROPERTY_FLAGS: usize = NiProperty::flags;
/// The bits of an alpha property's flags: blending, and the no-sorter hint (bit 13).
const ALPHA_BLEND: u16 = 1;
const ALPHA_NO_SORTER: u16 = 0x2000;
/// The engine camera's culling planes in their order (`NiCamera::UpdateWorldData` 0x6CC940),
/// then any plane after the six.
const PLANE_NAMES: [&str; 7] = ["near", "far", "left", "right", "top", "bottom", "another"];
/// The display of a shape calls it for each of its controllers before the draw (a morpher
/// writes its vertices then).
const SLOT_ON_PRE_DISPLAY: usize = InterfaceNiTimeController::OnPreDisplay;
const RTTI_NAME: usize = NiRTTI::name;
const RTTI_BASE: usize = NiRTTI::baseRTTI;
/// The frames between two checks of the handles of an instance. This note is a safety net for
/// changes of a retained shape's material that no event reports. Followed by events: the
/// property and effect states (`recheck`), the Lua changes (`note_change`), the live
/// controllers (each display), a texture effect's words ([`EffectWatch`]), the freed textures
/// ([`note_materials_again`]). Left to the net: a map's texture swapped in place without a Lua
/// note (59 of the 1051 changes it caught in the captures of 2026-10-27). The rechecks line
/// counts what the net finds, by group (`periodic_changes`). A group leaves the net only when
/// an event covers every path that writes it, never because a run counted 0.
const CHECK_FRAMES: u32 = 64;

/// The node kinds that keep a shape under them on the engine's path. A sort adjust node
/// draws its subtree in an order of its own, which the table does not follow. The other
/// kinds that change what or how they draw need nothing of the table, as long as the
/// engine's traversal reaches the shape. Engine-rules R5 (the skill wgpu-renderer-dev).
const REJECTED_KINDS: [&[u8]; 1] = [b"NiSortAdjustNode"];
/// The list of `NIRENDER_NODE_KINDS=0`: every kind that changes what or how it draws.
const REJECTED_KINDS_BEFORE: [&[u8]; 7] = [b"NiBillboardNode", b"NiSwitchNode", b"NiLODNode", b"NiBSPNode", b"NiSortAdjustNode", b"BSAnimationNode", b"BSParticleNode"];
/// The list of `NIRENDER_NODE_KINDS=0` for a skinned shape: only the kinds that change
/// which shape draws. The palette of each draw carries the animated bones.
const REJECTED_KINDS_SKINNED_BEFORE: [&[u8]; 5] = [b"NiBillboardNode", b"NiSwitchNode", b"NiLODNode", b"NiBSPNode", b"NiSortAdjustNode"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Dual,
    Gpu,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "off" => Some(Mode::Off),
            "dual" => Some(Mode::Dual),
            "gpu" => Some(Mode::Gpu),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Dual => "dual",
            Mode::Gpu => "gpu",
        }
    }
}

/// The reasons a shape is not retained, for the report.
const REASON_CONTROLLER: usize = 0;
const REASON_NODE_KIND: usize = 1;
const REASON_PROPERTY_CONTROLLER: usize = 2;
const REASON_LATER: usize = 3;
/// The depth test of the shape is equal, greater, not equal or greater or equal: the draw
/// depends on the depth of a draw before it, so its place in the order of the game matters.
const REASON_DEPTH: usize = 4;
/// More nodes above the shape than the walk allows ([`crate::parent_chain::GUARD`]): the nodes
/// beyond are unread, so the shape stays on the engine's path.
const REASON_CHAIN: usize = 5;
const REASONS: usize = 6;

struct Instance {
    id: u32,
    class: u32,
    geometry: u32,
    material: u32,
    /// The rows 0 to 2 of the world matrix of the draw (the rotation with the scale).
    rotation: [u32; 12],
    /// The stored translation of the world transform of the shape, as bits.
    stored: [u32; 3],
    /// The colours of the material, as the shape record has them.
    colours: [u32; 14],
    /// The world bound of the shape: the centre and the radius, and its world transform.
    /// Zero for a skinned instance: its bound changes with each pose.
    bound: [f32; 4],
    /// The posed world bound of a skinned instance at its last bones (`send_bones`): the
    /// bound the engine culls the shape by (0x6F4160, 0x6D1250). `scene_end` tests it.
    posed_bound: [f32; 4],
    transform: [u32; TRANSFORM_WORDS],
    /// The keys of the lights the helper has for the instance (mode gpu), and the ambient
    /// light of its own effect state at the last lights record.
    lights: Vec<u32>,
    ambient: u32,
    /// The views that show the instance in the helper (bit v for view slot v, `view_slots`),
    /// and its key in the visible lists of the views other than view 0 ([`ViewLists`]):
    /// [`NO_KEY`] until its first display in one of them.
    visible: u32,
    list_key: u32,
    /// The count of view 0's closes at its last display in view 0
    /// ([`ViewLists::main_closes`]): view 0 displayed it since its last close when the two are
    /// equal. 0 before its first display there.
    main_shown: u32,
    /// The number of the main view's send that it joined last ([`ShownList::send`]); 0 before
    /// its first.
    listed_send: u32,
    /// Its `INSTANCE` record goes again for the start of a capture file (`reset`): the state
    /// records of its views follow the record (`restate`).
    restate: bool,
    /// A billboard node is above the shape: its display reads the world transform of the
    /// last view's turn (design 3.4a, until stage 2b). The frame of its last display and the
    /// views that displayed it in that frame, for [`BILLBOARD_IN_TWO_VIEWS`].
    under_billboard: bool,
    views_frame: u32,
    frame_views: u32,
    /// The `INSTANCE` record with these values went into the stream or the file.
    sent: bool,
    /// The matrices of its last `INSTANCE` record were relative to the camera's origin.
    relative: bool,
    /// The values above were taken from a draw.
    filled: bool,
    /// The generation of the tables at the last check of the handles.
    generation: u32,
    /// A skinned shape (an actor). In mode gpu the helper draws it from its table, and its
    /// display sends its bones in each frame the engine displays it (`send_bones`); with
    /// `NIRENDER_SKIN_TABLE=0` its draw carries the palette of its bones each frame, so it
    /// stays on the draw records (never in the helper's table, never skipped).
    skinned: bool,
    /// A controller on the shape or on one of its properties changes its geometry or its
    /// material (a morpher, a UV controller, a flip, alpha or colour controller): the
    /// display runs the controllers' `OnPreDisplay` as the engine's display does, and
    /// notes the geometry and the material every time.
    live: bool,
    /// The shape got a new property or effect state since its classification: its next
    /// display classifies it again and notes it.
    recheck: bool,
    /// The kind of its draw (`link::DRAW_SHAPE` or `DRAW_STRIPS`), 0 until known.
    kind: u32,
    /// The game skins the shape in software, as its last display read the flag.
    software: bool,
    /// A copy of the shape's property pointers at its last display.
    properties: [usize; PROPERTY_SLOTS],
    /// The count of Lua's change notes at the last check of the notes for this shape
    /// ([`check_notes`]).
    checked: u64,
    /// The frame in which the engine's main sorted queue took the shape (`queue_blend`), or
    /// [`u32::MAX`].
    queued: u32,
    /// Its environment map and fog map at its last note of the material: the address and
    /// the version of each ([`EffectWatch`]). Another version means another material.
    effect_versions: [(usize, u32); 2],
    /// At its last display its alpha property blends without the no-sorter bit: the engine's
    /// main queue sorts it (`NiAlphaAccumulator::RegisterObject` 0x429B90), and so does the
    /// helper's table.
    sorted_blend: bool,
    /// A particle system (a `RenderPoints` draw): its particles go as an `INSTANCE_PARTICLES`
    /// record before each `INSTANCE_DRAW`, at the place of the engine's draw, and the table
    /// never draws it by itself (particles plan, step 2).
    points: bool,
}

/// A texture effect of the table's instances (an environment or a fog map, whose words hold no
/// projection when it moves in the play: `effect_chain`), by its address:
/// its words as the material record copies them, read at most once a frame, and a version
/// that grows when they change. An effect with a controller has a chain in place of its
/// projection words, and any other effect's projection follows its world transform
/// (`NiTextureEffect::UpdateWorldData` 0x6E3970 calls `UpdateProjection` 0x6E37C0), so a
/// move of a node above it changes the material of every shape it lights.
#[derive(Default)]
struct EffectWatch {
    words: [u32; crate::material_stream::EFFECT_SIGNATURE_WORDS],
    version: u32,
    frame: u32,
}

impl EffectWatch {
    /// The version of the effect in `frame`: `read` gives its words, at most once a frame.
    fn observe(&mut self, frame: u32, read: impl FnOnce() -> [u32; crate::material_stream::EFFECT_SIGNATURE_WORDS]) -> u32 {
        if self.version == 0 || self.frame != frame {
            self.frame = frame;
            let words = read();
            if self.version == 0 || words != self.words {
                self.words = words;
                self.version = self.version.wrapping_add(1).max(1);
            }
        }
        self.version
    }
}

/// The frames an effect stays in the watch list without a display of a shape it lights.
const EFFECT_FRAMES: u32 = 4096;

/// The property slots of `NiPropertyState` from `propAlpha` (+0x08) to `field_30` (+0x30).
const PROPERTY_SLOTS: usize = 11;

struct Tables {
    instances: HashMap<usize, Instance>,
    rejected: HashSet<usize>,
    next_id: u32,
    frame: u32,
    /// Grows when the handles of every instance need a check: the next draw of each takes
    /// the full path once.
    generation: u32,
    /// The draw in progress is of a retained shape: the shape and its id.
    pending: Option<(usize, u32)>,
    mode: Option<Mode>,
    /// The camera of the world scene in progress, for the check of the visibility at its
    /// end (mode gpu).
    world_camera: usize,
    /// The kind of the scene in progress (`SCENE_*` of `link.rs`), for `skipped_display`.
    scene_kind: u32,
    /// Where the engine's displays of retained shapes go in the scene in progress.
    scene_route: Route,
    /// The view slot of the scene in progress (`view_slots`); 0 for a scene that is not a
    /// view, whose displays show in view 0 as before. `scene_camera` is the scene's camera
    /// when the scene is a view, else 0.
    scene_view: u8,
    scene_camera: usize,
    /// The visible list of each view slot ([`ViewLists`]), and the culling planes of its camera
    /// at the end of its last scene (`NiPlane`: the normal, then the constant).
    lists: ViewLists,
    view_planes: Vec<Vec<[f32; 4]>>,
    /// The main view's list of displays ([`ShownList`]).
    shown: ShownList,
    states: u64,
    light_records: u64,
    /// The displays the engine skipped, and the device values of the lights of the frame
    /// (built once a frame for each light, as the light manager builds its entry).
    skipped: u64,
    skip: bool,
    /// The frame of the last `RETAINED_SCENE` record and its values; the skipped shapes
    /// whose own ambient light or fog differed from them.
    scene_frame: u32,
    scene_values: (u32, Option<[u32; 4]>),
    scene_mixed: u64,
    /// Risk 34, for a while: the shapes other than sorted blends that the engine did not
    /// display in a world scene and that `scene_end` left visible because an engine plane
    /// puts them out of view, by that plane ([`PLANE_NAMES`]): the helper's own frustum
    /// decides for them, and a wider one draws in the main view what the engine does not.
    undisplayed_left: [u64; 7],
    /// The texture effects of the instances ([`EffectWatch`]), and the notes of the material
    /// of a sent instance because one of its effects changed.
    effects: HashMap<usize, EffectWatch>,
    effect_notes: u64,
    /// The instances marked for a new note because their material named a freed texture.
    texture_notes: u64,
    /// The notes that only the every-64th-frame turn made due and that found another
    /// material, by the groups of words that changed (`material_stream::GROUP_NAMES`): what
    /// that note catches that no event reports.
    periodic_changes: [u64; crate::material_stream::GROUPS],
    /// The landscape (class 11) is retained too (`NIRENDER_LANDSCAPE=0` keeps it out).
    landscape: bool,
    /// The shapes of class 0 (under no root the classes know: the actors and their parts, and
    /// objects hung elsewhere) are retained too (`NIRENDER_ACTORS=0` keeps them out). The class
    /// does not say what a shape is; the reference of its scene node would.
    actors: bool,
    /// A shape under an animated node, and a shape of class 0 that is not skinned, are
    /// retained too (`NIRENDER_ANIMATED=0` keeps them out).
    animated: bool,
    /// A shape under a billboard, a switch, a level of detail, a BSP, an animation or a
    /// particle node is retained too (`NIRENDER_NODE_KINDS=0` keeps them out). See
    /// [`REJECTED_KINDS`].
    node_kinds: bool,
    /// A shape with a controller on itself or on a property is retained as a live shape
    /// (`NIRENDER_CONTROLLERS=0` rejects it).
    controllers: bool,
    /// In mode gpu the helper draws the skinned instances from its table.
    /// `NIRENDER_SKIN_TABLE=0` keeps them on the engine's display with a palette in each draw.
    skin_table: bool,
    /// The bones of the instance in progress (`send_bones`), kept between the instances; the
    /// `INSTANCE_BONES` records, and the skinned instances hidden because their bones could
    /// not be made.
    scratch_bones: Vec<u8>,
    bone_records: u64,
    bone_failures: u64,
    /// The displays of skinned instances whose own ambient light or fog differed from the
    /// scene's values (the table draws them with the scene's).
    skin_scene_mixed: u64,
    /// The frame each light (by its index) was noted last: its values are made and its LIGHT
    /// record sent at most once a frame, whatever number of shapes it lights.
    light_noted: Vec<u32>,
    scratch_entry: [u32; E::SIZE / 4],
    scratch_lights: Vec<usize>,
    /// The keys of the lights of the shape in progress, kept between the shapes so that a
    /// display does not allocate.
    scratch_keys: Vec<u32>,
    fast_draws: u64,
    records: u64,
    /// The instance records sent again: for changed handles, and for changed values of
    /// the draw (the transform, the colours).
    handle_updates: u64,
    value_updates: u64,
    deletes: u64,
    /// The instances that ended with `sent` clear (`note_end`).
    ends_without_record: u64,
    /// The draws of particle instances sent as `INSTANCE_DRAW` records (particles plan, step 2).
    particle_draws: u64,
    /// The particle instances made (the log names the first 100).
    particle_instances_made: u64,
    rejected_by: [u64; REASONS],
    /// The draws that `consider` did not take, by the first cause: not a plain world draw
    /// of a retained class, no geometry handle, no material handle, the path off, the
    /// material not modelled.
    skipped_by: [u64; 5],
}

/// The key of an instance that no view listed yet ([`Instance::list_key`]).
const NO_KEY: u32 = u32::MAX;
/// The place of a key that a view does not list ([`ViewLists::places`]).
const NOT_LISTED: u32 = u32::MAX;
/// The index bits of a place; bit 31 is the list.
const PLACE_INDEX: u32 = 0x7FFF_FFFF;

/// The visible lists of the view slots (`view_slots`, design 3.1). Each view has two lists of
/// entries (the shape and the instance's key): one holds the entries that the view's scenes
/// displayed since its last close, the other the entries that they did not display yet. A
/// display moves its entry into the first list; a close visits only the second, and then the
/// first list becomes the second.
///
/// View 0 closes in every frame and has no lists: a display in it marks the instance with the
/// count of its closes ([`Instance::main_shown`]), and its close sweeps the instances in the
/// order of the table ([`close_main_view`]).
///
/// An instance has one key for all views, from its first display to its end ([`forget`] and
/// [`Tables::reject_pending`] give it back). For each view, the place of each key tells the
/// list (bit 31) and the index of its entry.
struct ViewLists {
    /// The closes of view 0 since the start, from 1; 0 is never a count.
    main_closes: u32,
    /// For each view: its two lists.
    entries: Vec<[Vec<(usize, u32)>; 2]>,
    /// For each view: the list (0 or 1) of the entries displayed since the last close.
    shown: Vec<u8>,
    /// For each view, by key: the place of the key's entry, or [`NOT_LISTED`].
    places: Vec<Vec<u32>>,
    free_keys: Vec<u32>,
    next_key: u32,
}

impl ViewLists {
    fn new() -> ViewLists {
        ViewLists {
            main_closes: 1,
            entries: (0..VIEW_SLOTS).map(|_| [Vec::new(), Vec::new()]).collect(),
            shown: vec![0; VIEW_SLOTS],
            places: vec![Vec::new(); VIEW_SLOTS],
            free_keys: Vec::new(),
            next_key: 0,
        }
    }

    /// A key for an instance at its first display.
    fn take_key(&mut self) -> u32 {
        self.free_keys.pop().unwrap_or_else(|| {
            let key = self.next_key;
            self.next_key += 1;
            key
        })
    }

    fn place(&self, view: u8, key: u32) -> u32 {
        self.places[view as usize].get(key as usize).copied().unwrap_or(NOT_LISTED)
    }

    fn set_place(&mut self, view: u8, key: u32, place: u32) {
        let places = &mut self.places[view as usize];
        if places.len() <= key as usize {
            places.resize(key as usize + 1, NOT_LISTED);
        }
        places[key as usize] = place;
    }

    /// Appends the entry to the view's list `list`.
    fn push(&mut self, view: u8, list: usize, shape: usize, key: u32) {
        let entries = &mut self.entries[view as usize][list];
        let place = (list as u32) << 31 | entries.len() as u32;
        entries.push((shape, key));
        self.set_place(view, key, place);
    }

    /// Takes the entry of `key` out of its list in the view, if the view lists it. The last
    /// entry of that list takes its index.
    fn take_out(&mut self, view: u8, key: u32) {
        let place = self.place(view, key);
        if place == NOT_LISTED {
            return;
        }
        let entries = &mut self.entries[view as usize][(place >> 31) as usize];
        let index = (place & PLACE_INDEX) as usize;
        entries.swap_remove(index);
        if let Some(&(_, moved)) = entries.get(index) {
            self.places[view as usize][moved as usize] = place;
        }
        self.places[view as usize][key as usize] = NOT_LISTED;
    }

    /// A display in the view: the entry is in the list of the displayed entries until the
    /// view's close.
    fn show(&mut self, view: u8, shape: usize, key: u32) {
        let shown = self.shown[view as usize] as usize;
        let place = self.place(view, key);
        if place != NOT_LISTED && (place >> 31) as usize == shown {
            return;
        }
        self.take_out(view, key);
        self.push(view, shown, shape, key);
    }

    /// The instance of the key ends: its entries leave every list, and the key is free.
    fn give_back(&mut self, key: u32) {
        if key == NO_KEY {
            return;
        }
        for view in 0..VIEW_SLOTS as u8 {
            self.take_out(view, key);
        }
        self.free_keys.push(key);
    }

    /// The entries of the view, in both lists.
    #[cfg(test)]
    fn listed(&self, view: u8) -> Vec<(usize, u32)> {
        self.entries[view as usize].iter().flatten().copied().collect()
    }
}

/// The main view's list of displays (form D, `INSTANCE_SHOWN`): the ids of the retained
/// instances that the engine displayed in a world or water scene into the back buffer since the
/// last send. A send goes at Present and at a read of the back buffer, whichever comes first,
/// and the list then starts again. Each send has a number; an instance keeps the number of the
/// send it joined last ([`Instance::listed_send`]), so it joins a send once.
struct ShownList {
    ids: Vec<u32>,
    /// The number of the send in progress, from 1; 0 is never a number.
    send: u32,
    /// A scene of the list's kind began since the last send: the next send writes the list,
    /// even an empty one. Without such a scene (a load, mode off) no list goes.
    open: bool,
    /// The sends at Present and at a read of the back buffer, the ids sent, and the most ids
    /// of one send.
    sends: [u64; 2],
    ids_sent: u64,
    most: usize,
}

impl ShownList {
    const fn new() -> ShownList {
        ShownList { ids: Vec::new(), send: 1, open: false, sends: [0; 2], ids_sent: 0, most: 0 }
    }

    /// The instance joins the send in progress, once. A display before the list opened (a mode
    /// change in the middle of a scene) joins no list: the next send would carry it.
    fn add(&mut self, id: u32, listed_send: &mut u32) {
        if self.open && *listed_send != self.send {
            *listed_send = self.send;
            self.ids.push(id);
        }
    }

    /// The instance leaves the send in progress, if it joined it.
    fn remove(&mut self, id: u32, listed_send: &mut u32) {
        if *listed_send != self.send {
            return;
        }
        *listed_send = 0;
        if let Some(index) = self.ids.iter().position(|&listed| listed == id) {
            self.ids.swap_remove(index);
        }
    }

    /// The list of the send in progress, when a scene of its kind began since the last send;
    /// the next send starts empty. `at_read`: the send is at a read of the back buffer.
    fn take(&mut self, at_read: bool) -> Option<Vec<u32>> {
        if !core::mem::take(&mut self.open) {
            return None;
        }
        let ids = core::mem::take(&mut self.ids);
        self.send = self.send.wrapping_add(1).max(1);
        self.sends[at_read as usize] += 1;
        self.ids_sent += ids.len() as u64;
        self.most = self.most.max(ids.len());
        Some(ids)
    }

    /// No list goes for the displays so far (mode off): the list is empty and closed.
    fn drop_all(&mut self) {
        self.ids.clear();
        self.open = false;
        self.send = self.send.wrapping_add(1).max(1);
    }

    fn line(&self) -> String {
        let sends = self.sends[0] + self.sends[1];
        format!(
            "main view list: {} sends at Present, {} at a read of the back buffer, {:.1} ids a send, the most {}",
            self.sends[0],
            self.sends[1],
            self.ids_sent as f64 / sends.max(1) as f64,
            self.most
        )
    }
}

/// The scene kinds whose displays join the main view's list, and whose displays may go into the
/// engine's main sorted queue ([`may_queue`]): the world camera on the world root or on the
/// water plane. The plugin gives no sorted-alpha scene on wgpu (`link_client::scene_kind`).
fn main_view_kind(kind: u32) -> bool {
    matches!(kind, SCENE_WORLD | SCENE_WATER)
}

/// A display in a scene of this kind and route joins the main view's list: the kind of
/// [`main_view_kind`] into the back buffer ([`Route::Main`]). A world scene into a texture (the
/// picture of a save) is not the main view.
fn lists_main_view(kind: u32, route: Route) -> bool {
    main_view_kind(kind) && route == Route::Main
}

static TABLES: MainCell<Option<Tables>> = MainCell::new(None);

/// The table, for the state summary: the id, `sent`, `visible` and light list of each instance,
/// the mode and the generation. The note counts the records sent
/// (`geometry_stream::state_part`). Not `light_noted`: frames, which move by themselves.
pub fn state_part(mt: &MainThread) -> (crate::state_summary::Part, Option<Mode>, u32) {
    let t = tables(mt);
    let mut part = crate::state_summary::Part::default();
    let (mut sent, mut visible) = (0, 0);
    for instance in t.instances.values() {
        let mut words = vec![instance.id, instance.sent as u32, instance.visible];
        words.extend_from_slice(&instance.lights);
        part.add(&words);
        sent += instance.sent as u32;
        visible += (instance.visible != 0) as u32;
    }
    let records: u64 = (*RECORD_COUNTS.get(mt))[RC_FIRST..=RC_MOVED_COLOURS].iter().sum();
    part.note = format!("sent {sent}, visible {visible}, records sent {records}");
    (part, t.mode, t.generation)
}

/// [`tables`] when no open of them is live; [`None`] while one is. Before the first open the
/// table does not exist, and gives [`None`] too: it has no marks to clear then.
fn tables_try(mt: &MainThread) -> Option<Open<'_, Tables>> {
    TABLES.try_get(mt).and_then(|tables| tables.filter_map(Option::as_mut))
}

/// The table, for a holder of the proof of the main thread; made at the first open.
#[track_caller]
fn tables(mt: &MainThread) -> Open<'_, Tables> {
    TABLES.get(mt).map(|tables| tables.get_or_insert_with(|| Tables {
        instances: HashMap::default(),
        rejected: HashSet::default(),
        next_id: 1,
        frame: 0,
        generation: 1,
        pending: None,
        mode: None,
        world_camera: 0,
        scene_kind: SCENE_OTHER,
        scene_route: Route::Main,
        scene_view: 0,
        scene_camera: 0,
        lists: ViewLists::new(),
        view_planes: vec![Vec::new(); VIEW_SLOTS],
        shown: ShownList::new(),
        states: 0,
        light_records: 0,
        skipped: 0,
        skip: crate::switches::on("NIRENDER_SKIP") != Some(false),
        scene_frame: u32::MAX,
        scene_values: (0, None),
        scene_mixed: 0,
        undisplayed_left: [0; 7],
        effects: HashMap::default(),
        effect_notes: 0,
        texture_notes: 0,
        periodic_changes: [0; crate::material_stream::GROUPS],
        landscape: crate::switches::on("NIRENDER_LANDSCAPE") != Some(false),
        actors: crate::switches::on("NIRENDER_ACTORS") != Some(false),
        animated: crate::switches::on("NIRENDER_ANIMATED") != Some(false),
        node_kinds: crate::switches::on("NIRENDER_NODE_KINDS") != Some(false),
        controllers: crate::switches::on("NIRENDER_CONTROLLERS") != Some(false),
        skin_table: crate::switches::on("NIRENDER_SKIN_TABLE") != Some(false),
        scratch_bones: Vec::new(),
        bone_records: 0,
        bone_failures: 0,
        skin_scene_mixed: 0,
        light_noted: Vec::new(),
        scratch_entry: [0; E::SIZE / 4],
        scratch_lights: Vec::new(),
        scratch_keys: Vec::new(),
        fast_draws: 0,
        records: 0,
        handle_updates: 0,
        value_updates: 0,
        deletes: 0,
        ends_without_record: 0,
        particle_draws: 0,
        particle_instances_made: 0,
        rejected_by: [0; REASONS],
        skipped_by: [0; 5],
    }))
}

impl Instance {
    /// An instance of a shape at its first draw: no record of it went yet.
    #[allow(clippy::too_many_arguments)]
    fn new(id: u32, class: u32, geometry: u32, material: u32, generation: u32, skinned: bool, live: bool, points: bool) -> Instance {
        Instance { id, class, geometry, material, rotation: [0; 12], software: false, stored: [0; 3], colours: [0; 14], bound: [0.0; 4], posed_bound: [0.0; 4], transform: [0; TRANSFORM_WORDS], lights: Vec::new(), ambient: 0xFF00_0000, visible: 0, list_key: NO_KEY, main_shown: 0, listed_send: 0, restate: false, under_billboard: false, views_frame: 0, frame_views: 0, sent: false, relative: false, filled: false, generation, skinned, live, recheck: false, kind: 0, properties: [0; PROPERTY_SLOTS], checked: 0, effect_versions: [(0, 0); 2], sorted_blend: false, queued: u32::MAX, points }
    }
}

impl Tables {
    /// The mode, from `NIRENDER_RETAINED` at the first call.
    fn mode(&mut self) -> Mode {
        *self.mode.get_or_insert_with(|| crate::switches::parsed("NIRENDER_RETAINED", "off, dual or gpu", Mode::parse).unwrap_or(Mode::Gpu))
    }
}

/// The mode, from `NIRENDER_RETAINED` at the first call.
pub unsafe fn mode(mt: &MainThread) -> Mode {
    tables(mt).mode()
}

/// Sets the mode and returns the one before. A change of the mode sends the `INSTANCE`
/// record of each shape again at its next draw: the record says which mode draws it.
/// Engine-rules R18 (the skill wgpu-renderer-dev).
pub unsafe fn set_mode(mt: &MainThread, mode: Mode) -> Mode {
    let before = tables(mt).mode();
    // Drain the cell worker's events first: a shape it destroyed leaves the table.
    crate::worker_events::drain();
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    t.mode = Some(mode);
    // Only mode gpu writes the main view's list: in another mode the engine draws the shapes.
    if mode != Mode::Gpu {
        t.shown.drop_all();
    }
    if mode != before {
        // In mode off the engine draws every shape, so the helper must hide its table. The
        // next display of a shape in mode gpu shows its instance again.
        let hide = mode == Mode::Off && capture::recording();
        // Write no mark here: the cell worker can free a kept shape at any time. In mode
        // gpu `draw` marks a shape again at its next draw; in another mode the display
        // clears a stale mark (`skipped_display_timed`).
        let mut hidden = 0;
        for instance in t.instances.values_mut() {
            instance.sent = false;
            instance.lights.clear();
            if hide && instance.filled {
                hidden += hide_everywhere(instance);
            }
        }
        t.states += hidden;
    }
    before
}

/// True for a class of object the plugin retains: the objects of the references, the
/// landscape, and class 0. Class 0 is a shape of the world scene under no root the classes
/// know: the actors and their parts, and objects hung elsewhere. By default every shape of
/// class 0 is retained, skinned or not. `NIRENDER_ACTORS=0` keeps class 0 out;
/// `NIRENDER_ANIMATED=0` keeps out the class 0 shapes that are not skinned.
pub unsafe fn class_retained(mt: &MainThread, class: u32, skinned: bool) -> bool {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    class == CLASS_OBJECT
        || (class == CLASS_LANDSCAPE && t.landscape)
        || (class == CLASS_UNKNOWN && t.actors && (skinned || t.animated))
        || (sky_table_class(class) && sky_table_on())
        || (class == CLASS_WEATHER_PARTICLES && weather_table_on())
}

/// Sets or clears the mark of a retained shape.
unsafe fn mark(shape: usize, on: bool) {
    let flags: u16 = rd(shape, FLAGS);
    wr::<u16>(shape, FLAGS, if on { flags | RETAINED_BIT } else { flags & !RETAINED_BIT });
}

/// True when the retained path is in use: a mode other than off, and the matrices of the
/// draws are made by the plugin (the exact translations exist then).
unsafe fn enabled(mt: &MainThread) -> bool {
    mode(mt) != Mode::Off && crate::camera_space::own()
}

/// Forgets which instances were sent: for the start of a capture file from the stream, so
/// that the next draw of each sends its `INSTANCE` record into the file. The capture purges
/// the source textures too, so the handles of every instance get a check.
pub unsafe fn reset(mt: &MainThread, cause: &str) {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    crate::log!("instances: the records go again: {cause}");
    t.generation = t.generation.wrapping_add(1);
    for instance in t.instances.values_mut() {
        instance.sent = false;
        instance.lights.clear();
        instance.restate = true;
    }
    // The LIGHT records go again (`light_stream::reset`): the lights are noted again too.
    t.light_noted.clear();
}

/// Sends the ids of the instances (`INSTANCE_CENSUS`). The destroyed shapes that the cell
/// worker queued leave first, with their deletes; one that it destroys after the drain only
/// makes the list longer. So the helper holds no id that the list lacks, unless a delete was
/// lost.
///
/// # Safety
///
/// The caller does not hold the tables open: the drain and the list open them.
pub unsafe fn send_census(mt: &MainThread) {
    crate::worker_events::drain();
    if !capture::recording() {
        return;
    }
    let ids: Vec<u32> = tables(mt).instances.values().map(|instance| instance.id).collect();
    capture::instance_census(&ids);
}

/// MWSE_NATIVE_EVENT_PLAYER_CELL_CHANGED, on the main thread.
unsafe extern "C" fn player_cell_changed(_user: *mut core::ffi::c_void, _packet: *const core::ffi::c_void) {
    if let Some(t) = MainThread::here() {
        send_census(&t);
    }
}

/// A census at each cell change of the player, from MWSE's native event.
///
/// # Safety
///
/// As for `mwse_native::subscribe`: the listener is a function of this DLL, which stays loaded.
pub unsafe fn install_census() {
    use crate::mwse_native::{subscribe, NATIVE_EVENT_PLAYER_CELL_CHANGED};
    if let Err(error) = subscribe(NATIVE_EVENT_PLAYER_CELL_CHANGED, player_cell_changed) {
        crate::log!("instances: {error}: no census at a cell change");
    }
}

/// A scene of the game begins: the camera of the world scene is kept for the check at its
/// end.
pub unsafe fn scene_begin(mt: &MainThread, kind: u32, camera: usize, route: Route, view: u32) {
    // The shapes destroyed on another thread leave the tables before any walk of the scene.
    crate::worker_events::drain();
    {
        let mut tables_open = tables(mt);
        let t = &mut *tables_open;
        t.world_camera = if kind == SCENE_WORLD { camera } else { 0 };
        t.scene_kind = kind;
        t.scene_route = route;
        // `view` is the slot + 1 (`view_open`), 0 for a scene that is not a view.
        t.scene_view = view.saturating_sub(1) as u8;
        t.scene_camera = if view != 0 { camera } else { 0 };
        if t.mode == Some(Mode::Gpu) && capture::recording() && lists_main_view(kind, route) {
            t.shown.open = true;
        }
    }
    if kind == SCENE_WORLD && camera != 0 {
        crate::animation::note_camera(mt, camera);
    }
    // The lights of the world go before the draws of the world.
    if kind == SCENE_WORLD {
        crate::world_lights::send(mt);
    }
}

/// A scene of the game ends. The scene's view keeps the culling planes of its camera for its
/// close, and a view into a texture closes: it is one scene (design 3.1), so all its state
/// records come before its `SCENE_END`. A view into the back buffer closes at the swap
/// (`view_swap`). At the end of the world scene the effect checks go (`effect_chain`).
pub unsafe fn scene_end(mt: &MainThread) {
    let view = {
        let mut tables_open = tables(mt);
        let t = &mut *tables_open;
        let camera = core::mem::replace(&mut t.world_camera, 0);
        if camera != 0 && t.mode() == Mode::Gpu && capture::recording() {
            crate::effect_chain::send_checks(mt, camera);
        }
        let view = (t.scene_camera != 0).then_some(t.scene_view);
        if let Some(view) = view {
            let planes = &mut t.view_planes[view as usize];
            planes.clear();
            let count = rd::<i32>(t.scene_camera, C::countCullingPlanes).max(0) as usize;
            let storage: usize = rd(t.scene_camera, C::cullingPlanes_storage);
            planes.extend((0..count).map(|i| rd::<[f32; 4]>(rd::<usize>(storage, i * 4), 0)));
        }
        t.scene_view = 0;
        t.scene_camera = 0;
        view
    };
    if let Some(view) = view {
        let closed = VIEWS.get(mt).as_mut().and_then(|views| views.scene_end(view));
        close_views(mt, closed);
    }
}

/// Views close (`view_slots`, design 3.1): in each, a shape that its scenes did not display
/// since its last close and that is inside the culling planes of its camera is hidden in it
/// (`INSTANCE_STATE` with the view). After a root change the plane test is skipped: every
/// shape not displayed is hidden. View 0's close sweeps the instances in the order of the
/// table; another view's close visits only the entries of its list that its scenes did not
/// display (`ViewLists`, design 3.1a). Engine-rules R14, R16 (the skill wgpu-renderer-dev).
/// After the first `display_cost` call the closes are timed.
pub fn close_views(mt: &MainThread, closed: impl IntoIterator<Item = Closed>) {
    // SAFETY: the close passes the bounds of the table's instances and copies of the camera's
    // planes, which `NiBound::WhichSide` reads.
    unsafe { close_views_with(mt, closed, engine_side) };
}

/// The side of the plane on which the bound is (`NiBound::WhichSide`): 2 is outside.
///
/// # Safety
///
/// `bound` is the address of an `NiBound` (the centre, then the radius).
unsafe fn engine_side(bound: usize, plane: &[f32; 4]) -> u32 {
    thiscall1(WHICH_SIDE, bound, plane.as_ptr() as u32)
}

/// The side of a plane for a bound: [`engine_side`], or a stand-in in a test.
type WhichSide = unsafe fn(usize, &[f32; 4]) -> u32;

/// [`close_views`] with the plane test `side`.
///
/// # Safety
///
/// `side` is safe to call for the bound of each instance of the table and the planes of the
/// closed views.
unsafe fn close_views_with(mt: &MainThread, closed: impl IntoIterator<Item = Closed>, side: WhichSide) {
    use core::sync::atomic::Ordering::Relaxed;
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    let hide = t.mode() == Mode::Gpu && capture::recording();
    let timed = COST_ON.load(Relaxed);
    let start = if timed { capture::ticks() } else { 0 };
    for (slot, root_changed) in closed {
        let planes = core::mem::take(&mut t.view_planes[slot as usize]);
        let close = Close { slot, root_changed, hide, planes: &planes, side };
        let (visits, hides) = if slot == crate::view_slots::MAIN_VIEW { close_main_view(t, &close) } else { close_view(t, &close) };
        t.view_planes[slot as usize] = planes;
        t.states += hides[0] + hides[1];
        CLOSE_HIDES[0].fetch_add(hides[0], Relaxed);
        CLOSE_HIDES[1].fetch_add(hides[1], Relaxed);
        CLOSE_VISITS.fetch_add(visits, Relaxed);
    }
    if timed {
        CLOSE_TICKS.fetch_add(capture::ticks() - start, Relaxed);
    }
}

/// The close of one view: the view, a root change since its last close, whether the close
/// hides (mode gpu and a capture running), the planes of its camera, and the plane test.
struct Close<'a> {
    slot: u8,
    root_changed: bool,
    hide: bool,
    planes: &'a [[f32; 4]],
    side: WhichSide,
}

/// The close of view 0, in every frame: a sweep of the instances in the order of the table, as
/// the close at the end of stage 1's world scene. Each instance that is visible in view 0 and
/// that view 0 did not display since its last close stays or is hidden
/// ([`Instance::stays_at_close`]). Then the count of the closes grows, so that every mark of a
/// display is old. Without `hide` the close keeps every instance and visits none. Returns the
/// instances visited and the hides (by the plane test, after a root change).
///
/// # Safety
///
/// `close.side` is safe to call for the bound of each instance of the table.
unsafe fn close_main_view(t: &mut Tables, close: &Close) -> (u64, [u64; 2]) {
    let stamp = t.lists.main_closes;
    t.lists.main_closes = stamp.wrapping_add(1).max(1);
    let mut hides = [0u64; 2];
    let mut visits = 0u64;
    if close.hide {
        let bit = 1u32 << close.slot;
        for instance in t.instances.values_mut() {
            if instance.visible & bit == 0 || instance.main_shown == stamp {
                continue;
            }
            visits += 1;
            instance.stays_at_close(close, &mut t.undisplayed_left, &mut hides);
        }
    }
    (visits, hides)
}

/// The close of a view other than view 0. Each entry that the view's scenes did not display
/// since the last close stays or leaves ([`Instance::stays_at_close`]); an entry whose instance
/// ended without giving its key back leaves too. Then the displayed entries become the entries
/// that wait for the next close. Returns the entries visited and the hides.
///
/// # Safety
///
/// `close.side` is safe to call for the bounds of the listed instances.
unsafe fn close_view(t: &mut Tables, close: &Close) -> (u64, [u64; 2]) {
    let (slot, view) = (close.slot, close.slot as usize);
    let shown = t.lists.shown[view] as usize;
    let waiting_list = shown ^ 1;
    let mut waiting = core::mem::take(&mut t.lists.entries[view][waiting_list]);
    let mut hides = [0u64; 2];
    for &(shape, key) in &waiting {
        let stays = match t.instances.get_mut(&shape).filter(|instance| instance.list_key == key) {
            Some(instance) => instance.stays_at_close(close, &mut t.undisplayed_left, &mut hides),
            None => false,
        };
        if stays {
            t.lists.push(slot, shown, shape, key);
        } else {
            t.lists.set_place(slot, key, NOT_LISTED);
        }
    }
    let visits = waiting.len() as u64;
    waiting.clear();
    t.lists.entries[view][waiting_list] = waiting;
    t.lists.shown[view] = waiting_list as u8;
    (visits, hides)
}

impl Instance {
    /// At the close of a view, for an instance that the view's scenes did not display since its
    /// last close: true when it stays visible in the view (and in its list); false when it is
    /// not visible in the view, or when the close hides it (`hide`, with its values from a
    /// draw): after a root change, or when it is inside every plane. A hide sends its state
    /// record and counts in `hides` (by the root change). An instance outside a plane stays; in
    /// view 0 it counts in `undisplayed_left` by that plane, unless it is a sorted blend.
    ///
    /// # Safety
    ///
    /// `close.side` is safe to call for the bound of the instance.
    unsafe fn stays_at_close(&mut self, close: &Close, undisplayed_left: &mut [u64; 7], hides: &mut [u64; 2]) -> bool {
        let bit = 1u32 << close.slot;
        if self.visible & bit == 0 {
            return false;
        }
        if !close.hide || !self.filled {
            return true;
        }
        if !close.root_changed {
            let bound = if self.skinned { self.posed_bound.as_ptr() } else { self.bound.as_ptr() } as usize;
            let outside = rejecting_plane(close.planes.iter().map(|plane| (close.side)(bound, plane)));
            // Out of the camera's view the instance stays visible and the helper's view
            // culls it with its own frustum. A sorted blend the engine did not display has
            // no place in the frame, and the helper's main view does not draw it (risk 34);
            // for a while the other shapes left so in view 0 are counted by the plane that
            // puts them out.
            if let Some(plane) = outside {
                if close.slot == crate::view_slots::MAIN_VIEW && !self.sorted_blend {
                    undisplayed_left[plane.min(PLANE_NAMES.len() - 1)] += 1;
                }
                return true;
            }
        }
        self.visible &= !bit;
        capture::instance_state(self.id, false, close.slot);
        hides[close.root_changed as usize] += 1;
        false
    }
}

/// The hides of the views' closes: by the plane test, and after a root change.
static CLOSE_HIDES: [core::sync::atomic::AtomicU64; 2] = [core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0)];
/// The instances and list entries that the views' closes visited, and the time of the closes
/// after the first `display_cost` call (`display_cost` reads and resets both).
static CLOSE_VISITS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static CLOSE_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);

/// A view slot retires: no instance shows in it any more. The helper clears bit v at the
/// retirement (`EVENT_VIEW_RETIRE`). View 0 has no lists: its bit clears in every instance.
fn forget_view(t: &mut Tables, slot: u8) {
    let bit = 1u32 << slot;
    let view = slot as usize;
    if slot == crate::view_slots::MAIN_VIEW {
        for instance in t.instances.values_mut() {
            instance.visible &= !bit;
        }
    }
    for list in &mut t.lists.entries[view] {
        for (shape, key) in list.drain(..) {
            if let Some(instance) = t.instances.get_mut(&shape).filter(|instance| instance.list_key == key) {
                instance.visible &= !bit;
            }
        }
    }
    t.lists.places[view].clear();
    t.view_planes[view].clear();
}

/// The work of a destructor of a rendered texture that found a table open
/// (`view_target_destroyed`): the targets whose slots wait for their keys to go, and the slots
/// whose marks wait for their clear. `view_open` does it before it takes a slot.
static PENDING_TARGETS: MainCell<Vec<usize>> = MainCell::new(Vec::new());
static PENDING_FORGETS: MainCell<Vec<u8>> = MainCell::new(Vec::new());
/// The destructors whose work waited so.
pub static VIEW_TARGETS_WAITED: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// The work that the destructors left (`PENDING_TARGETS`, `PENDING_FORGETS`), before a slot is
/// taken: the targets first, then the marks of the freed slots.
fn do_pending_destructors(mt: &MainThread) {
    let targets = core::mem::take(&mut *PENDING_TARGETS.get(mt));
    for target in targets {
        view_target_destroyed(mt, target);
    }
    let slots = core::mem::take(&mut *PENDING_FORGETS.get(mt));
    forget_views(mt, &slots);
    // Their retirements go after the marks clear: `view_open` writes them next.
    RETIRED_VIEWS.get(mt).extend(slots);
}

/// [`forget_view`] for each retired slot.
fn forget_views(mt: &MainThread, retired: &[u8]) {
    if retired.is_empty() {
        return;
    }
    let mut tables_open = tables(mt);
    for &slot in retired {
        forget_view(&mut tables_open, slot);
    }
}

/// The views of a mask, from view 0.
fn views_of(mask: u32) -> impl Iterator<Item = u8> {
    (0..VIEW_SLOTS as u8).filter(move |view| mask & (1 << view) != 0)
}

/// The instances under a billboard that a second view displayed in one frame, once for each
/// instance and frame: they draw in both views with the world transform of the last view's
/// turn, until stage 2b turns them for each view (design 3.4a).
pub static BILLBOARD_IN_TWO_VIEWS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// A billboard node of any mode is above the shape (`NiBillboardNode`). Mode 3 does not turn,
/// and no NIF block of the content has it (design 3.4a, the scan of 2026-11-03).
unsafe fn under_billboard(shape: usize) -> bool {
    let found = crate::parent_chain::first_answer(
        rd(shape, PARENT),
        crate::parent_chain::GUARD,
        |node| rd(node, PARENT),
        |node| crate::ports::ni::is_kind_of(node, crate::effect_chain::RTTI_BILLBOARD_NODE).then_some(()),
    );
    matches!(found, Ok(Some(())))
}

/// A display of the instance in the scene's view `view` in the frame `frame`: bit v with its
/// state record when the bit was clear; in view 0 the count of its closes
/// ([`Instance::main_shown`]), in another view the shape in v's list of the entries displayed
/// since its last close ([`ViewLists::show`]). An instance under a billboard notes the views of
/// the frame. With `shown`, the display is in the main view's list ([`lists_main_view`]): the
/// instance joins it.
unsafe fn show_in_view(instance: &mut Instance, shape: usize, view: u8, frame: u32, lists: &mut ViewLists, states: &mut u64, shown: Option<&mut ShownList>) {
    if let Some(shown) = shown {
        shown.add(instance.id, &mut instance.listed_send);
    }
    let bit = 1u32 << view;
    if instance.under_billboard {
        if instance.views_frame != frame {
            instance.views_frame = frame;
            instance.frame_views = 0;
        }
        if instance.frame_views & bit == 0 {
            if instance.frame_views.count_ones() == 1 {
                BILLBOARD_IN_TWO_VIEWS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            instance.frame_views |= bit;
        }
    }
    if instance.visible & bit == 0 {
        instance.visible |= bit;
        capture::instance_state(instance.id, true, view);
        *states += 1;
    }
    if view == crate::view_slots::MAIN_VIEW {
        instance.main_shown = lists.main_closes;
        return;
    }
    if instance.list_key == NO_KEY {
        instance.list_key = lists.take_key();
    }
    lists.show(view, shape, instance.list_key);
}

/// A display of the instance of `shape` in the scene in progress, by `draw` or by the display
/// work ([`show_in_view`]): in a scene of [`lists_main_view`] the instance joins the main view's
/// list.
unsafe fn shown_in_scene(t: &mut Tables, shape: usize) {
    let Some(instance) = t.instances.get_mut(&shape) else { return };
    let shown = lists_main_view(t.scene_kind, t.scene_route).then_some(&mut t.shown);
    show_in_view(instance, shape, t.scene_view, t.frame, &mut t.lists, &mut t.states, shown);
}

/// Hides the instance in every view that shows it, with a state record for each. Returns the
/// records.
unsafe fn hide_everywhere(instance: &mut Instance) -> u64 {
    let mut records = 0;
    for view in views_of(instance.visible) {
        capture::instance_state(instance.id, false, view);
        records += 1;
    }
    instance.visible = 0;
    records
}

/// After an `INSTANCE` record that went again for the start of a capture file (`reset`): the
/// state records of the views that show the instance. The file's helper starts a new id hidden
/// in every view (`HEADER_VIEW_BITS`); the live helper keeps its bits, and the records repeat
/// them.
unsafe fn restate(instance: &mut Instance, states: &mut u64) {
    if !core::mem::take(&mut instance.restate) {
        return;
    }
    for view in views_of(instance.visible) {
        capture::instance_state(instance.id, true, view);
        *states += 1;
    }
}

/// The first plane whose side (`NiBound::WhichSide`) is 2, outside: the plane that puts the
/// bound out of view; [`None`] in view.
fn rejecting_plane(sides: impl Iterator<Item = u32>) -> Option<usize> {
    sides.enumerate().find(|&(_, side)| side == 2).map(|(index, _)| index)
}

/// The helper draws the skinned instances from its table: mode gpu, and the switch on.
unsafe fn table_draws_skins(t: &Tables) -> bool {
    t.mode == Some(Mode::Gpu) && t.skin_table
}

/// Whether the table draws the skinned instances (`NIRENDER_SKIN_TABLE`, `nirender.skinTable`).
pub unsafe fn skin_table(mt: &MainThread) -> bool {
    tables(mt).skin_table
}

/// Sets whether the table draws the skinned instances (`nirender.skinTable`) and returns
/// the state before. Each skinned instance sends its record again at its next draw: on,
/// `draw` marks it; off, its display clears the mark, and its draw sends the record without
/// the table's flag, which takes it out of the helper's table.
pub unsafe fn set_skin_table(mt: &MainThread, on: bool) -> bool {
    let before = tables(mt).skin_table;
    if on == before {
        return before;
    }
    // Drain the cell worker's events first, as `set_mode` does.
    crate::worker_events::drain();
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    t.skin_table = on;
    // Write no mark here, as in `set_mode`: `draw` marks a skinned shape again at its next
    // draw, and the display clears a stale mark when the table does not draw skins.
    for instance in t.instances.values_mut() {
        if !instance.skinned {
            continue;
        }
        instance.sent = false;
        instance.lights.clear();
    }
    before
}

/// True when the draw function of the instance is that of strips.
fn instance_strips(t: &Tables, shape: usize) -> bool {
    t.instances.get(&shape).is_some_and(|instance| instance.kind == crate::link::DRAW_STRIPS)
}

/// Sends the bones of a skinned instance (`INSTANCE_BONES`) with the shape's world bound.
/// Returns false when the bones cannot be made: the instance is hidden then, and counted.
unsafe fn send_bones(mt: &MainThread, t: &mut Tables, shape: usize, id: u32) -> bool {
    let mut bones = core::mem::take(&mut t.scratch_bones);
    let software = crate::ports::draw::skinned_in_software(instance_strips(t, shape), rd(shape, GEOMETRY_MODEL_DATA));
    let result = crate::camera_space::skin_bones(mt, shape, rd(shape, GEOMETRY_SKIN), software, &mut bones);
    let sent = match result {
        Ok(count) => {
            let posed: [f32; 4] = rd(shape, WORLD_BOUND);
            capture::instance_bones(id, count, &posed, &bones);
            t.bone_records += 1;
            if let Some(instance) = t.instances.get_mut(&shape) {
                instance.posed_bound = posed;
            }
            true
        }
        Err(why) => {
            t.bone_failures += 1;
            if t.bone_failures <= 4 {
                crate::log!("instances: no bones for the skinned instance {id} (shape 0x{shape:x}): {why}; it is hidden");
            }
            if let Some(instance) = t.instances.get_mut(&shape) {
                t.states += hide_everywhere(instance);
                t.shown.remove(instance.id, &mut instance.listed_send);
            }
            false
        }
    };
    t.scratch_bones = bones;
    sent
}

/// A device texture went away: the material handle of an instance can be stale. The next
/// draw of each instance notes its material and geometry again.
pub unsafe fn textures_changed(mt: &MainThread) {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    t.generation = t.generation.wrapping_add(1);
}

/// The start of a draw of the engine: a pending draw that no shape record took (a skinned
/// shape whose palette failed) must not reach the next record.
pub unsafe fn begin_draw(mt: &MainThread) {
    tables(mt).pending = None;
}

/// The main view's list for the record of the frame's end (`capture::frame_end`): [`None`]
/// when no world or water scene into the back buffer began since the last send.
pub fn take_shown_list(mt: &MainThread) -> Option<Vec<u32>> {
    tables(mt).shown.take(false)
}

/// A read of the back buffer (`own_host`: a lock of it, or a copy out of it): the main view's
/// list goes before the read's record, so that the helper draws the picture that is read with
/// its own displays. The next displays start a new list.
pub fn send_shown_list(mt: &MainThread) {
    let Some(mut tables_open) = tables_try(mt) else {
        SHOWN_TABLES_BUSY.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return;
    };
    let Some(ids) = tables_open.shown.take(true) else { return };
    drop(tables_open);
    // SAFETY: the caller is a call of the device on the main thread (the proof says so).
    unsafe { capture::instance_shown(&ids) };
}

/// The reads of the back buffer that found the table open, so that their list waited for the
/// next send (the report's main view list line).
static SHOWN_TABLES_BUSY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// The end of a frame.
pub unsafe fn frame_tick(mt: &MainThread) {
    crate::worker_events::drain();
    // The change notes that Lua made on another thread (MWSE fires some events on the cell
    // worker): the main thread notes them here.
    let queued = OFF_THREAD_CHANGES.lock().map(|mut q| core::mem::take(&mut *q)).unwrap_or_default();
    for address in queued {
        note_change(mt, address);
    }
    change_frame_end(mt);
    crate::effect_chain::frame_tick(mt);
    FRAME_DISPLAYS.get(mt).frame_end();
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    t.frame = t.frame.wrapping_add(1);
    // An effect that lit no displayed shape for a while goes; its address can be another
    // object's by now, and a watch made again reads it anew.
    if t.frame.is_multiple_of(EFFECT_FRAMES) {
        let now = t.frame;
        t.effects.retain(|_, watch| now.wrapping_sub(watch.frame) < EFFECT_FRAMES);
    }
    // The material handles that went with their textures and that no instance names leave
    // the material table.
    if t.frame.is_multiple_of(crate::material_stream::SWEEP_FRAMES) {
        let in_use: HashSet<u32> = t.instances.values().map(|instance| instance.material).collect();
        crate::material_stream::sweep(mt, &in_use, t.frame);
    }
    drop(tables_open);
    view_present(mt);
}

/// The view slots of D82 ([`ViewTable`]), and the slots that retired and whose retirement is
/// not written yet: at Present `capture::frame_end` writes them after `FRAME_END`, and after a flush's record the
/// flush's caller does ([`write_flush_retirements`]).
static VIEWS: MainCell<Option<ViewTable>> = MainCell::new(None);
static RETIRED_VIEWS: MainCell<Vec<u8>> = MainCell::new(Vec::new());
/// The slots taken so far, for the rate of their log lines.
static VIEW_TAKES: MainCell<u64> = MainCell::new(0);

/// The take or forced flush number `n` (from 1) gets a log line: the first 64, then each power
/// of 2.
fn view_line_due(n: u64) -> bool {
    n <= 64 || n.is_power_of_two()
}

/// The words of a view key for a log line: the camera, the root, the target and the scenes of
/// the same texture key before it since the last flush (more than 0: a second slot for the key).
fn key_words(key: ViewKey) -> String {
    match key {
        ViewKey::Screen { camera } => format!("camera 0x{camera:x} into the back buffer"),
        ViewKey::Texture { camera, root, target, again } => {
            format!("camera 0x{camera:x}, root 0x{root:x}, target 0x{target:x}, again {again}{}", if again > 0 { " (a second slot for the key)" } else { "" })
        }
    }
}

/// A scene starts: its view slot plus 1, for `SceneV1::view`, or 0 for a scene that is not a
/// view. 0 while the stream does not record: the slots retire only at a Present of the
/// recording (`view_present`). No scene is refused (design 3.1, the slot pressure):
/// - the retirements that waited for a recording stream go first, before any slot is taken;
/// - a slot taken back has its retirement written here, before the scene's record;
/// - a full table forces a flush: the open views into a texture close, the helper flushes at
///   `EVENT_FLUSH`, the second slots' retirements follow it, and the scene opens again.
///
/// A retirement clears the slot's bit in the plugin's marks too (`forget_view`).
///
/// A take of a slot and a forced flush each give a log line, at the rate of `view_line_due`.
pub fn view_open(mt: &MainThread, click: &Click) -> u32 {
    view_open_with_first(mt, click).0
}

/// [`view_open`] and, for a scene that repeats its texture key in the same flush, the slot of
/// the key's first scene plus 1 (else 0): the low byte of `SceneV1::repeat_walk`.
pub fn view_open_with_first(mt: &MainThread, click: &Click) -> (u32, u32) {
    if !capture::recording() {
        return (0, 0);
    }
    do_pending_destructors(mt);
    write_flush_retirements(mt);
    let mut views_open = VIEWS.get(mt);
    let views = views_open.get_or_insert_with(ViewTable::default);
    let mut opening = views.open(click);
    if opening == Opening::Full {
        let end = views.flush(true);
        if view_line_due(views.forced_flushes) {
            crate::log!("views: forced flush {} at frame {}: every slot is open or used since the last flush; {} views into a texture close", views.forced_flushes, views.frame(), end.closed.len());
        }
        // The closes' hides go before the flush, so that the helper runs the views closed.
        close_views(mt, end.closed);
        forget_views(mt, &end.retired);
        let _lock = crate::sync::device_lock();
        // SAFETY: the stream records (checked above) and the device lock is held, so the
        // events go into the ring in the order of this thread's records.
        unsafe {
            capture::frame_event(crate::link::EVENT_FLUSH, 0, 0);
            for slot in end.retired {
                capture::frame_event(crate::link::EVENT_VIEW_RETIRE, slot as u32, 0);
            }
        }
        opening = views.open(click);
    }
    let Opening::Opened(opened) = opening else { return (0, 0) };
    if opened.taken_back {
        forget_views(mt, &[opened.slot]);
        let _lock = crate::sync::device_lock();
        // SAFETY: as at the forced flush above.
        unsafe { capture::frame_event(crate::link::EVENT_VIEW_RETIRE, opened.slot as u32, 0) };
    }
    if opened.new {
        let mut takes = VIEW_TAKES.get(mt);
        *takes += 1;
        if let Some(key) = views.key(opened.slot).filter(|_| view_line_due(*takes)) {
            let back = if opened.taken_back { " (taken back)" } else { "" };
            crate::log!("views: take {} at frame {}: slot {}{back}, scene kind {}, {}; {} slots in use", *takes, views.frame(), opened.slot, click.kind, key_words(key), views.in_use());
        }
    }
    if opened.new {
        crate::view_options::taken(opened.slot);
    }
    if let Some(parts) = crate::view_options::scene(opened.slot, click) {
        // SAFETY: the caller is the scene start on the main thread; it writes the scene's own record next.
        unsafe { capture::frame_event(crate::link::EVENT_VIEW_OPTIONS, opened.slot as u32, parts) };
    }
    (opened.slot as u32 + 1, opened.first.map_or(0, |first| first as u32 + 1))
}

/// The view field of a scene on its route and the first slot of a repeat ([`view_open_with_first`]):
/// 0 and 0 for a scene on the engine's path, which the table does not serve (its displays set
/// no marks, `skipped_display`) and which takes no slot.
pub fn view_of_scene(mt: &MainThread, click: &Click, route: Route) -> (u32, u32) {
    if matches!(route, Route::Engine(_)) {
        return (0, 0);
    }
    view_open_with_first(mt, click)
}

/// The renderer's SwapBuffers, with the device's target (`None` for the back buffer): the open
/// views into it close, with their hides.
pub fn view_swap(mt: &MainThread, target: Option<usize>) {
    let closed = VIEWS.get(mt).as_mut().map(|views| views.swap(target)).unwrap_or_default();
    close_views(mt, closed);
}

/// A readback or a tile's copy runs the frame's work so far: the open views into a texture
/// close, with their hides before the flush's record, and the second slots go. Their
/// retirements wait for [`write_flush_retirements`] after the flush's own record, so that the
/// helper runs their views first.
pub fn view_flush(mt: &MainThread) {
    let Some(end) = VIEWS.get(mt).as_mut().map(|views| views.flush(false)) else { return };
    close_views(mt, end.closed);
    forget_views(mt, &end.retired);
    RETIRED_VIEWS.get(mt).extend(end.retired);
}

/// After the record of a flush (`READ_PIXELS`, `COPY_TILE`, a copy of pixels), and before a
/// scene takes a slot: the retirements that wait. Those of a flush come after its record, so
/// that the helper runs their views first. Those that waited for a recording stream
/// (`view_target_destroyed`) can come after other records, even after another scene opened:
/// that is safe only because their slots' items ran before the retirement was queued.
pub fn write_flush_retirements(mt: &MainThread) {
    let _lock = crate::sync::device_lock();
    for slot in take_retired_views(mt) {
        // SAFETY: only a recording stream reaches the write (`write_record` checks it), and
        // the device lock is held.
        unsafe { capture::frame_event(crate::link::EVENT_VIEW_RETIRE, slot as u32, 0) };
    }
}

/// The renderer data of the rendered texture `target` is destroyed (rule 3 of the slot
/// pressure): the views into it lose their keys, before a new texture can take the same
/// address. A slot whose items ran retires at once; the others retire after the next flush
/// record or `FRAME_END` ([`ViewTable::target_gone`]). Without a recording stream the
/// retirements wait for the next point that writes them.
///
/// The engine can destroy a rendered texture while this module holds a table: then the work
/// waits for the next `view_open` (`PENDING_TARGETS`, `PENDING_FORGETS`), before any slot is
/// taken, and `VIEW_TARGETS_WAITED` counts it. The retirements of slots whose marks wait go
/// after the marks clear, so that no state record sets their bits again in between.
pub fn view_target_destroyed(mt: &MainThread, target: usize) {
    let Some(mut views_open) = VIEWS.try_get(mt) else {
        PENDING_TARGETS.get(mt).push(target);
        VIEW_TARGETS_WAITED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return;
    };
    let retired = views_open.as_mut().map(|views| views.target_gone(target)).unwrap_or_default();
    drop(views_open);
    if retired.is_empty() {
        return;
    }
    match tables_try(mt) {
        Some(mut tables_open) => {
            for &slot in &retired {
                forget_view(&mut tables_open, slot);
            }
        }
        None => {
            // The retirements wait with the marks: while the marks keep bit v, a retirement in
            // the stream would let a restate (`reset`) set bit v again in the helper.
            PENDING_FORGETS.get(mt).extend_from_slice(&retired);
            VIEW_TARGETS_WAITED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            return;
        }
    }
    if !capture::recording() {
        RETIRED_VIEWS.get(mt).extend(retired);
        return;
    }
    let _lock = crate::sync::device_lock();
    for slot in retired {
        // SAFETY: the stream records (checked above) and the device lock is held.
        unsafe { capture::frame_event(crate::link::EVENT_VIEW_RETIRE, slot as u32, 0) };
    }
}

/// The destructors of a rendered texture's renderer data off the main thread: their views keep
/// their slots until the 60-frame rule frees them.
pub static VIEW_TARGETS_OFF_MAIN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Present, the end of the frame (`frame_tick`): the views close and the second and idle slots
/// retire.
fn view_present(mt: &MainThread) {
    let Some(end) = VIEWS.get(mt).as_mut().map(|views| views.present()) else { return };
    close_views(mt, end.closed);
    forget_views(mt, &end.retired);
    RETIRED_VIEWS.get(mt).extend(end.retired);
}

/// The slots that retired and whose retirement is not written yet.
pub fn take_retired_views(mt: &MainThread) -> Vec<u8> {
    core::mem::take(&mut *RETIRED_VIEWS.get(mt))
}

/// The report line of the view slots.
fn view_report(mt: &MainThread) -> String {
    let views_open = VIEWS.get(mt);
    let Some(views) = views_open.as_ref() else {
        return "views: no scene yet".to_owned();
    };
    format!(
        "views: {} of {VIEW_SLOTS} slots in use, {} slots retired, {} root changes; {} second slots (at most {} in one flush), {} slots taken back, {} flushes forced, {} slots of a destroyed target ({} destructors off the main thread, {} waited for an open table); hides at the closes: {} by the planes, {} after a root change; {} instances under a billboard displayed in two views of a frame (they draw with the last view's turn)",
        views.in_use(),
        views.retired,
        views.root_changes,
        views.second_slots,
        views.most_second_in_flush,
        views.taken_back,
        views.forced_flushes,
        views.targets_gone,
        VIEW_TARGETS_OFF_MAIN.load(core::sync::atomic::Ordering::Relaxed),
        VIEW_TARGETS_WAITED.load(core::sync::atomic::Ordering::Relaxed),
        CLOSE_HIDES[0].load(core::sync::atomic::Ordering::Relaxed),
        CLOSE_HIDES[1].load(core::sync::atomic::Ordering::Relaxed),
        BILLBOARD_IN_TWO_VIEWS.load(core::sync::atomic::Ordering::Relaxed)
    )
}

/// The shape is deleted (the destructor of `NiGeometry`): its instance goes, with a record
/// for the helper. `Instance::sent` does not say whether the helper holds the instance (a
/// reset clears it), and the helper ignores an id it does not hold, so the record always
/// goes. True when the shape was an instance.
pub unsafe fn forget(mt: &MainThread, shape: usize) -> bool {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    t.rejected.remove(&shape);
    if let Some(instance) = t.instances.remove(&shape) {
        t.lists.give_back(instance.list_key);
        t.note_end(&instance, "destroyed");
        if capture::recording() {
            capture::instance_delete(instance.id);
        }
        t.deletes += 1;
        return true;
    }
    false
}

/// The fast path of a draw, before the notes of the draw: true when the shape is a
/// retained instance whose handles need no check this frame. The handles of the draw are
/// then those of the instance, and the draw in progress is pending for `draw`.
pub unsafe fn seen(mt: &MainThread, shape: usize, kind: u32, class_out: &mut u32) -> Option<(u32, u32)> {
    if mode(mt) != Mode::Gpu {
        return None;
    }
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    let instance = t.instances.get_mut(&shape)?;
    // A change of MWSE's flag sends the geometry again, with the notes of the draw.
    let software = instance.skinned && crate::ports::draw::skinned_in_software(kind == crate::link::DRAW_STRIPS, rd(shape, GEOMETRY_MODEL_DATA));
    let software_changed = instance.software != software;
    instance.software = software;
    if software_changed || !instance.filled || !instance.sent || instance.live || instance.generation != t.generation || instance.id.wrapping_add(t.frame) % CHECK_FRAMES == 0 {
        return None;
    }
    *class_out = instance.class;
    t.pending = Some((shape, instance.id));
    t.fast_draws += 1;
    Some((instance.geometry, instance.material))
}

/// After the notes of a draw: decides if the shape is retained, and makes the draw in
/// progress pending for `draw` when it is. `property_state` and `effect_state` are those
/// of the renderer for the draw.
pub unsafe fn consider(mt: &MainThread, shape: usize, eligible: bool, skinned: bool, points: bool, class: u32, geometry: u32, material: u32, property_state: usize) {
    let cause = if !eligible {
        Some(0)
    } else if geometry == 0 {
        Some(1)
    } else if material == 0 {
        Some(2)
    } else if !enabled(mt) {
        Some(3)
    } else if !crate::material_stream::last_modelled(mt) {
        Some(4)
    } else {
        None
    };
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    if let Some(cause) = cause {
        t.skipped_by[cause] += 1;
        return;
    }
    if let Some(instance) = t.instances.get_mut(&shape) {
        instance.generation = t.generation;
        if instance.geometry != geometry || instance.material != material || instance.class != class {
            instance.geometry = geometry;
            instance.material = material;
            instance.class = class;
            instance.sent = false;
            t.handle_updates += 1;
        }
        t.pending = Some((shape, instance.id));
        return;
    }
    if t.rejected.contains(&shape) {
        return;
    }
    let mut live = false;
    if let Some(reason) = reject_reason(t, shape, property_state, skinned, &mut live) {
        t.rejected.insert(shape);
        t.rejected_by[reason] += 1;
        return;
    }
    // A particle system changes its particles every frame: live from the start.
    if points {
        live = true;
        t.particle_instances_made += 1;
    }
    if points && t.particle_instances_made <= 100 {
        let text: usize = rd(shape, 8);
        let name = if text != 0 { core::ffi::CStr::from_ptr(text as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
        let parent: usize = rd(shape, crate::gen::layouts::NiAVObject::parentNode);
        let parent_text: usize = if parent != 0 { rd(parent, crate::gen::layouts::NiObjectNET::name) } else { 0 };
        let parent_name = if parent_text != 0 { core::ffi::CStr::from_ptr(parent_text as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
        crate::log!("particles: instance {} of class {class}, '{name}' under '{parent_name}'", t.next_id);
    }
    // The engine changes the sky by writes of its vertices with a new revision, not with
    // controllers: a sky instance is live from the start.
    if sky_table_class(class) {
        live = true;
        let text: usize = rd(shape, 8);
        let name = if text != 0 { core::ffi::CStr::from_ptr(text as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
        crate::log!("sky: instance {} of class {class}, '{name}'", t.next_id);
    }
    let id = t.next_id;
    t.next_id = t.next_id.wrapping_add(1).max(1);
    let generation = t.generation;
    let mut instance = Instance::new(id, class, geometry, material, generation, skinned, live, points);
    instance.under_billboard = under_billboard(shape);
    // The classification reads the shape as it is now: the notes before it are done.
    instance.checked = change_count(mt);
    t.instances.insert(shape, instance);
    t.pending = Some((shape, id));
}

/// Why the table does not take the shape yet, or [`None`] when it does. Each reason is a class
/// of shapes left on the engine's path: unfinished work, not a rule to keep (charter 1). `live`
/// is set when a controller on the shape or on a property changes it.
/// Engine-rules R1, R6, R7 (the skill wgpu-renderer-dev).
unsafe fn reject_reason(t: &Tables, shape: usize, property_state: usize, skinned: bool, live: &mut bool) -> Option<usize> {
    // A controller on the shape changes its data (a morpher, a UV controller): the shape is
    // live. A controller on a node above only moves the shape, and `skipped_display` moves
    // the instance in place.
    if rd::<usize>(shape, CONTROLLERS) != 0 {
        if !t.controllers {
            return Some(REASON_CONTROLLER);
        }
        *live = true;
    }
    let animated = t.animated;
    let above = crate::parent_chain::first_answer(
        rd(shape, PARENT),
        crate::parent_chain::GUARD,
        |node| rd(node, PARENT),
        |node| {
            if !animated && !skinned && rd::<usize>(node, CONTROLLERS) != 0 {
                Some(REASON_CONTROLLER)
            } else if kind_rejected(t.node_kinds, node, skinned) {
                Some(REASON_NODE_KIND)
            } else {
                None
            }
        },
    );
    match above {
        Ok(Some(reason)) => return Some(reason),
        Ok(None) => {}
        Err(crate::parent_chain::ChainTooLong) => {
            crate::parent_chain::note_too_long("the table's reject test");
            return Some(REASON_CHAIN);
        }
    }
    if property_state != 0 {
        for slot in 0..STATE_COUNT {
            let property: usize = rd(property_state, STATE_FIRST + slot * 4);
            if property != 0 && rd::<usize>(property, CONTROLLERS) != 0 {
                // A flip, alpha or colour controller changes the material record: live.
                if !t.controllers {
                    return Some(REASON_PROPERTY_CONTROLLER);
                }
                *live = true;
            }
        }
        // The test function of NiZBufferProperty is the NetImmerse enum, not D3DCMPFUNC:
        // 0 always, 1 less, 2 equal, 3 less or equal, 4 greater, 5 not equal, 6 greater or
        // equal, 7 never. With the test off (flags bit 0) the engine uses "always", so the
        // function counts only with the test on (engine-rules R1).
        let z: usize = rd(property_state, PS::propZBuffer);
        if z != 0 && rd::<u16>(z, PROPERTY_FLAGS) & 1 != 0 && !matches!(rd::<u32>(z, ZP::testFunction), 0 | 1 | 3 | 7) {
            return Some(REASON_DEPTH);
        }
    }
    // A texture effect is retained in any coordinate mode (engine-rules R24). The material record
    // holds the projection, or, when it moves in the play, names the chain the helper makes it from
    // (`effect_chain`).
    None
}

/// The switch of the sky (`NIRENDER_SKY`): 0 not read, 1 off (`off`), 2 the census (`count`),
/// 3 the sky's instances in the table (the default; milestone 6).
static SKY_MODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
/// The sky shapes the census logged: once each.
static SKY_SEEN: MainCell<Vec<usize>> = MainCell::new(Vec::new());

fn sky_mode() -> u8 {
    use core::sync::atomic::Ordering::Relaxed;
    match SKY_MODE.load(Relaxed) {
        0 => {
            let mode = match crate::switches::choice("NIRENDER_SKY", &["off", "count", "on"]) {
                Some("off") => 1,
                Some("count") => 2,
                _ => 3,
            };
            SKY_MODE.store(mode, Relaxed);
            mode
        }
        mode => mode,
    }
}

/// True when the census of the sky is on.
pub fn sky_census_on() -> bool {
    sky_mode() == 2
}

/// True when the table takes the sky's shapes (milestone 6): each draw of a sky shape goes as
/// an `INSTANCE_DRAW` record in the place of the engine's shape record (`draw`).
pub fn sky_table_on() -> bool {
    sky_mode() == 3
}

/// The draws of the sky instances go as `INSTANCE_DRAW` records (`nirender.skyDraws`; on by
/// default): off, they go as the engine's shape records, for an A/B in one session.
static SKY_DRAWS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// True when the draws of the sky instances go as `INSTANCE_DRAW` records.
pub fn sky_draws() -> bool {
    SKY_DRAWS.load(core::sync::atomic::Ordering::Relaxed)
}

/// Sets whether the draws of the sky instances go as `INSTANCE_DRAW` records.
pub fn set_sky_draws(on: bool) {
    SKY_DRAWS.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// The switch of the weather's shapes, the rain, snow and storm drops of class 9
/// (`NIRENDER_WEATHER`): 0 not read, 1 off (the default: the engine's
/// shape records), 2 the census (`count`), 3 the drops' instances in the
/// table (`on`), drawn as the sky's are.
static WEATHER_MODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

fn weather_mode() -> u8 {
    use core::sync::atomic::Ordering::Relaxed;
    match WEATHER_MODE.load(Relaxed) {
        0 => {
            let mode = match crate::switches::choice("NIRENDER_WEATHER", &["off", "count", "on"]) {
                Some("count") => 2,
                Some("on") => 3,
                _ => 1,
            };
            WEATHER_MODE.store(mode, Relaxed);
            mode
        }
        mode => mode,
    }
}

/// True when the census of the weather's shapes is on.
pub fn weather_census_on() -> bool {
    weather_mode() == 2
}

/// True when the table takes the weather's shapes: each draw of a drop goes as an
/// `INSTANCE_DRAW` record in the place of the engine's shape record (`draw`), as a sky shape's.
pub fn weather_table_on() -> bool {
    weather_mode() == 3
}

/// The draws of the weather instances go as `INSTANCE_DRAW` records (`nirender.weatherDraws`;
/// on when the table takes them): off, they go as the engine's shape records, for an A/B in one
/// session.
static WEATHER_DRAWS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// True when the draws of the weather instances go as `INSTANCE_DRAW` records.
pub fn weather_draws() -> bool {
    WEATHER_DRAWS.load(core::sync::atomic::Ordering::Relaxed)
}

/// Sets whether the draws of the weather instances go as `INSTANCE_DRAW` records.
pub fn set_weather_draws(on: bool) {
    WEATHER_DRAWS.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// The switch of the particle systems (`NIRENDER_PARTICLES`; milestone 5): 0 not read, 1 off
/// (`off`: the engine's shape records), 2 the table (the default since step 5: a particle system
/// in a world camera's scene is an instance whose draws go as `INSTANCE_PARTICLES` and
/// `INSTANCE_DRAW` records at the places of the engine's draws).
static PARTICLE_MODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// True when the table takes the particle systems.
pub fn particle_table_on() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    let mode = match PARTICLE_MODE.load(Relaxed) {
        0 => {
            let mode = if crate::switches::choice("NIRENDER_PARTICLES", &["off", "on"]) == Some("off") { 1 } else { 2 };
            PARTICLE_MODE.store(mode, Relaxed);
            mode
        }
        mode => mode,
    };
    mode == 2
}

/// The draws of the particle instances go as `INSTANCE_DRAW` records (`nirender.particleDraws`;
/// on when the table takes them): off, they go as the engine's shape records, for an A/B in one
/// session.
static PARTICLE_DRAWS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// True when the draws of the particle instances go as `INSTANCE_DRAW` records.
pub fn particle_draws() -> bool {
    PARTICLE_DRAWS.load(core::sync::atomic::Ordering::Relaxed)
}

/// Sets whether the draws of the particle instances go as `INSTANCE_DRAW` records.
pub fn set_particle_draws(on: bool) {
    PARTICLE_DRAWS.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// What the table knows of a shape, for a log line: an instance (with its id and kind),
/// rejected, or not considered.
pub unsafe fn describe(mt: &MainThread, shape: usize) -> String {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    if let Some(instance) = t.instances.get(&shape) {
        format!("instance {} (particles {}, class {}, live {})", instance.id, instance.points, instance.class, instance.live)
    } else if t.rejected.contains(&shape) {
        "rejected".to_string()
    } else {
        "not considered".to_string()
    }
}

/// True when the shape is a particle instance (`shape_stream::record` sends its particles
/// before its `INSTANCE_DRAW`, or keeps the engine's record when `particle_draws` is off).
pub unsafe fn is_points(mt: &MainThread, shape: usize) -> bool {
    tables(mt).instances.get(&shape).is_some_and(|instance| instance.points)
}

/// A class of the sky the table can take: the sky root's shapes in the world camera's walk.
/// The glare (class 8) draws in the first-person scene.
fn sky_table_class(class: u32) -> bool {
    (CLASS_SKY_DOME..=CLASS_SKY_OTHER).contains(&class)
}

/// The census of the sky (milestone 6, step 0) and of the weather's shapes (class 9): what the
/// table would say about a shape of those classes, without a mark or a skip; `what` names the
/// set in the log line ("sky", "weather"). Logs each shape once: the name, the class,
/// the handles, whether the material is modelled, the reject reason, live, the depth and alpha
/// flags, and the controllers on the shape, above it and on its properties.
pub unsafe fn sky_census(mt: &MainThread, what: &str, shape: usize, class: u32, plain_world_shape: bool, geometry: u32, material: u32, property_state: usize) {
    let seen = &mut *SKY_SEEN.get(mt);
    if seen.contains(&shape) {
        return;
    }
    seen.push(shape);
    let text: usize = rd(shape, 8);
    let name = if text != 0 { core::ffi::CStr::from_ptr(text as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
    let mut live = false;
    let reason = reject_reason(&tables(mt), shape, property_state, false, &mut live);
    let (z_flags, z_function) = match if property_state != 0 { rd::<usize>(property_state, PS::propZBuffer) } else { 0 } {
        0 => (None, None),
        z => (Some(rd::<u16>(z, PROPERTY_FLAGS)), Some(rd::<u32>(z, ZP::testFunction))),
    };
    let alpha_flags = match if property_state != 0 { rd::<usize>(property_state, PS::propAlpha) } else { 0 } {
        0 => None,
        alpha => Some(rd::<u16>(alpha, PROPERTY_FLAGS)),
    };
    let mut node_controllers = 0;
    let mut node: usize = rd(shape, PARENT);
    for _ in 0..crate::parent_chain::GUARD {
        if node == 0 {
            break;
        }
        if rd::<usize>(node, CONTROLLERS) != 0 {
            node_controllers += 1;
        }
        node = rd(node, PARENT);
    }
    let property_controllers = if property_state != 0 { (0..STATE_COUNT).filter(|slot| { let p: usize = rd(property_state, STATE_FIRST + slot * 4); p != 0 && rd::<usize>(p, CONTROLLERS) != 0 }).count() } else { 0 };
    crate::log!(
        "{what} census: '{name}' class {class}, world shape {plain_world_shape}, geometry {geometry:#x}, material {material:#x}, modelled {}, reject {reason:?}, live {live}, zbuffer flags {z_flags:?} function {z_function:?}, alpha flags {alpha_flags:?}, controllers: shape {}, nodes above {node_controllers}, properties {property_controllers}",
        crate::material_stream::last_modelled(mt),
        rd::<usize>(shape, CONTROLLERS) != 0
    );
}

/// True when the run-time type of the node, or a base of it, is one of the rejected kinds
/// (with `NIRENDER_NODE_KINDS=0`, the `*_BEFORE` lists: the narrower one for a skinned shape).
unsafe fn kind_rejected(node_kinds: bool, node: usize, skinned: bool) -> bool {
    let kinds: &[&[u8]] = if node_kinds {
        &REJECTED_KINDS
    } else if skinned {
        &REJECTED_KINDS_SKINNED_BEFORE
    } else {
        &REJECTED_KINDS_BEFORE
    };
    let mut rtti = crate::ports::ni::get_rtti(node);
    for _ in 0..16 {
        if rtti == 0 {
            return false;
        }
        let name: usize = rd(rtti, RTTI_NAME);
        if name != 0 {
            let text = core::ffi::CStr::from_ptr(name as *const core::ffi::c_char).to_bytes();
            if kinds.iter().any(|kind| text.ends_with(kind)) {
                return true;
            }
        }
        rtti = rd(rtti, RTTI_BASE);
    }
    false
}

/// Takes the draw in progress when it is of a retained shape.
pub unsafe fn take_pending(mt: &MainThread) -> Option<(usize, u32)> {
    tables(mt).pending.take()
}

/// The causes of a rejection at the record, for the report: no geometry handle, no material
/// handle, a material the helper does not model, no exact translation.
static LATER_BY: MainCell<[u32; 4]> = MainCell::new([0; 4]);
const LATER_GEOMETRY: usize = 0;
const LATER_MATERIAL: usize = 1;
const LATER_NOT_MODELLED: usize = 2;
const LATER_TRANSLATION: usize = 3;

/// `reject_pending` with the cause counted.
unsafe fn reject_at_record(mt: &MainThread, t: &mut Tables, shape: usize, cause: usize) {
    (*LATER_BY.get(mt))[cause] += 1;
    t.reject_pending(shape, true);
}

/// The lines of `Tables::note_end` in the log of a session; the report counts them all.
const MOST_END_LINES: u64 = 64;

impl Tables {
    /// An instance that the helper holds ends while `sent` is clear: the case in which a delete
    /// once did not go. The id is that of the helper's line of an orphan (`INSTANCE_CENSUS`).
    fn note_end(&mut self, instance: &Instance, how: &str) {
        if !instance.filled || instance.sent {
            return;
        }
        self.ends_without_record += 1;
        if self.ends_without_record <= MOST_END_LINES {
            crate::log!("instances: instance {} {how} with no record since a reset: class {}, geometry {:#x}, visible in the views {:#x}", instance.id, instance.class, instance.geometry, instance.visible);
        }
    }

    /// The shape of a pending draw turned out not retainable at the record (a kind of draw
    /// other than a shape, a picture of the Code Patch, no handle): it goes to the rejected
    /// set.
    unsafe fn reject_pending(&mut self, shape: usize, reason_later: bool) {
        mark(shape, false);
        // The record always goes, as in `forget`.
        if let Some(instance) = self.instances.remove(&shape) {
            self.lists.give_back(instance.list_key);
            self.note_end(&instance, "rejected");
            if capture::recording() {
                capture::instance_delete(instance.id);
            }
        }
        self.rejected.insert(shape);
        if reason_later {
            self.rejected_by[REASON_LATER] += 1;
        }
    }
}

/// See `Tables::reject_pending`.
pub unsafe fn reject_pending(mt: &MainThread, shape: usize, reason_later: bool) {
    tables(mt).reject_pending(shape, reason_later);
}

/// The records of a draw of a retained shape: the `INSTANCE` record when the values of the
/// draw differ from the retained ones or the record was not sent, then `INSTANCE_DRAW`.
/// `world` is the world matrix of the draw as the renderer has it (relative to the origin
/// of the scene when the plugin makes the matrices: `relative` is the flag of the shape
/// record), `effects` the stages of the texture effects of the draw and `fog` its fog, for
/// an `INSTANCE_DRAW` record.
/// Returns false when the shape has no exact translation (the draw then goes as a shape
/// record, and the shape is rejected).
pub unsafe fn draw(mt: &MainThread, shape: usize, id: u32, world: &[f32; 16], relative: bool, colours: &[u32; 14], effects: Option<&[u32]>, fog: Option<[u32; 4]>, ambient: u32, lights: &[u32], bones: &[u8], bone_count: u32, queued: bool) -> bool {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    let Some(instance) = t.instances.get_mut(&shape) else { return false };
    if sky_table_class(instance.class) && !sky_draws() {
        return false;
    }
    if instance.class == CLASS_WEATHER_PARTICLES && !weather_draws() {
        return false;
    }
    if instance.points && !particle_draws() {
        return false;
    }
    // A skinned shape's world matrix is the identity (the palette has the transforms), so
    // only its colours and its first record send the instance again. In mode gpu the table
    // draws it with the bones of `send_bones`; with `NIRENDER_SKIN_TABLE=0` it stays on the
    // draw records, with its palette in each draw.
    let skinned = instance.skinned;
    // A sky instance goes as an `INSTANCE_DRAW` record at the place of the engine's draw (step
    // 2 of milestone 6): the helper draws it from its record there, in the stream's order. In
    // mode gpu the caller writes no shape record after it.
    // A particle instance goes the same way: its particles change every frame (step 2 of the
    // particles plan). A weather instance too: the drops blend, so they keep the stream's order.
    let at_place = sky_table_class(instance.class) || instance.class == CLASS_WEATHER_PARTICLES || instance.points;
    let gpu = t.mode == Some(Mode::Gpu) && (!skinned || t.skin_table) && !at_place;
    let rotation: [u32; 12] = core::array::from_fn(|i| world[i].to_bits());
    let stored: [u32; 3] = if skinned { [0; 3] } else { rd(shape, WORLD_TRANSLATION) };
    let bound: [f32; 4] = if skinned { [0.0; 4] } else { rd(shape, WORLD_BOUND) };
    let changed = !instance.filled || instance.rotation != rotation || instance.stored != stored || instance.colours != *colours || instance.bound.map(f32::to_bits) != bound.map(f32::to_bits);
    if changed || !instance.sent {
        let exact = if skinned {
            [0.0; 3]
        } else {
            let Some(exact) = crate::camera_space::exact_translation(mt, shape, 0) else {
                reject_at_record(mt, t, shape, LATER_TRANSLATION);
                return false;
            };
            exact
        };
        if instance.filled && changed {
            t.value_updates += 1;
            if t.value_updates <= 4 {
                let what = if instance.rotation != rotation { "rotation" } else if instance.stored != stored { "translation" } else { "colours" };
                crate::log!("instances: the {what} of instance {id} changed (shape 0x{shape:x})");
            }
        }
        instance.rotation = rotation;
        instance.stored = stored;
        instance.colours = *colours;
        instance.bound = bound;
        instance.transform = rd(shape, WORLD);
        instance.filled = true;
        instance.sent = true;
        // The flag as the shape record of the draw has it: the helper takes the translation
        // of the two the same way. In mode gpu the helper draws the instance from its table;
        // a skinned instance has the flag `SKINNED` and an identity world.
        let flags = if relative { crate::shape_stream::RELATIVE } else { 0 } | if gpu { BLOCK } else { 0 } | if skinned { crate::shape_stream::SKINNED } else { 0 };
        let mut words = [0u32; 44];
        words[0] = id;
        words[1] = instance.class;
        words[2] = flags;
        words[3] = instance.geometry;
        words[4] = instance.material;
        words[5..17].copy_from_slice(&rotation);
        for (i, value) in exact.iter().enumerate() {
            let bits = value.to_bits();
            words[17 + i * 2] = bits as u32;
            words[18 + i * 2] = (bits >> 32) as u32;
        }
        words[23..26].copy_from_slice(&stored);
        words[26..40].copy_from_slice(colours);
        words[40..44].copy_from_slice(&bound.map(f32::to_bits));
        let name_at: usize = rd(shape, 8);
        let name: &[u8] = if name_at != 0 { core::ffi::CStr::from_ptr(name_at as *const core::ffi::c_char).to_bytes() } else { &[] };
        capture::instance(&words, &name[..name.len().min(96)]);
        t.records += 1;
        restate(instance, &mut t.states);
    }
    if gpu {
        // The helper draws the instance: the changes of its lights and its visibility. From
        // now on the engine does not display the shape (`skipped_display`).
        if t.skip {
            mark(shape, true);
        }
        if instance.lights != lights || instance.ambient != ambient {
            instance.lights.clear();
            instance.lights.extend_from_slice(lights);
            instance.ambient = ambient;
            capture::instance_lights(id, lights, ambient);
            t.light_records += 1;
        }
        // A skinned instance whose bones cannot be made is hidden (`send_bones`): it does not
        // join the main view's list.
        if !skinned || send_bones(mt, t, shape, id) {
            shown_in_scene(t, shape);
        }
        // In a scene of `Route::Places` every displayed shape has its place, also at this draw
        // (the first one, or one after a change), which does not go through `skipped_display`.
        if t.scene_route == Route::Places {
            if queued {
                capture::instance_queued(id);
                QUEUE_COUNTS.get(mt)[1] += 1;
            } else {
                capture::instance_walk(id);
                *VIEW_PLACES.get(mt) += 1;
            }
        }
        return true;
    }
    capture::instance_draw(id, ambient, effects, fog, lights, bones, bone_count, queued);
    if instance.points {
        t.particle_draws += 1;
    }
    true
}

/// How a move of a sent instance goes (`NIRENDER_COMPACT_MOVES`): as `INSTANCE_MOVE` (the
/// default); as the whole `INSTANCE` record (`0`, as before, for the A/B); or both (`check`:
/// the helper compares the values of the two).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CompactMoves {
    On,
    Off,
    Check,
}

fn compact_moves() -> CompactMoves {
    static MODE: std::sync::OnceLock<CompactMoves> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match crate::switches::choice("NIRENDER_COMPACT_MOVES", &["0", "1", "check"]) {
        Some("0") => CompactMoves::Off,
        Some("check") => CompactMoves::Check,
        _ => CompactMoves::On,
    })
}

/// Sends the `INSTANCE_MOVE` record of a sent instance: its rotation, its exact translation,
/// its stored translation and its bound.
unsafe fn send_move(instance: &Instance, exact: [f64; 3]) {
    let mut words = [0u32; 26];
    words[0] = instance.id;
    words[1..13].copy_from_slice(&instance.rotation);
    for (i, value) in exact.iter().enumerate() {
        let bits = value.to_bits();
        words[13 + i * 2] = bits as u32;
        words[14 + i * 2] = (bits >> 32) as u32;
    }
    words[19..22].copy_from_slice(&instance.stored);
    words[22..26].copy_from_slice(&instance.bound.map(f32::to_bits));
    capture::instance_move(&words);
}

/// Sends the words of the `INSTANCE` record of an instance of the mode gpu.
unsafe fn send_instance(mt: &MainThread, shape: usize, instance: &Instance, exact: [f64; 3], relative: bool) {
    let flags = if relative { crate::shape_stream::RELATIVE } else { 0 } | BLOCK | if instance.skinned { crate::shape_stream::SKINNED } else { 0 };
    let mut words = [0u32; 44];
    words[0] = instance.id;
    words[1] = instance.class;
    words[2] = flags;
    words[3] = instance.geometry;
    words[4] = instance.material;
    words[5..17].copy_from_slice(&instance.rotation);
    for (i, value) in exact.iter().enumerate() {
        let bits = value.to_bits();
        words[17 + i * 2] = bits as u32;
        words[18 + i * 2] = (bits >> 32) as u32;
    }
    words[23..26].copy_from_slice(&instance.stored);
    words[26..40].copy_from_slice(&instance.colours);
    words[40..44].copy_from_slice(&instance.bound.map(f32::to_bits));
    let name_at: usize = rd(shape, 8);
    let name: &[u8] = if name_at != 0 { core::ffi::CStr::from_ptr(name_at as *const core::ffi::c_char).to_bytes() } else { &[] };
    (*RECORD_COUNTS.get(mt))[RC_NAME_BYTES] += name.len().min(96) as u64;
    capture::instance(&words, &name[..name.len().min(96)]);
}

/// The lights of an effect state that the light manager can turn on for a draw: not an
/// ambient light; in the order of the manager's list (the last of the state first). A light
/// that is off or too dim stays in the list: its record has zero colours
/// (`note_light_once`), so that it gives light again when it is turned on, with no new work
/// of the instance. The light manager's other gates (`setState` 0x6BB200: the replace mode
/// of the base map, the lighting mode with `useAmbientDiffuse` for `updateLights` 0x6BB6C0)
/// are rules of the helper's material (`host/src/material.rs`, `derive`): the lights are sent,
/// and a material that the engine lights with no lights does not use them.
unsafe fn state_lights(effect_state: usize, out: &mut Vec<usize>) {
    out.clear();
    if effect_state == 0 {
        return;
    }
    let mut iter: usize = rd(effect_state, DES::lights);
    while iter != 0 && out.len() < MOST_LIGHTS {
        let light: usize = rd(iter, DL::effect);
        iter = rd(iter, DL::next);
        if light == 0 {
            continue;
        }
        if crate::ports::light_manager::light_get_type(light) == 0 {
            continue;
        }
        out.push(light);
    }
    out.reverse();
}

/// The highest light index noted (the indexes of the lights of the game are small numbers).
const MOST_LIGHT_INDEX: usize = 1 << 20;

/// Notes a light of the game once a frame: its device light as the light manager builds its
/// entry, and its LIGHT record when the values changed (`light_stream::note_values`). Keyed
/// by the index of the light, an array and not a hash: a frame notes each light of each
/// retained shape.
unsafe fn note_light_once(mt: &MainThread, t: &mut Tables, light: usize, key: u32) {
    let counts = &mut *LIGHT_COUNTS.get(mt);
    counts[LC_NOTES] += 1;
    let k = key as usize;
    if k >= MOST_LIGHT_INDEX {
        return;
    }
    if t.light_noted.len() <= k {
        t.light_noted.resize(k + 1, u32::MAX);
    }
    if t.light_noted[k] == t.frame {
        return;
    }
    t.light_noted[k] = t.frame;
    counts[LC_BUILT] += 1;
    let entry = t.scratch_entry.as_mut_ptr() as usize;
    core::ptr::write_bytes(entry as *mut u8, 0, E::SIZE);
    wr::<u32>(entry, E::revisionId, !rd::<u32>(light, L::revisionID));
    crate::ports::light_manager::light_entry_update(entry, light);
    let mut values: [u32; 26] = rd(entry, E::d3dLight);
    // A light that is off or too dim gives no light, as the light manager does not turn it on
    // for a draw: the instances keep its key, and the record has zero colours.
    if rd::<u8>(light, L::enabled) == 0 || rd::<f32>(light, L::dimmer) < f32::from_bits(crate::ports::light_manager::MIN_DIMMER) {
        values[crate::light_stream::COLOUR_WORDS].fill(0);
        counts[LC_DARK] += 1;
    }
    if crate::light_stream::note_values(mt, key, &values, crate::link_client::point_light_radius(light)) {
        counts[LC_SENT] += 1;
    }
}

/// The counts of the instance records, by `RC_*`: why a record went (the first
/// send or a new handle; a move of the transform; the bound alone; the colours alone; a move
/// and the colours), whether a moved instance is live or not; the bytes of the names; the
/// records that went as `INSTANCE_MOVE`.
static RECORD_COUNTS: MainCell<[u64; 9]> = MainCell::new([0; 9]);
const RC_FIRST: usize = 0;
const RC_MOVED: usize = 1;
const RC_BOUND: usize = 2;
const RC_COLOURS: usize = 3;
const RC_MOVED_COLOURS: usize = 4;
const RC_LIVE: usize = 5;
const RC_STILL: usize = 6;
const RC_NAME_BYTES: usize = 7;
const RC_COMPACT_MOVES: usize = 8;
/// The time from the decision to send a record to its end, on the main thread: the matrix
/// (the engine's conversion of the world transform), the exact translation, the record.
static RECORD_TICKS: MainCell<[i64; 3]> = MainCell::new([0; 3]);

unsafe fn note_record_cause(mt: &MainThread, instance: &Instance, moved: bool, bound: bool, colours: bool) {
    let c = &mut *RECORD_COUNTS.get(mt);
    let cause = match (instance.sent, moved, bound, colours) {
        (false, ..) => RC_FIRST,
        (true, true, _, false) => RC_MOVED,
        (true, true, _, true) => RC_MOVED_COLOURS,
        (true, false, true, false) => RC_BOUND,
        _ => RC_COLOURS,
    };
    c[cause] += 1;
    if instance.sent && moved {
        c[if instance.live { RC_LIVE } else { RC_STILL }] += 1;
    }
}

pub unsafe fn record_report(mt: &MainThread) -> String {
    let c = *RECORD_COUNTS.get(mt);
    let parts = (*RECORD_TICKS.get(mt)).map(|t| t as f64 * 1000.0 / capture::ticks_per_second() as f64);
    let ms: f64 = parts.iter().sum();
    let total: u64 = c[RC_FIRST..=RC_MOVED_COLOURS].iter().sum();
    format!(
        "instance records: {} ({:.3} ms, {:.2} us each: the matrix {:.3} ms, the exact translation {:.3}, the record {:.3}); first or new handle {}, moved {}, bound alone {}, colours alone {}, moved and colours {}; the moved: live {}, not live {}; name bytes {}; as INSTANCE_MOVE {}",
        total,
        ms,
        ms * 1000.0 / total.max(1) as f64,
        parts[0],
        parts[1],
        parts[2],
        c[RC_FIRST],
        c[RC_MOVED],
        c[RC_BOUND],
        c[RC_COLOURS],
        c[RC_MOVED_COLOURS],
        c[RC_LIVE],
        c[RC_STILL],
        c[RC_NAME_BYTES],
        c[RC_COMPACT_MOVES]
    )
}

/// The counts of the lights, by `LC_*`: the light loops of the displays and their
/// lights; the notes of a light, the entries built (one a light a frame), those with zero
/// colours (off or too dim), and the records sent.
static LIGHT_COUNTS: MainCell<[u64; 6]> = MainCell::new([0; 6]);
const LC_LOOPS: usize = 0;
const LC_LOOP_LIGHTS: usize = 1;
const LC_NOTES: usize = 2;
const LC_BUILT: usize = 3;
const LC_DARK: usize = 4;
const LC_SENT: usize = 5;
/// The time of the light loops of the displays.
static LIGHT_TICKS: MainCell<i64> = MainCell::new(0);

pub unsafe fn light_report(mt: &MainThread) -> String {
    let c = *LIGHT_COUNTS.get(mt);
    let ms = *LIGHT_TICKS.get(mt) as f64 * 1000.0 / capture::ticks_per_second() as f64;
    format!(
        "lights: display loops {} ({:.2} lights each, {:.3} ms, {:.2} us each); notes {}, entries built {} ({} with zero colours), records sent {}",
        c[LC_LOOPS],
        c[LC_LOOP_LIGHTS] as f64 / c[LC_LOOPS].max(1) as f64,
        ms,
        ms * 1000.0 / c[LC_LOOPS].max(1) as f64,
        c[LC_NOTES],
        c[LC_BUILT],
        c[LC_DARK],
        c[LC_SENT]
    )
}

/// Why an instance is checked again.
#[derive(Clone, Copy)]
pub enum Recheck {
    Properties = 0,
    Effects = 1,
    /// A Lua mod changed the shape or a property of it (`nirender.changed`).
    Notified = 2,
}

/// The rechecks asked for (by cause: 0 a property state, 1 an effect state, 2 a Lua
/// change), the rechecks done (3), the instances they rejected (4), and the instances whose
/// live state changed (5).
static RECHECKS: MainCell<[u64; 6]> = MainCell::new([0; 6]);

/// The objects a Lua mod changed (`nirender.changed`, from
/// `mod/MWSE/mods/nirender/notifier.lua`): [`ChangeNotes`].
static CHANGES: MainCell<Option<ChangeNotes>> = MainCell::new(None);
/// The counts of the display checks of the notes and of the shapes they hit.
static CHANGE_NOTES: MainCell<[u64; 2]> = MainCell::new([0; 2]);
/// The processor ticks of the display checks: all of them, this frame's, and the worst
/// frame's.
static CHANGE_TICKS: MainCell<[i64; 3]> = MainCell::new([0; 3]);
/// The changed objects that [`ChangeNotes`] keeps; one more clears them all.
const CHANGED_CAP: usize = 4096;

/// The objects that Lua changed, each with its stamp: the count of notes at its last note.
/// A retained shape keeps the count at its last check ([`Instance::checked`]). At its next
/// display it looks up itself and its properties ([`ChangeNotes::hits`]). No scan of the
/// table and no index of the properties' holders: the check runs only at the displays.
#[derive(Default)]
struct ChangeNotes {
    /// The notes received. The stamp of the last note.
    count: u64,
    /// The changed objects and their stamps, at most [`CHANGED_CAP`].
    changed: HashMap<usize, u64>,
    /// The stamp of the note that cleared `changed`: a shape checked before it checks again.
    cleared: u64,
    /// The number of clears.
    clears: u64,
}

impl ChangeNotes {
    fn note(&mut self, address: usize) {
        self.count += 1;
        if self.changed.len() >= CHANGED_CAP && !self.changed.contains_key(&address) {
            self.changed.clear();
            self.cleared = self.count;
            self.clears += 1;
        }
        self.changed.insert(address, self.count);
    }

    /// A shape checked at stamp `checked` must be checked again: a note after that stamp
    /// named the shape or one of its properties, or cleared the changed objects. A freed
    /// property's address used again only gives a recheck too many.
    fn hits(&self, checked: u64, shape: usize, properties: &[usize; PROPERTY_SLOTS]) -> bool {
        let after = |address: usize| address != 0 && self.changed.get(&address).is_some_and(|&stamp| stamp > checked);
        self.cleared > checked || after(shape) || properties.iter().any(|&property| after(property))
    }
}

/// The count of notes so far: the stamp of a new instance, which a note before it does not
/// hit.
unsafe fn change_count(mt: &MainThread) -> u64 {
    CHANGES.get(mt).as_ref().map_or(0, |notes| notes.count)
}

/// At a display of a retained shape: if a note came since its last check, a note of the
/// shape or of one of its `properties` marks it for a recheck at this display. In a frame
/// without notes, one compare.
unsafe fn check_notes(mt: &MainThread, instance: &mut Instance, shape: usize, properties: &[usize; PROPERTY_SLOTS]) {
    let changes_open = CHANGES.get(mt);
    let Some(notes) = changes_open.as_ref() else { return };
    if instance.checked >= notes.count {
        return;
    }
    let start = capture::ticks();
    let hit = notes.hits(instance.checked, shape, properties);
    instance.checked = notes.count;
    let counts = &mut *CHANGE_NOTES.get(mt);
    counts[0] += 1;
    if hit && !instance.recheck {
        instance.recheck = true;
        (*RECHECKS.get(mt))[Recheck::Notified as usize] += 1;
        counts[1] += 1;
    }
    let ticks = &mut *CHANGE_TICKS.get(mt);
    let pass = capture::ticks() - start;
    ticks[0] += pass;
    ticks[1] += pass;
}

/// The end of a frame for the cost of the display checks: the worst frame.
unsafe fn change_frame_end(mt: &MainThread) {
    let ticks = &mut *CHANGE_TICKS.get(mt);
    ticks[2] = ticks[2].max(ticks[1]);
    ticks[1] = 0;
}
/// The change notes made on another thread, for the main thread's next `frame_tick`.
static OFF_THREAD_CHANGES: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// A change note from a thread that is not the main thread (a Lua handler of an event that
/// MWSE fires on the cell worker): queued, never applied here.
pub fn queue_change(address: usize) {
    if address != 0 {
        if let Ok(mut queue) = OFF_THREAD_CHANGES.lock() {
            queue.push(address);
        }
    }
}

/// A Lua mod changed the object at `address` (a shape, a property, a controller's target).
/// The main thread only: other threads use `queue_change`.
pub unsafe fn note_change(mt: &MainThread, address: usize) {
    if address == 0 {
        return;
    }
    (*CHANGES.get(mt)).get_or_insert_with(ChangeNotes::default).note(address);
}

/// The materials of these handles named a device texture that was freed
/// (`material_stream::drain_freed_textures`): the instances that use one note their material
/// again at their next display, as at a change of the device textures. Before, they waited
/// for the note of every 64th frame, and the helper drew them with the freed texture until
/// then (a pinned slot keeps its view while an instance uses it).
pub fn note_materials_again(mt: &MainThread, materials: &[u32]) {
    let gone: HashSet<u32> = materials.iter().copied().collect();
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    let generation = t.generation;
    let marked = mark_for_note(t.instances.values_mut().map(|i| (i.material, &mut i.generation)), &gone, generation);
    t.texture_notes += marked;
}

/// Gives each instance whose material is in `gone` another generation than `generation`, so
/// that its next display notes its handles (`display_work`). Returns their number.
fn mark_for_note<'a>(instances: impl Iterator<Item = (u32, &'a mut u32)>, gone: &HashSet<u32>, generation: u32) -> u64 {
    let mut marked = 0;
    for (material, instance_generation) in instances {
        if gone.contains(&material) {
            *instance_generation = generation.wrapping_sub(1);
            marked += 1;
        }
    }
    marked
}

/// A shape got a new property state or effect state (`ports::state_update`): its retained
/// instance, if it has one, is classified again and noted at its next display.
pub unsafe fn recheck(mt: &MainThread, shape: usize, cause: Recheck) {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    if let Some(instance) = t.instances.get_mut(&shape) {
        instance.recheck = true;
        (*RECHECKS.get(mt))[cause as usize] += 1;
    }
}

/// The switch that notes the material and the geometry of every retained shape at every
/// display, not every [`CHECK_FRAMES`] frames (`NIRENDER_NOTE_EVERY=1`,
/// `nirender.noteEvery(on)`); 255 means not read yet. Then the counters of the cost of the
/// skipped displays (`display_cost`).
static NOTE_EVERY: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(255);
static COST_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static COST_NOTES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static COST_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);

pub fn note_every() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    let value = NOTE_EVERY.load(Relaxed);
    if value != 255 {
        return value != 0;
    }
    let on = crate::switches::on("NIRENDER_NOTE_EVERY") == Some(true);
    NOTE_EVERY.store(on as u8, Relaxed);
    on
}

/// Sets the switch; returns the state before.
pub fn set_note_every(on: bool) -> bool {
    let before = note_every();
    NOTE_EVERY.store(on as u8, core::sync::atomic::Ordering::Relaxed);
    before
}

/// The cost of the skipped displays and of the views' closes since the last call
/// ([`display_cost`]); the counters reset.
pub struct DisplayCost {
    pub displays: u64,
    pub notes: u64,
    pub display_ms: f64,
    /// The time of the views' closes ([`close_views`]) and the list entries that they visited.
    pub close_ms: f64,
    pub close_visits: u64,
}

/// The skipped displays since the last call, the notes among them, and their time; the time
/// of the views' closes and the entries that they visited. The counters reset.
pub fn display_cost() -> DisplayCost {
    use core::sync::atomic::Ordering::Relaxed;
    COST_ON.store(true, Relaxed);
    let ms = |ticks: i64| ticks as f64 * 1000.0 / capture::ticks_per_second() as f64;
    DisplayCost {
        displays: COST_CALLS.swap(0, Relaxed),
        notes: COST_NOTES.swap(0, Relaxed),
        display_ms: ms(COST_TICKS.swap(0, Relaxed)),
        close_ms: ms(CLOSE_TICKS.swap(0, Relaxed)),
        close_visits: CLOSE_VISITS.swap(0, Relaxed),
    }
}

/// The cost of the display hook is timed only after the first `display_cost` call: the
/// timer reads at every display have a cost of their own.
static COST_ON: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The sorted blends of the table in the engine's main sorted queue: the shapes the queue
/// took (0), their places written at the flush (1), and the places of shapes the flush
/// displayed without a queue entry of their own (2: the children of a sort adjust node the
/// queue took).
static QUEUE_COUNTS: MainCell<[u64; 3]> = MainCell::new([0; 3]);

/// True when a retained shape that the engine displays now may go into the engine's main
/// sorted queue: mode gpu, the skip on, a capture running, a scene of the world camera. The
/// shape ports then push the shape's state and ask the renderer's accumulator, as the
/// engine's display does (`NiTriShape::Display`); the queue sorts the table's blends with
/// the engine's own, and its flush gives their places (`flush_place`).
pub unsafe fn may_queue(mt: &MainThread) -> bool {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    t.mode == Some(Mode::Gpu) && t.skip && capture::recording() && main_view_kind(t.scene_kind)
}

/// True in a scene of [`Route::Places`].
pub fn places_route(mt: &MainThread) -> bool {
    tables(mt).scene_route == Route::Places
}

/// Notes that the engine's main sorted queue took the shape in this frame.
pub unsafe fn note_queued(mt: &MainThread, shape: usize) {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    if let Some(instance) = t.instances.get_mut(&shape) {
        instance.queued = t.frame;
        QUEUE_COUNTS.get(mt)[0] += 1;
    }
}

/// At the flush of the engine's main sorted queue: true when the queue took the shape in
/// this frame. Its display work is done; the caller writes its place (`flush_place`).
pub unsafe fn queued_now(mt: &MainThread, shape: usize) -> bool {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    t.instances.get(&shape).is_some_and(|i| i.queued == t.frame)
}

/// Writes the place of a retained shape in the flush of the engine's main sorted queue
/// (`INSTANCE_QUEUED`). `own` is false for a shape the flush displays without a queue entry
/// of its own.
pub unsafe fn flush_place(mt: &MainThread, shape: usize, own: bool) {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    let Some(instance) = t.instances.get(&shape) else { return };
    capture::instance_queued(instance.id);
    QUEUE_COUNTS.get(mt)[if own { 1 } else { 2 }] += 1;
}

/// The places of the table's blends in the engine's walk (`walk_place`).
static WALK_PLACES: MainCell<u64> = MainCell::new(0);

/// Writes the place of a retained shape in the engine's walk (`INSTANCE_WALK`) when its alpha
/// property blends: the engine draws such a shape at its display when the main queue does
/// not take it (the no-sorter bit, no accumulator), so the helper draws it at this place, in
/// the walk's order. The caller calls it only for a shape the queue did not take. An opaque
/// shape gets no place, except in a scene of [`Route::Places`]: there every displayed shape
/// gets one, and the helper draws the scene's shapes only at their places.
/// Engine-rules R2 (the skill wgpu-renderer-dev).
pub unsafe fn walk_place(mt: &MainThread, shape: usize) {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    let Some(instance) = t.instances.get(&shape) else { return };
    if t.scene_route == Route::Places {
        capture::instance_walk(instance.id);
        *VIEW_PLACES.get(mt) += 1;
        return;
    }
    let property_state: usize = rd(shape, GEOMETRY_PROPERTY_STATE);
    let alpha: usize = if property_state != 0 { rd(property_state, PS::propAlpha) } else { 0 };
    if alpha == 0 || rd::<u16>(alpha, PROPERTY_FLAGS) & 1 == 0 {
        return;
    }
    capture::instance_walk(instance.id);
    *WALK_PLACES.get(mt) += 1;
}

/// The display of a retained shape by the engine (mode gpu): what the draw of the stream
/// did for the instance, without the draw. True when the engine must not display the
/// shape; false when the shape is not retained after all (the mark goes).
/// Engine-rules R5 (the skill wgpu-renderer-dev).
pub unsafe fn skipped_display(mt: &MainThread, shape: usize, kind: u32) -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    if !COST_ON.load(Relaxed) {
        return skipped_display_timed(mt, shape, kind);
    }
    let start = capture::ticks();
    let result = skipped_display_timed(mt, shape, kind);
    COST_TICKS.fetch_add(capture::ticks() - start, Relaxed);
    COST_CALLS.fetch_add(1, Relaxed);
    result
}

/// The scenes in which the engine displays a retained shape on its own path (the record path),
/// because the helper draws no table there, with their names for the report.
const ENGINE_PATH_SCENES: [(u32, &str); 4] = [
    (SCENE_LOCAL_MAP, "local map"),
    (SCENE_CHARACTER_PREVIEW, "character preview"),
    (SCENE_RACE_MENU_HEAD, "race menu head"),
    (SCENE_WATER_REFLECTION, "water reflection"),
];
/// The displays of retained shapes on the engine's path, by the scenes of [`ENGINE_PATH_SCENES`],
/// then those into a target that the helper does not have.
static ENGINE_PATH_DISPLAYS: MainCell<[u64; 5]> = MainCell::new([0; 5]);
/// The places written in the scenes of [`Route::Places`].
static VIEW_PLACES: MainCell<u64> = MainCell::new(0);
/// The displays of retained shapes in a scene into the back buffer of a kind outside the main
/// view's list, which the engine draws (`skipped_display_timed`).
static HANDED_BACK: MainCell<u64> = MainCell::new(0);
/// The displays of retained shapes in each frame, by route (D82 stage 0).
static FRAME_DISPLAYS: MainCell<FrameDisplays> = MainCell::new(FrameDisplays::new());

/// The displays of retained shapes that reach the skip, by the route of their scene: the main
/// view, the views of their own, the engine's path. The frame in progress, and over the run
/// the sum, the worst frame and the frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrameDisplays {
    frame: [u64; 3],
    sum: [u64; 3],
    worst: [u64; 3],
    frames: u64,
}

impl FrameDisplays {
    const NAMES: [&'static str; 3] = ["main view", "views of their own", "the engine's path"];

    const fn new() -> FrameDisplays {
        FrameDisplays { frame: [0; 3], sum: [0; 3], worst: [0; 3], frames: 0 }
    }

    fn count(&mut self, route: Route) {
        let index = match route {
            Route::Main => 0,
            Route::Places => 1,
            Route::Engine(_) => 2,
        };
        self.frame[index] += 1;
    }

    /// The end of a frame: its counts go into the sums.
    fn frame_end(&mut self) {
        for i in 0..3 {
            self.sum[i] += self.frame[i];
            self.worst[i] = self.worst[i].max(self.frame[i]);
        }
        self.frame = [0; 3];
        self.frames += 1;
    }

    fn line(&self) -> String {
        let frames = self.frames.max(1) as f64;
        let parts: Vec<String> = Self::NAMES
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{name} {:.1} (worst {})", self.sum[i] as f64 / frames, self.worst[i]))
            .collect();
        format!("retained displays for each of {} frames: {}", self.frames, parts.join(", "))
    }
}

/// Where the engine's displays of retained shapes go in a scene.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    /// The table's main view draws them: a scene into the back buffer.
    Main,
    /// The helper draws each one at its place in a view of the scene's own: every display
    /// writes its place (`INSTANCE_WALK`), or its place in the flush of the queue. A scene
    /// into a rendered texture of the helper (a save's picture, a mod's mirror), or a scene of
    /// another camera, or of the world camera on another root, into the back buffer.
    Places,
    /// The engine draws them: the scene of `ENGINE_PATH_SCENES` at this index, or with
    /// the index 4 a target that the helper does not have.
    Engine(usize),
}

/// The route of a scene of this kind. `back_buffer`: the game draws into the back buffer;
/// `known_target`: the helper has the target of the game (the back buffer, or a rendered
/// texture of the plugin's device).
pub fn route_of(kind: u32, back_buffer: bool, known_target: bool) -> Route {
    if let Some(index) = ENGINE_PATH_SCENES.iter().position(|&(scene, _)| scene == kind) {
        return Route::Engine(index);
    }
    if back_buffer && kind != SCENE_OTHER {
        return Route::Main;
    }
    if !known_target {
        return Route::Engine(ENGINE_PATH_SCENES.len());
    }
    Route::Places
}

/// The displays of retained shapes on the engine's path outside the table's scenes, and the
/// places in the views of their own, for the `shapes:` line: the work that the table does not
/// take yet, and the work of those views.
pub fn engine_path_report(mt: &MainThread) -> String {
    let counts = *ENGINE_PATH_DISPLAYS.get(mt);
    let names = ENGINE_PATH_SCENES.iter().map(|&(_, name)| name).chain(["a target the helper lacks"]);
    let parts: Vec<String> = names.zip(counts).map(|(name, count)| format!("{name} {count}")).collect();
    format!(
        "{} displays of retained shapes on the engine's path outside the table's scenes ({}); {} in scenes into the back buffer outside the main view's list; {} places in views of their own; {}",
        counts.iter().sum::<u64>(),
        parts.join(", "),
        *HANDED_BACK.get(mt),
        *VIEW_PLACES.get(mt),
        FRAME_DISPLAYS.get(mt).line()
    )
}

unsafe fn skipped_display_timed(mt: &MainThread, shape: usize, kind: u32) -> bool {
    {
        let mut tables_open = tables(mt);
        let t = &mut *tables_open;
        // Clear a stale mark here, on the shape the engine displays: `set_mode` and
        // `set_skin_table` write no mark into kept shapes.
        if t.mode != Some(Mode::Gpu) {
            mark(shape, false);
            return false;
        }
        if !t.skip || !capture::recording() {
            return false;
        }
        FRAME_DISPLAYS.get(mt).count(t.scene_route);
        // A scene of [`ENGINE_PATH_SCENES`] (the local map's tiles, the character preview, the
        // race menu's head, the engine's water reflection), or a scene into a target that the
        // helper does not have: the engine draws the shape on the record path (the draw is not
        // a plain world shape there, `link_client`). The mark stays: the shape is still
        // retained in the world scenes. Without this a map tile drawn at a walking cross lost
        // its landscape (records entry 55). The displays are counted ([`engine_path_report`]).
        if let Route::Engine(index) = t.scene_route {
            ENGINE_PATH_DISPLAYS.get(mt)[index] += 1;
            return false;
        }
        // A scene into the back buffer of a kind outside the main view's list (first person, a
        // shadow scene, the interface, the splash): the helper draws no table there, so the
        // engine draws the shape at its place, into the scene's target, as on Direct3D 9. The
        // mark stays, as on the engine's path.
        if t.scene_route == Route::Main && !main_view_kind(t.scene_kind) {
            *HANDED_BACK.get(mt) += 1;
            return false;
        }
        if let Some(instance) = t.instances.get_mut(&shape) {
            if instance.skinned && !t.skin_table {
                mark(shape, false);
                return false;
            }
            if instance.kind == 0 {
                instance.kind = kind;
            }
        }
    }
    display_work(mt, shape, kind)
}

/// The work of a display of a retained shape by the engine (`skipped_display`): the
/// recheck, the live controllers, the notes when due, the transform, the scene values, the
/// lights, the visible flag.
unsafe fn display_work(mt: &MainThread, shape: usize, kind: u32) -> bool {
    let renderer = crate::engine::renderer();
    let property_state: usize = rd(shape, GEOMETRY_PROPERTY_STATE);
    let effect_state: usize = rd(shape, GEOMETRY_EFFECT_STATE);
    let (frame, generation, id, live) = {
        let mut tables_open = tables(mt);
        let t = &mut *tables_open;
        let (frame, generation) = (t.frame, t.generation);
        let properties = if property_state != 0 { core::array::from_fn(|i| rd(property_state, PS::propAlpha + i * 4)) } else { [0; PROPERTY_SLOTS] };
        // Lua changed the shape or one of its properties since its last check.
        if let Some(instance) = t.instances.get_mut(&shape) {
            check_notes(mt, instance, shape, &properties);
        }
        // A new property or effect state since the classification, or a Lua change: classify
        // again.
        if let Some((skinned, was_live)) = t.instances.get_mut(&shape).filter(|i| i.recheck).map(|i| {
            i.recheck = false;
            (i.skinned, i.live)
        }) {
            let rechecks = &mut *RECHECKS.get(mt);
            rechecks[3] += 1;
            let mut live = false;
            if reject_reason(t, shape, property_state, skinned, &mut live).is_some() {
                rechecks[4] += 1;
                t.reject_pending(shape, true);
                return false;
            }
            if let Some(instance) = t.instances.get_mut(&shape) {
                if instance.live != live {
                    rechecks[5] += 1;
                }
                instance.live = live || was_live;
                instance.generation = instance.generation.wrapping_sub(1);
            }
        }
        let Some(instance) = t.instances.get_mut(&shape) else {
            mark(shape, false);
            return false;
        };
        if !instance.filled || renderer == 0 {
            return false;
        }
        instance.properties = properties;
        instance.sorted_blend = instance.properties[0] != 0 && {
            let flags = rd::<u16>(instance.properties[0], PROPERTY_FLAGS);
            flags & ALPHA_BLEND != 0 && flags & ALPHA_NO_SORTER == 0
        };
        (frame, generation, instance.id, instance.live)
    };
    // The controllers of a live shape run before its draw, as the engine's display runs
    // them (`NiTriShape::Display`): a morpher writes its vertices then. A controller is
    // engine code that can come back into the plugin: the tables are closed here.
    if live {
        let mut controller: usize = rd(shape, CONTROLLERS);
        while controller != 0 {
            vcall0(controller, SLOT_ON_PRE_DISPLAY);
            controller = rd(controller, TC::nextController);
        }
    }
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    // The texture effects of the shape, read while the shape displays (it holds its effect
    // state): each at most once a frame, whatever number of shapes it lights.
    let effects = if effect_state != 0 { [rd::<usize>(effect_state, DES::environmentMap), rd(effect_state, DES::fogMap)] } else { [0; 2] };
    let effect_versions = effects.map(|effect| {
        let version = if effect == 0 { 0 } else { t.effects.entry(effect).or_default().observe(frame, || crate::material_stream::effect_signature(mt, renderer, effect)) };
        (effect, version)
    });
    let Some(instance) = t.instances.get_mut(&shape) else {
        mark(shape, false);
        return false;
    };
    let effects_changed = instance.effect_versions != effect_versions;
    // The handles, when due: every 64th frame, after a change of the device textures, for
    // a record that must go again (the start of a capture), and at every display of a
    // live shape (a new revision of the data sends the geometry again under its handle).
    // And when an effect of the shape changed: its material record copies the effect.
    // A flag of MWSE changed on the data of a skinned shape: the geometry goes again with
    // another form of its bone influences.
    let software = instance.skinned && crate::ports::draw::skinned_in_software(instance.kind == crate::link::DRAW_STRIPS, rd(shape, GEOMETRY_MODEL_DATA));
    let software_changed = instance.software != software;
    instance.software = software;
    let event_due = note_every() || instance.live || !instance.sent || effects_changed || software_changed || instance.generation != generation;
    let due = event_due || id.wrapping_add(frame).is_multiple_of(CHECK_FRAMES);
    if due {
        COST_NOTES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        // A skinned geometry is packed with the bone influences of its skin.
        let skin = if instance.skinned { rd(shape, GEOMETRY_SKIN) } else { 0 };
        let geometry = crate::geometry_stream::note(mt, kind, rd(shape, GEOMETRY_MODEL_DATA), skin, shape);
        let material = crate::material_stream::note(mt, renderer, property_state, effect_state);
        if geometry == 0 || material == 0 || !crate::material_stream::last_modelled(mt) {
            let cause = if geometry == 0 {
                LATER_GEOMETRY
            } else if material == 0 {
                LATER_MATERIAL
            } else {
                LATER_NOT_MODELLED
            };
            reject_at_record(mt, t, shape, cause);
            return false;
        }
        instance.generation = generation;
        if effects_changed && instance.sent && instance.effect_versions != [(0, 0); 2] {
            t.effect_notes += 1;
        }
        instance.effect_versions = effect_versions;
        // A change that only the periodic turn found: count it by its groups of words.
        if !event_due && instance.material != material {
            if let Some(bits) = crate::material_stream::changed_groups(mt, instance.material, material) {
                for (group, count) in t.periodic_changes.iter_mut().enumerate() {
                    *count += u64::from(bits >> group & 1);
                }
            }
        }
        if instance.geometry != geometry || instance.material != material {
            instance.geometry = geometry;
            instance.material = material;
            instance.sent = false;
            t.handle_updates += 1;
        }
    }
    // The transform (the rows of the world matrix come from the engine's own conversion
    // when it moved), the bound, the colours. A skinned instance keeps the identity and no
    // bound in its record: its bones and its bound go in `INSTANCE_BONES` below.
    let skinned = instance.skinned;
    let (transform, bound): ([u32; TRANSFORM_WORDS], [f32; 4]) = if skinned { (instance.transform, instance.bound) } else { (rd(shape, WORLD), rd(shape, WORLD_BOUND)) };
    let colours = crate::shape_stream::material_values(property_state);
    let moved = instance.transform != transform;
    let bound_changed = instance.bound.map(f32::to_bits) != bound.map(f32::to_bits);
    let colours_changed = instance.colours != colours;
    let changed = moved || bound_changed || colours_changed;
    if changed || !instance.sent {
        let record_ticks = &mut *RECORD_TICKS.get(mt);
        let record_start = capture::ticks();
        note_record_cause(mt, instance, moved, bound_changed, colours_changed);
        if moved {
            crate::ports::renderer_frame::write_world_matrix(renderer, shape + WORLD);
            let world = crate::shape_stream::current_world(renderer);
            instance.rotation = core::array::from_fn(|i| world[i].to_bits());
            instance.transform = transform;
            instance.stored = [transform[9], transform[10], transform[11]];
        }
        instance.bound = bound;
        instance.colours = colours;
        let matrix_end = capture::ticks();
        let exact = if skinned {
            [0.0; 3]
        } else {
            let Some(exact) = crate::camera_space::exact_translation(mt, shape, 0) else {
                reject_at_record(mt, t, shape, LATER_TRANSLATION);
                return false;
            };
            exact
        };
        if changed {
            t.value_updates += 1;
        }
        // The plugin makes the matrices of every draw relative when it owns the camera
        // space: the record says so whether or not a draw happened this frame. A move of a
        // sent instance with the same colours and space goes as the compact record.
        let relative = crate::camera_space::own();
        let moves = compact_moves();
        let move_only = instance.sent && !colours_changed && instance.relative == relative && moves != CompactMoves::Off;
        instance.sent = true;
        instance.relative = relative;
        let exact_end = capture::ticks();
        if move_only {
            send_move(instance, exact);
            (*RECORD_COUNTS.get(mt))[RC_COMPACT_MOVES] += 1;
        }
        if !move_only || moves == CompactMoves::Check {
            send_instance(mt, shape, instance, exact, relative);
            restate(instance, &mut t.states);
        }
        t.records += 1;
        let send_end = capture::ticks();
        record_ticks[0] += matrix_end - record_start;
        record_ticks[1] += exact_end - matrix_end;
        record_ticks[2] += send_end - exact_end;
    }
    // The bones of a skinned instance, in each frame the engine displays it.
    if skinned && !send_bones(mt, t, shape, id) {
        return false;
    }
    // The table draws a skinned part with the scene's ambient light and fog: count the parts
    // whose own values differ (the engine's light manager and render state would set those).
    if skinned && t.scene_frame == frame {
        let ambient = crate::ports::light_manager::ambient_of_state(effect_state);
        let cell_fog = crate::engine::game_fog_property();
        let fog = if cell_fog != 0 { crate::shape_stream::fog_values_of(renderer, cell_fog) } else { crate::shape_stream::fog_values(renderer, property_state) };
        if (ambient, fog) != t.scene_values {
            t.skin_scene_mixed += 1;
            if t.skin_scene_mixed <= 4 {
                crate::log!("instances: the skinned instance {id} (shape 0x{shape:x}) has other scene values: ambient {ambient:#x} fog {fog:?}, the scene's ambient {:#x} fog {:?}", t.scene_values.0, t.scene_values.1);
            }
        }
    }
    // The values of the frame for the retained draws: the ambient light of this shape's
    // effect state, and the fog of the cell's property (`TES3::Game::fogProperty`, the
    // weather's), as the light manager and the render state would set them. A shape with
    // a fog property of its own carries that fog in its material record. One record in a
    // frame; a shape whose values differ is counted.
    let ambient = crate::ports::light_manager::ambient_of_state(effect_state);
    let cell_fog = crate::engine::game_fog_property();
    let fog = if cell_fog != 0 {
        crate::shape_stream::fog_values_of(renderer, cell_fog)
    } else {
        crate::shape_stream::fog_values(renderer, property_state)
    };
    if t.scene_frame != frame {
        t.scene_frame = frame;
        t.scene_values = (ambient, fog);
        let sun = scene_sun(mt, t);
        capture::retained_scene(ambient, fog, crate::shape_stream::fog_color_of(cell_fog), sun.as_ref());
    } else if t.scene_frame == frame && t.scene_values != (ambient, fog) {
        t.scene_mixed += 1;
    }
    // The lights of the effect state of the shape, their device values when they changed,
    // and the keys when the set changed.
    let light_start = capture::ticks();
    let mut lights = core::mem::take(&mut t.scratch_lights);
    state_lights(effect_state, &mut lights);
    let mut keys = core::mem::take(&mut t.scratch_keys);
    keys.clear();
    for &light in &lights {
        let key: u32 = rd(light, L::index);
        note_light_once(mt, t, light, key);
        keys.push(key);
    }
    let counts = &mut *LIGHT_COUNTS.get(mt);
    counts[LC_LOOPS] += 1;
    counts[LC_LOOP_LIGHTS] += lights.len() as u64;
    *LIGHT_TICKS.get(mt) += capture::ticks() - light_start;
    t.scratch_lights = lights;
    let Some(instance) = t.instances.get_mut(&shape) else {
        t.scratch_keys = keys;
        return false;
    };
    // The ambient light of the instance's own effect state goes with its lights: the helper
    // lights each instance with its own, as the engine's light manager does for each draw,
    // without its gates (see `state_lights`).
    let ambient = crate::ports::light_manager::ambient_of_state(effect_state);
    if instance.lights != keys || instance.ambient != ambient {
        capture::instance_lights(id, &keys, ambient);
        instance.lights.clear();
        instance.lights.extend_from_slice(&keys);
        instance.ambient = ambient;
        t.light_records += 1;
    }
    t.scratch_keys = keys;
    shown_in_scene(t, shape);
    t.skipped += 1;
    true
}

/// The values of the frame for the world, at the start of the world (`world_walk`), in every
/// mode: the ambient light of the lights on the world camera's root and on the game's world
/// root (`TES3Game` +0x9C), the fog of the cell's property, and the weather's sky light. The
/// sky light affects the node "worldRoot" (`WeatherController::initSkyLight` 0x43BB50); the
/// engine's ambient lights affect the effects' root and the menu (`TES3Game::initGame`
/// 0x4185E6, 0x418D6E), so the world's ambient is black unless a mod adds one. One record in a
/// frame: the table's shapes then count only a difference (`scene_mixed`).
pub unsafe fn world_scene(mt: &MainThread, camera_root: usize) {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    let frame = t.frame;
    if t.scene_frame == frame {
        return;
    }
    let renderer = crate::engine::renderer();
    if renderer == 0 {
        return;
    }
    let game: usize = rd(TES3_GAME, 0);
    let world_root: usize = if game != 0 { rd(game, GAME_WORLD_ROOT) } else { 0 };
    let ambient = crate::ports::light_manager::ambient_of_effect_lists(&[camera_root, world_root]);
    let cell_fog = crate::engine::game_fog_property();
    let fog = if cell_fog != 0 { crate::shape_stream::fog_values_of(renderer, cell_fog) } else { None };
    t.scene_frame = frame;
    t.scene_values = (ambient, fog);
    let sun = scene_sun(mt, t);
    capture::retained_scene(ambient, fog, crate::shape_stream::fog_color_of(cell_fog), sun.as_ref());
}

/// `TES3Game` and its world root (MWSE `TES3Game.h`).
const TES3_GAME: usize = 0x7C6CDC;
const GAME_WORLD_ROOT: usize = crate::gen::layouts::TES3Game::worldRoot;

/// `WorldController` (0x7C67DC) -> `weatherController` -> `sgSkyLight`: the weather's
/// directional light, the sun of the hour (MWSE `TES3WeatherController.h`).
const WORLD_CONTROLLER: usize = 0x7C67DC;
const WEATHER_CONTROLLER: usize = crate::gen::layouts::WorldController::weatherController;
const SKY_LIGHT: usize = crate::gen::layouts::WeatherController::sgSkyLight;

/// The sun of the frame for the helper's own draws (the distant land, the shadows): the
/// weather controller's sky light, converted as the engine converts a light for the device
/// (`light_entry_update`, the 26 words of a `D3DLIGHT8`). The helper draws the retained
/// shapes with no draw of the engine, so it needs the sun from here. None when the light is
/// off or dim, as `note_light_once`.
unsafe fn scene_sun(mt: &MainThread, t: &mut Tables) -> Option<[u32; 26]> {
    static LOGGED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    static LAST_DIFFUSE: MainCell<[u32; 3]> = MainCell::new([0; 3]);
    let log_once = |what: String| {
        if LOGGED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 24 {
            crate::log!("scene sun: {what}");
        }
    };
    let world: usize = rd(WORLD_CONTROLLER, 0);
    if world == 0 {
        log_once(String::from("no world controller"));
        return None;
    }
    let weather: usize = rd(world, WEATHER_CONTROLLER);
    if weather == 0 {
        log_once(String::from("no weather controller"));
        return None;
    }
    let light: usize = rd(weather, SKY_LIGHT);
    if light == 0 {
        log_once(String::from("no sky light"));
        return None;
    }
    let enabled: u8 = rd(light, L::enabled);
    let dimmer: f32 = rd(light, L::dimmer);
    if enabled == 0 || dimmer < f32::from_bits(crate::ports::light_manager::MIN_DIMMER) {
        log_once(format!("the sky light 0x{light:x} is off or dim: enabled {enabled}, dimmer {dimmer}"));
        return None;
    }
    let entry = t.scratch_entry.as_mut_ptr() as usize;
    core::ptr::write_bytes(entry as *mut u8, 0, E::SIZE);
    crate::ports::light_manager::light_entry_update(entry, light);
    let values: [u32; 26] = rd(entry, E::d3dLight);
    // The values go to the log when the diffuse colour changes (the hour moves it).
    let diffuse = [values[1], values[2], values[3]];
    if diffuse == *LAST_DIFFUSE.get(mt) {
        return Some(values);
    }
    *LAST_DIFFUSE.get(mt) = diffuse;
    log_once(format!(
        "frame {}: the sky light 0x{light:x} goes into the scene record: type {}, diffuse {:.3} {:.3} {:.3}, ambient {:.3} {:.3} {:.3}, direction {:.3} {:.3} {:.3}",
        t.frame,
        values[0],
        f32::from_bits(values[1]),
        f32::from_bits(values[2]),
        f32::from_bits(values[3]),
        f32::from_bits(values[9]),
        f32::from_bits(values[10]),
        f32::from_bits(values[11]),
        f32::from_bits(values[16]),
        f32::from_bits(values[17]),
        f32::from_bits(values[18])
    ));
    Some(values)
}

pub unsafe fn report(mt: &MainThread) -> String {
    // Copies of the counts: each table opens once here.
    let (rechecks, change_notes, queue_counts, walk_places) = (*RECHECKS.get(mt), *CHANGE_NOTES.get(mt), *QUEUE_COUNTS.get(mt), *WALK_PLACES.get(mt));
    let change_ms = (*CHANGE_TICKS.get(mt)).map(|ticks| ticks as f64 * 1000.0 / capture::ticks_per_second() as f64);
    let (notes, clears) = CHANGES.get(mt).as_ref().map_or((0, 0), |notes| (notes.count, notes.clears));
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    let reasons = ["controller", "node kind", "property controller", "at the record", "depth function", "parent chain too long"];
    let mut rejected: Vec<String> = reasons.iter().zip(t.rejected_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    let later = *LATER_BY.get(mt);
    if later.iter().any(|&n| n != 0) {
        rejected.push(format!("at the record by cause: geometry {}, material {}, not modelled {}, translation {}", later[0], later[1], later[2], later[3]));
    }
    let shown_line = format!("{}; {} reads of the back buffer found the table open", t.shown.line(), SHOWN_TABLES_BUSY.load(core::sync::atomic::Ordering::Relaxed));
    let causes = ["not a plain world draw of a retained class", "no geometry", "no material", "path off", "not modelled"];
    let skipped: Vec<String> = causes.iter().zip(t.skipped_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    format!(
        "instances: mode {}, {} retained ({} live, {} skinned: the table draws them {}, {} bone records, {} hidden with no bones, {} works with other scene values), {} rejected ({}), draws not taken ({}), {} fast draws, {} displays skipped ({} with other scene values), {} instance records, {} updates of the handles, {} of the values, {} deletes ({} with no record since a reset), {} light records, {} state records",
        t.mode().name(),
        t.instances.len(),
        t.instances.values().filter(|i| i.live).count(),
        t.instances.values().filter(|i| i.skinned).count(),
        if table_draws_skins(&t) { "yes" } else { "no" },
        t.bone_records,
        t.bone_failures,
        t.skin_scene_mixed,
        t.rejected.len(),
        rejected.join(", "),
        skipped.join(", "),
        t.fast_draws,
        t.skipped,
        t.scene_mixed,
        t.records,
        t.handle_updates,
        t.value_updates,
        t.deletes,
        t.ends_without_record,
        t.light_records,
        t.states
    ) + "\n"
        + &format!(
            "rechecks: asked by a new property state {}, by a new effect state {}, by a Lua change {}; done {}, rejected {}, live changed {}; Lua change notes {}, display checks {} ({:.3} ms, the worst frame {:.3} ms), shapes hit {}, clears of the changed objects {}; texture effects watched {}, materials noted again for a changed effect {}, for a freed texture {}; the note of every 64th frame found changed: {}\n",
            rechecks[0], rechecks[1], rechecks[2], rechecks[3], rechecks[4], rechecks[5], notes, change_notes[0], change_ms[0], change_ms[1].max(change_ms[2]), change_notes[1], clears, t.effects.len(), t.effect_notes, t.texture_notes,
            crate::material_stream::GROUP_NAMES.iter().zip(t.periodic_changes).map(|(name, n)| format!("{name} {n}")).collect::<Vec<_>>().join(", ")
        )
        + &format!(
            "sorted queue: {} blends of the table taken, {} places written at the flush, {} places of shapes under a queued node; {} places of blends in the walk; shapes other than sorted blends the engine did not display, left visible by the plane that puts them out of the engine's view: {}\n",
            queue_counts[0], queue_counts[1], queue_counts[2], walk_places,
            PLANE_NAMES.iter().zip(t.undisplayed_left).map(|(name, n)| format!("{name} {n}")).collect::<Vec<_>>().join(", ")
        )
        + &{
            let engine = crate::shape_stream::engine_particle_records(mt);
            format!(
                "particles: the table {}, draws as instance records {}; {} particle instances ({} made), {} draws as INSTANCE_DRAW records; as the engine's shape records {} in scenes of the world camera ({} frames), {} in other scenes\n",
                if particle_table_on() { "on" } else { "off" },
                if particle_draws() { "on" } else { "off" },
                t.instances.values().filter(|i| i.points).count(),
                t.particle_instances_made,
                t.particle_draws,
                engine[0],
                engine[2],
                engine[1]
            )
        }
        + &crate::animation::report()
        + "\n"
        + &light_report(mt)
        + "\n"
        + &record_report(mt)
        + "\n" + &crate::worker_events::report()
        + "\n" + &crate::freed_fill::report()
        + "\n" + &crate::camera_space::skin_report(mt)
        + "\n" + &view_report(mt)
        + "\n" + &shown_line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material_stream::EFFECT_SIGNATURE_WORDS;

    /// The displays of a frame count by the route of their scene, and the worst frame of a
    /// route is its own: a frame with many displays in one route does not raise another's.
    #[test]
    fn frame_displays_count_each_route_and_its_worst_frame() {
        let mut displays = FrameDisplays::new();
        for _ in 0..3 {
            displays.count(Route::Main);
        }
        displays.count(Route::Places);
        displays.frame_end();
        displays.count(Route::Main);
        displays.count(Route::Engine(4));
        displays.count(Route::Engine(0));
        displays.frame_end();
        assert_eq!(displays.sum, [4, 1, 2]);
        assert_eq!(displays.worst, [3, 1, 2]);
        assert_eq!(displays.frames, 2);
        assert_eq!(displays.frame, [0; 3]);
        assert_eq!(
            displays.line(),
            "retained displays for each of 2 frames: main view 2.0 (worst 3), views of their own 0.5 (worst 1), the engine's path 1.0 (worst 2)"
        );
    }

    /// The route of a scene: the back buffer keeps the main view, except for a scene of another
    /// camera or root; a rendered texture of the helper gives the scene a view of its own; the
    /// engine's own scenes and a target the helper lacks stay on the engine's path.
    #[test]
    fn a_scene_takes_its_route_from_its_kind_and_its_target() {
        use crate::link::{SCENE_FIRST_PERSON, SCENE_UI};
        // The save's picture: the world camera into a rendered texture of the helper.
        assert_eq!(route_of(SCENE_WORLD, false, true), Route::Places);
        assert_eq!(route_of(SCENE_FIRST_PERSON, false, true), Route::Places);
        assert_eq!(route_of(SCENE_WORLD, true, true), Route::Main);
        assert_eq!(route_of(SCENE_UI, true, true), Route::Main);
        // A mod's camera: into its texture, or into the back buffer (another root).
        assert_eq!(route_of(SCENE_OTHER, false, true), Route::Places);
        assert_eq!(route_of(SCENE_OTHER, true, true), Route::Places);
        // A target the helper lacks: the engine draws.
        assert_eq!(route_of(SCENE_WORLD, false, false), Route::Engine(ENGINE_PATH_SCENES.len()));
        // The engine's own scenes keep their path, into any target.
        assert_eq!(route_of(SCENE_LOCAL_MAP, false, true), Route::Engine(0));
        assert_eq!(route_of(SCENE_WATER_REFLECTION, false, true), Route::Engine(3));
        assert_eq!(route_of(SCENE_CHARACTER_PREVIEW, true, true), Route::Engine(1));
    }

    /// The view field of a scene start: slot + 1 for a view, 0 for a scene that is not one. The
    /// slot of a texture view that no scene uses retires at Present after 60 frames, once.
    #[test]
    fn a_scene_start_gives_its_view_and_present_retires_idle_slots() {
        use crate::link::SCENE_SHADOW_CASTERS;
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            let world = Click { kind: SCENE_WORLD, camera: 0x7900_0000, root: 0x7900_1000, target: None, world_camera: true };
            let shadow = Click { kind: SCENE_SHADOW_CASTERS, camera: 0x7900_2000, root: 0x7900_3000, target: None, world_camera: false };
            let picture = Click { kind: SCENE_OTHER, camera: 0x7900_0000, root: 0x7900_1000, target: Some(0x7900_4000), world_camera: false };
            // No view before the recording starts: no Present would retire its slot.
            assert_eq!(view_open(&mt, &picture), 0);
            capture::test_record_deletes(true);
            assert_eq!(view_open(&mt, &world), 1);
            assert_eq!(view_open(&mt, &shadow), 0);
            let picture_view = view_open(&mt, &picture);
            assert!(picture_view > 1);
            view_swap(&mt, Some(0x7900_4000));
            take_retired_views(&mt);
            let mut retired = Vec::new();
            for _ in 0..=crate::view_slots::RETIRE_FRAMES {
                view_present(&mt);
                retired.extend(take_retired_views(&mt));
            }
            assert!(retired.contains(&((picture_view - 1) as u8)), "{retired:?}");
            assert!(!retired.contains(&0), "the main view retired");
            assert!(view_report(&mt).starts_with("views: "));
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// The take lines: the first 64, then each power of 2; a texture key used again before a
    /// flush is named as a second slot.
    /// The marks of each view (D82 stage 2): a display in view v sets bit v and lists the shape
    /// in v; the close of v keeps a shape that its scenes displayed and hides one that they did
    /// not (no planes: inside); a display in a scene that is not a view marks view 0 only; a
    /// retirement clears bit v.
    #[test]
    fn a_view_shows_what_its_scenes_displayed_and_hides_the_rest_at_its_close() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let (a, b) = (0x7920_0000usize, 0x7920_1000usize);
            let mode = {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                for (shape, id) in [(a, 91), (b, 92)] {
                    let mut instance = Instance::new(id, 0, 1, 2, 0, false, false, false);
                    instance.filled = true;
                    t.instances.insert(shape, instance);
                }
                t.mode.replace(Mode::Gpu)
            };
            let show = |shape: usize, view: u8| {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                let instance = t.instances.get_mut(&shape).expect("a test instance");
                show_in_view(instance, shape, view, t.frame, &mut t.lists, &mut t.states, None);
            };
            let mask = |shape: usize| tables(&mt).instances[&shape].visible;
            show(a, 1);
            show(b, 1);
            assert_eq!((mask(a), mask(b)), (0b10, 0b10));
            close_views(&mt, [(1, false)]);
            assert_eq!((mask(a), mask(b)), (0b10, 0b10));
            // The next scene of view 1 displays only a: b goes at the close.
            show(a, 1);
            close_views(&mt, [(1, false)]);
            assert_eq!((mask(a), mask(b)), (0b10, 0));
            assert_eq!(listed_shapes(&mt, 1), vec![a]);
            // A display in a scene that is not a view: view 0 only.
            show(b, 0);
            assert_eq!(mask(b), 0b1);
            // The retirement of view 1.
            forget_views(&mt, &[1]);
            assert_eq!((mask(a), listed_shapes(&mt, 1)), (0, vec![]));
            {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                for shape in [a, b] {
                    let instance = t.instances.remove(&shape).expect("a test instance");
                    t.lists.give_back(instance.list_key);
                }
                t.mode = mode;
            }
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// The shapes in a view's lists.
    fn listed_shapes(mt: &MainThread, view: u8) -> Vec<usize> {
        tables(mt).lists.listed(view).into_iter().map(|(shape, _)| shape).collect()
    }

    /// The shapes of a view's list of the entries not displayed since its last close.
    fn waiting_shapes(mt: &MainThread, view: u8) -> Vec<usize> {
        let t = tables(mt);
        let waiting = t.lists.shown[view as usize] as usize ^ 1;
        t.lists.entries[view as usize][waiting].iter().map(|&(shape, _)| shape).collect()
    }

    /// The shapes that view 0's next close visits: visible in view 0, and not displayed in it
    /// since its last close.
    fn main_waiting(mt: &MainThread) -> Vec<usize> {
        let t = tables(mt);
        let stamp = t.lists.main_closes;
        t.instances.iter().filter(|(_, instance)| instance.visible & 1 != 0 && instance.main_shown != stamp).map(|(&shape, _)| shape).collect()
    }

    /// View 0 closes in every frame: a shape displayed in frame n and not in frame n + 1 stays
    /// visible at the close of n and is hidden at the close of n + 1. Each close visits only
    /// the visible shapes that view 0 did not display since its last close, and view 0 lists
    /// nothing.
    #[test]
    fn a_shape_displayed_in_one_frame_and_not_in_the_next_is_hidden_at_the_next_close() {
        use core::sync::atomic::Ordering::Relaxed;
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let main = crate::view_slots::MAIN_VIEW;
            let (a, b) = (0x7925_0000usize, 0x7925_1000usize);
            let mode = {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                for (shape, id) in [(a, 81), (b, 82)] {
                    let mut instance = Instance::new(id, 0, 1, 2, 0, false, false, false);
                    instance.filled = true;
                    t.instances.insert(shape, instance);
                }
                t.mode.replace(Mode::Gpu)
            };
            let show = |shape: usize| {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                let instance = t.instances.get_mut(&shape).expect("a test instance");
                show_in_view(instance, shape, main, t.frame, &mut t.lists, &mut t.states, None);
            };
            let mask = |shape: usize| tables(&mt).instances[&shape].visible;
            // A close of view 0: it visits the shapes that waited.
            let close = || {
                let waited = main_waiting(&mt).len() as u64;
                let start = CLOSE_VISITS.load(Relaxed);
                close_views(&mt, [(main, false)]);
                let visits = CLOSE_VISITS.load(Relaxed) - start;
                assert_eq!(visits, waited);
            };
            // Frame n displays a and b.
            show(a);
            show(b);
            show(a);
            assert!(!main_waiting(&mt).iter().any(|&shape| shape == a || shape == b));
            assert!(listed_shapes(&mt, main).is_empty());
            close();
            assert_eq!((mask(a), mask(b)), (1, 1));
            // Frame n + 1 displays only a: b waits and goes at the close.
            show(a);
            let waiting = main_waiting(&mt);
            assert!(!waiting.contains(&a) && waiting.contains(&b), "{waiting:?}");
            close();
            assert_eq!((mask(a), mask(b)), (1, 0));
            assert!(!main_waiting(&mt).contains(&b));
            // Frame n + 2 displays nothing: a goes.
            assert!(main_waiting(&mt).contains(&a));
            close();
            assert_eq!(mask(a), 0);
            assert!(listed_shapes(&mt, main).is_empty());
            {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                for shape in [a, b] {
                    let instance = t.instances.remove(&shape).expect("a test instance");
                    t.lists.give_back(instance.list_key);
                }
                t.mode = mode;
            }
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// A stand-in for `NiBound::WhichSide` in the tests: outside (2) when the x of the bound's
    /// centre is above the plane's constant, else 1.
    unsafe fn test_side(bound: usize, plane: &[f32; 4]) -> u32 {
        if *(bound as *const f32) > plane[3] {
            2
        } else {
            1
        }
    }

    /// View 0's sweep gives the result of another view's list close (design 3.1a): the same
    /// instances, displayed in both views in the same frames, with the same planes, stay visible
    /// or go in both. A shape displayed stays; one not displayed and inside the planes goes; one
    /// outside stays; one without values from a draw stays; a sorted blend outside stays; one
    /// never displayed stays hidden. After a root change each one not displayed goes, except the
    /// one without values. Only view 0 counts the shape outside, and not the sorted blend.
    #[test]
    fn view_0s_sweep_and_a_views_list_close_give_the_same_result() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let (main, other) = (crate::view_slots::MAIN_VIEW, 6u8);
            let shapes = [0x7928_0000usize, 0x7928_1000, 0x7928_2000, 0x7928_3000, 0x7928_4000, 0x7928_5000];
            let [shown, inside, outside, unfilled, blend, never] = shapes;
            let mode = {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                assert!(t.lists.listed(other).is_empty(), "view {other} lists entries of another test");
                for (&shape, id) in shapes.iter().zip(61..) {
                    let mut instance = Instance::new(id, 0, 1, 2, 0, false, false, false);
                    instance.filled = shape != unfilled;
                    instance.sorted_blend = shape == blend;
                    instance.bound[0] = if shape == outside || shape == blend { 20.0 } else { 0.0 };
                    t.instances.insert(shape, instance);
                }
                for view in [main, other] {
                    t.view_planes[view as usize] = vec![[0.0, 0.0, 0.0, 10.0]];
                }
                t.mode.replace(Mode::Gpu)
            };
            let show = |shape: usize| {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                for view in [main, other] {
                    let instance = t.instances.get_mut(&shape).expect("a test instance");
                    show_in_view(instance, shape, view, t.frame, &mut t.lists, &mut t.states, None);
                }
            };
            let close = |root_changed: bool| close_views_with(&mt, [(main, root_changed), (other, root_changed)], test_side);
            // The shape's bits in view 0 and in the other view.
            let bits = |shape: usize| {
                let visible = tables(&mt).instances[&shape].visible;
                (visible & 1 != 0, visible & (1 << other) != 0)
            };
            let left = || tables(&mt).undisplayed_left[0];
            // Frame n displays all but `never`.
            for shape in [shown, inside, outside, unfilled, blend] {
                show(shape);
            }
            close(false);
            for shape in [shown, inside, outside, unfilled, blend] {
                assert_eq!(bits(shape), (true, true), "{shape:#x}");
            }
            // Frame n + 1 displays only `shown`.
            show(shown);
            let start = left();
            close(false);
            for (shape, visible) in [(shown, true), (inside, false), (outside, true), (unfilled, true), (blend, true), (never, false)] {
                assert_eq!(bits(shape), (visible, visible), "{shape:#x}");
            }
            assert_eq!(left() - start, 1);
            // Frame n + 2 displays only `shown`, after a root change.
            show(shown);
            close(true);
            for (shape, visible) in [(shown, true), (inside, false), (outside, false), (unfilled, true), (blend, false), (never, false)] {
                assert_eq!(bits(shape), (visible, visible), "{shape:#x}");
            }
            {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                for shape in shapes {
                    let instance = t.instances.remove(&shape).expect("a test instance");
                    t.lists.give_back(instance.list_key);
                }
                assert!(t.lists.listed(other).is_empty());
                for view in [main, other] {
                    t.view_planes[view as usize].clear();
                }
                t.mode = mode;
            }
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// Every entry of the view's lists is at the place that its key names, and no other key has
    /// a place.
    fn assert_places(mt: &MainThread, view: u8) {
        let t = tables(mt);
        let lists = &t.lists;
        let mut count = 0;
        for (list, entries) in lists.entries[view as usize].iter().enumerate() {
            for (index, &(_, key)) in entries.iter().enumerate() {
                assert_eq!(lists.place(view, key), (list as u32) << 31 | index as u32, "key {key}");
                count += 1;
            }
        }
        assert_eq!(lists.places[view as usize].iter().filter(|&&place| place != NOT_LISTED).count(), count);
    }

    /// A display or an end takes an entry out of its list, and the list's last entry takes its
    /// index. In a view with four entries the moved entry is displayed, or its shape ends, right
    /// after, in the same frame and in the next: each entry keeps the place its key names, and
    /// the closes keep, hide and drop the right shapes.
    #[test]
    fn an_entry_moved_by_a_take_out_keeps_its_place() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let view = 5u8;
            let shapes = [0x7927_0000usize, 0x7927_1000, 0x7927_2000, 0x7927_3000];
            let [a, b, c, d] = shapes;
            let mode = {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                assert!(t.lists.listed(view).is_empty(), "view {view} lists entries of another test");
                for (shape, id) in shapes.iter().zip(71..) {
                    let mut instance = Instance::new(id, 0, 1, 2, 0, false, false, false);
                    instance.filled = true;
                    t.instances.insert(*shape, instance);
                }
                t.mode.replace(Mode::Gpu)
            };
            let show = |shape: usize| {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                let instance = t.instances.get_mut(&shape).expect("a test instance");
                show_in_view(instance, shape, view, t.frame, &mut t.lists, &mut t.states, None);
                drop(tables_open);
                assert_places(&mt, view);
            };
            let mask = |shape: usize| tables(&mt).instances.get(&shape).map(|instance| instance.visible);
            let bit = Some(1u32 << view);
            // Frame n: all four; at the close they wait as a, b, c, d.
            for shape in shapes {
                show(shape);
            }
            close_views(&mt, [(view, false)]);
            assert_places(&mt, view);
            assert_eq!(waiting_shapes(&mt, view), vec![a, b, c, d]);
            // Frame n + 1: a goes out and d takes its index; then d itself is displayed, and c
            // takes the index; then b ends.
            show(a);
            assert_eq!(waiting_shapes(&mt, view), vec![d, b, c]);
            show(d);
            assert_eq!(waiting_shapes(&mt, view), vec![c, b]);
            assert!(forget(&mt, b));
            assert_places(&mt, view);
            assert_eq!(waiting_shapes(&mt, view), vec![c]);
            close_views(&mt, [(view, false)]);
            assert_places(&mt, view);
            assert_eq!((mask(a), mask(b), mask(c), mask(d)), (bit, None, Some(0), bit));
            assert_eq!(waiting_shapes(&mt, view), vec![a, d]);
            // Frame n + 2: a goes out and d takes its index; then d ends.
            show(a);
            assert_eq!(waiting_shapes(&mt, view), vec![d]);
            assert!(forget(&mt, d));
            assert_places(&mt, view);
            assert!(waiting_shapes(&mt, view).is_empty());
            close_views(&mt, [(view, false)]);
            assert_places(&mt, view);
            assert_eq!((mask(a), mask(d)), (bit, None));
            assert_eq!(listed_shapes(&mt, view), vec![a]);
            forget_views(&mt, &[view]);
            assert_places(&mt, view);
            {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                for shape in [a, c] {
                    let instance = t.instances.remove(&shape).expect("a test instance");
                    t.lists.give_back(instance.list_key);
                }
                t.mode = mode;
            }
            assert_eq!(capture::test_record_deletes(false), [72, 74]);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// A shape destroyed, and a new one at the same address displayed in the same view before
    /// the close: the old shape's entry leaves the view's list with it, and the new shape stays
    /// visible.
    #[test]
    fn a_new_shape_at_a_freed_address_stays_visible_at_the_close() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let a = 0x7930_0000usize;
            let insert = |id: u32| {
                let mut instance = Instance::new(id, 0, 1, 2, 0, false, false, false);
                instance.filled = true;
                tables(&mt).instances.insert(a, instance);
            };
            let show = || {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                let instance = t.instances.get_mut(&a).expect("a test instance");
                show_in_view(instance, a, 1, t.frame, &mut t.lists, &mut t.states, None);
            };
            let mode = tables(&mt).mode.replace(Mode::Gpu);
            insert(94);
            show();
            assert!(forget(&mt, a));
            insert(95);
            show();
            close_views(&mt, [(1, false)]);
            assert_eq!(tables(&mt).instances[&a].visible, 0b10);
            let key = tables(&mt).instances[&a].list_key;
            assert_eq!(tables(&mt).lists.listed(1), vec![(a, key)]);
            forget_views(&mt, &[1]);
            tables(&mt).instances.remove(&a);
            tables(&mt).mode = mode;
            assert_eq!(capture::test_record_deletes(false), [94]);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// A texture view whose items ran, with one shape shown in it: gives its slot and the shape.
    /// The view closes at a swap into its target and a flush runs its items.
    unsafe fn a_shown_texture_view(mt: &MainThread, target: usize, shape: usize, id: u32) -> u8 {
        let picture = Click { kind: SCENE_OTHER, camera: target + 0x100, root: target + 0x200, target: Some(target), world_camera: false };
        let slot = (view_open(mt, &picture) - 1) as u8;
        view_swap(mt, Some(target));
        view_flush(mt);
        take_retired_views(mt);
        let mut tables_open = tables(mt);
        let t = &mut *tables_open;
        let mut instance = Instance::new(id, 0, 1, 2, 0, false, false, false);
        instance.filled = true;
        t.instances.insert(shape, instance);
        let instance = t.instances.get_mut(&shape).expect("a test instance");
        show_in_view(instance, shape, slot, t.frame, &mut t.lists, &mut t.states, None);
        slot
    }

    /// The destructor of a rendered texture while the tables are open: no panic. The clear of
    /// its slot's marks waits for the next scene's start (`view_open`), and so does the slot's
    /// retirement, which goes only after the clear.
    #[test]
    fn a_target_destroyed_while_the_tables_are_open_waits_for_the_next_scene() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let a = 0x7940_8000usize;
            let slot = a_shown_texture_view(&mt, 0x7940_4000, a, 96);
            {
                let _held = tables(&mt);
                view_target_destroyed(&mt, 0x7940_4000);
            }
            assert_eq!(tables(&mt).instances[&a].visible, 1 << slot);
            assert_eq!(*PENDING_FORGETS.get(&mt), vec![slot]);
            assert!(RETIRED_VIEWS.get(&mt).is_empty(), "a retirement before the clear of the marks");
            let next = Click { kind: SCENE_OTHER, camera: 0x7941_0000, root: 0x7941_1000, target: Some(0x7941_4000), world_camera: false };
            assert!(view_open(&mt, &next) > 0);
            assert_eq!(tables(&mt).instances[&a].visible, 0);
            assert!(PENDING_FORGETS.get(&mt).is_empty() && RETIRED_VIEWS.get(&mt).is_empty());
            view_swap(&mt, Some(0x7941_4000));
            tables(&mt).instances.remove(&a);
            take_retired_views(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// The destructor of a rendered texture while the view table is open: the target waits for
    /// the next scene's start, which frees its slot and clears its marks.
    #[test]
    fn a_target_destroyed_while_the_view_table_is_open_waits_for_the_next_scene() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let a = 0x7950_8000usize;
            let slot = a_shown_texture_view(&mt, 0x7950_4000, a, 99);
            {
                let _held = VIEWS.get(&mt);
                view_target_destroyed(&mt, 0x7950_4000);
            }
            assert_eq!(*PENDING_TARGETS.get(&mt), vec![0x7950_4000]);
            assert_eq!(tables(&mt).instances[&a].visible, 1 << slot);
            let next = Click { kind: SCENE_OTHER, camera: 0x7951_0000, root: 0x7951_1000, target: Some(0x7951_4000), world_camera: false };
            assert!(view_open(&mt, &next) > 0);
            assert!(PENDING_TARGETS.get(&mt).is_empty());
            assert_eq!(tables(&mt).instances[&a].visible, 0);
            view_swap(&mt, Some(0x7951_4000));
            tables(&mt).instances.remove(&a);
            take_retired_views(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// A retirement clears a view's bit only in the instance that its list entry names: a new
    /// instance at the address of a stale entry keeps its bits.
    #[test]
    fn a_retirement_leaves_a_new_instance_at_a_stale_address_alone() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            let a = 0x7960_0000usize;
            {
                let mut tables_open = tables(&mt);
                let t = &mut *tables_open;
                t.lists.push(2, 0, a, 100);
                let mut instance = Instance::new(101, 0, 1, 2, 0, false, false, false);
                instance.visible = 0b100;
                t.instances.insert(a, instance);
            }
            forget_views(&mt, &[2]);
            assert_eq!(tables(&mt).instances[&a].visible, 0b100);
            assert!(listed_shapes(&mt, 2).is_empty());
            tables(&mt).instances.remove(&a);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// An instance under a billboard that a second view displays in a frame is counted once for
    /// that frame; a third view adds nothing; the next frame starts again. One not under a
    /// billboard is not counted.
    #[test]
    fn an_instance_under_a_billboard_in_two_views_of_a_frame_is_counted_once() {
        use core::sync::atomic::Ordering::Relaxed;
        let mut lists = ViewLists::new();
        let mut states = 0;
        let mut under = Instance::new(97, 0, 1, 2, 0, false, false, false);
        under.under_billboard = true;
        let mut plain = Instance::new(98, 0, 1, 2, 0, false, false, false);
        let count = || BILLBOARD_IN_TWO_VIEWS.load(Relaxed);
        let _turn = crate::main_thread::TestTurn::take();
        let start = count();
        unsafe {
            show_in_view(&mut under, 0x10, 1, 5, &mut lists, &mut states, None);
            show_in_view(&mut under, 0x10, 1, 5, &mut lists, &mut states, None);
            assert_eq!(count(), start);
            show_in_view(&mut under, 0x10, 0, 5, &mut lists, &mut states, None);
            assert_eq!(count(), start + 1);
            show_in_view(&mut under, 0x10, 2, 5, &mut lists, &mut states, None);
            assert_eq!(count(), start + 1);
            show_in_view(&mut under, 0x10, 0, 6, &mut lists, &mut states, None);
            assert_eq!(count(), start + 1);
            show_in_view(&mut under, 0x10, 1, 6, &mut lists, &mut states, None);
            assert_eq!(count(), start + 2);
            show_in_view(&mut plain, 0x20, 1, 6, &mut lists, &mut states, None);
            show_in_view(&mut plain, 0x20, 0, 6, &mut lists, &mut states, None);
            assert_eq!(count(), start + 2);
        }
    }

    /// A scene on the engine's path takes no slot: the local map's tile, the character preview
    /// and the water reflection come with view 0, and the slots in use do not change. A scene
    /// that the table serves takes one.
    #[test]
    fn a_scene_on_the_engine_path_takes_no_slot() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let in_use = || VIEWS.get(&mt).as_ref().map_or(0, ViewTable::in_use);
            let start = in_use();
            for kind in [SCENE_LOCAL_MAP, SCENE_CHARACTER_PREVIEW, SCENE_WATER_REFLECTION] {
                let click = Click { kind, camera: 0x7970_0000, root: 0x7970_1000, target: Some(0x7970_4000), world_camera: false };
                assert_eq!(view_of_scene(&mt, &click, route_of(kind, false, true)), (0, 0));
            }
            assert_eq!(in_use(), start);
            let picture = Click { kind: SCENE_OTHER, camera: 0x7970_0000, root: 0x7970_1000, target: Some(0x7970_4000), world_camera: false };
            assert!(view_of_scene(&mt, &picture, route_of(SCENE_OTHER, false, true)).0 > 0);
            view_swap(&mt, Some(0x7970_4000));
            take_retired_views(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    #[test]
    fn a_read_or_a_copy_that_writes_no_record_does_not_flush_the_views() {
        // With no stream (the test's recording has no state) neither writes its record, so the
        // helper does not flush: the table must not either (D82 design, 3.1 rule 1a).
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let picture = Click { kind: SCENE_OTHER, camera: 0x7910_0000, root: 0x7910_1000, target: Some(0x7910_4000), world_camera: false };
            assert!(view_open(&mt, &picture) > 0);
            let flushes = VIEWS.get(&mt).as_ref().map(ViewTable::flushes);
            assert_eq!(capture::copy_tile(0x7910_4000, 0x7910_5000), None);
            assert!(capture::read_pixels(0x7910_4000).is_none());
            assert_eq!(VIEWS.get(&mt).as_ref().map(ViewTable::flushes), flushes);
            view_swap(&mt, Some(0x7910_4000));
            take_retired_views(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    #[test]
    fn the_view_take_lines_name_a_second_slot_and_keep_their_rate() {
        assert!((1..=64).all(view_line_due));
        assert!(!view_line_due(65) && !view_line_due(127) && view_line_due(128) && view_line_due(1024));
        let again = key_words(ViewKey::Texture { camera: 0x10, root: 0x20, target: 0x30, again: 1 });
        assert!(again.ends_with("again 1 (a second slot for the key)"), "{again}");
        let first = key_words(ViewKey::Texture { camera: 0x10, root: 0x20, target: 0x30, again: 0 });
        assert!(first.ends_with("again 0"), "{first}");
        assert_eq!(key_words(ViewKey::Screen { camera: 0x10 }), "camera 0x10 into the back buffer");
    }

    /// A note before a display marks the shape at that display, once. A display with no new
    /// note does no lookup. A note between two displays (a mod's view that changes a property
    /// between its renders) reaches the next display.
    #[test]
    fn a_display_checks_each_new_note_once() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            let shape = 0x7600_0000;
            let mut properties = [0; PROPERTY_SLOTS];
            properties[0] = 0x7600_1000;
            let mut instance = Instance::new(76, 0, 1, 2, 0, false, false, false);
            instance.checked = change_count(&mt);
            let checks = (*CHANGE_NOTES.get(&mt))[0];

            note_change(&mt, properties[0]);
            check_notes(&mt, &mut instance, shape, &properties);
            assert!(instance.recheck, "the display after the note did not mark the shape");
            assert_eq!(instance.checked, change_count(&mt));
            // The next display, with no new note: no check.
            instance.recheck = false;
            check_notes(&mt, &mut instance, shape, &properties);
            assert!(!instance.recheck);
            assert_eq!((*CHANGE_NOTES.get(&mt))[0], checks + 1);
            // A note of another object: a check, no mark.
            note_change(&mt, 0x7600_2000);
            check_notes(&mt, &mut instance, shape, &properties);
            assert!(!instance.recheck);
            // A note of the shape itself between two displays.
            note_change(&mt, shape);
            check_notes(&mt, &mut instance, shape, &properties);
            assert!(instance.recheck);
            assert_eq!((*CHANGE_NOTES.get(&mt))[0], checks + 3);
            change_frame_end(&mt);
            assert_eq!((*CHANGE_TICKS.get(&mt))[1], 0);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// The display check marks the same shapes as a full scan of the table at the note, for
    /// the shapes displayed after the note. A shape not displayed in the note's frame is marked
    /// at its display in a later frame. A new instance is not marked by an older note of its
    /// address. A clear of the changed objects marks every shape checked before it.
    #[test]
    fn the_display_check_marks_the_same_shapes_as_a_full_scan() {
        let slots = |properties: &[usize]| {
            let mut copy = [0; PROPERTY_SLOTS];
            copy[..properties.len()].copy_from_slice(properties);
            copy
        };
        // 10 and 11 share property 100; 12 holds 101 in two slots; 13 holds 102.
        let shapes = [(10, slots(&[100, 0, 103])), (11, slots(&[100])), (12, slots(&[101, 101])), (13, slots(&[102]))];
        let set = |addresses: &[usize]| addresses.iter().copied().collect::<HashSet<usize>>();
        let by_scan = |changed: &HashSet<usize>| shapes.iter().filter(|(shape, properties)| changed.contains(shape) || properties.iter().any(|p| *p != 0 && changed.contains(p))).map(|(shape, _)| *shape).collect::<HashSet<usize>>();
        let all_notes = [set(&[100]), set(&[101]), set(&[13]), set(&[103, 102]), set(&[999]), set(&[100, 101, 102, 103, 10, 11, 12, 13, 999])];
        for changed in &all_notes {
            let mut notes = ChangeNotes::default();
            notes.note(5000);
            // Every shape is checked after the first note, then the notes of the case come.
            let checked = notes.count;
            for &address in changed {
                notes.note(address);
            }
            let by_display = shapes.iter().filter(|(shape, properties)| notes.hits(checked, *shape, properties)).map(|(shape, _)| *shape).collect::<HashSet<usize>>();
            assert_eq!(by_display, by_scan(changed), "notes {changed:?}");
            // Checked after the notes: no shape is hit again.
            assert!(shapes.iter().all(|(shape, properties)| !notes.hits(notes.count, *shape, properties)));
        }
        assert_eq!(by_scan(&set(&[100])), set(&[10, 11]));

        // Frame 1: a note of 100; only shape 12 displays. Frame 2: a note of 101; 10 displays.
        let mut notes = ChangeNotes::default();
        notes.note(100);
        let (properties_10, properties_12) = (&shapes[0].1, &shapes[2].1);
        assert!(!notes.hits(0, 12, properties_12));
        let checked_12 = notes.count;
        notes.note(101);
        assert!(notes.hits(0, 10, properties_10), "a shape not displayed in the note's frame missed it");
        assert!(notes.hits(checked_12, 12, properties_12));

        // A new instance at an address that a note named before it.
        let made = notes.count;
        assert!(!notes.hits(made, 100, &slots(&[])));
        assert!(!notes.hits(made, 77, &slots(&[101])));

        // The cap: one changed object more clears them all; every shape checked before rechecks.
        let mut notes = ChangeNotes::default();
        for address in 0..CHANGED_CAP {
            notes.note(0x1_0000 + address);
        }
        let checked = notes.count;
        assert_eq!(notes.clears, 0);
        notes.note(0x9_0000);
        assert_eq!((notes.clears, notes.changed.len()), (1, 1));
        assert!(notes.hits(checked, 10, properties_10), "a clear did not mark a shape checked before it");
        assert!(!notes.hits(notes.count, 10, properties_10));
        // A note again of an object already kept clears nothing.
        notes.note(0x9_0000);
        assert_eq!(notes.clears, 1);
    }

    /// A shape that the engine does not display after a reset (a culled weapon, a shape out of
    /// the view) and that is then destroyed: the helper holds its instance, so the delete must
    /// go, or the helper draws it at its last place for the rest of the session.
    #[test]
    fn a_shape_destroyed_after_a_reset_sends_its_delete() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let mut forgotten = Instance::new(77, 0, 1, 2, 0, false, false, false);
            forgotten.filled = true;
            forgotten.sent = true;
            let mut rejected = Instance::new(78, 0, 1, 2, 0, false, false, false);
            rejected.filled = true;
            rejected.sent = true;
            // The rejection clears the shape's mark, so it needs flags to write.
            let mut rejected_shape = [0u8; 64];
            let rejected_at = rejected_shape.as_mut_ptr() as usize;
            let forgotten_at = 0x7700_0000;
            tables(&mt).instances.insert(forgotten_at, forgotten);
            tables(&mt).instances.insert(rejected_at, rejected);

            reset(&mt, "a test");
            assert!(!tables(&mt).instances[&forgotten_at].sent, "the reset did not clear the flag");
            assert!(forget(&mt, forgotten_at));
            reject_pending(&mt, rejected_at, false);
            tables(&mt).rejected.remove(&rejected_at);

            let deletes = capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
            assert_eq!(deletes, [77, 78]);
        }
    }

    #[test]
    fn an_effect_watch_grows_its_version_only_when_the_words_change() {
        let mut watch = EffectWatch::default();
        let mut words = [0u32; EFFECT_SIGNATURE_WORDS];
        words[4] = 0.5f32.to_bits();
        let first = watch.observe(1, || words);
        assert_ne!(first, 0);
        // The same words in the next frame: the same version, so no note.
        assert_eq!(watch.observe(2, || words), first);
        // The projection turned: a new version, so the material is noted again.
        let mut turned = words;
        turned[4] = 0.0f32.to_bits();
        turned[5] = 0.5f32.to_bits();
        let second = watch.observe(3, || turned);
        assert_ne!(second, first);
        // Read at most once a frame: a second read in frame 3 is not made.
        assert_eq!(watch.observe(3, || panic!("read twice in a frame")), second);
    }

    #[test]
    fn a_freed_texture_marks_only_the_instances_of_its_materials() {
        let gone: HashSet<u32> = [7u32, 9].into_iter().collect();
        let mut instances = vec![(7u32, 5u32), (8, 5), (9, 4)];
        let marked = mark_for_note(instances.iter_mut().map(|(m, g)| (*m, g)), &gone, 5);
        assert_eq!(marked, 2);
        // Material 7 and 9: another generation than the tables', so a note at the next display.
        assert_ne!(instances[0].1, 5);
        assert_ne!(instances[2].1, 5);
        // Material 8 named no freed texture: unchanged.
        assert_eq!(instances[1].1, 5);
    }

    #[test]
    fn the_first_plane_with_the_bound_outside_is_named() {
        assert_eq!(rejecting_plane([0u32, 1, 0, 0, 0, 0].into_iter()), None);
        // Side 2 is outside: the far plane (index 1) before the top one.
        assert_eq!(rejecting_plane([0u32, 2, 0, 0, 2, 0].into_iter()), Some(1));
        assert_eq!(PLANE_NAMES[1], "far");
    }

    #[test]
    fn an_effect_that_gives_no_stage_still_has_a_version() {
        let mut watch = EffectWatch::default();
        assert_ne!(watch.observe(0, || [0; EFFECT_SIGNATURE_WORDS]), 0);
    }

    /// Instances for the tests of the main view's list (form D), in mode gpu. Each shape is a
    /// block of zeroed memory, so that `draw` can read it, and its instance has the values of a
    /// draw with the zero world: a draw of it takes the table's branch and writes no record.
    struct ListTest {
        _memory: Vec<Vec<u32>>,
        shapes: Vec<usize>,
        mode: Option<Mode>,
    }

    impl ListTest {
        fn new(mt: &MainThread, ids: &[u32]) -> ListTest {
            let memory: Vec<Vec<u32>> = ids.iter().map(|_| vec![0u32; 0x100]).collect();
            let shapes: Vec<usize> = memory.iter().map(|block| block.as_ptr() as usize).collect();
            let mut tables_open = tables(mt);
            let t = &mut *tables_open;
            for (&shape, &id) in shapes.iter().zip(ids) {
                let mut instance = Instance::new(id, CLASS_UNKNOWN, 1, 2, t.generation, false, false, false);
                instance.filled = true;
                instance.sent = true;
                t.instances.insert(shape, instance);
            }
            t.shown.drop_all();
            let mode = t.mode.replace(Mode::Gpu);
            ListTest { _memory: memory, shapes, mode }
        }

        /// The table as it was before the test.
        fn end(self, mt: &MainThread) {
            let mut tables_open = tables(mt);
            let t = &mut *tables_open;
            for shape in &self.shapes {
                if let Some(instance) = t.instances.remove(shape) {
                    t.lists.give_back(instance.list_key);
                }
            }
            t.shown.drop_all();
            t.mode = self.mode;
        }
    }

    /// A draw of the retained shape, with the values of [`ListTest`].
    unsafe fn table_draw(mt: &MainThread, shape: usize, id: u32) -> bool {
        draw(mt, shape, id, &[0.0; 16], true, &[0; 14], None, None, 0xFF00_0000, &[], &[], 0, false)
    }

    /// The ids of the main view's send in progress.
    fn listed_ids(mt: &MainThread) -> Vec<u32> {
        tables(mt).shown.ids.clone()
    }

    /// A world display joins view 0's list once, however often the engine displays the shape
    /// before the send.
    #[test]
    fn a_world_display_joins_view_0s_list_once() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let test = ListTest::new(&mt, &[71, 72]);
            let (a, b) = (test.shapes[0], test.shapes[1]);
            scene_begin(&mt, SCENE_WORLD, 0, Route::Main, 1);
            assert!(table_draw(&mt, a, 71));
            assert!(table_draw(&mt, a, 71));
            assert!(table_draw(&mt, b, 72));
            assert!(table_draw(&mt, a, 71));
            assert_eq!(listed_ids(&mt), vec![71, 72]);
            assert_eq!(take_shown_list(&mt), Some(vec![71, 72]));
            test.end(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// The water scene's displays (the queue's flush at its end) join the same list as the
    /// world's: one list for the frame, each id once.
    #[test]
    fn a_water_display_joins_the_same_list() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let test = ListTest::new(&mt, &[73, 74]);
            let (a, b) = (test.shapes[0], test.shapes[1]);
            scene_begin(&mt, SCENE_WORLD, 0, Route::Main, 1);
            assert!(table_draw(&mt, a, 73));
            scene_end(&mt);
            scene_begin(&mt, SCENE_WATER, 0, Route::Main, 1);
            assert!(table_draw(&mt, a, 73));
            assert!(table_draw(&mt, b, 74));
            scene_end(&mt);
            assert_eq!(take_shown_list(&mt), Some(vec![73, 74]));
            test.end(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// At Present the list is the record before `FRAME_END`; the next send starts empty, and a
    /// frame with no world or water scene into the back buffer sends no list.
    #[test]
    fn the_list_goes_before_frame_end_and_starts_again() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let test = ListTest::new(&mt, &[75]);
            let a = test.shapes[0];
            scene_begin(&mt, SCENE_WORLD, 0, Route::Main, 1);
            assert!(table_draw(&mt, a, 75));
            let shown = take_shown_list(&mt);
            let records = capture::frame_end_records(5, 9, shown.as_deref(), &[3]);
            let tags: Vec<u32> = records.iter().map(|(tag, _)| *tag).collect();
            assert_eq!(tags, vec![capture::tag::INSTANCE_SHOWN, capture::tag::FRAME_END, capture::tag::FRAME_EVENT]);
            assert_eq!(records[0].1, [1u32.to_le_bytes(), 75u32.to_le_bytes()].concat());
            // No scene of the list since the send: no list at the next Present.
            assert_eq!(take_shown_list(&mt), None);
            // The next frame's world scene lists the shape again, in a new send.
            scene_begin(&mt, SCENE_WORLD, 0, Route::Main, 1);
            assert_eq!(take_shown_list(&mt), Some(vec![]));
            scene_begin(&mt, SCENE_WORLD, 0, Route::Main, 1);
            assert!(table_draw(&mt, a, 75));
            assert_eq!(take_shown_list(&mt), Some(vec![75]));
            test.end(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// Mode off writes no list: the switch drops the list so far, and a world scene in mode off
    /// opens none.
    #[test]
    fn mode_off_writes_no_list() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let test = ListTest::new(&mt, &[76]);
            let a = test.shapes[0];
            scene_begin(&mt, SCENE_WORLD, 0, Route::Main, 1);
            assert!(table_draw(&mt, a, 76));
            set_mode(&mt, Mode::Off);
            assert_eq!(take_shown_list(&mt), None);
            scene_begin(&mt, SCENE_WORLD, 0, Route::Main, 1);
            assert_eq!(take_shown_list(&mt), None);
            assert!(listed_ids(&mt).is_empty());
            test.end(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// The two predicates on one kind set: `may_queue` takes the world and the water scene into
    /// any target (the picture of a save keeps the engine's sort); the list takes them only into
    /// the back buffer (`Route::Main`). No other kind is in the set.
    #[test]
    fn may_queue_needs_the_kind_and_the_list_needs_the_kind_and_route_main() {
        use crate::link::{SCENE_FIRST_PERSON, SCENE_SHADOW_CASTERS, SCENE_SHADOW_VOLUME, SCENE_SORTED_ALPHA, SCENE_SPLASH, SCENE_UI};
        assert!(main_view_kind(SCENE_WORLD) && main_view_kind(SCENE_WATER));
        for kind in [SCENE_OTHER, SCENE_SORTED_ALPHA, SCENE_SHADOW_VOLUME, SCENE_SHADOW_CASTERS, SCENE_FIRST_PERSON, SCENE_UI, SCENE_SPLASH, SCENE_LOCAL_MAP] {
            assert!(!main_view_kind(kind), "kind {kind}");
            assert!(!lists_main_view(kind, Route::Main), "kind {kind}");
        }
        assert!(lists_main_view(SCENE_WORLD, Route::Main));
        assert!(lists_main_view(SCENE_WATER, Route::Main));
        assert!(!lists_main_view(SCENE_WORLD, Route::Places));
        assert!(!lists_main_view(SCENE_WORLD, Route::Engine(ENGINE_PATH_SCENES.len())));
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let test = ListTest::new(&mt, &[]);
            scene_begin(&mt, SCENE_WORLD, 0, Route::Places, 2);
            assert!(may_queue(&mt));
            scene_begin(&mt, SCENE_FIRST_PERSON, 0, Route::Main, 0);
            assert!(!may_queue(&mt));
            scene_begin(&mt, SCENE_WATER, 0, Route::Main, 1);
            assert!(may_queue(&mt));
            scene_end(&mt);
            test.end(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// A display in a scene into a rendered texture (a mod's mirror) writes its place and joins
    /// no list.
    #[test]
    fn a_display_into_a_rendered_texture_writes_a_place_and_joins_no_list() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let test = ListTest::new(&mt, &[77]);
            let a = test.shapes[0];
            scene_begin(&mt, SCENE_OTHER, 0, Route::Places, 3);
            let places = *VIEW_PLACES.get(&mt);
            walk_place(&mt, a);
            assert_eq!(*VIEW_PLACES.get(&mt), places + 1);
            shown_in_scene(&mut tables(&mt), a);
            assert!(listed_ids(&mt).is_empty());
            assert_eq!(take_shown_list(&mt), None);
            test.end(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// A world scene into a texture (the picture of a save): `draw` still takes the table's
    /// branch (the mark, and the place of the scene's view), and the shape joins no list.
    #[test]
    fn a_world_scene_into_a_texture_keeps_the_table_branch_and_joins_no_list() {
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let test = ListTest::new(&mt, &[78]);
            let a = test.shapes[0];
            scene_begin(&mt, SCENE_WORLD, 0, Route::Places, 2);
            let places = *VIEW_PLACES.get(&mt);
            assert!(table_draw(&mt, a, 78));
            assert_eq!(*VIEW_PLACES.get(&mt), places + 1);
            assert_ne!(rd::<u16>(a, FLAGS) & RETAINED_BIT, 0);
            assert!(listed_ids(&mt).is_empty());
            assert_eq!(take_shown_list(&mt), None);
            test.end(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }

    /// A retained shape displayed in a scene into the back buffer of a kind outside the main
    /// view's list (first person, the interface) goes back to the engine: the display is not
    /// skipped, the mark stays, and the shape joins no list.
    #[test]
    fn a_display_in_a_first_person_scene_goes_back_to_the_engine() {
        use crate::link::{DRAW_SHAPE, SCENE_FIRST_PERSON, SCENE_UI};
        let _turn = crate::main_thread::TestTurn::take();
        unsafe {
            let before = crate::shadow::main_thread_for_test();
            let mt = MainThread::claim();
            capture::test_record_deletes(true);
            let test = ListTest::new(&mt, &[79]);
            let a = test.shapes[0];
            mark(a, true);
            let handed_back = *HANDED_BACK.get(&mt);
            for kind in [SCENE_FIRST_PERSON, SCENE_UI] {
                scene_begin(&mt, kind, 0, Route::Main, 0);
                assert!(!skipped_display(&mt, a, DRAW_SHAPE), "kind {kind}");
                assert_ne!(rd::<u16>(a, FLAGS) & RETAINED_BIT, 0, "kind {kind}");
            }
            assert_eq!(*HANDED_BACK.get(&mt), handed_back + 2);
            assert_eq!(take_shown_list(&mt), None);
            test.end(&mt);
            capture::test_record_deletes(false);
            crate::shadow::restore_main_thread(before);
        }
    }
}

/// For the tests of `own_host`: a world scene into the back buffer opens the main view's list,
/// and the instance `id` joins it.
#[cfg(test)]
pub fn test_list_one(mt: &MainThread, id: u32) {
    let mut tables_open = tables(mt);
    let t = &mut *tables_open;
    t.shown.drop_all();
    t.shown.open = true;
    let mut listed_send = 0;
    t.shown.add(id, &mut listed_send);
}

/// For the tests of `own_host`: the ids of the main view's send in progress, whether a send
/// goes, and the sends at a read of the back buffer so far.
#[cfg(test)]
pub fn test_shown_state(mt: &MainThread) -> (Vec<u32>, bool, u64) {
    let t = tables(mt);
    (t.shown.ids.clone(), t.shown.open, t.shown.sends[1])
}
