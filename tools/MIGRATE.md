# Move a port file onto the shared modules

Written for: an agent that cleans one group of port files. Stage 2 of the audit of 2026-10-10.

The ports are verified in the game: each makes the same device calls and memory writes as the
game code, in the same order. This work must keep that. Change how the code is written, never
what it does or in which order.

## Shared modules (read them first)

- `src/engine.rs`: call primitives (`thiscall0..5`, `cdecl0..3`, `virtual_at`, `vcall0..3`),
  `operator_new`, `operator_delete`, the eh vector iterators, debug print, critical section,
  `ni_object_release`, `ni_pointer_set`, the two NiPointer assign functions, pixel format and
  pixel data constructors, and the data addresses that several files use.
- `src/d3d.rs`: every wrapper of a device, IDirect3D8, buffer, texture, surface or COM method,
  and the D3D constants.
- `src/ports/ni.rs`: the inline NetImmerse idioms (release, pointer assign, delete, RTTI).
- `src/ports/hash_map.rs`: the `THashMap` of the game.
- `src/x87.rs`: `ftol_asm!` and the `_ftol` helpers.

## What to do in each of your files

1. Replace each private helper that a shared module has with the shared one, and delete the
   private copy. Before you replace one, read both and confirm: same address or vtable slot,
   same calling convention, same argument widths, same order of reads and writes. When the
   private copy differs in a way that can matter, keep it and say why in its doc comment.
   A private copy that only drops the result or takes `u32` where the shared one takes
   `usize` is the same: adapt the call sites.
2. Replace private constants that `engine.rs` or `d3d.rs` define with the shared ones.
3. `shadow::Regions` has no limit now. Delete code that exists only for the old limit of 8
   regions: counters, "skip when more than 8", merging of adjacent blocks to save slots.
   Add every block the call writes. Keep each `set.skip()` that has another reason (the call
   cannot run twice), and keep `set.allocates()`.
4. Do not change the body of a port in any other way. Keep the `crate::shadow::touch(...)`
   line that starts each port function. Keep `#[cfg(test)]` stand-ins working: a file with a
   test `eng` module uses the `_with` forms of `hash_map` where the shared function calls
   the game allocator.
5. Comments: every comment follows `tools/STE.md`. Each regions function has a doc comment
   that says when the call is a skip and why, in the form "The call is a skip when ...".
   Use that form in the whole file. Fix a comment that states a wrong fact (the list in your
   task names the known ones); read the code to get the fact right.
6. Do not move functions between files. Do not rename a port function or a port module.

## Checks

    export PATH="$HOME/.cargo/bin:$PATH"
    cargo test
    cargo build --release

Both must pass with no warnings. Commit on your branch with a message in Simplified
Technical English that ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
Do not push. Do not run the game. Edit only your files (and a shared module only when your
task says so).

## Report

Under 400 words: lines before and after for each file; the helpers you replaced; each helper
you kept private and why; each skip you removed with the old reason; each comment fact you
corrected; anything in the code that looks wrong (do not fix behaviour, report it).
