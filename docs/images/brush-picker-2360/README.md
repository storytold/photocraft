# Brush Preset picker opening tap (#2360)

Before and after the fix, rendered offscreen at 1200 × 800, one physical pixel
per egui point, with a synthetic 400 × 300 blank document and the Brush tool.

A temporary copy of the `snapshot` example sent secondary press/release at
(457.5, 435.5), a primary outside click at (200, 200), then the secondary tap
again. Each tap placed its press and release together in `RawInput.events`;
`Harness::event` processes queued events in separate frames and cannot model
this timing. Four frames settled after each tap.

- `before.png`: original code at ab15e4ea; the picker dismisses in its opening frame.
- `after.png`: fixed code; the reopened picker remains visible.

The full-UI regression in `paint_mouse::tests::secondary_taps_reopen_the_picker_after_closing`
covers this event timing, repeat reopening, later outside dismissal, and no
painting or history change from opening. The contributor also verified the fix
with a real macOS two-finger trackpad click.

Screenshots are original PhotoCraft UI captures, licensed MIT OR Apache-2.0
(see `LICENSE.txt`); no external artwork is included.
