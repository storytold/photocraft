# Global Perspective Warp - solver and engine command

This first independent contribution adds only the solver and headless engine
command edit.perspectiveWarpGlobal; the guide editor UI is a separate PR.

Independent door/window quadrilaterals constrain ONE global homography of
the entire active raster photo layer. No piecewise patch warps, folds or
stitching are performed. Incompatible guides are reported. The transform is
validated across the entire source domain (finite denominator, positive
Jacobian and bounded raster allocation). Physical aspect cannot be inferred
without metric information; options.aspect is a global manual correction.

Engine command: edit.perspectiveWarpGlobal
Example params: {"guides":[{"id":1,"corners":[[25,20],[100,16],[109,105],[21,110]]}],
"options":{"fitCanvas":true,"aspect":1},"solveOnly":true}

Guide IDs must be unique, corners ordered TL/TR/BR/BL, up to 128 guides,
weights 0.05..20, aspect 0.25..4, maximum 32768 pixels per image side,
and maximum final allocation 67,108,864 output pixels. Invalid geometry,
selection, masks, locked or non-raster layers are explicitly refused.
Optional layer, document, interpolation and solveOnly are supported.
solveOnly returns diagnostics without document/history mutation.
An effective warp commits once and supports Undo/Redo. Rejection is atomic.

Tests: crates/algo/src/global_perspective.rs and
crates/engine/src/global_perspective_cmds.rs.

Provenance: original full V3.2 source at SHA
8946cc97cc3fb37ad967064d053c40c79879bfbb in PR #5651.
Prior portable Windows acceptance applies only to that exact older source;
this split branch requires its own build/tests/CI before integration.
