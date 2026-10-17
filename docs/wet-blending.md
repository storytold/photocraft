# Wet Blending

Wet Blending is an optional color mixing mode for PhotoCraft's Mixer Brush. Enable it in Brush Settings › Transfer. The Blending, Dilution and Persistence values are saved with the brush settings and can also be passed to `paint.mixerBrush`.

## Controls

Each slider uses a value from 0 to 1. The model converts those values into per-dab coefficients:

```text
B0 = 1 - Blending
D0 = 1 - Dilution
R0 = 1 - Persistence
```

- **Blending** controls how much the brush color contributes to the mixed paint.
- **Dilution** controls how much sampled canvas color contributes to the mixture.
- **Persistence** controls how much of the previous dab's mixture carries into the next dab.

## How it works

For each dab, the brush samples color and opacity under its footprint. It combines that sample with the brush color using Dilution and Blending, then carries part of the previous mixture forward using Persistence. The first dab initializes the carried mixture. Each dab deposits the result using its coverage, brush flow, opacity, stroke opacity, selection and transparency lock. The carried mixture resets at the start of each stroke.

While drawing, the live preview includes the finishing dabs that releasing the pointer would add, including smoothing catch-up. These dabs use a copy of the carried mixture and working pixels, so showing the preview does not change the paint carried into the next pointer movement.

For an opaque brush color, the component-wise calculation is:

```text
Cd = lerp(Cs, Cu, D0)
Ad = lerp(As, 1, D0)
candidate.rgb = lerp(Cd, Ad * Cf, B0)
candidate.a = lerp(Ad, Ad * Af, B0)
carried = lerp(previous_carried, candidate, R0)
```

Here, `Cs` and `As` are the sampled color and opacity, `Cu` is the brush color, and `Cf` and `Af` are the brush color and opacity used for deposition. `lerp(a, b, t)` interpolates from `a` to `b` by `t`.
