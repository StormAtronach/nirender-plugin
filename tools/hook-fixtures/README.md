# The hook fixtures of the replay gate

These files are fixtures of the replay gate (W528 item 5, F-G-12), not mods. The baseline stage
of `tools/validate_retained.py` replays its captures two times: first with no core mod
(`MGE_XE_CORE_MODS=0`), then with these files, read by the helper from this folder only
(`MGE_XE_CORE_MODS_DIR`). The second pass compares its pictures with `accepted-hooks/` of the
baseline folder. Each of its replays must show the route: the helper names this folder, reads
each `XE Mod *.wgsl` file of it and no other file, uses each file with all of its hooks, and
prints no other line of the core mods. This README is not a fixture.

There is one file for each kind of hook of interface `mge-xe-core 1`. Each hook calls its
library, and each file changes the picture where its home is drawn:

| File | Hooks | The change |
| --- | --- | --- |
| `XE Mod Shadow.wgsl` | `shadow_esm` | 1.5 times the exponent (a sharper edge) |
| `XE Mod Shadow Data.wgsl` | none (values) | `shade = 0.6` (the built-in is 0.4) |
| `XE Mod Water.wgsl` | `water_normal`, `water_color`, `underwater_color` | half the slope of the waves; a colour of the water of its own, tinted red |
| `XE Mod Sky.wgsl` | `clouds_color` | the built-in colour of the clouds, tinted red |
| `XE Mod Particles.wgsl` | `particle_color` | the colour of the particles, tinted red for a draw that adds and green for any other |

A file is its own text: a later change of a built-in hook does not change it. A change of a
fixture plans the gate (`merge_step.py`, `GATE_PATHS`), and its new pictures are accepted again
by the Coordinator's word.
