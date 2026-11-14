//! Capability policy shared by existing Character buttons and Type › OpenType menus.

use super::defaults::target_doc;
use super::{PhotocraftApp, text_layer};
use photocraft_doc::text::CharStyle;
use photocraft_text::FaceRequest;

/// Whether every affected face advertises the required GSUB tags, or inspection must wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Availability {
    Available,
    Unsupported,
    Unknown,
    Busy,
}

impl Availability {
    /// Missing/unreadable fonts cannot establish support. Saved feature flags remain untouched.
    pub(crate) fn enabled(self) -> bool {
        self == Self::Available
    }

    /// Explain a disabled control, distinguishing missing font information from a temporary lock.
    pub(super) fn explanation(self) -> &'static str {
        match self {
            Self::Available => "",
            Self::Unsupported => "Not supported by all selected font faces",
            Self::Unknown => "Feature support is unknown: font information is unavailable",
            Self::Busy => "Font engine is busy",
        }
    }
}

/// GSUB tags associated with an existing command. Numerals combinations require both tags;
/// Default resets saved numeral flags and does not require a substitution in the font.
fn command_tags(id: &str) -> Option<Vec<&'static str>> {
    use photocraft_engine::type_extra_cmds::{NumeralsStyle, STYLISTIC_SET_TAGS};
    if let Some((tag, _, _)) = super::panel::OPENTYPE_BUTTONS.iter().find(|(_, command, _)| *command == id) {
        return Some(vec![*tag]);
    }
    match id {
        "type.openType.oldstyle" => return Some(vec!["onum"]),
        "type.openType.ornaments" => return Some(vec!["ornm"]),
        _ => {}
    }
    if let Some(style) = id.strip_prefix("type.openType.numerals.").and_then(NumeralsStyle::from_id) {
        return Some(style.tags().to_vec());
    }
    let set = id.strip_prefix("type.openType.stylisticSets.set")?.parse::<usize>().ok()?;
    Some(vec![*STYLISTIC_SET_TAGS.get(set.checked_sub(1)?)?])
}

#[cfg(test)]
thread_local! {
    /// Snapshot work, independent of renderer timing or allocation instrumentation.
    pub(super) static TARGET_SNAPSHOTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Raw run visits prove target admission can stop before traversing an oversized tail.
    pub(super) static RUN_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Borrow normalized run identities without copying CharStyle payloads. Match char_runs:
/// zero runs disappear, UTF-8 ends snap forward, the final run stretches to the text end,
/// and empty text keeps the first typing style. The range uses character offsets, as commands do;
/// once an overlapping run reaches its end, later runs cannot affect the target.
pub(super) fn layer_requests(t: &super::TextLayer, range: Option<[usize; 2]>) -> impl Iterator<Item = FaceRequest<'_>> {
    let total = t.text.len();
    let selected = range.map(|[a, b]| super::byte_range(&t.text, a, b));
    let base = FaceRequest { family: &t.font_family, style: "", postscript: None, weight: 400, italic: false };
    let mut at = 0usize;
    let mut source = t.runs.iter();
    let mut normalized = std::iter::from_fn(move || {
        for run in source.by_ref() {
            #[cfg(test)]
            RUN_VISITS.with(|count| count.set(count.get().saturating_add(1)));
            if at >= total {
                break;
            }
            let mut end = at.saturating_add(run.len).min(total);
            while !t.text.is_char_boundary(end) {
                end = end.saturating_add(1);
            }
            if end > at {
                let start = at;
                at = end;
                return Some((start, end, FaceRequest::from(&run.style)));
            }
        }
        None
    })
    .peekable();
    let mut first = true;
    let mut finished = false;
    std::iter::from_fn(move || {
        while !finished {
            if total == 0 {
                if !first {
                    return None;
                }
                first = false;
                return Some(t.runs.first().map_or(base, |run| FaceRequest::from(&run.style)));
            }
            let (start, mut end, request) = match normalized.next() {
                Some(run) => run,
                None if first => (0, total, base),
                None => return None,
            };
            first = false;
            if selected.is_none_or(|(_, b)| end < b) && normalized.peek().is_none() {
                end = total;
            }
            finished = selected.is_some_and(|(_, b)| end >= b);
            if selected.is_none_or(|(a, b)| start < b && end > a) {
                return Some(request);
            }
        }
        None
    })
}

/// One snapshot for a complete button row. Capabilities describe all affected faces; an unknown
/// face makes support unknown without borrowing another face's metadata.
pub(crate) enum TargetFeatures {
    Ready(Option<Vec<std::sync::Arc<photocraft_text::FaceFeatures>>>),
    Busy,
}

impl TargetFeatures {
    /// A menu built while shaping holds the engine must be refreshed after that hold.
    pub(crate) fn busy(&self) -> bool {
        matches!(self, Self::Busy)
    }

    /// Look up any existing OpenType UI command against this target snapshot. Default Numerals
    /// resets stored tags and remains available even when a font cannot be inspected. A busy
    /// engine disables other features temporarily so a live click can ask the user to retry.
    pub(crate) fn availability(&self, id: &str) -> Option<Availability> {
        let tags = command_tags(id)?;
        if tags.is_empty() {
            return Some(Availability::Available);
        }
        let faces = match self {
            Self::Ready(Some(faces)) => faces,
            Self::Ready(None) => return Some(Availability::Unknown),
            Self::Busy => return Some(Availability::Busy),
        };
        let supported = |face: &photocraft_text::FaceFeatures| {
            // Layout's Standard Ligatures switch controls both default ligature tags.
            if tags.as_slice() == ["liga"] { face.contains("liga") || face.contains("clig") } else { tags.iter().all(|tag| face.contains(tag)) }
        };
        Some(if faces.iter().all(|face| supported(face)) { Availability::Available } else { Availability::Unsupported })
    }
}

/// Inspect the formatting target once for the whole menu. Menu commands have no default
/// editing target, so opening a menu never resolves the saved Character defaults.
pub(crate) fn target_features(app: &PhotocraftApp) -> TargetFeatures {
    target_features_with_defaults(app, None)
}

/// Inspect all target faces in one pass. Character supplies its already-resolved style for
/// default editing. A menu without a type target has no font metadata to inspect.
pub(super) fn target_features_with_defaults(app: &PhotocraftApp, defaults: Option<&CharStyle>) -> TargetFeatures {
    #[cfg(test)]
    TARGET_SNAPSHOTS.with(|count| count.set(count.get().saturating_add(1)));
    if let Some(edit) = &app.ui.text_edit {
        let Some(text) = target_doc(app).and_then(|doc| text_layer(&doc.doc, photocraft_doc::LayerId(edit.layer))) else {
            return TargetFeatures::Ready(None);
        };
        let range = (edit.anchor != edit.caret).then_some([edit.anchor, edit.caret]);
        let mut requests = layer_requests(text, range).peekable();
        if requests.peek().is_none() {
            return TargetFeatures::Ready(None);
        }
        inspect_requests(app, requests)
    } else {
        let mut requests = target_doc(app)
            .into_iter()
            .flat_map(|doc| doc.selected_layers().into_iter().filter_map(move |id| text_layer(&doc.doc, id)).flat_map(|text| layer_requests(text, None)))
            .peekable();
        if requests.peek().is_some() {
            inspect_requests(app, requests)
        } else {
            defaults.map_or(TargetFeatures::Ready(None), |style| inspect_requests(app, [FaceRequest::from(style)]))
        }
    }
}

/// Optional metadata never waits on a worker holding the shared engine. Ordinary unit
/// fixtures model a settled catalog despite other tests sharing the same process mutex.
/// The contention fixture runs a real worker and exercises the nonblocking path.
fn inspect_requests<'a>(app: &PhotocraftApp, requests: impl IntoIterator<Item = FaceRequest<'a>>) -> TargetFeatures {
    let Some(engine) = photocraft_text::try_shared() else { return TargetFeatures::Ready(None) };
    let mut engine = if cfg!(test) && !app.session.has_jobs() {
        engine.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    } else {
        match engine.try_lock() {
            Ok(engine) => engine,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return TargetFeatures::Busy,
        }
    };
    TargetFeatures::Ready(engine.fonts.face_features_for_requests(requests))
}

/// Availability for a live menu activation. Queued native clicks can arrive after control
/// switched documents or layers but before the shell closes the old edit. Reject that stale
/// edit so an old face never authorizes a command on a different layer with the same id.
pub(crate) fn availability(app: &PhotocraftApp, id: &str) -> Option<Availability> {
    let tags = command_tags(id)?;
    if let Some(edit) = &app.ui.text_edit
        && (super::editing::edit_home(app, edit).is_none_or(|home| Some(home) != app.session.active_index())
            || app.session.active().and_then(|doc| doc.active_layer) != Some(photocraft_doc::LayerId(edit.layer)))
    {
        return Some(Availability::Unknown);
    }
    if tags.is_empty() {
        return Some(Availability::Available);
    }
    target_features(app).availability(id)
}
