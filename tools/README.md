# The tools

Written for: a person or an agent who runs or changes a tool of this repo.

Run a tool from the repo root with `python tools/<name>.py` (Python 3.12 with Pillow; the
batch files at the repo root take the interpreter from `local.bat`). Each tool answers
`--help` or has its command in its docstring. A tool in `tools/` finds the shared modules
(`common.py`, `session.py`) by its own folder; a tool in a subfolder adds `tools/` to its path
itself. The places of this machine that a tool needs come from `local.json` at the repo root
(`common.local`; `local.example.json` names the keys), with a default next to this repo or
under the home folder. A worktree without `local.json` reads the main checkout's file, and the
tool names that file and its keys on stderr. Every key of that file then applies to the
worktree: give a worktree its own `local.json` for a key that must differ in it. The harness's
default is next to the main checkout when it is not next to the worktree (`common.sibling`).

## The shared modules

| Tool | What it is |
| --- | --- |
| `common.py` | What the tools share: the names of the renderer functions, the port macros, the report reader. |
| `session.py` | One run of the game with the nirender mod through the testing instance: the build, the mod, the run spec, the results folder. The harness itself is in the docs repo (`scripts/test-harness-2026-10-03`). |

## The game runs

| Tool | What it is |
| --- | --- |
| `run_own_device.py` | The game without a Direct3D device: the `d3d8.dll` of this repo takes the place of MGE XE's, and the host program draws the frames. The runner of every test of the wgpu renderer. `--plugin-dll` names the plugin of the run, `--env NAME=VALUE` a variable for the game's process (again for more), and `--log` the file of its output, so a run's command needs no variable prefix or redirect; `--engine-reference` runs the same scenario on the system's Direct3D 8 with `runtime.disabled` (pictures `eng-<step>.png`, 2048 x 1024, the world without the interface; not for timing); `--archive` is the release check: an extracted release archive gives the run's build, plugin, MWSE.dll and Lua, and a Lua file that the archive loads and lacks is not in the run, as in a player's game; `--self-test` tests the argument handling (`checks.py` does). |
| `notifier_check.lua` | `notifier.lua` of the plugin's mod outside the game, on a stand-in of a sol type: its reports, and no new function for a second read of a wrapped method. Run by hand: `luajit tools/notifier_check.lua mod/MWSE/mods/nirender/notifier.lua`, with the LuaJIT of the MWSE repository (`deps/luajit2/src`); the hook has no path to it. |
| `run_capture.py` | A capture of the render link stream in the game. |
| `run_life.py` | The game with the plugin as the host of its own link, and the things that need more than draws: the character image, the screenshot, a save with its picture, changes of cell, a load. |
| `play.py` | A game to play by hand, with the wgpu renderer of a build of MGE XE; the files of the instance go back when the game ends. The testing instance, or another with `--instance`, `--game-dir`, `--profile` and `--config-mod` (`play-manual-check-outlander.bat` at the repo root plays in Outlander-test). `--stage-only` checks the staging without a game. |
| `make_play_set.py` | The owner's play set: builds master and render-link's tip, copies them to the next `mgebuilds/f<N>` with no pdb, runs a smoke, and only on a pass names the set in `local.bat`. A failed smoke leaves the last good set in place. |
| `merge_step.py` | The checks of a merge on the merged tree, in one hold of the build slot, with one PASS or FAIL line for each stage: tests, the clippy baseline, `checks.py` and `docs.py`; a d3d8 build and `check_contract.py` for d3d8, `mge-config` or the contract; the replay gate for the host's source, the helper's renderer, the shaders of the assets or a manifest or lock of the build. The paths of the diff from `--base`, the tip before the merge, decide. It pushes nothing. |
| `live.py` | A live game of the testing instance, started and stopped for work through the mwse-mcp tools. |
| `own_device.py` | The testing instance put into the mode without a Direct3D device, for a game by hand, and taken out again. |
| `release.py` | A test build release on GitHub (the archive of `cargo xtask dist` of MGE XE G7), and the removal of the releases before it. Before the build it stops on uncommitted, unpushed or untracked source files and runs the checks and the unit tests of the three trees; after the build it runs the replay gate (`validate_retained.py baseline`) on the helper unpacked from the archive and writes the result into the notes (`{gate}`); the notes follow `notes-template.md`, whose Tested block must name the gate and the run folder of the live checks. The archive gets `MANIFEST.txt` (the commits, the compiler, the SHA-256 of each file), and its own SHA-256 goes beside it as `<archive>.sha256`, up with the release and into the notes (`{sha256}`). It puts the license texts, `THIRD-PARTY-NOTICES.txt` and `SOURCES.txt` into the archive (`--source` names the parts added from outside the two repositories). `--self-test` runs the tests of its pure parts (`checks.py` does). |
| `third_party_notices.py` | The third-party notices of the crates inside the programs of the archive, from `cargo metadata` of the MGE XE workspace and the license files beside each crate's manifest; `--self-test` runs its fixture (`checks.py` does). To switch to `cargo-about` once it is installed: run it in the MGE XE worktree with an `about.toml` and a template, and point `release.py` at its output in place of this script. |
| `clean_watchers.py` | Lists, and with `--end` ends, the shell tools (tail, grep, sed) that log watchers of Git Bash leave behind (harness trap T58). |
| `clean_results.py` | The expiry of the data rule (D71) for each run, dump and set, wherever it lives: `results/` of the checkouts, `E:\nirender-results`, `E:\nirender-keep\runs-*`, `C:\tmp\mwdumps`, `E:\nirender-keep\full-dumps-*`, `E:\nirender-keep\mgebuilds`. A cited run stays whole; an uncited one loses its pictures, captures and film at 7 days and goes at 30. `--apply` refuses when the text of the citations cannot be read whole. An uncited dump is compressed after a day and goes at 30 days. A set stays when it is cited, is the current play set or the one before, or is in use; others go at 30 days. A folder that is no run and a link are not touched. Lists by default; `--list` names each one; `--apply` acts. It stops while a game runs; `--during-game` gives the list anyway. Run it from the main checkout. |
| `clean_merged_worktrees.py` | A step of each merge: removes the side worktrees of nirender and MGE XE whose branch is merged, clean and idle, with no process in them and no other tree's `Cargo.toml` naming them, and the checkouts of an old play set. It copies what the expiry of `clean_results.py` leaves of a real `results` folder to `E:\nirender-keep` and checks the copy, removes each reparse point as a link, then runs `git worktree remove` without `--force`. Branches stay. Each tree of nirender, MGE XE and MWSE, main and standing checkouts too, also loses a build output with no write for 48 hours: `target`, `target-*` and MWSE's object folders; MWSE's `build` stays. Lists by default; `--apply` acts; `--name <path>` adds a worktree that is not merged or has no branch, when a ref holds its commit. It stops while a game runs; `--during-game` gives the list anyway. It does not see a session that has a worktree in mind with no process there, or a junction elsewhere that points into a worktree. Run it from the main checkout. |
| `clean_common.py` | The shared parts of the two cleaning tools: the walk and the removal that never go through a link, the processes, the log, the citations and the expiry. No entry point. |

## The gates

| Tool | What it is |
| --- | --- |
| `validate_retained.py` | The checks of the retained pipeline, one stage at a time: `capture`, `baseline`, `only`, `clusters`, `weathers`, `tour`. The gate of a change. The baseline stage records the adapter of the accepted pictures (`accepted/adapter.txt`) and names it with the result; `--self-test` tests its pure parts (`checks.py` does). |
| `validate_tour.py` | The stops of a tour of a test level replayed and compared with a baseline. |
| `fuzz_capture.py` | A capture file corrupted at seeded random places (bytes flipped, a record's head changed, the file cut inside or between records; each mutation printed) and each mutant replayed with a `nirender-host.exe`: it passes when no replay aborts and every cut inside a record is refused or skipped with a line; a changed head may pass as valid data (a tag changed into another known tag is a valid record). `--self-test` tests its pure parts (`checks.py` does). |
| `checks.py` | The content checks that need no compile, as one list for each repository (`plugin`, `mge`, `all`): the readme, the contract, MWSE's native ABI, the module headers and the skills' names, and the self-tests of the tools. The commit hooks and `release.py` run it; with `--strict` (release.py) a check that cannot run on this machine is a failure. |
| `install_hooks.py` | Writes the pre-commit hooks that run `checks.py` against the tree the commit is in (`git rev-parse --show-toplevel`): this repository with that tree's own tools, the MGE XE repository in every worktree that has the renderer (`checks.py mge --mge <tree>`; a tree without `mgeHost64/src/renderer.rs` is skipped with a line), and the docs repository (through its own `--install-hook`). `git commit --no-verify` skips a hook once. `--self-test` checks the hook texts (`checks.py` does). |
| `check_readme.py` | README.md against the code: every switch the code reads, every path README names, a row here for every tool. |
| `check_mwse_native.py` | The plugin's and MGE XE's copies of MWSE's native ABI against MWSE's header `MWSENativeApi.h`. |
| `docs.py` | The rustdoc pages of the plugin, the host crate and the MGE XE helper, with the private items; fails on a broken link in a doc comment of our code; `target/docs.html` links the three (`--open`). `release.py` runs it. It runs cargo. |
| `doc_links.py` | The names in backticks of the doc comments of one crate as links that rustdoc checks: it keeps the links that resolve and writes back the rest. Without `--apply` every file is restored. It runs cargo. |
| `check_contract.py` | The render link contract: `contract/mge_render_link.h` against `src/link.rs` (the numbers and the struct sizes), and each function of d3d8.dll that the header declares is looked up by name in `src`; `--mge DIR` checks that d3d8.dll exports each of them (`d3d8/cpp/exports.def`); `--self-test` tests the parsing. |
| `check_handshake.py` | The version handshake: the host crate reads the plugin's `src/stream_format.rs` (one STREAM_FORMAT), and no constant of that file (the HEADER layout, the words of the ring head and of the reply, tile and message memories, the suffixes of the names of the events and memories, the exit codes) has a second definition in the two crates, and no name suffix comes back as a literal; with `--mge`, MGE XE's MGE_EXTENSION_FORMAT equals the helper's EXTENSION_FORMAT, and mgeHost64 has no copy of a constant of the file. `--self-test` tests the search. `checks.py` runs all three. |
| `check_comments_only.py` | That an edit changed comments only. |
| `hand_offsets.py` | The engine field offsets written by hand outside `src/gen/layouts.rs`; `--check` fails on a file above its count in `hand_offsets.baseline`, `--update` lowers the counts. `checks.py` runs it. |
| `clippy_baseline.py` | The Clippy warnings of each lint in the plugin, the host crate and the render-link workspace; `--check` fails on a lint above its count in `clippy.baseline` (the head of the file names the commits of its counts), `--update` lowers the counts. `--mge-host DIR` builds the render-link place against the host crate in DIR (cargo's `--config paths` override), for a render-link branch ahead of the main checkout's host crate; `--self-test` tests the arguments. `release.py` runs it without `--mge-host`. It runs cargo. |
| `check_sharedse_layouts.py` | The IDA types of the layouts against MWSE's own (SharedSE `NI*.h`, MWSE `TES3*.h`): each `static_assert` size and each field offset MWSE writes down. `--all` for every struct MWSE asserts. `checks.py` runs it. |
| `check_offset_moves.py` | That each offset moved into the generated layouts keeps its old value (the staged diff with `--cached`). `checks.py` runs it. |
| `unused_shared.py` | The public items of the shared modules that no other code uses. |

## The pictures and the captures

| Tool | What it is |
| --- | --- |
| `replay.py` | A frame of a capture file drawn with the MGE XE helper, offline: the same picture from the same file each time. `--marks-every N`: the helper's time for each frame by part after every N frames, with a wait for the GPU after each frame (the replay bench of the docs, `scripts/wgpu-bench/bench_replay.py`). |
| `picture_diff.py` | Two pictures of the same size compared; how different they are. |
| `compare_pictures.py` | The pictures of two folders compared by name, with `picture_diff`. |
| `diff_crops.py` | Crops of the places where two pictures differ, side by side and enlarged. |
| `region_means.py` | The mean colour of named regions of two pictures, and the ratio. |
| `shader_delta.py` | The effect of a post shader on each renderer, with the tone difference of the renderers out of the comparison. |
| `shadow_factor.py` | The shadow factor of each of two renderers, from pictures with and without the shadows. |
| `capture_dump.py` | The records of a capture, for the root cause of a difference of a picture; the interface's shapes, for a picture that a menu lacks. |
| `lights_count.py` | What clustered distant lights and a 3D light volume would hold, from a `lights.data`. |
| `capture_space.py` | Which space the records of a capture are in (an origin, relative flags). |
| `inventory.py` | The inventory of a capture, for the material model of the retained pipeline. |
| `light_values_check.py` | The light values of a capture two ways: the LIGHT records against the lights inline in the SHAPE records. |
| `retained_inputs_check.py` | The inputs of the retained table against the record path, from two captures of one view (mode gpu and mode off). |
| `translation_error.py` | The error of the engine's 32-bit translations in a capture against the exact 64-bit ones. |

## The NIF test level

| Tool | What it is |
| --- | --- |
| `nif40.py` | A reader and writer of NetImmerse 4.0.0.2 files for the blocks the test assets need. |
| `make_rain_nif.py` | Placeable rain for a view out of an interior, frame rate independent: a NIF for each size of box and the streak texture. |
| `make_test_nifs.py` | The test NIFs of the flag support: one file for each flag value the engine supports and the content does not use; two boxes with a BC7 texture, plain and sRGB; and a texture effect under a turning billboard, for each pair of modes. |
| `effect_turn_vectors.lua` | In a running game: the engine's own billboard and projection functions on made objects, written as the test vectors of `host/src/effect_turn.rs`. |
| `effect_turn_walk.lua` | In a running game with `NIRENDER_EFFECT_TURN_CHECK=1`: places the `turn_` exhibits and flies the camera around them, for the helper's comparison with the engine's words. |
| `bc7.py` | A small encoder and decoder of BC7 mode 6, for the BC7 textures of `make_test_nifs.py` (no other BC7 tool is installed); `--self-test` tests the round trip. |
| `nif_block_census.py` | Which NIF block types occur in the game's BSAs, a mod folder or a test level. |
| `nif_flag_census.py` | How many NIFs carry a flat-shading, specular or dither property, and with which flag. |

## `ports/`: writing a port

The tools of `PORTING.md`. Three of them run inside IDA Pro (`ida_*.py`).

| Tool | What it is |
| --- | --- |
| `ports/ida_export_inventory.py` | In IDA: the inventory of the renderer functions and the struct layouts, to `tools/out/*.json`. |
| `ports/ida_dump_funcs.py` | In IDA: the pseudocode and the disassembly of each function, to `tools/out/dump/<address>.txt`. |
| `ports/ida_abi_audit.py` | In IDA: the registers a caller reads after each call, to `tools/out/abi.json`. |
| `ports/gen.py` | `src/gen/layouts.rs` and `src/gen/funcs.rs` from the IDA export; `--layouts-only` leaves the function table and its bytes alone. A struct gets into the layouts through `LAYOUT_RE`. |
| `ports/layout_lookup.py` | The members of an IDA struct at given offsets, with the names `gen.py` gives them: the name for a hand-written offset that moves into the layouts. |
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
