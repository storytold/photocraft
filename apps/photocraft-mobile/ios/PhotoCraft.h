// The C interface of libphotocraft_mobile.a (apps/photocraft-mobile/src/ios.rs). Call everything
// from the main thread. Calls before pc_start or after pc_stop do nothing.
#include <stdbool.h>
#include <stdint.h>

// Start PhotoCraft drawing into `layer`, a CAMetalLayer that must stay alive until pc_stop.
// Sizes are in pixels; `scale` is pixels per point. Returns whether it started.
bool pc_start(void *layer, uint32_t width_px, uint32_t height_px, float scale);
// Drop the app and its surface; call before the layer goes away.
void pc_stop(void);
void pc_resize(uint32_t width_px, uint32_t height_px, float scale);
// Call on every display refresh: draws only when something changed. Returns whether it drew.
bool pc_frame(void);
// phase: 0 began, 1 moved, 2 ended, 3 cancelled. Position in points; force 0...1, or 0 if none.
void pc_touch(uint64_t id, uint32_t phase, float x, float y, float force);
