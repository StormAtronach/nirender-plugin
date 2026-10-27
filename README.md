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
  records 23 to 28). The helper draws them from its own table (`host/src/retained.rs`).
- The modes: `NIRENDER_RETAINED=off|dual|gpu`, default `gpu`. `off` is the reference: the
  engine's own path. `nirender.retained(mode)` changes the mode in a live game.
- The threads: a port runs its Rust code on the main thread only. The cell worker's calls
  run the engine function and reach the plugin as events (`src/worker_events.rs`). A table
  of the main thread opens only for a proof that the caller is on it
  (`src/main_thread.rs`): the dispatch of a port gives the proof to a port declared with
  `rust_main`, and the Lua gate and the frame hook claim it.
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

`results/` is not in git. The accepted pictures live only on disk.

The checks of the repo itself: `tools/check_readme.py` (every switch the code reads is in
this file; every path this file names exists) and `tools/check_contract.py` (the header and
`src/link.rs` agree).

Other tools: `tools/replay.py` (a capture to a picture), `tools/picture_diff.py` and
`tools/diff_crops.py` (two pictures compared), `tools/capture_dump.py` (the records of a
capture), `tools/run_own_device.py` (a game run with the harness mods).

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
  Direct3D 12 card has. `downlevel`: wgpu's
  downlevel limits, which the probe refuses with the lacks named. The log and the probe's line
  say `MGE_XE_OLD_GPU=<profile>`.
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
- `NIRENDER_NOTIFIER=0`: the mod does not load the stand-in Lua change notifier.

The places of this machine (the harness, the mwse-mcp package, the OBS recorder, the video
folder, the Code Patch files of the unit test) are in `local.json` at the repo root, and the
places of the batch files (the Python interpreter, the MGE XE build, the Outlander-test
instance) in `local.bat`. Neither is tracked; `local.example.json` and `local.example.bat`
name the keys. A key that is not there takes the tool's default: a place next to this repo or
under the home folder.

The A/B:

- `NIRENDER_PORT_TIMING=1` times each live port call.
- `NIRENDER_COMPACT_MOVES=0` sends a move of a retained instance as the whole `INSTANCE`
  record, as before `INSTANCE_MOVE`; `check` sends both, and the helper counts the moves whose
  values differ from the full record's.
- `NIRENDER_UV_STEPS=0` sends a geometry under a UV controller whole at each new revision,
  as before `GEOMETRY_UV`; `check` sends both, and the plugin compares the step from the sent
  coordinates with the engine's (the geometry line of the log).
- `NIRENDER_MORPH_WEIGHTS=0` sends a geometry under a morph controller whole at each new
  revision, as before `MORPH_TARGETS` and `MORPH_WEIGHTS`; `check` sends both, and the plugin
  compares the weighted targets with the engine's positions (the geometry line of the log).
- `NIRENDER_GEOMETRY_CENSUS=1` counts the geometries sent again, by the controllers of their
  shape, and logs the census at each stream mark (`geometry_stream::census_take`).
- `NIRENDER_SKIP=0` makes the engine display the retained shapes too.
- `NIRENDER_LANDSCAPE=0`, `NIRENDER_ACTORS=0` and `NIRENDER_ANIMATED=0` keep that class on
  the engine's path.
- `NIRENDER_NODE_KINDS=0` keeps a shape under a billboard, switch, level of detail, BSP,
  animation or particle node on the engine's path (the rule before 2026-10-16).
- `NIRENDER_PARTICLES=off` keeps the particle systems on the engine's path (the rule before
  2026-10-24). `nirender.particleDraws(false)` keeps them in the table but sends their draws
  as the engine's records, live.
- `MGE_XE_PARTICLE_QUADS=cpu` makes the quads of the particle systems on the CPU, not on the
  GPU.
- `MGE_XE_DEPTH_FORMAT=24` keeps the depth of the frame in `Depth24PlusStencil8` when the
  adapter has `Depth32FloatStencil8` (the format before 2026-10-27; the helper's log line
  `depth:` names the format in use).
- `MGE_XE_DEPTH_REVERSED=0` keeps the float depth in the plain convention (near 0, far 1), not
  reversed (near 1, far 0): for an A/B of the reversed depth (`host/src/depth.rs`).
- `MGE_XE_ONE_DEPTH=1` draws the distant land and the game into one depth buffer, with an
  infinite far plane for every perspective projection of the world view (step 6 of the depth
  design; only with the reversed float depth). Temporary: it goes when step 6 is accepted.
- `NIRENDER_CONTROLLERS=0` keeps a shape with a controller on itself or on a property (a
  morpher, a UV, flip, alpha or colour controller) on the engine's path (the rule before).
- `NIRENDER_SKIN_TABLE=0` keeps the skinned shapes on the engine's display, with the palette
  of the bones in each draw (`nirender.skinTable`).
- `NIRENDER_ANIMATION=view|shadows|all`: which actors animate (`nirender.animationPolicy`);
  default the view and the shadow reach.
- `NIRENDER_NOTE_EVERY=1`: the material and the geometry of every retained shape are noted at
  every display, not every 64th frame (`nirender.noteEvery`).
- `NIRENDER_SKIP_PORT=<name>`: one port left uninstalled, to measure its cost. Only a port
  whose function is not the renderer's.
- `NIRENDER_SKY=off` keeps the sky on the engine's records; `NIRENDER_SKY=count` also logs
  each sky shape once. The default: the sky's instances in the table.
- `NIRENDER_WEATHER=on` puts the rain, snow and storm drops (class 9) into the table, drawn at
  the engine's places as the sky is (`nirender.weatherDraws(false)` turns their draws back to
  the engine's records in a session); `NIRENDER_WEATHER=count` logs each drop shape once. The
  default (`off`): the engine's records, until the gate of the plan note's section 3.1.
- `NIRENDER_MAP_FOG=0`: the engine reads the fog of war of the local map from the GPU; by
  default the plugin keeps a CPU copy (`src/map_fog.rs`).
- `NIRENDER_MAP_TILES=0`: the engine reads each map tile from the GPU at once; by default the
  helper copies the tile on the GPU and sends the pixels some frames later
  (`src/map_tiles.rs`).
- `NIRENDER_READ_CACHE=0` turns off the cache of the reads of rendered textures
  (`src/own_host.rs`).
- `NIRENDER_PACE=0`: no pacing of the game to the helper (`src/pacing.rs`). The pacing waits
  at MWSE's start of a frame (native event 8, before the input of the frame) until the
  helper will need the frame, so that the picture is as new as the helper allows. Without it
  a frame waits about one frame of the helper in the ring.
- `NIRENDER_PACE_LEAD_US=<n>`: the lead of the pacing in microseconds (default 200): the
  frame is complete this long before the helper is free for it. For a test.

The instruments and the tests:

- `NIRENDER_ENTRY_TRACE=1`: an entry into the plugin made while another is live writes the
  outer entry, the call into the engine between them and the inner entry to the log, once
  for each triple (`src/entry_trace.rs`; the re-entrancy map in the docs reads these lines).
- `NIRENDER_FILL_FREED=1`: every block the engine frees is filled with a pattern first; a
  read of freed memory faults at the read (`src/freed_fill.rs`).
- `NIRENDER_FORCE_GROWTH=<frames>`: the helper makes its record buffer again every so many
  world frames, as a growth does; with `NIRENDER_GROWTH_SHOTS=<prefix>` it takes the picture
  of each growth frame and of the frame after it (`host/src/player/items.rs`).
- `NIRENDER_FOG_CHECK=1`: each lock of a fog texture also reads the GPU and compares it with
  the CPU copy; `NIRENDER_FOG_DUMP=<folder>` writes the first 8 pairs that differ.
- `NIRENDER_MAP_TILES_HOLD=<N>`: a map tile completes only N Presents after it opened, so that
  a save or a purge soon after a cell cross finds open tiles.
- `NIRENDER_MCPATCH=<folders>` and `NIRENDER_VANILLA_EXE=<path>`: the inputs of the unit
  test that checks every Morrowind Code Patch record inside a ported function
  (`src/install.rs`); without them the test uses the paths of this machine.

The helper's diagnostic and test switches (`host/src/switches.rs`, read once; none is a
setting). The three of the A/B list above (`MGE_XE_PARTICLE_QUADS`, `MGE_XE_DEPTH_FORMAT`,
`MGE_XE_DEPTH_REVERSED`) are there too. MGE XE's own switches are in the RENDERER.md of the
MGE-XE-G7 render-link worktree.

- `MGE_XE_RETAINED=0`: the table draws nothing. With a capture of the plugin's mode `dual`
  (and the engine's walk), the engine's own draws of the same capture: the reference of a
  debug toggle.
- `MGE_XE_CLUSTERS=1` or `0`: the clusters of the lights on or off, over the setting. The
  baseline gate replays with both.
- `MGE_XE_LIGHTS=world`: a lit draw of the world takes the point lights of the world that
  reach its pixel (the clusters of `WORLD_LIGHTS`), not the lights the engine chose for it.
- `MGE_XE_INDIRECT=0`: one draw call for each retained draw, no indirect runs: it rules the
  indirect runs out when a picture is wrong, and goes around a driver's fault in them.
- `MGE_XE_INSTANCE_UPLOAD=all`: the retained segment of the instances goes up whole when more
  than 64 slots changed in a frame, as before the copy of `retained_upload`.
- `MGE_XE_LAZY_PASS=0`: a pass on the game's targets begins when it is asked for, and a clear
  does not carry into the next pass, as before `Player::begin_pass`.
- `MGE_XE_PASS_GROUPS=0`: the game's draws before the world and after the post chain bind the
  whole group of the texture arrays, as before `Renderer::small_array_group`.
- `MGE_XE_SAMPLES=<n>`: the samples for each pixel of the targets of the frame (1, 2, 4 or 8),
  over the setting of the extension.
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
  for a test of the plugin's own limit of 10 s on a helper that reads no record.
- `MGE_XE_CORES=0`: the threads of the helper are left to the scheduler.
- `MGE_XE_ARRAY_CAP=<n>`: the room of the texture arrays at most n, for the test of a full
  room; below 256 the request refuses the adapter, for the test of `numidium.exe --probe`
  and of the live start with a GPU the renderer cannot use (the exit code 20).
- `MGE_XE_RECORD_LIMIT=<n>`: at most n draw records (at least 1024), for the test of the
  guard of the device's limit.
- `MGE_XE_GPU_TIME=1`: time stamps of the parts of the frame (they cost time).
- `MGE_XE_RUNS_DUMP=1`: the runs of the table of each frame.
- `MGE_XE_DRAW_DUMP=<geometry handle, hex>`: the words of each draw of that geometry.
- `MGE_XE_REPLAY_DEBUG=1`: a line for each draw a replay issues, and the values of the
  world's start.

## Where the plan is

The plan and the record of each step are in the docs repo:
`moreFPS/docs/Numidium Project/plans/plan.md`.
