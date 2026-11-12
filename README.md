# nirender

Written for: a person or an agent who works on this repo.

nirender is an MWSE plugin for Morrowind. It replaces the renderer of the game. The game
makes no Direct3D device. The plugin writes each frame as records into a shared-memory ring.
A 64-bit helper, `numidium.exe` of MGE XE G7, reads the ring and draws the frame with wgpu.

## The parts

| Part | Path | Builds to |
| --- | --- | --- |
| The plugin (32-bit) | `src/` | `target/i686-pc-windows-msvc/release/akulakhan.dll` |
| The `d3d8.dll` of the game folder (32-bit) | `d3d8/` | a `d3d8.dll` that loads the plugin and gives `Direct3DCreate8` to it; it takes the place of the `d3d8.dll` of MGE XE |
| The host crate `nirender_wgpu` (64-bit) | `host/` | a library; the helper of MGE XE G7 depends on it by path. The same crate is the program `nirender-host` (`host/src/main.rs`), which draws a capture file without the game |
| The contract of the render link | `contract/mge_render_link.h` | the C side of `src/link.rs`; `tools/check_contract.py` compares the two |
| The Lua side of the plugin | `mod/MWSE/mods/nirender/` | installed with the DLL; `mod/meshes/` has the NIF test level |
| The tools | `tools/` | Python scripts (Python 3.12 with Pillow; the batch files take the interpreter from `local.bat`) |
| The spike | `spikes/wgpu32/` | the test of 2026-10-10 that wgpu runs in a 32-bit process; kept for its numbers |

The helper is in the sibling repo `MGE-XE-G7-render-link`. A change of `host/` changes the
helper build. Build the plugin before `cargo xtask build` of that repo.

## Known items

- **The renderer needs our MWSE fork.** Use the test line of our MWSE fork (today the branch
  test/abi-fixes-15 of the MWSE repo). Stock MWSE does not have the native ABI that the plugin
  and MGE XE use. It also does not have the fixes of the MWSE risks that the project closed in
  the fork (risks 25, 31 and 36; the owner's D158).
- **The list of the fork-only changes** is `Numidium Project/notes/mwse-changes.md` in the docs
  repo. It is checked each week against MWSE upstream (the owner's D157).
- **The project may keep the MWSE fork for a while** (the owner's D159).

## How the plugin works

- The ports: Rust functions that replace the renderer functions of the game, one for one.
  `PORTING.md` says how to write one. `src/ports/` holds them.
- The records: the plugin writes the objects, the shapes, the geometry, the materials, the
  lights and the draws of a frame into the ring (`src/capture.rs`). A capture file
  (`.nircap`) holds the same records, for a replay offline.
- The display: `src/ports/scene.rs` holds the 7 ports of the display of a shape (the states,
  the accumulator, the controllers, the render call) and of the destructor of a geometry.
  The traversal of the nodes is the engine's own code.
- The retained instances: `src/instance_stream.rs` keeps a table of the shapes the helper
  holds. The plugin skips their display and sends a record only when a shape changes (the
  records 23 to 26 and 28). It also sends the list of the instances that the engine displayed in
  the main view (INSTANCE_SHOWN, 49; form D). The helper draws them from its own table (`host/src/retained.rs`).
- The modes: `NIRENDER_RETAINED=off|gpu`, default `gpu`. `off` is the reference: the
  engine's own path. `nirender.retained(mode)` changes the mode in a live game; another mode
  changes nothing and writes a line to the log.
- The threads: a port runs its Rust code on the main thread only. The cell worker's calls
  run the engine function and reach the plugin as events (`src/worker_events.rs`). A table
  of the main thread opens only for a proof that the caller is on it
  (`src/main_thread.rs`): the dispatch of a port gives the proof to a port declared with
  `rust_main`, and the Lua gate and the frame hook claim it.
- The views (D82): each render of the engine that the table serves (a `NiCamera::Click`) is
  a scene of a view, and each view has a slot (`src/view_slots.rs`); the helper keeps its
  side in `host/src/engine/views.rs`. A mod asks for the parts of MGE XE in a view that it
  draws itself with `mge.camera.setViewOptions` (`src/view_options.rs`); with
  `intoTextures` the options are the camera's default for each texture it draws into (D93).
- The count writes: the plugin locks the reference count writes of the engine's objects on
  the property paths, and the Rust ports change counts atomically (`src/atomic_count.rs`,
  risk 56). The plugin's log names them: `atomic count: N of N sites locked (...)`; when the
  bytes of a site are not the game's, all sites are refused and the line says so.
- The comparison rig (the cargo feature `rig`, not in a player's build): a port can also
  run in the mode shadow, which runs the Rust and the engine function both and compares
  them call by call (`src/shadow.rs`), and the frame comparison compares the device calls
  of two frames (`src/frame.rs`). The rig needs a real Direct3D 8 device: a test run with
  `tools/rig/`, never a player's game.

## Build and test

```text
cargo build --release                   the plugin
cargo test --release                    the tests of the plugin
cargo test --release --features rig     the tests of the plugin with the comparison rig
cd host && cargo test --release         the tests of the host crate (its own workspace)
```

The gate of a change: `tools/validate_retained.py --mge-build <folder>`. Pass the folder of
the current helper build with its `d3d8.dll` and `Data Files`. The stages:

1. `capture`: a run at Seyda Neen and at Narsis with a capture, the counts, a replay.
2. `baseline`: the kept captures of `results/baseline-records` against the accepted pictures.
3. `only`: a run with the frame rate, the port times and the busy time of the helper.
4. `clusters`: two replays of the capture, with the light clusters and with the lights of the
   draws, compared; then the frame rate and the GPU time of each.
5. `weathers`: the frame rate in each of the 10 weathers, two passes.
6. `tour`: the test level in one session; at each stop two frames in mode gpu and two in mode
   off, which `tools/validate_tour.py` replays and compares.

`results/` is not in git. The accepted pictures live only on disk.

The checks of the repo itself: `tools/check_readme.py` (every switch the code reads is in
this file; every path this file names exists) and `tools/check_contract.py` (the header and
`src/link.rs` agree).

Other tools: `tools/replay.py` (a capture to a picture), `tools/picture_diff.py` and
`tools/diff_crops.py` (two pictures compared), `tools/capture_dump.py` (the records of a
capture), `tools/lights_count.py` (what clustered distant lights would hold, from a
`lights.data`), `tools/run_own_device.py` (a game run with the harness mods).

## Switches

Never for a player. Each is an environment variable of the game process. The helper's own
switches (`MGE_XE_*`) are in `host/src/switches.rs`.

The run:

- `NIRENDER_HOST_EXE=<path>`: the host program to start; without it, the one in the folder
  of the plugin.
- `NIRENDER_HOST_ARGS=<text>`: more arguments for the host program.
- `NIRENDER_HOST_LOG=<path>`: the file for the text of the host program; without it,
  `nirender-host.log` in the folder of the game.
- `NIRENDER_BACKEND=vulkan|dx12`: the backend of the host program.
- `NIRENDER_PLUGIN_DIR=<path>`: the plugin's folder for `cargo xtask` of MGE XE; without it,
  the sibling folder. `tools/merge_step.py` removes it from its stages.
- `NIRENDER_RING_MB=<n>`: the size of the ring in MB, 1 to 512, default 128 (a texture
  level must fit it whole; a 4096 x 4096 ARGB level is 64 MiB); a value outside is named
  in the plugin's log and 128 is taken.
- `NIRENDER_PLUGIN_DLL=<path>`: the tools install that DLL for a run (one arm of an A/B); the runner's `--plugin-dll <path>` says the same without a variable.
- `MGE_XE_TEXTURE_TIER=a|c`: the texture binding tier of the helper, for an A/B on one
  machine. `a`: the binding arrays (the default when the adapter has them). `c`: no binding
  arrays, a small group of the 6 textures and 6 samplers of each set (the default on an
  adapter without them). The helper's log and the probe say `texture tier: a` or `c`. Tier
  `a` forced on an adapter without the arrays is a refusal (the code 20).
- `MGE_XE_OLD_GPU=least|downlevel`: the helper and its probe take the graphics card as an
  old one, for a test of the path of old hardware on any machine: the BC compression and the
  line mode are the only features (texture tier C, no float depth, no time stamps, no first
  instance or count of indirect draws), and the device has the limits of the profile, so that
  wgpu refuses anything above them. `least`: the least that the renderer asks for (32 sampled
  textures in a stage, wgpu's defaults for the rest), with textures of 16384 as every
  Direct3D 12 card has. `downlevel`: wgpu's downlevel limits, which the probe refuses with the
  lacks named. The log and the probe's line say `MGE_XE_OLD_GPU=<profile>`. Not emulated: the
  sample counts of multisampling and the formats' capabilities stay those of the real card.
- `MGE_XE_MEMORY_MINUTES=<n>`: the helper's log gets a memory report at the first frame and
  then every n minutes, as `memory:` lines: the GPU memory of wgpu's allocator by the kind of
  object (the label, its numbers as `#`), and the counts of wgpu's objects; 0 or unset, none.
  For a long run (risk 43).
- `MGE_XE_BOX_SECONDS=<n>`: a message box of the plugin, of the helper or of MGE XE's
  d3d8.dll closes itself after n seconds, and its text is in that part's log as `box:` lines
  (always); unset or 0, the box waits for the click. The runner sets 5.
- `NIRENDER_FOREGROUND=1`: the tools run the game in the foreground; a run is a background
  run without it. For a background run the runner stages a copy of the instance's MWSE.json
  with MWSE's setting `RunInBackground = true` into the run mod; the mod patches nothing.
- `NIRENDER_WALK_MINUTES=<n>`: the minutes of the walk of a land walk batch (the batch's
  default without it, 60 in the hour walks). The runner refuses a run whose `--budget` is
  shorter than the walk and 120 s.
- `NIRENDER_NOTIFIER=0`: the mod does not load the stand-in Lua change notifier.

The places of this machine (the harness, the mwse-mcp package, the OBS recorder, the video
folder, the Code Patch files of the unit test) are in `local.json` at the repo root, and the
places of the batch files (the Python interpreter, the MGE XE build, the Outlander-test
instance) in `local.bat`. Neither is tracked; `local.example.json` and `local.example.bat`
name the keys. A key that is not there takes the tool's default: a place next to this repo or
under the home folder.

The A/B:

- `NIRENDER_PORT_TIMING=1` times each live port call.
- `NIRENDER_COMPACT_MOVES=check` sends a move of a retained instance as `INSTANCE_MOVE` and as
  the whole `INSTANCE` record, and the helper counts the moves whose values differ from the
  full record's.
- `NIRENDER_UV_STEPS=check` sends a new revision of a geometry under a UV controller as
  `GEOMETRY_UV` and whole, and the plugin compares the step from the sent coordinates with the
  engine's (the geometry line of the log).
- `NIRENDER_MORPH_WEIGHTS=check` sends a new revision of a geometry under a morph controller
  as `MORPH_WEIGHTS` and whole, and the plugin compares the weighted targets with the engine's
  positions (the geometry line of the log).
- `NIRENDER_GEOMETRY_CENSUS=1` counts the geometries sent again, by the controllers of their
  shape, and logs the census at each stream mark (`geometry_stream::census_take`).
- `NIRENDER_EFFECT_TURN_CHECK=1` (a test switch): at the end of each world scene the plugin
  sends the engine's projection of each texture effect under a turning billboard, after the
  billboard's display, and clears the billboard's displayed flag. The helper compares it with
  its own (`host/src/effect_turn.rs`) and reports the largest difference by the pair of modes.
- `MGE_XE_DEPTH_FORMAT=24` keeps the depth of the frame in `Depth24PlusStencil8` when the
  adapter has `Depth32FloatStencil8` (the format before 2026-10-27; the helper's log line
  `depth:` names the format in use).
- `MGE_XE_ONE_DEPTH=1` draws the distant land and the game into one depth buffer, with an
  infinite far plane for every perspective projection of the world view (step 6 of the depth
  design; only with the reversed float depth). Off: depth step 6 stopped.
- `NIRENDER_ANIMATION=view|shadows|all`: which actors animate (`nirender.animationPolicy`);
  default the view and the shadow reach.
- `NIRENDER_NOTE_EVERY=1`: the material and the geometry of every retained shape are noted at
  every display, not every 64th frame (`nirender.noteEvery`).
- `NIRENDER_SKIP_PORT=<name>`: one port left uninstalled, to measure its cost. Only a port
  whose function is not the renderer's.
- `NIRENDER_SKY=count` keeps the sky on the engine's records and logs each sky shape once.
  The default: the sky's instances in the table.
- `NIRENDER_WEATHER=on` puts the rain, snow and storm drops (class 9) into the table, drawn at
  the engine's places as the sky is (`nirender.weatherDraws(false)` turns their draws back to
  the engine's records in a session); `NIRENDER_WEATHER=count` logs each drop shape once. The
  default (`off`): the engine's records; the takeover of the drops is milestone 11.
- `NIRENDER_CAPTURE_READS_MS=N`: the time in milliseconds for a capture's reads of the
  rendered textures, 500 by default, for a test of the stop (`src/own_host.rs`).
- `NIRENDER_PACE_LEAD_US=<n>`: the lead of the pacing of the game to the helper
  (`src/pacing.rs`) in microseconds (default 200): the frame is complete this long before the
  helper is free for it. For a test.

The instruments and the tests:

- `NIRENDER_ENTRY_TRACE=1`: an entry into the plugin made while another is live writes the
  outer entry, the call into the engine between them and the inner entry to the log, once
  for each triple (`src/entry_trace.rs`; the re-entrancy map in the docs reads these lines).
- `NIRENDER_FILL_FREED=1`: every block the engine frees is filled with a pattern first; a
  read of freed memory faults at the read (`src/freed_fill.rs`).
- `NIRENDER_STATE_SUMMARY=1`: `nirender.stateSummary()` gives a summary of the state of the
  engine and of the plugin by part, for the test that a capture leaves the game as it found it
  (`src/state_summary.rs`).
- `NIRENDER_FORCE_GROWTH=<frames>`: the helper makes its record buffer again every so many
  world frames, as a growth does; with `NIRENDER_GROWTH_SHOTS=<prefix>` it takes the picture
  of each growth frame and of the frame after it (`host/src/player/items.rs`).
- `NIRENDER_FORCE_SEGMENT_GROWTH=<n>`: the segment of the table doubles in the frame of the
  n-th draw into a small rendered texture (a fog texture of the map), for the test of a growth
  in the middle of a frame (`host/src/player/draws.rs`).
- `NIRENDER_FOG_CHECK=1`: each lock of a fog texture also reads the GPU and compares it with
  the CPU copy; `NIRENDER_FOG_DUMP=<folder>` writes the first 8 pairs that differ.
- `NIRENDER_MAP_TILES_HOLD=<N>`: a map tile completes only N Presents after it opened, so that
  a save or a purge soon after a cell cross finds open tiles.
- `NIRENDER_MCPATCH=<folders>` and `NIRENDER_VANILLA_EXE=<path>`: the inputs of the unit
  test that checks every Morrowind Code Patch record inside a ported function
  (`src/install.rs`); without them the test uses the paths of this machine.
- `NIRENDER_MWSE_DLL=<path>`: the MWSE.dll that the unit test `every_mwse_site_in_a_port_is_followed`
  checks against the ports (`src/mwse_sites.rs`); `tools/check_mwse_sites.py` sets it. Without it
  the test is skipped; with it and no file, the test fails.

The helper's diagnostic and test switches (`host/src/switches.rs`, read once; none is a
setting). The one of the A/B list above (`MGE_XE_DEPTH_FORMAT`) is there too. MGE XE's own
switches are in the RENDERER.md of the MGE-XE-G7 render-link worktree. One of them,
`MGE_XE_CORE_MODS=0` (no core mod joins the shaders), is set by the baseline gate for its
replays.

- `MGE_XE_CLUSTERS=1` or `0`: the clusters of the lights on or off, over the setting. The
  baseline gate replays with both.
- `MGE_XE_LIGHTS=world`: a lit draw of the world takes the point lights of the world that
  reach its pixel (the clusters of `WORLD_LIGHTS`), not the lights the engine chose for it.
- `NIRENDER_BLIND_LIGHT_KEY=0`: a draw lit for each pixel whose colour cannot take light (the
  diffuse and the ambient colours from the material and black) keeps its lights. On by
  default, the draw drops them (`host/src/light_keys.rs`); the picture is the same.
- `NIRENDER_ZERO_LIGHT_KEY=0`: a draw lit for each pixel with no lights keeps the light loop in
  its shader. On by default, the draw takes `NO_LIGHTS_BIT` and its stage key leaves the loop
  out; the picture is the same.
- `NIRENDER_OBJECTS_WARN_ALL=1`: each steady frame that makes GPU objects gives its warning
  line in the log (`host/src/objects.rs`). Unset, the first 10 such frames give one, and then
  each 600th; the report at the end counts them all.
- `MGE_XE_INDIRECT=0`: one draw call for each retained draw, no indirect runs: it rules the
  indirect runs out when a picture is wrong, and goes around a driver's fault in them.
- `MGE_XE_SAMPLES=<n>`: the samples for each pixel of the targets of the frame (1, 2, 4 or 8),
  over the setting of the extension. A count the device cannot use is lowered: without
  TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES only 1 and 4 stand (`host/src/request.rs`,
  `choose_samples`). The log says `samples: asked N, using M`.
- `MGE_XE_FRAME_LATENCY=<n>` (1 to 3): the frames that may wait for the GPU, over the setting
  of the extension: for an A/B of the mouse's lag.
- `MGE_XE_WAIT_YIELDS=<n>`: the looks with a yield before the helper blocks on the ring's event
  when no record waits (256 without it); 0 blocks at once, also while the game writes its
  frame. For the A/B of the helper's CPU in its wait for the game.
- `MGE_XE_PRESENT=immediate|mailbox|fifo|auto`: the present mode of the window, over the vsync
  setting of the extension (`graphics.vsync` of mgeXE.toml: FIFO with vsync on, else
  `AutoNoVsync`).
- `MGE_XE_WINDOW=game`: the surface goes on the window of the game itself, not on a child
  window of it: for an A/B of the mouse's lag.
- `MGE_XE_WATCHDOG=0`: no watchdog of the frame thread in the helper (`host/src/watchdog.rs`):
  for a test of the plugin's own limit, the allowance plus 5 s, on a helper that reads no record.
- `MGE_XE_CORES=0`: the threads of the helper are left to the scheduler.
- `MGE_XE_ARRAY_CAP=<n>`: the room of the texture arrays at most n, for the test of a full
  room; below 256 the adapter goes to texture tier C. With `MGE_XE_TEXTURE_TIER=a` the request
  refuses it, for the test of `numidium.exe --probe` and of the live start with a GPU the
  renderer cannot use (the exit code 20); `MGE_XE_OLD_GPU=downlevel` is the other way.
- `MGE_XE_RECORD_LIMIT=<n>`: at most n draw records (at least 1024), for the test of the
  guard of the device's limit.
- `MGE_XE_GPU_TIME=1`: time stamps of the parts of the frame (they cost time).
- `MGE_XE_STAGE_SHADERS=0`: no fragment shader for each stage state of the draws
  (`host/src/stage_shaders.rs`); the world shader's own `fs` (the uber-shader) draws every
  draw. On by default; the A/B and the look test of W293 compare the two.
- `MGE_XE_PIPELINE_LIST=<file>`: the list of the stage pipelines of a live session, in place of
  `%LOCALAPPDATA%\MGE XE\numidium-stage-pipelines.txt`. The next start makes the pipelines of
  the list on the helper's thread before they are needed; a list that is missing, of another
  world shader, or that cannot be written leaves each pipeline to its first use.
- `MGE_XE_GRASS_HASH=1`: the helper prints one line for each grass call with the hash of its records (MGE XE's
  RENDERER.md has the form). `tools/instance_ab.py` sets it for its replays.
- `MGE_XE_RUNS_DUMP=1`: the runs of the table of each frame.
- `MGE_XE_DRAW_DUMP=<geometry handle, hex>`: the words of each draw of that geometry.
- `MGE_XE_REPLAY_DEBUG=1`: a line for each draw a replay issues, and the values of the
  world's start.

## Where the plan is

The plan and the record of each step are in the docs repo:
`moreFPS/docs/Numidium Project/plans/plan.md`.
