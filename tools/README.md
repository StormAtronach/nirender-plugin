# The tools

Written for: a person or an agent who runs or changes a tool of this repo.

Run a tool from the repo root with `python tools/<name>.py` (Python 3.12 with Pillow; the
batch files at the repo root take the interpreter from `local.bat`). Each tool answers
`--help` or has its command in its docstring. A tool in `tools/` finds the shared modules
(`common.py`, `session.py`) by its own folder; a tool in a subfolder adds `tools/` to its path
itself. The places of this machine that a tool needs come from `local.json` at the repo root
(`common.local`; `local.example.json` names the keys), with a default next to this repo or
under the home folder.

## The shared modules

| Tool | What it is |
| --- | --- |
| `common.py` | What the tools share: the names of the renderer functions, the port macros, the report reader. |
| `session.py` | One run of the game with the nirender mod through the testing instance: the build, the mod, the run spec, the results folder. The harness itself is in the docs repo (`scripts/test-harness-2026-10-03`). |

## The game runs

| Tool | What it is |
| --- | --- |
| `run_own_device.py` | The game without a Direct3D device: the `d3d8.dll` of this repo takes the place of MGE XE's, and the host program draws the frames. The runner of every test of the wgpu renderer. |
| `run_capture.py` | A capture of the render link stream in the game. |
| `run_life.py` | The game with the plugin as the host of its own link, and the things that need more than draws: the character image, the screenshot, a save with its picture, changes of cell, a load. |
| `play.py` | A game to play by hand, with the wgpu renderer of a build of MGE XE; the files of the instance go back when the game ends. The testing instance, or another with `--instance`, `--game-dir`, `--profile` and `--config-mod` (`play-manual-check-outlander.bat` at the repo root plays in Outlander-test). `--stage-only` checks the staging without a game. |
| `live.py` | A live game of the testing instance, started and stopped for work through the mwse-mcp tools. |
| `own_device.py` | The testing instance put into the mode without a Direct3D device, for a game by hand, and taken out again. |
| `release.py` | A test build release on GitHub (the archive of `cargo xtask dist` of MGE XE G7), and the removal of the releases before it. |

## The gates

| Tool | What it is |
| --- | --- |
| `validate_retained.py` | The checks of the retained pipeline, one stage at a time: `capture`, `baseline`, `only`, `clusters`, `weathers`, `tour`. The gate of a change. |
| `validate_tour.py` | The stops of a tour of a test level replayed and compared with a baseline. |
| `check_readme.py` | README.md against the code: every switch the code reads, every path README names. |
| `check_contract.py` | The render link contract: `contract/mge_render_link.h` against `src/link.rs`. |
| `check_comments_only.py` | That an edit changed comments only. |
| `unused_shared.py` | The public items of the shared modules that no other code uses. |

## The pictures and the captures

| Tool | What it is |
| --- | --- |
| `replay.py` | A frame of a capture file drawn with the MGE XE helper, offline: the same picture from the same file each time. |
| `picture_diff.py` | Two pictures of the same size compared; how different they are. |
| `compare_pictures.py` | The pictures of two folders compared by name, with `picture_diff`. |
| `diff_crops.py` | Crops of the places where two pictures differ, side by side and enlarged. |
| `region_means.py` | The mean colour of named regions of two pictures, and the ratio. |
| `shader_delta.py` | The effect of a post shader on each renderer, with the tone difference of the renderers out of the comparison. |
| `shadow_factor.py` | The shadow factor of each of two renderers, from pictures with and without the shadows. |
| `capture_dump.py` | The records of a capture, for the root cause of a difference of a picture. |
| `capture_space.py` | Which space the records of a capture are in (an origin, relative flags). |
| `inventory.py` | The inventory of a capture, for the material model of the retained pipeline. |
| `light_values_check.py` | The light values of a capture two ways: the LIGHT records against the lights inline in the SHAPE records. |
| `retained_inputs_check.py` | The inputs of the retained table against the record path, from two captures of one view (mode gpu and mode off). |
| `translation_error.py` | The error of the engine's 32-bit translations in a capture against the exact 64-bit ones. |

## The NIF test level

| Tool | What it is |
| --- | --- |
| `nif40.py` | A reader and writer of NetImmerse 4.0.0.2 files for the blocks the test assets need. |
| `make_test_nifs.py` | The test NIFs of the flag support: one file for each flag value the engine supports and the content does not use. |
| `nif_block_census.py` | Which NIF block types occur in the game's BSAs, a mod folder or a test level. |
| `nif_flag_census.py` | How many NIFs carry a flat-shading, specular or dither property, and with which flag. |

## `ports/`: writing a port

The tools of `PORTING.md`. Three of them run inside IDA Pro (`ida_*.py`).

| Tool | What it is |
| --- | --- |
| `ports/ida_export_inventory.py` | In IDA: the inventory of the renderer functions and the struct layouts, to `tools/out/*.json`. |
| `ports/ida_dump_funcs.py` | In IDA: the pseudocode and the disassembly of each function, to `tools/out/dump/<address>.txt`. |
| `ports/ida_abi_audit.py` | In IDA: the registers a caller reads after each call, to `tools/out/abi.json`. |
| `ports/gen.py` | `src/gen/layouts.rs` and `src/gen/funcs.rs` from the IDA export. |
| `ports/add_touch.py` | The execution counter at the top of each port function. |
| `ports/merge_port.py` | A port branch merged into the current branch, its module registered. |

## `rig/`: the comparison rig

The runs that compare the Rust code with the game code on a real Direct3D 8 device (the
mode shadow and the frame comparison). They build the plugin with the cargo feature `rig`.
Not used since 2026-10-11; they stay for a comparison run of a port.

| Tool | What it is |
| --- | --- |
| `rig/run_test.py` | The plugin built, installed into the testing instance, and the in-game test of the ports in shadow mode with the frame comparisons. |
| `rig/compare_runs.py` | The dumps of two game runs compared (`run_test.py --dump` makes them). |
| `rig/run_perf.py` | Frame times with the game code against frame times with the Rust code. |
| `rig/run_video.py` | Two OBS clips of the same view: the game code and the Rust code. |
| `rig/run_ab.py` | The picture of the host program against the picture of the game, view by view. |
| `rig/run_stream.py` | The game and the host program together, the stream in shared memory, the host in its own window. |
