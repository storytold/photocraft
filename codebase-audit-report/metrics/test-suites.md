### Test-suite matrix (counts from `cargo test -- --list`)

| Package | Suite | Kind | Tests | of which #[ignore] | Layer | Deciding signal |
|---|---|---|--:|--:|---|---|
| photocraft | src/main.rs | in-crate | 128 | 1 | unit | in-crate #[cfg(test)] modules |
| photocraft | tests/control_exit_code.rs | test target | 4 | 0 | acceptance | spawns the built app binary (`CARGO_BIN_EXE_`) |
| photocraft | tests/startup_without_display.rs | test target | 2 | 0 | acceptance | spawns the built app binary (`CARGO_BIN_EXE_`) |
| photocraft-affinity | src/lib.rs | in-crate | 20 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-affinity | tests/real_files.rs | test target | 2 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-algo | src/lib.rs | in-crate | 570 | 22 | unit | in-crate #[cfg(test)] modules |
| photocraft-algo | tests/retouch_blur_exact.rs | test target | 2 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-automation | src/lib.rs | in-crate | 68 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-automation | tests/agent_tasks.rs | test target | 5 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-automation | tests/agent_tasks_more.rs | test target | 1 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-automation | tests/bridge.rs | test target | 3 | 0 | acceptance | drives the TCP control channel (`TcpStream`) |
| photocraft-automation | tests/inactive_save.rs | test target | 1 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-automation | tests/krita.rs | test target | 1 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-automation | tests/layered_save_dirty.rs | test target | 2 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-automation | tests/mcp.rs | test target | 40 | 0 | acceptance | launches the app with --control (`--control`) |
| photocraft-automation | tests/mcp_wire.rs | test target | 2 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-automation | tests/missing_font_warnings.rs | test target | 1 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-automation | tests/native_save_name.rs | test target | 2 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-automation | tests/openraster.rs | test target | 3 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-cli | src/lib.rs | in-crate | 4 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-cli | tests/cli.rs | test target | 29 | 0 | acceptance | spawns the built app binary (`CARGO_BIN_EXE_`) |
| photocraft-cli | tests/orientation.rs | test target | 1 | 0 | acceptance | spawns the built app binary (`CARGO_BIN_EXE_`) |
| photocraft-cms | src/lib.rs | in-crate | 23 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-cms | tests/bench.rs | test target | 1 | 1 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-cms | tests/cms.rs | test target | 26 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-cms | tests/regen.rs | test target | 1 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-codecs | src/lib.rs | in-crate | 65 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-codecs | tests/camera_raw.rs | test target | 7 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/dds.rs | test target | 13 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/decode_warnings.rs | test target | 14 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/deep_exr.rs | test target | 15 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-codecs | tests/detect_caps.rs | test target | 29 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/dpx.rs | test target | 13 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/exr_cryptomatte.rs | test target | 13 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/exr_multipart.rs | test target | 10 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/fidelity.rs | test target | 27 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/gif_frames.rs | test target | 10 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/limits.rs | test target | 27 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/malformed.rs | test target | 14 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-codecs | tests/metadata.rs | test target | 27 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/orientation.rs | test target | 20 | 1 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/pcx.rs | test target | 6 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/png_jpeg.rs | test target | 14 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-codecs | tests/pnm_peak_memory.rs | test target | 1 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-codecs | tests/resolution.rs | test target | 18 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/roundtrip.rs | test target | 348 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/source_settings.rs | test target | 5 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/tiff_bench.rs | test target | 1 | 1 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/tiff_exif.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/tiff_pages.rs | test target | 20 | 1 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/tiff_photoshop.rs | test target | 4 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/tiff_regressions.rs | test target | 5 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/webp_lossy.rs | test target | 8 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-codecs | tests/xpm.rs | test target | 9 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-color | src/lib.rs | in-crate | 26 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-compose | src/lib.rs | in-crate | 260 | 1 | unit | in-crate #[cfg(test)] modules |
| photocraft-compose | tests/artboard.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-doc | src/lib.rs | in-crate | 60 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-engine | src/lib.rs | in-crate | 1616 | 13 | unit | in-crate #[cfg(test)] modules |
| photocraft-engine | tests/affinity.rs | test target | 4 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-engine | tests/background_layer_consistency.rs | test target | 5 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/brush_smoothing.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/camera_raw_noise_clamp.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/close_document.rs | test target | 5 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/command_lookup.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/current_affinity_artboards.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/duplicate_layer_comps.rs | test target | 5 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/empty_selection_combine.rs | test target | 4 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/eraser_locked.rs | test target | 6 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/explicit_layer_target.rs | test target | 2 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-engine | tests/fill_lock_transparency.rs | test target | 2 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/filter_locked.rs | test target | 6 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/flatten_background.rs | test target | 4 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/gallery_colours.rs | test target | 2 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/gauss_small_sigma.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/guide_layout_limit.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/guide_layout_preview.rs | test target | 4 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/hdr_gamma_direction.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/jpeg_exif_resolution.rs | test target | 1 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-engine | tests/keyboard_shortcut_bulk.rs | test target | 2 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/kra.rs | test target | 1 | 1 | integration | real-file corpus (`corpus`) |
| photocraft-engine | tests/kys_import.rs | test target | 2 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/layer_delete_last.rs | test target | 4 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/layer_mask_depth.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-engine | tests/layer_range_panel_order.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/live_line_transform.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/mask_cache_purge.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/merge_transparent_bounds.rs | test target | 6 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/panic_hunt.rs | test target | 1 | 1 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/path_blur_speed_clamp.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/path_transform_target.rs | test target | 2 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/pdn.rs | test target | 1 | 1 | integration | filesystem I/O (`std::fs`) |
| photocraft-engine | tests/pixel_lock_edits.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/prefs_usage.rs | test target | 4 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-engine | tests/rasterize_psd_reopen.rs | test target | 2 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-engine | tests/rasterize_selected_layers.rs | test target | 6 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/rendering_preferences.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/save_keeps_encoding.rs | test target | 4 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-engine | tests/selection_layer_commands.rs | test target | 7 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/smart_clipboard_sources.rs | test target | 10 | 0 | integration | filesystem round trip (`tempfile`) |
| photocraft-engine | tests/smart_duplicates.rs | test target | 5 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-engine | tests/smart_filter_canvas_geometry.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/smart_filter_stages.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/smart_psd.rs | test target | 11 | 3 | integration | filesystem I/O (`std::fs`) |
| photocraft-engine | tests/svg_smart_object.rs | test target | 2 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/text_gamma.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/type_format_targets.rs | test target | 5 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/undo_keeps_active_layer.rs | test target | 3 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/unicode_colors.rs | test target | 4 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/wand_transparent_copy.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-engine | tests/working_rgb_profile.rs | test target | 10 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-format | src/lib.rs | in-crate | 40 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-format | tests/analysis.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-format | tests/autosave.rs | test target | 9 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-format | tests/corruption.rs | test target | 24 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-format | tests/directory_symlinks.rs | test target | 6 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-format | tests/layer_labels.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-format | tests/roundtrip.rs | test target | 43 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-format | tests/smart_contents.rs | test target | 5 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-geom | src/lib.rs | in-crate | 21 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-gpu | src/lib.rs | in-crate | 29 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-gpu | tests/device_loss.rs | test target | 10 | 0 | integration | real GPU device (`wgpu::`) |
| photocraft-gpu | tests/parity.rs | test target | 50 | 0 | integration | real GPU device (`wgpu::`) |
| photocraft-gpu | tests/uniform_limits.rs | test target | 4 | 0 | integration | real GPU device (`wgpu::`) |
| photocraft-gpu | tests/vector_mask.rs | test target | 1 | 0 | integration | real GPU device (`wgpu::`) |
| photocraft-heif | src/lib.rs | in-crate | 4 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-io | src/lib.rs | in-crate | 192 | 1 | unit | in-crate #[cfg(test)] modules |
| photocraft-io | tests/adjust_blend_roundtrip.rs | test target | 4 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/adjust_warnings.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/affinity.rs | test target | 13 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/affinity_artboards.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/annotations.rs | test target | 3 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/artboard_guide_order.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/banded.rs | test target | 4 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/cancel.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/channels.rs | test target | 1 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/cmyk_composite_export.rs | test target | 11 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/color.rs | test target | 6 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/composite.rs | test target | 14 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/comps_artboards.rs | test target | 3 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/corpus_regressions.rs | test target | 5 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/effects_multi.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/exif_resolution.rs | test target | 1 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/export_progress.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/flat.rs | test target | 60 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/gif_frames.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/heif_unsupported.rs | test target | 1 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/krita_corpus.rs | test target | 2 | 2 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/lab16.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/large_psb.rs | test target | 2 | 1 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/layer_labels.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/modes.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/multichannel.rs | test target | 2 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/orientation.rs | test target | 4 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/pattern_fill_psd.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/pcraft_manifest_name.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/pdn_corpus.rs | test target | 1 | 1 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/png_text_metadata.rs | test target | 5 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd32_export.rs | test target | 6 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/psd_background.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_block_alignment.rs | test target | 1 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_block_layout.rs | test target | 6 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_empty_channels.rs | test target | 1 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_font_identity.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_kerning.rs | test target | 7 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/psd_layer_block_length.rs | test target | 1 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_maximize_compatibility.rs | test target | 4 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_roundtrip_bytes.rs | test target | 1 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/psd_structure.rs | test target | 35 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_style_sheets.rs | test target | 5 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_thumbnail.rs | test target | 9 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_type_bounds.rs | test target | 6 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/psd_vertical_type.rs | test target | 1 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/raw.rs | test target | 12 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/resolution_axes.rs | test target | 5 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/roundtrip.rs | test target | 37 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/smart_contents_identity.rs | test target | 8 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/svg.rs | test target | 16 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-io | tests/text_corpus.rs | test target | 1 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/text_styles.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/tiff_layers.rs | test target | 24 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/tiff_pages.rs | test target | 2 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-io | tests/vector.rs | test target | 4 | 0 | integration | real-file corpus (`corpus`) |
| photocraft-io | tests/xmp_export.rs | test target | 3 | 0 | integration | document <-> file I/O crate (`photocraft_io`) |
| photocraft-ops | src/lib.rs | in-crate | 16 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-paint | src/lib.rs | in-crate | 103 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-plugins | src/lib.rs | in-crate | 5 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-plugins | tests/abi.rs | test target | 14 | 0 | integration | embedded real files (`include_bytes!`) |
| photocraft-psd | photocraft_psd | doc-tests | 1 | 0 | unit | rustdoc examples |
| photocraft-psd | src/lib.rs | in-crate | 205 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-psd | tests/builder.rs | test target | 27 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-psd | tests/layer_block_order.rs | test target | 1 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-psd | tests/malformed.rs | test target | 28 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-psd | tests/proptests.rs | test target | 28 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-psd | tests/roundtrip.rs | test target | 59 | 0 | integration | filesystem I/O (`std::fs`) |
| photocraft-psd | tests/swatch_malformed.rs | test target | 30 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-psd | tests/tree.rs | test target | 15 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-raster | src/lib.rs | in-crate | 25 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-raw | src/lib.rs | in-crate | 86 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-raw | tests/cr3.rs | test target | 7 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-raw | tests/development_limits.rs | test target | 4 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-raw | tests/dng.rs | test target | 33 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-raw | tests/hostile.rs | test target | 6 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-raw | tests/raf.rs | test target | 7 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-raw | tests/vendor_codecs.rs | test target | 13 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-raw | tests/vendors.rs | test target | 9 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-tablet | src/lib.rs | in-crate | 14 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-testkit | src/lib.rs | in-crate | 6 | 0 | unit | in-crate #[cfg(test)] modules |
| photocraft-text | src/lib.rs | in-crate | 158 | 4 | unit | in-crate #[cfg(test)] modules |
| photocraft-ui-egui | src/lib.rs | in-crate | 2699 | 12 | unit | in-crate #[cfg(test)] modules |
| photocraft-ui-egui | tests/adjust_preview_gpu.rs | test target | 2 | 0 | integration | real GPU device (`wgpu::`) |
| photocraft-ui-egui | tests/arbitrary_rotation_preview.rs | test target | 2 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/batch_export_options.rs | test target | 8 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/camera_raw_histogram.rs | test target | 3 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/camera_raw_localization.rs | test target | 1 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/camera_raw_scope.rs | test target | 21 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/camera_raw_zoom.rs | test target | 6 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/canvas_16f.rs | test target | 7 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/checker_canvas.rs | test target | 7 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/color_managed_canvas.rs | test target | 2 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/crop_beyond_canvas.rs | test target | 1 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/crop_default_mode.rs | test target | 5 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/custom_print_paper.rs | test target | 3 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/drag_preview_canvas.rs | test target | 3 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/export_as_scaling.rs | test target | 1 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/full_res_preview.rs | test target | 3 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/gpu_device_loss.rs | test target | 7 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/layout_perf.rs | test target | 6 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/live_stroke_canvas.rs | test target | 1 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/marquee_autoscroll.rs | test target | 3 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/paint_bucket_all_layers.rs | test target | 1 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/pattern_preview_canvas.rs | test target | 1 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/system_locale.rs | test target | 1 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/ui_scale_canvas.rs | test target | 2 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/variables_data_sets.rs | test target | 3 | 0 | acceptance | renders the egui shell headlessly (`egui_kittest`) |
| photocraft-ui-egui | tests/workspace_presets.rs | test target | 9 | 0 | unit | pure in-memory API calls (no I/O, GPU or process signal) |
| photocraft-vector | src/lib.rs | in-crate | 58 | 1 | unit | in-crate #[cfg(test)] modules |
| xtask | src/main.rs | in-crate | 61 | 0 | unit | in-crate #[cfg(test)] modules |

### Pyramid roll-up

| Layer | Suites | Tests | Share |
|---|--:|--:|--:|
| unit | 111 | 7550 | 89.1% |
| integration | 97 | 745 | 8.8% |
| acceptance | 30 | 177 | 2.1% |
| **Total** | | **8472** | |
