//! Browser copies back the existing recent-file menu. A committed Blob, never a host path,
//! is the authority for reopening. The serial queue preserves the order of accepted imports.
use photocraft_ui_egui::{PhotocraftApp, RecentFileRequest};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::Arc};

const PREFIX: &str = "browser-recent/";

#[derive(Default)]
struct Pending {
    queue: VecDeque<(u64, RecentFileRequest)>,
    generation: u64,
}

enum Reply {
    Retained(String),
    Opened(String, Vec<u8>),
}

#[derive(Default)]
pub struct Recent {
    pending: Rc<RefCell<Pending>>,
    result: Rc<RefCell<Option<(u64, Result<Reply, String>)>>>,
    busy: bool,
}

impl Recent {
    pub fn service(&self) -> photocraft_ui_egui::RecentFilesFn {
        let pending = self.pending.clone();
        Box::new(move |request| {
            let mut pending = pending.borrow_mut();
            // Clearing/removing history must win against an already-running retention.
            if matches!(request, RecentFileRequest::Invalidate) {
                pending.generation = pending.generation.wrapping_add(1);
                pending.queue.clear();
            } else {
                let generation = pending.generation;
                pending.queue.push_back((generation, request));
            }
            Ok(())
        })
    }

    pub fn tick(&mut self, app: &mut PhotocraftApp, ctx: &egui::Context) {
        let result = self.result.borrow_mut().take();
        if let Some((generation, result)) = result {
            self.busy = false;
            // Preferences are persisted by app.logic; schedule its next pass even when the
            // user stops interacting immediately after the asynchronous copy commits.
            ctx.request_repaint();
            match result {
                Ok(Reply::Retained(key)) if generation == self.pending.borrow().generation => app.push_recent(&key),
                Ok(Reply::Retained(_)) => {}
                Ok(Reply::Opened(name, bytes)) if generation == self.pending.borrow().generation => {
                    if let Err(error) = app.open_bytes(&name, &bytes) {
                        report(app, error);
                    }
                }
                Ok(Reply::Opened(_, _)) => {}
                Err(error) if generation == self.pending.borrow().generation => report(app, error),
                Err(_) => {}
            }
        }
        if self.busy {
            return;
        }
        let next = self.pending.borrow_mut().queue.pop_front();
        let Some((generation, task)) = next else { return };
        self.busy = true;
        let result = self.result.clone();
        let wake = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            *result.borrow_mut() = Some((generation, perform(task).await));
            wake.request_repaint();
        });
    }
}

fn report(app: &mut PhotocraftApp, error: String) {
    let message = format!("Recent browser copy unavailable: {error}");
    app.ui.status = message.clone();
    app.ui.status_error = true;
    photocraft_ui_egui::notices::post(app, message, Vec::new(), true, None);
}

async fn perform(task: RecentFileRequest) -> Result<Reply, String> {
    let db = crate::indexed_presets::open("photocraft.recent", js_sys::Date::now() + 10_000.0).await?;
    let result = match task {
        RecentFileRequest::Remember(name, bytes) => {
            // Names and bytes both participate: identical filenames cannot overwrite each other,
            // and reopening the same retained version does not manufacture a duplicate.
            let mut hash = Sha256::new();
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update(bytes.as_slice());
            let key = format!("{PREFIX}{:x}/{name}", hash.finalize());
            let batch = [(key.clone(), Some(Arc::<[u8]>::from(bytes.as_slice())))].into_iter().collect();
            crate::indexed_presets::write(&db, &batch).await.map(|()| Reply::Retained(key))
        }
        RecentFileRequest::Open(key) => {
            let name = key
                .strip_prefix(PREFIX)
                .and_then(|tail| tail.split_once('/'))
                .filter(|(hash, name)| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !name.is_empty())
                .map(|(_, name)| name.to_string());
            match name {
                Some(name) => crate::indexed_presets::read_file(&db, &key).await.map(|bytes| Reply::Opened(name, bytes)),
                None => Err("This entry has no retained browser copy; use File → Open.".into()),
            }
        }
        RecentFileRequest::Invalidate => Err("invalid recent request".into()),
    };
    db.close();
    result
}
