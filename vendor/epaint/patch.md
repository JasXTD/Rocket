# epaint 0.37.0, modified for bidirectional text

This directory is the published crate [epaint 0.37.0](https://crates.io/crates/epaint/0.37.0)
(crates.io SHA-256 `97bac86ae3dde93fc3951e790951caea739ff39a6c89d909cdaf1d9278dbb1bd`,
upstream commit in `.cargo_vcs_info.json`), used through `[patch.crates-io]` in Rocket's
root `Cargo.toml`. The change is the bidirectional layout Download Engine ships in its own
`vendor/epaint`, and the code comments still name Download Engine. `src/text/bidi.rs` is
Download Engine's file, unchanged. Its changes to `src/text/text_layout.rs` were written
for epaint 0.36.2; egui 0.37 reworked that file's font handling, so Rocket re-applied them
by hand on 2026-11-12, and the two `text_layout.rs` files no longer compare line for line.
epaint is licensed under MIT OR Apache-2.0, and Rocket uses it under MIT, as
`THIRD-PARTY-NOTICES.txt` lists.

## Why

Upstream epaint 0.37.0 still splits text into runs by font only and lets the shaper guess
each run's direction from its first letter. In an Arabic run, Latin words, digits and
punctuation were therefore drawn reversed ("swodniW" for "Windows", "06" for "60"), and
wrapped right-to-left paragraphs showed their rows in reverse order. Rocket's Arabic
interface text needs them in reading order. Upstream has no bidirectional layout yet;
drop this patch when an egui release provides it.

## What changed

- `src/text/bidi.rs` (new): embedding levels from a subset of the Unicode Bidirectional
  Algorithm (UAX #9): weak types W1–W7, paired brackets N0, neutrals N1–N2, implicit
  levels I1–I2 and separators in L1. Character properties come from `icu_properties`.
  Explicit embeddings, overrides and isolates are not supported and stay invisible, as
  before.
- `src/text/text_layout.rs`: a paragraph that contains right-to-left script characters
  is laid out with bidi levels. It reads right to left when it contains a right-to-left
  letter. Runs split where the font or the level changes and are shaped in their own
  direction. Glyphs are kept in logical order for line breaking, and each row is then
  put into visual order (L1 trailing whitespace, L2), returning glyphs to their shaped
  positions. Text without right-to-left characters takes the upstream path unchanged.
- `src/text/mod.rs`: declares the `bidi` module.
- `Cargo.toml`: adds `icu_properties`. Rocket pins it to `=2.3.0`, and its Cargo.lock
  holds the same 20 ICU4X and helper crate versions as Download Engine's; see the
  2026-11-07 entry in `DEPENDENCIES.md`.

What the move to 0.37.0 changed in the patch, and nothing else:

- The layout functions take epaint's `FontsImpl` where 0.36.2 passed a `Font` borrow:
  faces are looked up with `fonts.face(key)`, and a cluster's face with
  `fonts.resolve_cluster_face`, as the upstream path now does.
- 0.37.0 can draw a cluster through a glyph rasterizer, or stand in for a family that has
  no font; such a cluster is a run of its own that is never shaped. In a bidirectional
  paragraph it gets its embedding level like any other run and is reordered as one unit
  (`segment_into_bidi_runs`, `layout_bidi_segment`). Rocket registers no glyph
  rasterizer, so this path is compiled but not exercised by Rocket's tests.

Not changed: sections of a layout job are each resolved on their own (with the
paragraph's direction); the caret, selection and cursor movement in text edits are
not direction-aware; Rocket's interface is not mirrored, so Arabic text keeps the
alignment of the place it is shown in.

## Checking or updating the copy

Extract the upstream crate from the local Cargo cache and compare:

```powershell
tar -xzf $env:USERPROFILE\.cargo\registry\cache\index.crates.io-1949cf8c6b5b557f\epaint-0.37.0.crate -C $env:TEMP
git diff --no-index $env:TEMP\epaint-0.37.0 vendor\epaint
```

Only the files listed above and this file differ (`.cargo-ok` is not copied). Cargo never
downloads a crate that a path patch replaces, so if the archive is not in the cache, take
it from `https://static.crates.io/crates/epaint/epaint-0.37.0.crate` and check its
SHA-256 against the value above first. When egui is upgraded, re-apply the change to the
new epaint version or remove the patch, along with the `[patch.crates-io]` entry.
