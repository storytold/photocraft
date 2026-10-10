# photocraft-adobe-assets

Readers and writers for Adobe preset and asset files, at the format level. Standalone: it depends
on no other workspace crate (`cargo xtask layers`).

| Module | Files | Status |
|---|---|---|
| `atn` | Photoshop actions (`.atn`, version 16) | read and write; unmodified files write back byte for byte |

Action steps keep their ActionDescriptors as raw bytes, found with a structural walk that accepts
the same value types and nesting bound as `photocraft-psd`. Decode one with
`photocraft_psd::Descriptor::from_bytes`.

```rust
let set = photocraft_adobe_assets::atn::parse(&std::fs::read("Actions.atn")?)?;
for action in &set.actions {
    for step in &action.steps {
        println!("{} {}", step.event.to_string_lossy(), step.enabled);
    }
}
```

A `cargo-fuzz` skeleton lives in `fuzz/` (its own workspace, excluded from photocraft's). Run it
with `cargo +nightly fuzz run atn` from `crates/adobe-assets`.
