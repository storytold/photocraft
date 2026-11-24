//! The Timeline panel's layer tracks: one row per layer, top to bottom as in the Layers panel, each
//! with a bar over the frames the layer shows on (Layer › Frame Range). Dragging a bar moves its
//! range, dragging an end trims it, the eye switch is the layer's visibility, and a click selects
//! the layer. Every change runs an engine command (`layer.frameRange.set`, `layer.setProps`,
//! `layer.select`, `timeline.setFrame`), so the control channel can do everything the tracks do.

use craft::behavior::{EditEvent, SelectionChange};
use craft::timeline::time::{FrameRate, Tick, TimeRange};
use craft::timeline::ui::{Timeline, TimelineEdit, TrackSwitch};
use craft::timeline::{ClipEdge, ClipEdit, ClipId, ClipInfo, TimeDisplay, TimeFormat, TimeView, TimelineModel, TrackId, TrackInfo, TrackKind};
use photocraft_doc::{Layer, LayerId};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The tracks' view state: zoom and scroll, and the bar drag under way (drawn where it would land).
#[derive(Clone, Debug, Default)]
pub(crate) struct LayerTracksUi {
    view: TimeView,
    preview: Option<ClipEdit>,
    /// The timeline (frames, fps bits) the view was last fitted to: a new length or rate fits the
    /// whole shot in the panel again; until then the user's own zoom and scroll stay.
    fitted: Option<(usize, u32)>,
}

/// Room left for the track headers when the view fits the shot, in points.
const HEADER_ROOM: f32 = 240.0;

/// One layer's row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerRow {
    pub id: LayerId,
    pub name: String,
    pub depth: u8,
    /// The frames the bar covers: the layer's range, or every frame.
    pub start: usize,
    pub end: usize,
    /// The layer has a range of its own.
    pub ranged: bool,
    pub visible: bool,
}

/// Every layer, top to bottom, a group's children under it.
pub(crate) fn rows(layers: &[Layer], duration: usize) -> Vec<LayerRow> {
    fn walk(layers: &[Layer], depth: u8, duration: usize, out: &mut Vec<LayerRow>) {
        for l in layers.iter().rev() {
            let (start, end) = l.span.map_or((0, duration), |sp| (sp.start, sp.end.min(duration.max(sp.start + 1))));
            out.push(LayerRow { id: l.id, name: l.name.clone(), depth, start, end, ranged: l.span.is_some(), visible: l.visible });
            if let Some(ch) = l.children() {
                walk(ch, depth.saturating_add(1), duration, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(layers, 0, duration, &mut out);
    out
}

/// Whole frames in `t` at `fps`, rounded.
fn frames_in(t: Tick, fps: f64) -> i64 {
    (t.seconds() * fps).round() as i64
}

/// The frames `row` would cover after `edit`: a move keeps the length inside the timeline, a trim
/// keeps at least one frame. `None` when the edit doesn't change this row.
pub(crate) fn edited(row: &LayerRow, edit: &ClipEdit, fps: f64, duration: usize) -> Option<(usize, usize)> {
    let id = ClipId(row.id.0);
    let (start, end, duration) = (row.start as i64, row.end as i64, duration.max(1) as i64);
    let (s, e) = match edit {
        ClipEdit::Move { clips, delta, .. } if clips.contains(&id) => {
            let len = end - start;
            let s = (start + frames_in(*delta, fps)).clamp(0, (duration - len).max(0));
            (s, s + len)
        }
        ClipEdit::Trim { clip, edge: ClipEdge::In, delta, .. } if *clip == id => ((start + frames_in(*delta, fps)).clamp(0, end - 1), end),
        ClipEdit::Trim { clip, edge: ClipEdge::Out, delta, .. } if *clip == id => (start, (end + frames_in(*delta, fps)).clamp(start + 1, duration)),
        _ => return None,
    };
    ((s, e) != (start, end)).then_some((s as usize, e as usize))
}

/// The commands a committed bar edit runs: one `layer.frameRange.set` per bar it changed.
pub(crate) fn commands_for(rows: &[LayerRow], edit: &ClipEdit, fps: f64, duration: usize) -> Vec<(&'static str, Value)> {
    rows.iter()
        .filter_map(|r| edited(r, edit, fps, duration).map(|(start, end)| ("layer.frameRange.set", json!({"layer": r.id.0, "start": start, "end": end}))))
        .collect()
}

/// The document as the timeline widget reads it.
struct Tracks<'a> {
    rows: &'a [LayerRow],
    rate: FrameRate,
    duration: usize,
    current: usize,
    view: &'a mut TimeView,
    selection: Vec<ClipId>,
}

impl TimelineModel for Tracks<'_> {
    fn duration(&self) -> Tick {
        self.rate.tick_of(self.duration as i64)
    }

    fn format(&self) -> TimeFormat {
        // Frames counted from 1, as the transport row counts them.
        let mut format = TimeFormat::timecode(self.rate).display(TimeDisplay::Frames);
        format.frame_offset = 1;
        format
    }

    fn playhead(&self) -> Tick {
        self.rate.tick_of(self.current as i64)
    }

    fn view(&mut self) -> &mut TimeView {
        self.view
    }

    fn tracks(&self) -> Vec<TrackInfo<'_>> {
        self.rows
            .iter()
            .map(|r| TrackInfo {
                id: TrackId(r.id.0),
                kind: TrackKind::Video,
                name: &r.name,
                height: None,
                depth: r.depth,
                label: None,
                locked: None,
                targeted: None,
                sync_lock: None,
                enabled: Some(r.visible),
                muted: None,
                solo: None,
            })
            .collect()
    }

    fn clips(&self, track: TrackId, visible: TimeRange) -> Vec<ClipInfo<'_>> {
        self.rows
            .iter()
            .filter(|r| r.id.0 == track.0)
            .map(|r| ClipInfo {
                id: ClipId(r.id.0),
                track,
                range: TimeRange::from_bounds(self.rate.tick_of(r.start as i64), self.rate.tick_of(r.end as i64)),
                name: &r.name,
                label: None,
                // A layer on every frame draws dimmed until it gets a range of its own.
                enabled: r.ranged,
                source: None,
            })
            .filter(|c| c.range.overlaps(&visible))
            .collect()
    }

    fn selection(&self) -> &[ClipId] {
        &self.selection
    }
}

/// Draws the layer tracks under the transport row and runs what the user did with them.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, fps: f32, duration: usize, current: usize) {
    let Some(doc) = app.session.active() else { return };
    let base = rows(&doc.doc.layers, duration);
    let mut drawn = base.clone();
    let active = doc.active_layer;
    let rate = FrameRate::from_f64(f64::from(fps));
    let fps = f64::from(fps);
    // A bar being dragged is drawn where it would land.
    if let Some(edit) = &app.ui.timeline.layers.preview {
        for r in &mut drawn {
            if let Some((s, e)) = edited(r, edit, fps, duration) {
                (r.start, r.end) = (s, e);
            }
        }
    }
    let state = &mut app.ui.timeline.layers;
    let key = (duration, (fps as f32).to_bits());
    if state.fitted != Some(key) {
        let seconds = duration.max(1) as f64 / fps.max(1e-3);
        let width = f64::from((ui.available_width() - HEADER_ROOM).max(120.0));
        state.view = TimeView::new(Tick::ZERO, width / seconds);
        state.fitted = Some(key);
    }
    let mut model = Tracks { rows: &drawn, rate, duration, current, view: &mut state.view, selection: active.map(|id| vec![ClipId(id.0)]).unwrap_or_default() };
    let out = Timeline::new(tl!("Layers"), &mut model).show(ui);
    let mut run: Vec<(&'static str, Value)> = Vec::new();
    for edit in out.edits {
        match edit {
            TimelineEdit::Clip(EditEvent::Update { value, .. }) => app.ui.timeline.layers.preview = Some(value),
            TimelineEdit::Clip(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
                app.ui.timeline.layers.preview = None;
                run.extend(commands_for(&base, &after, fps, duration));
            }
            TimelineEdit::Clip(_) => app.ui.timeline.layers.preview = None,
            TimelineEdit::Playhead(EditEvent::Update { value, .. } | EditEvent::Commit { after: value, .. } | EditEvent::Set { after: value, .. }) => {
                let frame = frames_in(value, fps).clamp(0, duration.saturating_sub(1) as i64);
                run.push(("timeline.setFrame", json!({ "frame": frame })));
            }
            TimelineEdit::Switch { track, switch: TrackSwitch::Enabled, value, .. } => {
                run.push(("layer.setProps", json!({ "layer": track.0, "visible": value })));
            }
            TimelineEdit::Select { change: SelectionChange::Set(keys), .. } => {
                if let Some(k) = keys.last() {
                    run.push(("layer.select", json!({ "layer": k.0 })));
                }
            }
            TimelineEdit::Select { change: SelectionChange::Toggle(k), .. } => run.push(("layer.select", json!({ "layer": k.0, "mode": "add" }))),
            _ => {}
        }
    }
    for (cmd, p) in run {
        if let Err(e) = app.run(cmd, p) {
            crate::notices::error(app, e);
        }
    }
}

#[cfg(test)]
mod tests;
