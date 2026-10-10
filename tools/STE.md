# Comments in Simplified Technical English

Written for: a person or an agent that writes or edits comments in this repo.

Every comment and every doc comment is in Simplified Technical English (STE).

## Rules

1. One idea in one sentence. A sentence has 20 words at most. An instruction or a
   description of what the code does has 25 words at most only when a list of names makes
   it long.
2. Active voice. Say who or what does the action: "The game reads the flag", not "the flag
   is read".
3. Simple present for what the code does. Simple past for what was found or done before.
   No "will", no "would", no "-ing" form as a noun when a noun or a verb fits.
4. Keep the articles ("the", "a").
5. No contractions (do not, it is, cannot).
6. No idioms, no figures of speech, no humour. Not "bail out", "kick in", "on the fly",
   "as-is", "happy path", "in practice", "under the hood".
7. One word for one thing, every time. Use the words this repo uses:
   - "the game" or "the game code" for Morrowind.exe; "the port" for the Rust function;
   - "device call" for a call of an `IDirect3DDevice8` method;
   - "the database" for the IDA database;
   - "run" for one execution; "the Rust run" and "the engine run" of a comparison;
   - "skip" for a call that is not compared.
8. Give numbers, addresses, sizes and units exactly: `0x6B42AC`, "8 bytes", "37 buckets".
9. No parenthesis that holds a second sentence. Make it a sentence of its own.
10. No symbols in place of words in a sentence: write "becomes", "then", "and", not an
    arrow, a slash between alternatives, or "&". Operators inside code are code.
11. ASCII only. No em dash, no arrows, no typographic quotes.
12. Start with a capital letter and end with a full stop when the comment is a sentence. A
    label of a few words (a section name, a field name) needs no full stop.

## What stays as it is

- Identifiers, engine names, addresses, instruction mnemonics, register names, byte
  sequences and code inside backticks.
- The information. Do not drop a fact, an address or a warning. Do not add a fact that the
  old comment did not have.
- The place of the comment and the kind (`//`, `///`, `//!`).

## What to remove

- A comment that only says again what the line of code says.
- Words that carry nothing: "simply", "just", "basically", "note that", "obviously".

## Example

Before:

    // NB: we bail early here if the tex isn't loaded yet - otherwise the upload path kicks in & we'd lock twice

After:

    // The port returns here when the texture has no device data. If it did not, the upload
    // would lock the surface a second time.
