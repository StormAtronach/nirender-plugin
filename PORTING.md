# How to port an engine function

Written for: a person or an agent that adds ports to this repo.

The goal of a port is the same device calls, the same memory writes and the same return value
as the function in Morrowind.exe, for every input. The shadow test in the game checks this on
every call, so a port that is only close will fail.

## Sources

- `tools/out/dump/<address>.txt`: the Hex-Rays pseudocode and the disassembly of each function
  of the renderer subsystem (made by `tools/ida_dump_funcs.py`; not in git).
- `src/gen/layouts.rs`: the offsets of the struct members, from the IDA database.
- `src/gen/funcs.rs`: every function with its size, its stack argument bytes and whether it
  can take a detour (`pic`).
- `tools/out/abi.json`: which registers a caller reads after the call.
- `moreFPS/docs/engineering-notes/renderer/*.md`: the characterization of each class.

The disassembly is the authority. The pseudocode shows the structure; it hides operand widths,
the order of writes and what a register holds.

Clean-room rule: the code comes from the binary. Do not copy from the Gamebryo source.

## Rules

1. One Rust function per engine function, in `src/ports/<class>.rs`, with the address and the
   engine name in its doc comment. Arguments are `usize` for pointers, `u32` for integers and
   enums, `u8` for `char` and `bool`, `f32` for floats.
2. Work on the engine objects in place: `rd::<T>(base, offset)` and `wr(base, offset, value)`
   with offsets from `gen::layouts`. Do not keep state in Rust statics.
3. Read each operand at the width the instruction reads it. `mov al, [x]` is `rd::<u8>`,
   `mov ax, [x]` is `rd::<u16>`. Look for `xor eax, eax` or `movzx` before you zero-extend.
4. Keep the order of device calls and of memory writes. Keep calls that look redundant.
5. Call the device through its vtable, as the game does (`engine::device_set_render_state` is
   the pattern; add helpers of that kind to `engine.rs` for other methods). Never cache the
   function pointer: the recorder replaces the vtable.
6. A call to a function that has a port is a direct Rust call to that port. A call to
   anything else goes through a helper in `engine.rs` that calls the fixed address. Add the
   helper with the engine name and the address.
7. Floats. The x87 control word in the game is 0x007F: 24-bit precision. So `f32` `+ - * /`
   and `sqrt` give the bits the x87 code gives, as long as you do one operation per source
   operation in the same order (no reassociation, no `mul_add`). A float that only passes
   through `fld`/`fstp` uses `mem::x87_pass`. For `fsin`, `fcos`, `fptan`, `fpatan`, `fistp`
   and calls to `_ftol` add a helper to `x87.rs` that runs the same instruction. Do not use
   `f32::sin` and friends.
8. Return what the function leaves in EAX when a caller can read it: the return type of the
   port is `u32`, `usize`, `u8` or `bool` and `compare_eax = true`. When EAX is a leftover,
   the type is `()` and `compare_eax = false`. `tools/out/abi.json` lists the call sites
   where EAX is read after the call ("live": ["eax"]).
9. `regions` lists every block of memory the function can write, for the call at hand: the
   object, and what it reaches through pointers. The shadow test saves and restores exactly
   these. A write outside them is not restored and breaks the game state in shadow mode.
10. What the shadow test can run twice, and what it cannot:
    - The Rust function runs first on a "dry" device: calls that set state or draw are
      recorded and return 0 without reaching the device (`device::dry_skips` has the list);
      calls that read are passed on. Then the regions are put back and the engine function
      runs for real.
    - Heap: `operator new` (0x727692) and `operator delete` (0x727530) are tracked. The engine
      run gets the blocks of the Rust run at the same addresses, and a free in the Rust run
      is put off. A call that allocates or frees through these two calls
      `set.allocates()` in `regions`, not `set.skip()`. The new blocks need no region; a
      block that is freed does not either. Memory that is written and already exists (a
      hash bucket array, a list neighbour) still needs a region.
    - Device resources: a `Create*` call of the Rust run is real, and the engine run gets
      the same object back. So a function that creates a vertex buffer, an index buffer or
      a texture can be compared. `Lock`/`Unlock` of vertex and index buffers are real in
      both runs, and the bytes written go into the log at `Unlock`.
    - A call still calls `set.skip()` when it does something that cannot happen twice and
      is not in the list above: `Present`, `Reset`, a `Release` that destroys a device
      object, a virtual deleting destructor of an engine object, `LockRect` of a texture
      or surface (not recorded yet), a critical section left locked, file access.
    - `Regions` holds 8 blocks. A call that needs more calls `set.skip()`.
    Skip only the calls that need it, decided from the arguments and the state before the
    call. Say in the doc comment of the regions function why.
11. Declare the port with `port_thiscall!` or `port_cdecl!` and add it to the `PORTS` list of
    the module. A new module also goes into `MODULES` in `src/ports/mod.rs`.
12. Add a unit test for logic that needs no game: build the structs in byte arrays, as
    `ports/material.rs` does.

## Calling conventions the macros cover

`port_thiscall!` and `port_cdecl!`. Every function of the table can take a detour, also the
short stubs and the deleting destructors. Not covered yet: `__stdcall` functions and
functions that return a float on the x87 stack. Leave those and report them.

Arguments of the device calls that are addresses of the caller's own storage (a matrix on
the stack, the place for a result) are not compared, only the data behind them. So a port
may keep such data in a Rust local.

## What not to port without a decision

`census` lines in `nirender.log` name the functions another mod changed in memory. The port
replicates the database, so replacing such a function would drop the change. Leave them and
report them.

## Checks

    cargo test
    cargo build --release
    python tools/run_test.py --label <name>      # starts the game; one run at a time

A port is done when the run prints it with calls compared and 0 different, and the frame
tests in menu mode pass.

## Things the game taught us

- Morrowind.exe was linked with whole-program optimisation. A caller can use ECX after a call
  when the callee does not change it (`SetCurrentMaterial` after `Entry::Update`). The entry
  thunk restores ECX and EDX for that reason. A port cannot return a value in ECX or EDX.
- The IDA database is of an executable with the Morrowind Code Patch applied. Functions with
  names like `patch*` are Code Patch code; call them as engine functions.
- Several small functions keep their result only by leaving a register as it was. Read the
  call sites when a function looks like it returns nothing.
