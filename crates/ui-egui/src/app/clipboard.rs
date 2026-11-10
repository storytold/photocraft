use super::PhotocraftApp;

const MAX_PASTE_IMAGE_DIM: usize = 16_384;
const MAX_PASTE_IMAGE_PIXELS: usize = 64 * 1024 * 1024;

/// Full pixel snapshot of the last mirrored or imported OS clipboard image or SVG raster mirror,
/// backed by tiled `Surface` storage to avoid retaining a duplicate uncompressed pixel buffer.
#[derive(Clone)]
pub(crate) enum OsClipPixels {
    Surface(photocraft_geom::Rect, photocraft_raster::Surface),
}

impl OsClipPixels {
    fn matches(&self, w: u32, h: u32, bytes: &[u8]) -> bool {
        match self {
            Self::Surface(b, surface) => {
                let (Ok(w_usize), Ok(h_usize)) = (usize::try_from(w), usize::try_from(h)) else {
                    return false;
                };
                if b.width() != w || b.height() != h || bytes.len() != w_usize.saturating_mul(h_usize).saturating_mul(4) {
                    return false;
                }
                let mut row = Vec::new();
                if row.try_reserve_exact(w_usize).is_err() {
                    return false;
                }
                row.resize(w_usize, [0u8; 4]);
                let row_bytes = w_usize.saturating_mul(4);
                for y in 0..h {
                    surface.read_rgba8_into(photocraft_geom::Rect::from_xywh(b.x0, b.y0.saturating_add(y as i32), w, 1), &mut row);
                    let start = (y as usize).saturating_mul(row_bytes);
                    let slice = &bytes[start..start + row_bytes];
                    if row.iter().flatten().ne(slice.iter()) {
                        return false;
                    }
                }
                true
            }
        }
    }
}

/// Pending asynchronous Wayland clipboard read for `File › New`'s `Clipboard` preset.
#[derive(Clone)]
pub(crate) struct PendingNewDocClipboard {
    pub(crate) initial_fields: serde_json::Map<String, serde_json::Value>,
    pub(crate) request_sent: bool,
}

/// Compare `bytes` (`w × h` RGBA8) row-by-row against `surface` after `egui::Color32`'s
/// premultiplied-alpha round-trip, without retaining a duplicate pixel buffer.
fn matches_wayland_surface(b: photocraft_geom::Rect, surface: &photocraft_raster::Surface, w: u32, h: u32, bytes: &[u8]) -> bool {
    if b.width() != w || b.height() != h || bytes.len() != w as usize * h as usize * 4 {
        return false;
    }
    let mut row = Vec::new();
    if row.try_reserve_exact(w as usize).is_err() {
        return false;
    }
    row.resize(w as usize, [0u8; 4]);
    for y in 0..h {
        surface.read_rgba8_into(photocraft_geom::Rect::from_xywh(b.x0, b.y0.saturating_add(y as i32), w, 1), &mut row);
        let start = y as usize * w as usize * 4;
        let slice = &bytes[start..start + w as usize * 4];
        for (px, &target) in row.iter().zip(slice.as_chunks::<4>().0) {
            let rt = egui::Color32::from_rgba_unmultiplied(px[0], px[1], px[2], px[3]).to_srgba_unmultiplied();
            if rt != target {
                return false;
            }
        }
    }
    true
}

/// Compute a fast sampled hash of an RGBA8 buffer and its dimensions to detect unchanged copies.
fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    let mut sig = (w as u64) << 32 | h as u64;
    for b in px.iter().step_by(997) {
        sig = sig.rotate_left(5) ^ *b as u64;
    }
    sig
}

/// Match the bitmap signature by sampling the bounded SVG mirror, without another full pixel buffer.
fn svg_clip_signature(clip: &photocraft_engine::edit_cmds::Clip) -> u64 {
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    let mut sig = (u64::from(w) << 32) | u64::from(h);
    if w == 0 || h == 0 {
        return sig;
    }
    for i in (0..b.size().area().saturating_mul(4)).step_by(997) {
        let x = b.x0.saturating_add((i / 4 % u64::from(w)) as i32);
        let y = b.y0.saturating_add((i / 4 / u64::from(w)) as i32);
        let mut pixel = [[0; 4]];
        clip.surface.read_rgba8_into(photocraft_geom::Rect::from_xywh(x, y, 1, 1), &mut pixel);
        let value = pixel[0].get((i % 4) as usize).copied().unwrap_or_default();
        sig = sig.rotate_left(5) ^ u64::from(value);
    }
    sig
}

impl PhotocraftApp {
    /// Capture SVG and its raster mirror together; failures leave the previous copy intact.
    pub(super) fn import_os_svg_clipboard(&mut self) -> Result<bool, String> {
        let Some(get) = self.services.clipboard_get_svg.as_mut() else { return Ok(false) };
        let source = match get() {
            Ok(source) => source,
            Err(_) if self.clip_unmirrored && !self.clip_external && self.session.clipboard.is_some() => return Ok(false),
            Err(error) => {
                self.ui.status = error.clone();
                self.ui.status_error = true;
                return Err(error);
            }
        };
        let Some(source) = source else {
            self.os_clip_svg = None;
            // Bitmap equality cannot prove that an earlier external vector source is current.
            if self.clip_external && self.session.clipboard.as_ref().is_some_and(|clip| clip.svg.is_some()) {
                self.clear_external_clipboard();
            }
            return Ok(false);
        };
        // Already imported, or a newer internal copy: either stays until the external bytes
        // change. A raster mirror an ordinary paste took of these bytes is not their vectors.
        let holds_vectors_or_newer = self.session.clipboard.as_ref().is_some_and(|clip| clip.svg.is_some() || !self.clip_external);
        if self.os_clip_svg.as_deref().is_some_and(|old| old == &source) && holds_vectors_or_newer {
            return Ok(true);
        }
        let clip = photocraft_engine::edit_cmds::Clip::from_svg(&source).map_err(|error| {
            let error = error.to_string();
            self.ui.status = error.clone();
            self.ui.status_error = true;
            error
        })?;
        self.os_clip_sig = Some(svg_clip_signature(&clip));
        self.os_clip_pixels = Some(OsClipPixels::Surface(clip.bounds, clip.surface.clone()));
        self.os_clip_wayland_sig = None;
        self.wayland_copy_flushed = false;
        if self.pending_copy_image.take().is_some() {
            self.os_clip_wayland_res.pop();
        }
        self.os_clip_svg = clip.svg.clone();
        self.session.clipboard = Some(clip);
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }

    /// The SVG bytes on the OS clipboard right now, read on an explicit paste without importing
    /// them; None when there is no SVG service or the read fails (nothing is known then).
    fn os_svg_now(&mut self) -> Option<Option<std::sync::Arc<Vec<u8>>>> {
        let get = self.services.clipboard_get_svg.as_mut()?;
        get().ok().map(|source| source.map(std::sync::Arc::new))
    }

    /// Mirror the session clipboard onto the OS clipboard (RGBA8).
    pub(super) fn export_os_clipboard(&mut self) {
        self.clip_unmirrored = true;
        self.os_clip_wayland_sig = None;
        self.wayland_copy_flushed = false;
        if self.pending_copy_image.take().is_some() {
            self.os_clip_wayland_res.pop();
        }
        if !self.session.prefs().general.export_clipboard {
            return;
        }
        let Some((b, surface)) = self.session.clipboard.as_ref().map(|c| (c.bounds, c.surface.clone())) else {
            return;
        };
        if self.services.clipboard_set_image.is_none() && !self.services.is_wayland {
            return;
        }
        if b.is_empty() || b.width() == 0 || b.height() == 0 {
            return;
        }
        let pixels_u64 = u64::from(b.width()).saturating_mul(u64::from(b.height()));
        let (Ok(w), Ok(_h), Ok(pixels)) = (usize::try_from(b.width()), usize::try_from(b.height()), usize::try_from(pixels_u64)) else {
            let message = "Copied internally only: image exceeds the system clipboard size limit";
            self.ui.status = tl!(message).into();
            self.ui.status_error = false;
            return;
        };
        if pixels > MAX_PASTE_IMAGE_PIXELS {
            let message = "Copied internally only: image exceeds the system clipboard size limit";
            self.ui.status = tl!(message).into();
            self.ui.status_error = false;
            return;
        }
        let Some(buf_bytes) = pixels.checked_mul(4) else { return };
        let Some(buf_bytes_u64) = u64::try_from(buf_bytes).ok() else { return };
        let Some(row_bytes_u64) = u64::from(b.width()).checked_mul(4) else { return };
        let Ok(bytes_res) = photocraft_raster::memory::try_reserve(buf_bytes_u64) else { return };
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(buf_bytes).is_err() {
            drop(bytes);
            drop(bytes_res);
            return;
        }
        {
            let Ok(row_res) = photocraft_raster::memory::try_reserve(row_bytes_u64) else {
                drop(bytes);
                drop(bytes_res);
                return;
            };
            let mut row = Vec::new();
            if row.try_reserve_exact(w).is_err() {
                drop(row);
                drop(row_res);
                drop(bytes);
                drop(bytes_res);
                return;
            }
            row.resize(w, [0u8; 4]);
            for y in 0..b.height() {
                surface.read_rgba8_into(photocraft_geom::Rect::from_xywh(b.x0, b.y0.saturating_add(y as i32), b.width(), 1), &mut row);
                for p in &row {
                    bytes.extend_from_slice(p);
                }
            }
            drop(row);
            drop(row_res);
        }
        let arboard_ok = self.services.clipboard_set_image.as_mut().is_some_and(|set| set(b.width(), b.height(), &bytes).is_ok());
        if self.services.is_wayland {
            self.prepare_wayland_copy_export(b.width(), b.height(), pixels, buf_bytes, buf_bytes_u64, &bytes);
        }
        if arboard_ok {
            self.os_clip_svg = None;
            self.os_clip_sig = Some(clip_signature(b.width(), b.height(), &bytes));
            self.os_clip_pixels = Some(OsClipPixels::Surface(b, surface));
            self.clip_unmirrored = false;
        }
        drop(bytes);
        drop(bytes_res);
    }

    /// Build and stage the bounded Wayland `egui::ColorImage` export and round-trip signature.
    fn prepare_wayland_copy_export(&mut self, w: u32, h: u32, pixels: usize, _buf_bytes: usize, buf_bytes_u64: u64, bytes: &[u8]) {
        let Ok(stage_res) = photocraft_raster::memory::try_reserve(buf_bytes_u64) else { return };
        let mut color_pixels = Vec::new();
        if color_pixels.try_reserve_exact(pixels).is_err() {
            drop(color_pixels);
            drop(stage_res);
            return;
        }
        let mut wayland_sig = (w as u64) << 32 | h as u64;
        for (idx, &[r, g, b, a]) in bytes.as_chunks::<4>().0.iter().enumerate() {
            let c = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
            color_pixels.push(c);
            let roundtrip = c.to_srgba_unmultiplied();
            let base = idx * 4;
            for (offset, &b) in roundtrip.iter().enumerate() {
                if (base + offset) % 997 == 0 {
                    wayland_sig = wayland_sig.rotate_left(5) ^ u64::from(b);
                }
            }
        }
        let color_image = egui::ColorImage::new([w as usize, h as usize], color_pixels);
        self.os_clip_wayland_sig = Some(wayland_sig);
        if self.pending_copy_image.take().is_some() {
            self.os_clip_wayland_res.pop();
        }
        self.pending_copy_image = Some(color_image);
        self.os_clip_wayland_res.push(stage_res);
    }

    /// Emit any pending Wayland clipboard image as `OutputCommand::CopyImage` on `ctx` during
    /// the current input batch so `egui-winit` can claim the selection with the active token.
    pub(super) fn flush_pending_copy_image(&mut self, ctx: &egui::Context) {
        let copied_text = ctx.output(|o| o.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
        if copied_text {
            self.pending_copy_image = None;
            ctx.output_mut(|o| o.commands.retain(|c| !matches!(c, egui::OutputCommand::CopyImage(_))));
            self.os_clip_wayland_res.clear();
            if !self.clip_unmirrored || self.clip_external || self.wayland_copy_flushed {
                self.clear_external_clipboard();
            }
            return;
        }
        if let Some(image) = self.pending_copy_image.take() {
            let pending_res = self.os_clip_wayland_res.pop();
            ctx.output_mut(|o| o.commands.retain(|c| !matches!(c, egui::OutputCommand::CopyImage(_))));
            self.os_clip_wayland_res.clear();
            ctx.copy_image(image);
            if let Some(res) = pending_res {
                self.os_clip_wayland_res.push(res);
            }
            self.wayland_copy_flushed = true;
        }
    }

    /// Clear a previously imported external clipboard entry once the OS clipboard no longer holds it.
    pub(super) fn clear_external_clipboard(&mut self) {
        self.os_clip_svg = None;
        self.os_clip_sig = None;
        self.os_clip_pixels = None;
        self.os_clip_wayland_sig = None;
        self.wayland_copy_flushed = false;
        if self.pending_copy_image.take().is_some() {
            self.os_clip_wayland_res.pop();
        }
        self.session.clipboard = None;
        self.clip_external = false;
        self.clip_unmirrored = true;
    }

    /// Clear any unconsumed `Event::PasteImage` payloads queued for paste.
    pub(crate) fn clear_pending_paste_images(&mut self) {
        self.pending_paste_image = None;
        self.pending_paste_images.clear();
        self.wayland_paste_doc = None;
    }

    /// Execute `edit.paste` after `import_os_clipboard` has already run for this paste, pasting in
    /// place when an internal copy's bounds intersect the active view and centering otherwise.
    pub(crate) fn run_edit_paste(&mut self) -> Result<serde_json::Value, String> {
        self.clip_read_for_paste = true;
        let external = self.clip_external;
        let visible = !external
            && self.session.active_index().zip(self.session.clipboard.as_ref()).is_some_and(|(i, clip)| {
                let v = &self.ui.views[i];
                let pz = self.point_zoom();
                let (hw, hh) = (self.last_canvas_rect.width() / 2.0 / pz / self.ui.view.display_aspect(), self.last_canvas_rect.height() / 2.0 / pz);
                let r = photocraft_geom::Rect::new((v.center[0] - hw) as i32, (v.center[1] - hh) as i32, (v.center[0] + hw) as i32, (v.center[1] + hh) as i32);
                !clip.bounds.intersect(&r).is_empty()
            });
        let p = match self.session.active_index() {
            Some(i) if !visible => serde_json::json!({"center": self.ui.views[i].center}),
            _ => serde_json::json!({}),
        };
        self.run("edit.paste", p)
    }

    /// Keep `pending_paste_cmd_meta` aligned with `pending_paste_cmd` and record the active
    /// document when a Wayland paste chord (`Ctrl+V` / `Shift+Insert`) is pressed.
    fn sync_pending_paste_context(&mut self, ctx: &egui::Context, events: &[egui::Event], editing_text: bool) -> Option<egui::Id> {
        if self.services.is_wayland && !editing_text {
            for event in events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = event {
                    let is_paste_key = (*key == egui::Key::V && (modifiers.command || modifiers.ctrl)) || (*key == egui::Key::Insert && modifiers.shift);
                    if is_paste_key {
                        self.wayland_paste_doc = Some(self.session.active().map(|d| d.doc.id));
                    }
                }
            }
        }
        let pass_nr = ctx.cumulative_pass_nr();
        let focused = ctx.memory(|m| m.focused());
        while self.pending_paste_cmd_meta.len() < self.pending_paste_cmd.len() {
            self.pending_paste_cmd_meta.push_back((pass_nr, focused));
        }
        while self.pending_paste_cmd_meta.len() > self.pending_paste_cmd.len() {
            self.pending_paste_cmd_meta.pop_front();
        }
        focused
    }

    /// Complete or request an asynchronous Wayland clipboard read for `File › New`.
    fn resolve_pending_new_doc_clipboard(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput, transfer_cancelled: bool) -> bool {
        let Some(mut pending_new) = self.pending_new_doc_clipboard.take() else { return false };
        if transfer_cancelled {
            if let Some(pos) = raw_input.events.iter().position(|e| matches!(e, egui::Event::PasteImage(_))) {
                raw_input.events.remove(pos);
                return true;
            }
            return false;
        }
        if let Some(pos) = raw_input.events.iter().position(|e| matches!(e, egui::Event::PasteImage(_))) {
            let egui::Event::PasteImage(image) = raw_input.events.remove(pos) else { return false };
            self.pending_paste_image = Some(image);
            if self.import_os_clipboard().is_ok() {
                if !self.clip_paste_rejected
                    && let Some(clip) = self.session.clipboard.as_ref()
                {
                    let b = clip.bounds;
                    if !b.is_empty() {
                        pending_new.initial_fields.insert("width".into(), serde_json::json!(b.width()));
                        pending_new.initial_fields.insert("height".into(), serde_json::json!(b.height()));
                    }
                }
                let _ = self.run("ui.dialog.newDocument", serde_json::Value::Object(pending_new.initial_fields));
            }
            self.pending_paste_image = None;
            return true;
        }
        let got_paste_text = raw_input.events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
        if !got_paste_text && !pending_new.request_sent {
            pending_new.request_sent = true;
            self.pending_new_doc_clipboard = Some(pending_new);
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        false
    }

    /// Execute or discard a single popped deferred menu paste command with `image`.
    fn apply_deferred_paste_command(
        &mut self,
        cmd: &str,
        doc: Option<photocraft_doc::DocId>,
        focus_ok: bool,
        transfer_cancelled: bool,
        image: std::sync::Arc<egui::ColorImage>,
    ) {
        if transfer_cancelled || cmd.is_empty() || !focus_ok || (cmd != "file.newFromClipboard" && self.session.active().map(|d| d.doc.id) != doc) {
            return;
        }
        self.pending_paste_image = Some(image);
        let res = self.import_os_clipboard().and_then(|_| {
            if self.clip_paste_rejected {
                Ok(serde_json::json!({"pasted": false}))
            } else if cmd == "edit.paste" {
                if self.session.clipboard.is_some() { self.run_edit_paste() } else { Ok(serde_json::json!({"pasted": false})) }
            } else {
                self.clip_read_for_paste = true;
                self.run(cmd, serde_json::json!({}))
            }
        });
        if let Err(error) = res {
            self.ui.status = error;
            self.ui.status_error = true;
        }
        self.pending_paste_image = None;
    }

    /// Mark queued deferred paste commands as canceled when the active document or focus changes,
    /// retaining their queue slots until the Wayland transfer completes or is cancelled so a late
    /// `Event::PasteImage` is consumed rather than falling back to ordinary Paste.
    fn update_invalidated_paste_commands(&mut self, transfer_cancelled: bool, focused: Option<egui::Id>) {
        if transfer_cancelled {
            self.pending_paste_cmd.clear();
            self.pending_paste_cmd_meta.clear();
            return;
        }
        let active_doc = self.session.active().map(|d| d.doc.id);
        for (i, (cmd, target_doc)) in self.pending_paste_cmd.iter_mut().enumerate() {
            let init_focus = self.pending_paste_cmd_meta.get(i).and_then(|&(_, f)| f);
            let doc_changed = *cmd != "file.newFromClipboard" && target_doc.is_some() && *target_doc != active_doc;
            let focus_changed = init_focus.is_some() && init_focus != focused;
            if doc_changed || focus_changed {
                cmd.clear();
            }
        }
    }

    /// Consume queued deferred menu paste commands in FIFO order against `Event::PasteImage` and
    /// `Event::Paste` events in `raw_input`, or invalidate entries on focus/document/input change.
    fn drain_pending_paste_commands(&mut self, raw_input: &mut egui::RawInput, transfer_cancelled: bool, focused: Option<egui::Id>) -> bool {
        let mut consumed = false;
        let mut idx = 0;
        while idx < raw_input.events.len() && !self.pending_paste_cmd.is_empty() {
            match &raw_input.events[idx] {
                egui::Event::PasteImage(image) => {
                    let image = std::sync::Arc::clone(image);
                    raw_input.events.remove(idx);
                    let Some((cmd, doc)) = self.pending_paste_cmd.pop_front() else { break };
                    let meta = self.pending_paste_cmd_meta.pop_front();
                    let focus_ok = meta.is_none_or(|(_, init_focus)| init_focus.is_none() || init_focus == focused);
                    self.apply_deferred_paste_command(&cmd, doc, focus_ok, transfer_cancelled, image);
                    consumed = true;
                }
                egui::Event::Paste(_) => {
                    idx += 1;
                    let Some((cmd, doc)) = self.pending_paste_cmd.pop_front() else { break };
                    let _ = self.pending_paste_cmd_meta.pop_front();
                    if !transfer_cancelled && !cmd.is_empty() && self.session.active().map(|d| d.doc.id) == doc {
                        let message = "Nothing to paste: the clipboard holds no image";
                        self.ui.status = tl!(message).into();
                        self.ui.status_error = false;
                    }
                    consumed = true;
                }
                _ => {
                    idx += 1;
                }
            }
        }
        self.update_invalidated_paste_commands(transfer_cancelled, focused);
        consumed
    }

    /// Record any `Event::PasteImage` in `raw_input`, execute or discard queued deferred menu paste
    /// commands bound to their initiating documents, and report `(async_wayland_paste, consumed_paste_event)`.
    pub(super) fn preprocess_paste_events(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput, editing_text: bool) -> (bool, bool) {
        if self.pending_copy_image.is_none() {
            self.os_clip_wayland_res.clear();
        }
        let focused = self.sync_pending_paste_context(ctx, &raw_input.events, editing_text);
        let mut got_paste_image = false;
        let mut got_paste_text = false;
        let mut transfer_cancelled = false;
        for event in &raw_input.events {
            match event {
                egui::Event::PasteImage(_) => got_paste_image = true,
                egui::Event::Paste(_) => got_paste_text = true,
                egui::Event::Key { pressed: true, .. }
                | egui::Event::PointerButton { pressed: true, .. }
                | egui::Event::WindowFocused(false)
                | egui::Event::Copy
                | egui::Event::Cut => transfer_cancelled = true,
                _ => {}
            }
        }
        let async_wayland_paste = self.services.is_wayland && (got_paste_image || got_paste_text);

        if editing_text {
            if let Some(pending_new) = self.pending_new_doc_clipboard.take()
                && pending_new.request_sent
                && !transfer_cancelled
            {
                self.pending_paste_cmd.push_back((String::new(), None));
                self.pending_paste_cmd_meta.push_back((ctx.cumulative_pass_nr(), focused));
            }
            for (cmd, _) in &mut self.pending_paste_cmd {
                cmd.clear();
            }
            let _ = self.drain_pending_paste_commands(raw_input, transfer_cancelled, focused);
            self.clear_pending_paste_images();
            if got_paste_image {
                raw_input.events.retain(|e| !matches!(e, egui::Event::PasteImage(_)));
            }
            return (async_wayland_paste, false);
        }

        if got_paste_text && !got_paste_image && self.services.is_wayland && (!self.clip_unmirrored || self.clip_external || self.wayland_copy_flushed) {
            self.clear_external_clipboard();
        }

        let mut consumed_paste_event = self.resolve_pending_new_doc_clipboard(ctx, raw_input, transfer_cancelled);
        consumed_paste_event |= self.drain_pending_paste_commands(raw_input, transfer_cancelled, focused);

        let remaining_images: Vec<std::sync::Arc<egui::ColorImage>> = raw_input
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PasteImage(img) => Some(std::sync::Arc::clone(img)),
                _ => None,
            })
            .collect();

        if !remaining_images.is_empty() {
            let active_doc = self.session.active().map(|d| d.doc.id);
            let target_doc = self.wayland_paste_doc.take().unwrap_or(active_doc);
            if target_doc != active_doc {
                raw_input.events.retain(|e| !matches!(e, egui::Event::PasteImage(_)));
                consumed_paste_event = true;
            } else {
                for image in remaining_images {
                    self.pending_paste_images.push_back((image, target_doc));
                }
                self.pending_paste_image = self.pending_paste_images.front().map(|(img, _)| std::sync::Arc::clone(img));
            }
        }

        (async_wayland_paste, consumed_paste_event)
    }

    /// Remove any queued `Event::PasteImage` entries targeted at a document other than the
    /// currently active one, returning `true` if all queued images were rejected due to a switch.
    pub(crate) fn cancel_mismatched_paste_images(&mut self) -> bool {
        let active_doc = self.session.active().map(|d| d.doc.id);
        if self.pending_paste_images.is_empty() {
            return false;
        }
        self.pending_paste_images.retain(|(_, target_doc)| target_doc.is_none() || *target_doc == active_doc);
        self.pending_paste_image = self.pending_paste_images.front().map(|(img, _)| std::sync::Arc::clone(img));
        self.pending_paste_images.is_empty()
    }

    /// Handle an empty native clipboard read without discarding a valid Wayland internal copy.
    fn handle_empty_os_clipboard(&mut self) -> Result<bool, String> {
        if self.services.is_wayland
            && !self.clip_external
            && self.session.clipboard.is_some()
            && (self.os_clip_wayland_sig.is_some() || self.wayland_copy_flushed)
        {
            return Ok(false);
        }
        if self.clip_unmirrored && !self.clip_external && self.session.clipboard.is_some() {
            self.os_clip_sig = None;
            self.os_clip_pixels = None;
            // Keep remembering an SVG only while it is still there (an SVG-only clipboard),
            // so a later copy of the same bytes in another app is imported again.
            if let Some(now) = self.os_svg_now()
                && now != self.os_clip_svg
            {
                self.os_clip_svg = None;
            }
            return Ok(false);
        }
        self.os_clip_svg = None;
        if (self.os_clip_sig.take().is_some() || self.clip_external) && !self.clip_unmirrored {
            self.clear_external_clipboard();
        }
        Ok(false)
    }

    /// Convert a validated `egui::ColorImage` from `Event::PasteImage` into RGBA8 bytes after
    /// reserving working memory fallibly.
    fn decode_pasted_color_image(&mut self, pasted: &egui::ColorImage) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        let [w, h] = pasted.size;
        let pixels = w.checked_mul(h);
        if w == 0
            || h == 0
            || w > MAX_PASTE_IMAGE_DIM
            || h > MAX_PASTE_IMAGE_DIM
            || pixels.is_none_or(|n| n > MAX_PASTE_IMAGE_PIXELS || pasted.pixels.len() != n)
        {
            self.clip_paste_rejected = true;
            if self.clip_external {
                self.clear_external_clipboard();
            }
            let message = "Cannot paste: clipboard image exceeds the maximum dimensions";
            self.ui.status = tl!(message).into();
            self.ui.status_error = true;
            return Ok(None);
        }
        let Some((w_u32, h_u32)) = u32::try_from(w).ok().zip(u32::try_from(h).ok()) else {
            self.clip_paste_rejected = true;
            return Ok(None);
        };
        let Some(buf_bytes) = pasted.pixels.len().checked_mul(4) else {
            self.clip_paste_rejected = true;
            return Ok(None);
        };
        let Ok(buf_bytes_u64) = u64::try_from(buf_bytes) else {
            self.clip_paste_rejected = true;
            return Ok(None);
        };
        let _res = photocraft_raster::memory::try_reserve(buf_bytes_u64).inspect_err(|error| {
            self.ui.status = error.clone();
            self.ui.status_error = true;
        })?;
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(buf_bytes).is_err() {
            let error = "not enough working memory; reduce the operation or wait for other jobs".to_string();
            self.ui.status = error.clone();
            self.ui.status_error = true;
            return Err(error);
        }
        for c in &pasted.pixels {
            bytes.extend_from_slice(&c.to_srgba_unmultiplied());
        }
        Ok(Some((w_u32, h_u32, bytes)))
    }

    /// If the OS clipboard holds an image that isn't the one we put there, make it the session
    /// clipboard (so ⌘V pastes screenshots and images copied in other apps, like Photoshop).
    /// Returns `Ok(true)` when a new external image was imported. Once the OS clipboard no longer holds
    /// the image we mirrored or imported (text or a file was copied since), that image is stale:
    /// it is dropped rather than pasted. Unexported internal copies survive unavailable or empty
    /// system clipboards.
    pub(crate) fn import_os_clipboard(&mut self) -> Result<bool, String> {
        self.clip_paste_rejected = false;
        let had_queued = !self.pending_paste_images.is_empty();
        if self.cancel_mismatched_paste_images() {
            return Ok(false);
        }
        let next_pasted = if had_queued {
            let popped = self.pending_paste_images.pop_front().map(|(img, _)| img);
            self.pending_paste_image = self.pending_paste_images.front().map(|(img, _)| std::sync::Arc::clone(img));
            popped
        } else {
            self.pending_paste_image.take()
        };
        let image = if let Some(pasted) = next_pasted {
            let Some(decoded) = self.decode_pasted_color_image(&pasted)? else {
                return Ok(false);
            };
            Some(decoded)
        } else {
            let Some(get) = self.services.clipboard_get_image.as_mut() else { return Ok(false) };
            // A read failure says nothing about the clipboard contents. Keep our editable copy and
            // its signature rather than treating the failure as an empty image.
            match get() {
                Ok(image) => image,
                Err(error) => {
                    // An unexported copy remains usable without an OS clipboard (for example, in
                    // a Wayland sandbox). Mirrored/imported copies still require a successful read
                    // before pasting, because their system contents may have changed.
                    if (self.clip_unmirrored
                        || (self.services.is_wayland && self.services.clipboard_unreadable.as_ref().is_some_and(|unreadable| unreadable())))
                        && !self.clip_external
                        && self.session.clipboard.is_some()
                    {
                        return Ok(false);
                    }
                    if self.services.clipboard_unreadable.as_ref().is_some_and(|unreadable| unreadable())
                        && (self.session.clipboard.is_none() || self.services.is_wayland)
                    {
                        if self.clip_external {
                            self.clear_external_clipboard();
                        }
                        // Let paste explain the unavailable backend without treating a saved copy as empty.
                        return Ok(false);
                    }
                    self.ui.status = error.clone();
                    self.ui.status_error = true;
                    return Err(error);
                }
            }
        };
        let image = image.filter(|(w, h, bytes)| *w > 0 && *h > 0 && bytes.len() == *w as usize * *h as usize * 4);
        let Some((w, h, bytes)) = image else {
            return self.handle_empty_os_clipboard();
        };
        self.finish_import_os_clipboard(w, h, bytes)
    }

    /// Store validated external RGBA8 pixels `(w, h, bytes)` into the session clipboard unless their
    /// signature and full pixel buffer match the current copy.
    fn finish_import_os_clipboard(&mut self, w: u32, h: u32, bytes: Vec<u8>) -> Result<bool, String> {
        let sig = clip_signature(w, h, &bytes);
        let matches_primary = self.os_clip_sig == Some(sig) && self.os_clip_pixels.as_ref().is_none_or(|p| p.matches(w, h, &bytes));
        let matches_wayland = self.os_clip_wayland_sig == Some(sig)
            && self.session.clipboard.as_ref().is_some_and(|clip| matches_wayland_surface(clip.bounds, &clip.surface, w, h, &bytes));
        if (matches_primary || matches_wayland) && self.session.clipboard.is_some() {
            return Ok(false);
        }
        let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        let mut surface = photocraft_raster::Surface::from_interleaved(photocraft_color::PixelFormat::RGBA8, r, &bytes);
        surface.prune();
        self.os_clip_pixels = Some(OsClipPixels::Surface(r, surface.clone()));
        self.session.clipboard = Some(photocraft_engine::edit_cmds::Clip {
            components: None,
            surface,
            bounds: r,
            layers: None,
            resolution: None,
            source: None,
            svg: None,
            board: None,
        });
        // The SVG that came with this image (a vector app puts both): a later internal copy is
        // newer than it, so Paste As must not bring it back while it is unchanged.
        self.os_clip_svg = self.os_svg_now().flatten();
        self.os_clip_sig = Some(sig);
        self.os_clip_wayland_sig = None;
        self.wayland_copy_flushed = false;
        if self.pending_copy_image.take().is_some() {
            self.os_clip_wayland_res.pop();
        }
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }
}
