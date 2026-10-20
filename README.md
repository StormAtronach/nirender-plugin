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
- `NIRENDER_RING_MB=<n>`: the size of the ring, default 64.
- `NIRENDER_PLUGIN_DLL=<path>`: the tools install that DLL for a run (one arm of an A/B).
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
- `NIRENDER_SKIP=0` makes the engine display the retained shapes too.
- `NIRENDER_LANDSCAPE=0`, `NIRENDER_ACTORS=0` and `NIRENDER_ANIMATED=0` keep that class on
  the engine's path.
- `NIRENDER_NODE_KINDS=0` keeps a shape under a billboard, switch, level of detail, BSP,
  animation or particle node on the engine's path (the rule before 2026-10-16).
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

The instruments and the tests:

- `NIRENDER_FILL_FREED=1`: every block the engine frees is filled with a pattern first; a
  read of freed memory faults at the read (`src/freed_fill.rs`).
- `NIRENDER_MCPATCH=<folders>` and `NIRENDER_VANILLA_EXE=<path>`: the inputs of the unit
  test that checks every Morrowind Code Patch record inside a ported function
  (`src/install.rs`); without them the test uses the paths of this machine.

## Where the plan is

The plan and the record of each step are in the docs repo:
`moreFPS/docs/Numidium Project/plans/plan.md`.
