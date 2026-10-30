use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn connected() -> Session {
    let mut session = Session::new();
    session.deviantart_submit = Some(Arc::new(|_| Ok(r#"{"itemid":42}"#.into())));
    session
}

#[test]
fn deviantart_is_an_engine_command_and_unconfigured_hosts_fail_explicitly() {
    assert!(!crate::shell_commands::SHELL_COMMANDS.iter().any(|row| row.0 == COMMAND));
    let mut session = Session::new();
    let error = session.execute(COMMAND, json!({})).unwrap_err().to_string();
    assert!(error.contains("unavailable"), "{error}");
    let spec = crate::commands::command_specs().iter().find(|spec| spec.id == COMMAND).unwrap();
    assert!(spec.params.contains("format") && spec.params.contains("30 MiB"));
    assert!((spec.enabled)(&session).is_err());
    session.deviantart_submit = connected().deviantart_submit;
    assert!((spec.enabled)(&session).is_ok());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn deviantart_dispatch_uploads_a_saved_file_and_keeps_secrets_out_of_the_journal() {
    let file = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
    std::fs::write(file.path(), b"encoded-image").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let copy = seen.clone();
    let mut session = connected();
    session.deviantart_submit = Some(Arc::new(move |payload| {
        copy.lock().unwrap().push((payload.bytes.clone(), payload.title.clone(), payload.tags.clone(), payload.comments.clone()));
        Ok(r#"{"itemid":42}"#.into())
    }));
    let result = session.execute(COMMAND, json!({"path":file.path(),"title":"Hill","tags":"Art, art, landscape!"})).unwrap();
    assert_eq!(result["body"], r#"{"itemid":42}"#);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].0, b"encoded-image");
    assert_eq!(seen[0].1, "Hill");
    assert_eq!(seen[0].2, ["Art", "landscape"]);
    assert!(!seen[0].3.contains(&file.path().display().to_string()));
    assert!(!session.journal.iter().any(|(id, _)| id == COMMAND));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn deviantart_png_and_psd_use_the_document_at_multiple_depths() {
    for depth in [8, 16] {
        for format in ["png", "psd"] {
            let mut session = connected();
            session.execute("file.new", json!({"width":2,"height":2,"depth":depth})).unwrap();
            if depth == 16 {
                session.authorize = Some(|_, _| Ok(()));
            }
            session.deviantart_submit = Some(Arc::new(move |payload| {
                assert_eq!(crate::jobs::authorization_installed(), depth == 16);
                assert!(if format == "png" { payload.bytes.starts_with(b"\x89PNG") } else { payload.bytes.starts_with(b"8BPS") });
                assert!(payload.name.ends_with(format));
                Ok(r#"{"itemid":42}"#.into())
            }));
            session.execute(COMMAND, json!({"format":format})).unwrap();
        }
    }
}

#[test]
fn deviantart_bad_params_fail_without_calling_the_transport() {
    let calls = Arc::new(AtomicUsize::new(0));
    let copy = calls.clone();
    let mut session = connected();
    session.deviantart_submit = Some(Arc::new(move |_| {
        copy.fetch_add(1, Ordering::SeqCst);
        Ok(r#"{"itemid":42}"#.into())
    }));
    for params in [
        Value::Null,
        json!([]),
        json!({}),
        json!({"path":9}),
        json!({"format":"raw"}),
        json!({"format":"png","path":"x"}),
        json!({"title":"a".repeat(201)}),
        json!({"tags":"a".repeat(4097)}),
        json!({"refreshToken":"must-not-be-a-param"}),
    ] {
        assert!(session.execute(COMMAND, params.clone()).is_err(), "{params}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn deviantart_oversize_file_is_rejected_before_reading_or_uploading() {
    let file = tempfile::NamedTempFile::new().unwrap();
    file.as_file().set_len(MAX_BYTES + 1).unwrap();
    let mut session = connected();
    session.deviantart_submit = Some(Arc::new(|_| panic!("oversize upload reached transport")));
    assert!(session.execute(COMMAND, json!({"path":file.path()})).unwrap_err().to_string().contains("30 MiB"));
}

#[test]
fn deviantart_transport_errors_are_reported_through_the_engine() {
    let mut session = connected();
    session.execute("file.new", json!({"width":1,"height":1})).unwrap();
    session.deviantart_submit = Some(Arc::new(|_| Err("network unavailable".into())));
    assert!(session.execute(COMMAND, json!({"format":"png"})).unwrap_err().to_string().contains("network unavailable"));
}

#[test]
fn deviantart_dispatch_honors_the_authorization_gate() {
    let mut session = connected();
    session.authorize = Some(|id, _| Err(crate::EngineError::Other(format!("denied {id}"))));
    let error = session.execute(COMMAND, json!({"format":"png"})).unwrap_err().to_string();
    assert!(error.contains("denied"), "{error}");
}

#[test]
fn deviantart_unconfirmed_responses_are_errors() {
    for body in ["not JSON", "{}", r#"{"itemid":0}"#, r#"{"error":"denied"}"#] {
        let mut session = connected();
        session.execute("file.new", json!({"width":1,"height":1})).unwrap();
        session.deviantart_submit = Some(Arc::new(move |_| Ok(body.into())));
        assert!(session.execute(COMMAND, json!({"format":"png"})).is_err());
    }
}

#[cfg(unix)]
#[test]
fn deviantart_named_pipe_is_rejected_without_waiting_for_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipe");
    let Ok(status) = std::process::Command::new("mkfifo").arg(&path).status() else {
        eprintln!("skipping: mkfifo unavailable");
        return;
    };
    assert!(status.success());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = connected().execute(COMMAND, json!({"path":path}));
        let _ = tx.send(result.is_err());
    });
    assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).expect("FIFO open must not block"));
}

#[cfg(unix)]
#[test]
fn deviantart_open_rejects_a_file_replaced_by_a_fifo_without_blocking() {
    use rustix::fs::{CWD, Mode, OFlags, mkfifoat, open};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.png");
    std::fs::write(&path, b"PNG").unwrap();
    assert!(std::fs::metadata(&path).unwrap().is_file());
    // Simulate a replacement after prepare's first metadata check.
    std::fs::remove_file(&path).unwrap();
    mkfifoat(CWD, &path, Mode::RUSR | Mode::WUSR).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let raced = path.clone();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(open_regular_file(&raced).is_err());
    });
    let result = rx.recv_timeout(std::time::Duration::from_secs(2));
    // Release an old blocking implementation before failing the regression test.
    let _release = open(&path, OFlags::WRONLY | OFlags::NONBLOCK, Mode::empty());
    worker.join().unwrap();
    assert!(result.expect("replacing a checked file with a FIFO must not block"));
}
