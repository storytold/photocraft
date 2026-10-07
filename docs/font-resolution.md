# PostScript font resolution

PSD text may identify a face by its PostScript name as well as a family/style.
The PostScript prefix need not match the family name. For example, a registered
face whose family is Inter and PostScript name is Other-Regular cannot be found
reliably by looking only for a family named Other.

## Resolution and invalidation

Retain the existing fast family-based lookup. If it fails, lazily index actual
PostScript names from available faces and use that index as a fallback. Invalidate
the index when fonts are registered or system fonts are loaded so a cached miss
does not outlive a new font. Common style suffixes without a hyphen are inferred
conservatively; ambiguous or missing fonts still follow existing fallback rules.

Missing-font detection and replacement use the same resolution decision. A run
that already resolves must not be replaced just because another unresolved run
shares its family label. Tests construct renamed metadata from the existing
permissively licensed test font in memory; no font binary is added.

## Verification

Run:

```sh
cargo test -p photocraft-text -p photocraft-engine --locked
cargo clippy -p photocraft-text -p photocraft-engine --all-targets --locked -- -D warnings
cargo xtask test-corpus
cargo test -p photocraft-engine --test panic_hunt -- --ignored
cargo xtask perf --quick
```

The font tests cover a PostScript name unrelated to the family prefix, repeated
lookup, registering a font after an earlier miss, and hyphenless style suffixes.
The engine test checks replacement leaves already-resolved runs intact.

## Limits and cost

The first fallback builds an index and therefore costs more than the fast path;
subsequent lookups reuse it. Measure on representative installed-font collections
before claiming a performance improvement. A previous local diagnostic observed
roughly 22.7 ms versus 0.42 ms for a first uncached miss; this is an illustrative
older measurement, not a cross-machine benchmark or a current branch benchmark.

This independently reproducible correction does not prove that PhotoCraft
[#456](https://github.com/storytold/photocraft/issues/456) is resolved. The original
PSD/font combination was not reproduced, and the public Gmarket fonts already
resolved in the baseline probe. Font availability, installation location and
PostScript identity still matter. Native IME startup and UI translation are
separate changes.
