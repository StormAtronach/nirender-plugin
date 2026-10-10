# wgpu in a 32-bit process

Question: does wgpu build and run in a 32-bit Windows process, on its Vulkan backend and on
its D3D12 backend, and what does it cost? Morrowind.exe is 32-bit, so a renderer inside it
must be 32-bit too.

    cargo build --release --target i686-pc-windows-msvc
    target\i686-pc-windows-msvc\release\wgpu32.exe [draws in a frame] [frames]

The program is linked with `/LARGEADDRESSAWARE`, as Morrowind.exe with the 4 GB patch is.
For each backend it opens a window and draws. Each draw sets a pipeline (two pipelines
alternate), sets a bind group with a new uniform buffer offset, and draws one triangle.
The time of a frame is the time to fill the uniform data, encode, submit and present,
without vertical sync.

## Result of 2026-10-10

wgpu 22.1.0, NVIDIA GeForce RTX 5070, driver 617.14, Windows 11.

| | Vulkan, 32-bit | D3D12, 32-bit | Vulkan, 64-bit | D3D12, 64-bit |
| --- | --- | --- | --- | --- |
| Builds and runs | yes | yes | yes | yes |
| 5000 draws, mean frame | 2.55 ms | 2.48 ms | 2.08 ms | 1.94 ms |
| 5000 draws, 99th percentile | 3.06 ms | 3.29 ms | 2.54 ms | 2.50 ms |
| Time for one draw | 0.51 us | 0.50 us | 0.42 us | 0.39 us |
| 1000 draws, mean frame | 0.56 ms | 0.78 ms | | |
| Private memory after 400 frames | 176 MB | about 85 MB more, in the same process | 239 MB | |
| Committed address space | 363 MB | 481 MB with the two devices | | |

The largest free block of address space stayed at 2042 MB of the 4 GB.

What this shows:

- wgpu works at 32 bits on the two backends. The 32-bit build is about 25 % slower than
  the 64-bit build for the same work.
- A scene of the game has some hundred to some thousand draws. At 0.5 microseconds for a
  draw, 2000 draws cost about 1 ms of processor time in a frame.
- One device takes about 175 MB of private memory and about 330 MB of address space before
  any texture of the game is loaded. The game process with MGE XE has a 4 GB limit, so this
  amount is important.

What this does not show:

- A current wgpu version. The test used 22.1.0, because the program was written for that
  interface.
- wgpu inside the game process, with the memory of the game and of MGE XE.
- The cost of real pipelines, textures and render targets.
