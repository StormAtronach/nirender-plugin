# How to port an engine function

Written for: a person or an agent that adds ports to this repo.

A port must make the same device calls, the same memory writes and the same return value as
the function in Morrowind.exe, for every input. The shadow test in the game checks this on
every call. A port that is only close fails the test.

The game is Morrowind.exe as it runs with MWSE and MGE XE. MWSE needs MGE XE, so there is no
other game to replicate.

## Sources

- `tools/out/dump/<address>.txt`: the Hex-Rays pseudocode and the disassembly of each
  function of the renderer subsystem. `tools/ports/ida_dump_funcs.py` makes these files. They are
  not in git.
- `src/gen/layouts.rs`: the offsets of the struct members, from the IDA database.
- `src/gen/funcs.rs`: every function with its size, its stack argument bytes and its bytes.
- `tools/out/abi.json`: the registers that a caller reads after the call.
- `moreFPS/docs/engineering-notes/renderer/*.md`: the description of each class.

The disassembly is the authority. The pseudocode shows the structure. It does not show the
operand widths, the order of the writes or the content of a register.

Clean-room rule: the code comes from the binary. Do not copy from the Gamebryo source.

## Shared modules

Use these before you write a helper. Do not make a private copy of something they have.

| Module | Content |
| --- | --- |
| `src/mem.rs` | `rd`, `wr`, `x87_pass`, `fnv1a` |
| `src/engine.rs` | Calls of engine functions at fixed addresses: `thiscall0..5`, `cdecl0..3`, `virtual_at`, `vcall0..3`, `operator_new`, `operator_delete`, and named wrappers. Addresses of game data. |
| `src/d3d.rs` | Wrappers of the device, `IDirect3D8`, buffer, texture, surface and COM methods. The D3D constants. |
| `src/x87.rs` | Float sequences that must run as x87 instructions. `ftol_asm!`. |
| `src/site.rs` | `call_target` and the macros that call the target of a call site. |
| `src/ports/ni.rs` | NetImmerse code that the game has inline: release, pointer assign, delete, RTTI. |
| `src/ports/hash_map.rs` | The `THashMap` of the game. |

## Rules

1. Write one Rust function for each engine function, in `src/ports/<class>.rs`. Its doc
   comment has the address and the engine name. An argument is `usize` for a pointer, `u32`
   for an integer or an enum, `u8` for a `char` or a `bool`, `f32` for a float.
2. Work on the engine objects in place. Use `rd::<T>(base, offset)` and
   `wr(base, offset, value)` with offsets from `gen::layouts`. Do not keep state in Rust
   statics.
3. Read each operand at the width that the instruction reads. `mov al, [x]` is `rd::<u8>`.
   `mov ax, [x]` is `rd::<u16>`. Zero-extend only when the code has `xor eax, eax` or `movzx`.
4. Keep the order of the device calls and of the memory writes. Keep calls that look
   redundant.
5. Call a device method through the vtable of the device on each call. Use the wrappers in
   `src/d3d.rs`. Do not keep the function pointer: the recorder replaces the vtable.
6. Call a function that has a port as a Rust function. Call every other function through
   `src/engine.rs`. Add a named wrapper there, with the engine name and the address, when a
   second file needs the same function.
7. Floats. The x87 control word in the game is 0x007F, which is 24-bit precision. So `f32`
   `+ - * /` and `sqrt` give the same bits as the x87 code. Do one operation for each x87
   operation, in the same order. Do not use `mul_add`. A float that only goes through
   `fld` and `fstp` uses `mem::x87_pass`. For `fsin`, `fcos`, `fptan`, `fpatan`, `fistp`, a
   call of `_ftol` and an operation with a double operand, write a helper in `x87.rs` that
   runs the same instructions. Do not use `f32::sin` and the functions like it.
8. Return what the function leaves in EAX when a caller can read it. Then the return type of
   the port is `u32`, `usize`, `u8` or `bool`, and `compare_eax = true`. When EAX holds no
   result, the type is `()` and `compare_eax = false`. `tools/out/abi.json` lists the call
   sites that read EAX after the call.
9. `regions` gives every block of memory that the function can write for the call: the
   object, and the memory that the function gets to through pointers. The shadow test saves
   and restores these blocks only. A write to other memory is not restored and breaks the
   state of the game in shadow mode. The list has no limit.
10. Put `crate::shadow::touch(&<port module>::STATE);` on the first line of the port
    function. `tools/ports/add_touch.py` adds the line to every port that does not have it.
11. Declare the port with `port_thiscall!` or `port_cdecl!` and add it to the `PORTS` list
    of the module. A new module also goes into `MODULES` in `src/ports/mod.rs`.
12. Add a unit test for logic that needs no game. Build the structs in byte arrays, as
    `ports/material.rs` does.
13. Write every comment in Simplified Technical English. `tools/STE.md` has the rules.

## What the shadow test can run two times

The Rust function runs first, on a dry device. Then the test puts the regions back and the
engine function runs for real.

- Dry device. A call that sets state or draws goes into the log and returns 0. It does not
  get to the device. `device::dry_skips` has the list. A call that reads goes to the device.
- Heap. The test tracks `operator new` (0x727692) and `operator delete` (0x727530). The
  engine run gets the blocks of the Rust run at the same addresses. A free in the Rust run
  waits until the engine run frees the block. A call that allocates or frees through these
  two functions calls `set.allocates()` in `regions`. A new block needs no region. A block
  that the call frees needs no region, unless the function changes the block before the
  free and the engine run reads it. Memory that exists and that the call writes needs a
  region: a bucket array, a list neighbour.
- Device resources. A `Create*` call of the Rust run is real, and the engine run gets the
  same object. So the test can compare a function that creates a vertex buffer, an index
  buffer or a texture. `Lock` and `Unlock` of a vertex or index buffer are real in both
  runs. The bytes of the buffer go into the log at `Unlock`.
- A call is a skip (`set.skip()`) when it does a thing that cannot happen two times and
  that is not in this list. Examples: `Present`, `Reset`, a `Release` that destroys a device
  object, a virtual deleting destructor of an engine object, `LockRect` of a texture or a
  surface, file access.

Make a call a skip only when it needs it. Decide from the arguments and from the state
before the call. The doc comment of the regions function says when the call is a skip and
why, in the form "The call is a skip when ...".

## Calling conventions

`port_thiscall!` and `port_cdecl!` cover `__thiscall` and `__cdecl`. Every function of the
table can take a detour, also the short stubs and the deleting destructors. The macros do
not cover a function that returns a float on the x87 stack. Do not port such a function.
Report it.

An argument of a device call can be the address of memory of the caller: a matrix on the
stack, the place for a result. The test does not compare such an address. It compares the
data at the address. So a port can keep such data in a Rust local.

## Changes that other mods make to the game code

The census compares each function in memory with the database and writes the differences to
`nirender.log`. A port must follow a change that it can follow:

- A mod changed the target of a `call`. The port calls the target that is in memory, through
  `site::call_target`, and lists the address of the call in the `SITES` of its module.
- A mod changed an operand or a few known bytes. The port reads the bytes from memory and
  does what they say. `FOLLOWED` in `src/ports/mod.rs` declares the place.

The installer locks the port of a function with any other change. A locked port stays game
code in that install. A function whose logic a mod replaces has no port:
`NiDX8VertexBufferManager::PackSkinnedVB` and `NiDX8SourceTextureData::Create` (MGE XE),
`NiDX8MaterialManager::UpdateToDevice` (Morrowind Code Patch).

The plugin must not change what MGE XE does. The runners read the log of MGE XE after each
run and fail the run when MGE XE installed no hooks or could not compile its shaders.

## Checks

    cargo test --features rig
    cargo build --release --features rig
    python tools/rig/run_test.py --label <name>      # starts the game; one run at a time

A port is complete when the run prints calls compared and 0 different for it, and the frame
tests in menu mode pass.

## Facts about the game

- Morrowind.exe was linked with whole-program optimisation. A caller can use ECX after a
  call when the callee does not change ECX. `SetCurrentMaterial` does this after
  `Entry::Update`. For that reason the entry thunk restores ECX and EDX. A port cannot
  return a value in ECX or EDX.
- The IDA database is of an executable with the Morrowind Code Patch. A function with a
  name like `patch*` is Code Patch code. Call it as an engine function.
- Some small functions give their result by leaving a register unchanged. Read the call
  sites when a function looks like it returns nothing.
- The Lua file of the plugin loads before the game creates its renderer. So the detours can
  be in place for the start of the game (`"early": true` in the config).
- MGE XE writes its call redirects into the draw functions after the plugin loads. MWSE
  writes its patches before.
