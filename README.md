# nirender

Written for: a person or an agent who works on this repo.

nirender is an MWSE plugin for Morrowind. It replaces the renderer of the game. The game
makes no Direct3D device. The plugin writes each frame as records into a shared-memory ring.
A 64-bit helper, `mgeHost64.exe` of MGE XE G7, reads the ring and draws the frame with wgpu.

## The parts

| Part | Path | Builds to |
| --- | --- | --- |
| The plugin (32-bit) | `src/` | `target/i686-pc-windows-msvc/release/nirender.dll` |
| The host crate `nirender_wgpu` (64-bit) | `host/` | a library; the helper of MGE XE G7 depends on it by path |
| The Lua side of the plugin | `mod/MWSE/mods/nirender/` | installed with the DLL |
| The tools | `tools/` | Python scripts (use `C:/OSGeo4W/apps/Python312/python.exe -I`) |

The helper is in the sibling repo `MGE-XE-G7-render-link`. A change of `host/` changes the
helper build. Build the plugin before `cargo xtask build` of that repo.

## How the plugin works

- The ports: Rust functions that replace the renderer functions of the game, one for one.
  `PORTING.md` says how to write one. `src/ports/` holds them.
- The records: the plugin writes the objects, the shapes, the geometry, the materials, the
  lights and the draws of a frame into the ring (`src/capture.rs`). A capture file
  (`.nircap`) holds the same records, for a replay offline.
- The traversal: `src/ports/scene.rs` holds 17 ports of the cull and display walk of the
  engine (`CullShow`, `NiNode::Display` and the node kinds).
- The retained instances: `src/instance_stream.rs` keeps a table of the shapes the helper
  holds. The plugin skips their display and sends a record only when a shape changes (the
  records 23 to 28). The helper draws them from its own table (`host/src/retained.rs`).
- The modes: `NIRENDER_RETAINED=off|dual|on|gpu`, default `gpu`. `off` is the reference: the
  engine's own path. `nirender.retained(mode)` changes the mode in a live game.

## Build and test

```text
cargo build --release          the plugin
cargo test --release           the tests of the plugin and of the host crate
```

The gate of a change: `tools/validate_retained.py --mge-build <folder>`. Pass the folder of
the current helper build with its `d3d8.dll` and `Data Files`. The stages:

1. `capture`: a run at Seyda Neen and at Narsis with a capture, the counts, a replay.
2. `baseline`: the kept captures of `results/baseline-records` against the accepted pictures.
3. `only`: a run with the frame rate, the port times and the busy time of the helper.

`results/` is not in git. The accepted pictures live only on disk.

Other tools: `tools/replay.py` (a capture to a picture), `tools/picture_diff.py` and
`tools/diff_crops.py` (two pictures compared), `tools/capture_dump.py` (the records of a
capture), `tools/run_own_device.py` (a game run with the harness mods).

## Switches for an A/B

Never for a player. `NIRENDER_PORT_TIMING=1` times each live port call.
`NIRENDER_PLUGIN_DLL=<path>` installs that DLL for a run. `NIRENDER_LANDSCAPE=0`,
`NIRENDER_ACTORS=0` and `NIRENDER_ANIMATED=0` keep that class on the engine's path.
`NIRENDER_SKIP=0` makes the engine display the retained shapes too.

## Where the plan is

The plan and the record of each step are in the docs repo:
`moreFPS/docs/plans/wgpu-renderer-gaps-and-architecture-plan-2026-10-14.md`, section 0.
