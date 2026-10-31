//! `gmic.*` commands: G'MIC filters through a user-installed `gmic` program (`photocraft-gmic`).
//!
//! G'MIC is found once per process (Preferences › Plug-ins › G'MIC executable, else
//! `$PHOTOCRAFT_GMIC`, else `PATH`) and its filter catalogue is loaded on first use and cached on
//! disk per G'MIC version. `gmic.filter` runs a catalogue filter with validated parameters;
//! `gmic.run` runs a raw G'MIC pipeline. Both work on the active pixel layer or a targeted
//! channel, respect the selection and lock transparency, and are one undo step (a background job
//! when started as one, cancellable).
//!
//! On a smart object only `gmic.filter` is recorded (re-run on re-render while G'MIC is
//! installed): a document can't carry a raw pipeline, because G'MIC script can run programs and
//! write files, and opening a file must never do that. For the same reason, catalogue filters
//! with file or folder parameters don't re-run from smart filters.

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_doc::{LayerContent, SmartFilter};
use photocraft_gmic::{Catalog, Filter, Gmic, ParamKind, RunOptions};
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The command ids.
pub const RUN: &str = "gmic.run";
pub const FILTER: &str = "gmic.filter";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn err(e: photocraft_gmic::Error) -> EngineError {
    match e {
        photocraft_gmic::Error::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Where the filter catalogue stands.
#[derive(Clone, Debug)]
pub enum CatalogState {
    /// Not requested yet.
    Idle,
    /// Being loaded or rebuilt in the background.
    Loading,
    Loaded(Arc<Catalog>),
    /// The last load failed (the message says why).
    Failed(String),
}

/// Process-wide G'MIC state: where it is, what was found there, the catalogue.
struct State {
    /// The preference's path (empty: look on PATH).
    configured: String,
    /// The last probe of `configured`.
    found: Option<std::result::Result<Gmic, String>>,
    catalog: CatalogState,
    /// Bumped when `configured` changes, so a load started for the old program is dropped.
    generation: u64,
    /// A background probe ([`detect_in_background`]) is running.
    probing: bool,
    /// Bumped whenever `found` or `catalog` change (menus rebuild).
    revision: u64,
}

static STATE: Mutex<State> = Mutex::new(State { configured: String::new(), found: None, catalog: CatalogState::Idle, generation: 0, probing: false, revision: 0 });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where the catalogue cache lives instead of the platform cache folder (tests, portable setups).
static CACHE_DIR: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

/// Overrides the catalogue cache folder (`None`: the platform's cache folder).
pub fn set_cache_dir(dir: Option<std::path::PathBuf>) {
    *CACHE_DIR.lock().unwrap_or_else(PoisonError::into_inner) = dir;
}

/// Changes whenever G'MIC is found again or the catalogue state changes.
pub fn revision() -> u64 {
    state().revision
}

/// Picks up Preferences › Plug-ins › G'MIC executable. Called whenever preferences are applied.
pub(crate) fn sync_prefs(s: &Session) {
    let want = s.prefs().plug_ins.gmic_path.trim().to_string();
    let mut st = state();
    if st.configured != want {
        st.configured = want;
        st.found = None;
        st.catalog = CatalogState::Idle;
        st.generation += 1;
        st.revision += 1;
    }
}

/// The G'MIC program, probing for it (once, or again with `refresh`). The probe runs without
/// holding the state lock.
pub fn gmic(refresh: bool) -> std::result::Result<Gmic, String> {
    let (configured, generation) = {
        let st = state();
        if !refresh && let Some(f) = &st.found {
            return f.clone();
        }
        (st.configured.clone(), st.generation)
    };
    let found = photocraft_gmic::runner::find(Some(&configured).filter(|c| !c.is_empty()).map(String::as_str)).map_err(|e| e.to_string());
    let mut st = state();
    if st.generation == generation {
        let same = st.found.as_ref().and_then(|f| f.as_ref().ok()) == found.as_ref().ok();
        if !same && !matches!(st.catalog, CatalogState::Loading) {
            st.catalog = CatalogState::Idle;
        }
        st.found = Some(found.clone());
        st.revision += 1;
    }
    found
}

/// The last probe's outcome, without probing (`None`: not probed yet).
pub fn found() -> Option<std::result::Result<Gmic, String>> {
    state().found.clone()
}

/// What a frontend calls whenever it shows G'MIC (Filter › G'MIC): finds G'MIC on a worker
/// thread if nobody has looked yet, then starts loading the catalogue once it is found. Never
/// blocks; watch [`revision`] for the outcome.
pub fn detect_in_background() {
    let (probe, load) = {
        let mut st = state();
        match &st.found {
            None if !st.probing => {
                st.probing = true;
                (true, false)
            }
            Some(Ok(_)) => (false, matches!(st.catalog, CatalogState::Idle)),
            _ => (false, false),
        }
    };
    if load {
        request_catalog(false);
    }
    if !probe {
        return;
    }
    let work = || {
        let ok = gmic(false).is_ok();
        state().probing = false;
        if ok {
            request_catalog(false);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    if std::thread::Builder::new().name("photocraft-gmic-probe".into()).spawn(work).is_err() {
        state().probing = false;
    }
    #[cfg(target_arch = "wasm32")]
    work();
}

/// The catalogue state (never starts G'MIC).
pub fn catalog_state() -> CatalogState {
    state().catalog.clone()
}

/// The catalogue if it is loaded (never starts G'MIC).
pub fn loaded_catalog() -> Option<Arc<Catalog>> {
    match &state().catalog {
        CatalogState::Loaded(c) => Some(c.clone()),
        _ => None,
    }
}

/// Loads the catalogue (from the disk cache, else rebuilt by G'MIC: about a minute the first
/// time) and stores the outcome, unless the preferences changed meanwhile.
fn load(refresh: bool, generation: u64) {
    let outcome = gmic(false).and_then(|g| {
        let cache = CACHE_DIR.lock().unwrap_or_else(PoisonError::into_inner).clone().or_else(photocraft_gmic::runner::default_cache_dir);
        g.load_catalog(cache.as_deref(), refresh, &RunOptions::default()).map_err(|e| e.to_string())
    });
    let mut st = state();
    if st.generation == generation {
        st.catalog = match outcome {
            Ok(c) => CatalogState::Loaded(Arc::new(c)),
            Err(e) => CatalogState::Failed(e),
        };
        st.revision += 1;
    }
}

/// Starts loading the catalogue in the background unless it is loaded (and not `refresh`) or
/// already loading, and returns the state. On the web it fails at once (no G'MIC there).
pub fn request_catalog(refresh: bool) -> CatalogState {
    let generation = {
        let mut st = state();
        match &st.catalog {
            CatalogState::Loading => return CatalogState::Loading,
            CatalogState::Loaded(c) if !refresh => return CatalogState::Loaded(c.clone()),
            _ => {}
        }
        st.catalog = CatalogState::Loading;
        st.revision += 1;
        st.generation
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let spawned = std::thread::Builder::new().name("photocraft-gmic-catalog".into()).spawn(move || load(refresh, generation));
        if let Err(e) = spawned {
            let mut st = state();
            st.catalog = CatalogState::Failed(format!("couldn't start loading the G'MIC filter list: {e}"));
            st.revision += 1;
        }
    }
    #[cfg(target_arch = "wasm32")]
    load(refresh, generation);
    catalog_state()
}

/// The catalogue, waiting (up to `timeout`) for a background load to finish.
pub fn catalog_wait(refresh: bool, timeout: std::time::Duration) -> std::result::Result<Arc<Catalog>, String> {
    let deadline = std::time::Instant::now() + timeout;
    let mut state = request_catalog(refresh);
    loop {
        match state {
            CatalogState::Loaded(c) => return Ok(c),
            CatalogState::Failed(e) => return Err(e),
            CatalogState::Idle => state = request_catalog(false),
            CatalogState::Loading if std::time::Instant::now() >= deadline => {
                return Err("the G'MIC filter list is still loading (the first time takes about a minute); try again shortly".into());
            }
            CatalogState::Loading => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                state = catalog_state();
            }
        }
    }
}

/// `{found, path, version, error, catalog, filters}`: what the About Plug-ins window and
/// `gmic.status` show.
pub fn status(refresh: bool) -> Value {
    let found = gmic(refresh);
    let (catalog, filters, catalog_error) = match catalog_state() {
        CatalogState::Idle => ("idle", None, None),
        CatalogState::Loading => ("loading", None, None),
        CatalogState::Loaded(c) => ("loaded", Some(c.filters.len()), None),
        CatalogState::Failed(e) => ("failed", None, Some(e)),
    };
    let available = !cfg!(target_arch = "wasm32");
    match found {
        Ok(g) => json!({"found": true, "path": g.path().display().to_string(), "version": g.version(), "catalog": catalog, "filters": filters, "catalogError": catalog_error, "available": available}),
        Err(e) => json!({"found": false, "error": e, "catalog": catalog, "filters": filters, "catalogError": catalog_error, "available": available}),
    }
}

/// The filter's own parameters from command params: `params` if given, else the top-level keys
/// minus the ones the command itself uses.
fn filter_params(p: &Value) -> Value {
    if let Some(v) = p.get("params") {
        return v.clone();
    }
    match p {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "id" | "layer" | "target" | "channel") && !k.starts_with("__"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        _ => Value::Object(Map::new()),
    }
}

/// JSON description of a catalogue filter.
pub fn describe(f: &Filter) -> Value {
    json!({"id": f.id, "name": f.name, "category": f.category, "command": f.command, "params": f.notation(), "defaults": f.defaults()})
}

fn filter_by_id(cmd: &str, p: &Value) -> Result<Filter> {
    let id = p.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "missing `id` (see gmic.list)"))?;
    let cat = match request_catalog(false) {
        CatalogState::Loaded(c) => c,
        CatalogState::Failed(e) => return Err(EngineError::Other(e)),
        _ => return Err(EngineError::Other("the G'MIC filter list is still loading (the first time takes about a minute); try again shortly".into())),
    };
    cat.get(id).cloned().ok_or_else(|| bad(cmd, format!("no G'MIC filter {id:?} (see gmic.list)")))
}

/// Runs `pipeline` on the target layer or channel as one undo step named `label`; on a smart
/// object, records `smart` (or refuses when there is none).
fn run_pipeline(s: &mut Session, cmd: &'static str, p: &Value, label: String, pipeline: String, smart: Option<SmartFilter>) -> Result<Value> {
    let layer = crate::commands::layer_param(s, p)?;
    let g = gmic(false).map_err(EngineError::Other)?;
    let p = p.clone();
    let layer_id = layer.0;
    let msg = label.clone();
    crate::jobs::edit_job(
        s,
        &label,
        move |doc, _, ctx| {
            ctx.progress(0.0, &msg);
            let cancelled = || ctx.cancelled();
            let opts = RunOptions { cancelled: Some(&cancelled), ..Default::default() };
            let selection = doc.selection.clone();
            let canvas = doc.bounds();
            if let Some(surf) = crate::channel_cmds::channel_surface_for_filter(doc, Some(layer), &p)? {
                *surf = photocraft_gmic::apply(&g, surf, canvas, selection.as_ref(), &pipeline, &opts).map_err(err)?;
                return Ok(());
            }
            let locks = doc.effective_locks(layer);
            let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
            if locks.pixels || locks.all {
                return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
            }
            match &mut l.content {
                LayerContent::Raster(surf) => {
                    let before = locks.transparency.then(|| surf.clone());
                    *surf = photocraft_gmic::apply(&g, surf, canvas, selection.as_ref(), &pipeline, &opts).map_err(err)?;
                    if let Some(before) = &before {
                        crate::filters::keep_alpha(before, surf, &|| ctx.cancelled())?;
                    }
                    Ok(())
                }
                LayerContent::Smart(_) => {
                    let sf = smart.ok_or_else(|| {
                        EngineError::Other("a raw G'MIC command can't be a smart filter; choose the filter from Filter › G'MIC, or rasterize the layer".into())
                    })?;
                    crate::smart_cmds::add_smart_filter(doc, layer, sf, selection.as_ref())
                }
                other => Err(EngineError::Other(format!("G'MIC filters need a pixel layer (active layer is {} {} layer)", other.article(), other.kind_name()))),
            }
        },
        move |()| json!({"layer": layer_id, "command": cmd}),
    )
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let pipeline = p.get("command").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty());
    let pipeline = pipeline.ok_or_else(|| bad(RUN, "missing `command`: a G'MIC pipeline, e.g. \"fx_smooth_anisotropic 60\""))?;
    if pipeline.len() > 1 << 20 || pipeline.contains('\0') {
        return Err(bad(RUN, "`command` is too long or contains a NUL character"));
    }
    run_pipeline(s, RUN, p, "G'MIC".into(), pipeline.to_string(), None)
}

fn filter(s: &mut Session, p: &Value) -> Result<Value> {
    let f = filter_by_id(FILTER, p)?;
    let values = filter_params(p);
    // Bad parameters fail before anything is touched.
    let pipeline = f.command_line(&values).map_err(|e| bad(FILTER, e.to_string()))?;
    let smart = SmartFilter {
        command: FILTER.into(),
        params: json!({"id": f.id, "params": values}),
        blend: photocraft_color::BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    };
    run_pipeline(s, FILTER, p, f.name.clone(), pipeline, Some(smart))
}

/// Smart-filter re-render of a recorded `gmic.filter` (see `filters::apply_filter_to_surface`).
/// `None` when `command` isn't one, G'MIC or the filter is missing, the filter takes file
/// parameters, or the run fails (the smart filter is then skipped). Never starts a catalogue
/// rebuild: it uses the catalogue already loaded.
pub(crate) fn apply_to_surface(command: &str, params: &Value, surf: &photocraft_raster::Surface, selection: Option<&photocraft_raster::Surface>, canvas: photocraft_geom::Rect) -> Option<photocraft_raster::Surface> {
    if command != FILTER {
        return None;
    }
    let cat = loaded_catalog()?;
    let f = cat.get(params.get("id")?.as_str()?)?;
    if f.params.iter().any(|p| matches!(p.kind, ParamKind::Path(_))) {
        return None;
    }
    let pipeline = f.command_line(params.get("params").unwrap_or(&Value::Null)).ok()?;
    let g = gmic(false).ok()?;
    photocraft_gmic::apply(&g, surf, canvas, selection, &pipeline, &RunOptions::default()).ok()
}

fn list(_: &mut Session, p: &Value) -> Result<Value> {
    let refresh = p.get("refresh").and_then(Value::as_bool).unwrap_or(false);
    let category = p.get("category").and_then(Value::as_str);
    let cat = if p.get("wait").and_then(Value::as_bool).unwrap_or(false) {
        catalog_wait(refresh, std::time::Duration::from_secs(900)).map_err(EngineError::Other)?
    } else {
        match request_catalog(refresh) {
            CatalogState::Loaded(c) => c,
            CatalogState::Failed(e) => return Err(EngineError::Other(e)),
            _ => return Ok(json!({"loading": true, "message": "loading the G'MIC filter list (the first time takes about a minute); call again, or pass \"wait\": true"})),
        }
    };
    let filters: Vec<Value> = cat.filters.iter().filter(|f| category.is_none_or(|c| f.category == c)).map(describe).collect();
    Ok(json!({"gmicVersion": cat.gmic_version, "categories": cat.categories(), "filters": filters}))
}

/// The G'MIC command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "gmic.status",
            label: "G'MIC Status",
            menu: &[],
            shortcut: None,
            params: r##"{"refresh":bool=false} → {found, path, version, error, filters}"##,
            enabled: always,
            run: |_, p| Ok(status(p.get("refresh").and_then(Value::as_bool).unwrap_or(false))),
            journal: false,
        },
        CommandSpec {
            id: "gmic.list",
            label: "List G'MIC Filters",
            menu: &[],
            shortcut: None,
            params: r##"{"refresh":bool=false,"wait":bool=false,"category":text} (loads in the background: {"loading": true} until ready unless wait; refresh rebuilds the list, about a minute)"##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: RUN,
            label: "G'MIC Command…",
            menu: &["Filter", "G'MIC"],
            shortcut: None,
            params: r##"{"command":text} (a G'MIC pipeline run on the active layer, e.g. "fx_smooth_anisotropic 60")"##,
            enabled: crate::filters::has_filterable_layer,
            run,
            journal: true,
        },
        CommandSpec {
            id: FILTER,
            label: "Run G'MIC Filter",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text,"params":json} (a filter id from gmic.list; its parameters under "params" or at the top level)"##,
            enabled: crate::filters::has_filterable_layer,
            run: filter,
            journal: true,
        },
    ]
}

#[cfg(test)]
#[path = "gmic_cmds/tests.rs"]
mod tests;
