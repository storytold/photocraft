use super::*;
use std::sync::{Arc, Mutex};

fn token() -> String {
    format!("123456789:{}", "A".repeat(35))
}

#[test]
fn successful_send_result_shows_the_message_and_link() {
    assert_result_dialog_visible("Sent sketch.psd.", Some("https://t.me/example/4"), false);
}

#[test]
fn failed_send_result_shows_the_error_without_a_link() {
    assert_result_dialog_visible("The file could not be sent.", None, true);
}

#[test]
fn successful_send_result_without_a_link_still_shows_the_status() {
    assert_result_dialog_visible("Sent sketch.psd. Telegram did not return a link for this chat.", None, false);
}

#[test]
fn automation_can_dismiss_a_send_result_without_dispatching_a_command() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    show_result(&mut app, "Sent sketch.psd.".into(), None);
    let id = app.ui.dialogs.last().unwrap().id;
    assert_eq!(crate::dialogs::confirm(&mut app, id).unwrap(), Value::Null);
    assert!(app.ui.dialogs.is_empty());
}

fn assert_result_dialog_visible(message: &str, url: Option<&str>, error: bool) {
    use egui_kittest::kittest::Queryable;
    let size = egui::vec2(900.0, 520.0);
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.status = message.into();
    h.state_mut().ui.status_error = error;
    show_result(h.state_mut(), message.into(), url.map(str::to_string));
    h.run_steps(4);
    for label in [message, "OK"] {
        let rect = h.get_by_label(label).rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= size.y, "{label} is outside the window: {rect:?}");
    }
    if let Some(url) = url {
        h.get_by_label(url);
        h.get_by_label("Open the Telegram message");
    } else {
        assert!(h.query_by_label("Open the Telegram message").is_none());
    }
    assert!(h.query_by_label("Cancel").is_none());
    if let Some(directory) = std::env::var_os("PHOTOCRAFT_TELEGRAM_RESULT_SCREENSHOTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let name = if error {
            "error"
        } else if url.is_some() {
            "success-link"
        } else {
            "success-no-link"
        };
        h.render().unwrap().save(directory.join(format!("telegram-result-{name}.png"))).unwrap();
    }
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(h.state().ui.status, message);
    assert_eq!(h.state().ui.status_error, error);
}

#[test]
fn broken_settings_file_is_not_set_up() {
    let s = settings_from_json("not json");
    assert!(!s.ready());
    assert!(!s.send_on_save && !s.send_on_save_as && !s.send_on_export);
    let s = settings_from_json(r#"{"botToken":"  ","chatId":"12","sendAfterExport":true}"#);
    assert!(!s.ready());
    assert!(!s.send_on_save && s.send_on_save_as && s.send_on_export);
}

#[test]
fn token_and_chat_rules() {
    assert!(validate_token(&token()).is_ok());
    assert!(validate_token("123:short").is_err());
    assert!(validate_token(&format!("123456789:{}", "A".repeat(35))).is_ok());
    assert!(validate_token("123456789/secret").is_err());
    assert!(validate_chat("123456789").is_ok());
    assert!(validate_chat("-100123").is_ok());
    assert!(validate_chat("@my_channel").is_ok());
    assert!(validate_chat("@no").is_err());
    assert!(validate_chat("hello").is_err());
    assert!(validate_chat("-").is_err());
}

#[test]
fn utc_stamp_is_the_civil_date() {
    assert_eq!(utc_stamp(1_577_836_800), "2020-01-01 00:00 UTC");
}

#[test]
fn caption_names_the_file_without_the_token() {
    let text = caption("/tmp/photo_1.psd", "photo_1.psd", 2048, 1_577_836_800);
    assert!(text.contains("photo_1.psd"), "{text}");
    assert!(text.contains("PSD"), "{text}");
    assert!(text.contains("2020-01-01"), "{text}");
    assert!(text.contains("/tmp/photo_1.psd"), "{text}");
    assert!(!text.contains('#'), "{text}");
    let (_, body) = multipart("1", &text, "photo_1.psd", b"x", None);
    let raw = String::from_utf8_lossy(&body);
    assert!(raw.contains("name=\"parse_mode\""));
    assert!(raw.contains("Markdown"));
}

#[test]
fn multipart_carries_the_file_and_not_the_token() {
    let (ctype, body) = multipart("99", "caption", "photo.png", b"PNGDATA", None);
    assert!(ctype.starts_with("multipart/form-data; boundary="));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("name=\"chat_id\""));
    assert!(text.contains("99"));
    assert!(text.contains("filename=\"photo.png\""));
    assert!(text.contains("PNGDATA"));
    assert!(!text.contains("reply_parameters"));
    assert!(!text.contains("123456789"));
}

#[test]
fn album_sends_two_documents() {
    let (_, body) = media_group("99", "cap", &[("a.psd", b"PSD"), ("a.png", b"PNG")]);
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("sendMediaGroup") || text.contains("attach://f0"));
    assert!(text.contains("attach://f1"));
    assert!(text.contains("\"type\":\"document\""));
    assert!(!text.contains("reply_parameters"));
    assert_eq!(first_message_id(r#"{"ok":true,"result":[{"message_id":7}]}"#), Some(7));
}

#[test]
fn remember_sent_files_caps_at_ten_even_when_fresh_is_larger() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { save_telegram: Some(Box::new(|_| Ok(()))), ..Default::default() });
    app.telegram.sent_files = vec![json!({"id": "old", "name": "old.psd"})];
    let fresh: Vec<Value> = (0..12).map(|n| json!({"id": format!("n{n}"), "name": format!("f{n}.psd")})).collect();
    remember_sent_files(&mut app, &fresh);
    assert_eq!(app.telegram.sent_files.len(), 10);
    assert_eq!(app.telegram.sent_files[0].get("id").and_then(Value::as_str), Some("n0"));
    assert!(app.telegram.sent_files.iter().all(|row| row.get("id").and_then(Value::as_str) != Some("old")));
}

#[test]
fn inbox_keeps_the_ten_newest_files() {
    let mut messages = Vec::new();
    for n in 0..12 {
        messages.push(format!(r#"{{"message":{{"message_id":{n},"document":{{"file_id":"id{n}","file_name":"f{n}.psd","file_size":10}}}}}}"#));
    }
    let text = format!(r#"{{"ok":true,"result":[{}]}}"#, messages.join(","));
    let files = files_from_updates(&text);
    assert_eq!(files.len(), 10);
    assert_eq!(files[0].get("name").and_then(|v| v.as_str()), Some("f11.psd"));
}

#[test]
fn inbox_rows_keep_sent_files_and_ten_from_the_bot() {
    let sent = vec![json!({"id": "sent1", "name": "a.psd"}), json!({"id": "sent2", "name": "b.psd"})];
    let mut messages = Vec::new();
    for n in 0..12 {
        messages.push(format!(r#"{{"message":{{"message_id":{n},"document":{{"file_id":"id{n}","file_name":"../../f{n}.psd","file_size":10}}}}}}"#));
    }
    let text = format!(r#"{{"ok":true,"result":[{}]}}"#, messages.join(","));
    let rows = inbox_rows(&sent, &text);
    assert_eq!(rows.len(), 12);
    assert_eq!(rows[0].get("id").and_then(Value::as_str), Some("sent1"));
    assert_eq!(rows[2].get("name").and_then(Value::as_str), Some("f11.psd"));
    assert_eq!(file_name("../../evil.psd"), "evil.psd");
    assert_eq!(file_name(".."), "file");
}

#[test]
fn command_without_setup_opens_the_dialog_and_does_not_send() {
    let sent = Arc::new(Mutex::new(0u32));
    let count = sent.clone();
    let services = crate::Services {
        telegram_send: Some(Arc::new(move |_, _, _| {
            *count.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            Ok("{}".to_string())
        })),
        save_telegram: Some(Box::new(|_| Ok(()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let result = crate::menus::invoke(&mut app, &egui::Context::default(), "file.sendTelegram", json!({})).unwrap();
    assert!(result.get("dialog").is_some());
    assert_eq!(*sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner), 0);
}

/// A generic command dialog for this id used to show only the raw id and OK/Cancel.
#[test]
fn generic_command_opener_shows_the_telegram_form() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let id = crate::dialogs::open_command_dialog(&mut app, "file.sendTelegram", "file.sendTelegram");
    let dialog = app.ui.dialogs.iter().find(|d| d.id == id).expect("dialog");
    assert!(owns(&dialog.fields));
    assert_eq!(dialog.fields.get("__label").and_then(Value::as_str), Some("Send to Telegram"));
    assert_ne!(crate::dialogs::title(dialog), "file.sendTelegram");
}

#[test]
fn command_sends_the_document_file_and_hides_the_token() {
    let dir = std::env::temp_dir().join(format!("photocraft-telegram-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("photo.png");
    std::fs::write(&path, b"PNGDATA").unwrap();
    let seen = Arc::new(Mutex::new(String::new()));
    let slot = seen.clone();
    let services = crate::Services {
        telegram_send: Some(Arc::new(move |url, _, body| {
            let text = String::from_utf8_lossy(body).into_owned();
            *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = format!("{url}\n{text}");
            Err(format!("refused {url}"))
        })),
        save_telegram: Some(Box::new(|_| Ok(()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.session.active_mut().unwrap().path = Some(path.display().to_string());
    let mut fields = serde_json::Map::new();
    app.telegram_token_draft = token();
    fields.insert("chat".into(), json!("42, 42"));
    fields.insert("format".into(), json!("file"));
    let started = confirm(&mut app, &fields).unwrap();
    assert_eq!(started["sending"], true);
    assert_eq!(parse_chats(&app.telegram.chat_id).unwrap(), vec!["42".to_string()]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while app.telegram_job.is_some() && std::time::Instant::now() < deadline {
        poll(&mut app, &egui::Context::default());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(app.ui.status_error, "{}", app.ui.status);
    assert!(!app.ui.status.contains(&token()), "{}", app.ui.status);
    let dumped = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    assert!(dumped.contains("PNGDATA"), "{dumped}");
    assert!(dumped.contains("sendDocument"), "{dumped}");
    assert!(!dumped.contains("sendPhoto"), "{dumped}");
    assert!(dumped.contains("name=\"chat_id\""));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn after_export_stays_quiet_until_the_option_is_on() {
    let sent = Arc::new(Mutex::new(0u32));
    let count = sent.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            telegram_send: Some(Arc::new(move |_, _, _| {
                *count.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
                Ok("{}".to_string())
            })),
            ..Default::default()
        },
    );
    app.telegram = Settings { bot_token: token(), chat_id: "42".into(), ..Settings::default() };
    after_written(&mut app, "/tmp/does-not-matter.png", WriteKind::Save);
    after_written(&mut app, "/tmp/does-not-matter.png", WriteKind::Export);
    assert_eq!(*sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner), 0);
    app.telegram.bot_token.clear();
    app.telegram.send_on_export = true;
    after_written(&mut app, "/tmp/out.png", WriteKind::Export);
    assert!(dialog_open(&app));
    assert_eq!(*sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner), 0);
}

#[test]
fn message_links_and_chat_list() {
    let channel = r#"{"ok":true,"result":{"message_id":9,"chat":{"id":-100555,"type":"channel"}}}"#;
    assert_eq!(message_link(channel).as_deref(), Some("https://t.me/c/555/9"));
    let public = r#"{"ok":true,"result":{"message_id":4,"chat":{"id":1,"username":"mychan"}}}"#;
    assert_eq!(message_link(public).as_deref(), Some("https://t.me/mychan/4"));
    assert_eq!(message_link(r#"{"ok":true,"result":{"message_id":1,"chat":{"id":42}}}"#).as_deref(), Some("tg://openmessage?user_id=42&message_id=1"));
    let sent = r#"{"ok":true,"result":{"message_id":2,"document":{"file_id":"abc","file_name":"hill.psd"}}}"#;
    assert_eq!(sent_files_from_reply(sent)[0].get("id").and_then(Value::as_str), Some("abc"));
    let updates = r#"{"ok":true,"result":[{"message":{"chat":{"id":42,"first_name":"Sam"}}},{"message":{"chat":{"id":42,"first_name":"Sam"}}},{"channel_post":{"chat":{"id":-1007,"title":"Desk"}}}]}"#;
    let chats = chats_from_updates(updates);
    assert_eq!(chats.len(), 2);
    assert_eq!(chats[0]["id"], "42");
    assert_eq!(chats[1]["label"], "Desk (-1007)");
}

#[test]
fn send_returns_before_telegram_answers_and_then_reports_the_link() {
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let wait = Arc::new(Mutex::new(wait));
    let held = wait.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            export: Some(Box::new(|_, _, _| Ok((b"PNGDATA".to_vec(), Vec::new())))),
            telegram_send: Some(Arc::new(move |_, _, body| {
                assert!(body.windows(7).any(|w| w == b"PNGDATA"));
                let _ = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner).recv();
                Ok(r#"{"ok":true,"result":{"message_id":4,"chat":{"id":-100999,"username":"mychan"}}}"#.to_string())
            })),
            save_telegram: Some(Box::new(|_| Ok(()))),
            ..Default::default()
        },
    );
    app.session.execute("file.new", json!({"width": 8, "height": 8, "name": "Sketch"})).unwrap();
    let mut fields = serde_json::Map::new();
    app.telegram_token_draft = token();
    fields.insert("chat".into(), json!("42"));
    fields.insert("format".into(), json!("png"));
    let started = confirm(&mut app, &fields).unwrap();
    assert_eq!(started["sending"], true);
    let temp = app.telegram_job.as_ref().map(|job| job.path.clone()).unwrap();
    assert!(std::path::Path::new(&temp).exists(), "{temp}");
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while app.telegram_job.is_some() && std::time::Instant::now() < deadline {
        poll(&mut app, &egui::Context::default());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.status.contains("https://t.me/mychan/4"), "{}", app.ui.status);
    assert!(!std::path::Path::new(&temp).exists(), "{temp}");
}

#[test]
fn telegram_copy_is_translated_in_every_complete_catalog() {
    const STRINGS: &[&str] = &[
        "Send to Telegram…",
        "Send to Telegram",
        "Bot token",
        "Send as",
        "Saved file",
        "Saved file sends the copy on disk. PSD and PNG are a new copy of the open document, including changes you have not saved.",
        "Chats or channels. Several are allowed, separated by commas.",
        "Create a bot with BotFather, then send that bot any message. Use Find chats to fill the id, or paste a number or @channelname.",
        "Open the official bot guide",
        "Find chats for this token",
        "No chats yet. Send your bot a message, then try again.",
        "Also send when I Save",
        "Also send when I Save As",
        "Also send when I Export",
        "Also send when I Save a Copy",
        "Also send a PNG preview with a PSD",
        "Send automatically",
        "The bot token is kept in the system keychain, not in PhotoCraft's settings file.",
        "Sending needs an internet connection.",
        "Pin the message",
        "Pinning needs the bot to be an admin. If it is not, the file is still sent.",
        "Telegram did not return a link for this chat.",
        "Sent {name}. {url}",
        "Could not pin the message. The file was still sent.",
        "Open the Telegram message",
        "Files sent from PhotoCraft, and the ten newest files sent to this bot. Click one to open it in PhotoCraft.",
        "Get files",
        "No files yet. Send one from here, or send the bot a file, then try again.",
        "*File:* `{name}`",
        "*Size:* {size}",
        "*Time:* {time}",
        "*Path:* `{path}`",
    ];
    for lang in crate::i18n::Lang::all().filter(|l| l.complete_menus()) {
        for source in STRINGS {
            assert_ne!(crate::i18n::tr(lang, source), *source, "{}: {source}", lang.code());
        }
    }
}

#[test]
fn invalid_input_keeps_the_telegram_dialog_and_drafts() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let id = open_dialog(&mut app, None);
    app.telegram_token_draft = "typo".into();
    app.ui.dialog_mut(id).unwrap().fields.insert("chat".into(), json!("42"));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert!(app.ui.dialog_mut(id).is_some());
    assert_eq!(app.telegram_token_draft, "typo");
    app.telegram_token_draft = token();
    app.ui.dialog_mut(id).unwrap().fields.insert("chat".into(), json!("!invalid!"));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields.get("chat"), Some(&json!("!invalid!")));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn preview_uses_the_saved_source_instead_of_the_selected_tab() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            export: Some(Box::new(move |doc, _, _| {
                captured.lock().unwrap().push(doc.id);
                Ok((b"PNG".to_vec(), Vec::new()))
            })),
            ..Default::default()
        },
    );
    app.session.execute("file.new", json!({"width":8,"height":8,"name":"source"})).unwrap();
    let source = app.session.active().unwrap().doc.clone();
    app.session.execute("file.new", json!({"width":8,"height":8,"name":"other"})).unwrap();
    app.telegram.also_png = true;
    let preview = protocol::png_preview_for(&mut app, "source.psd", Some(&source)).unwrap().unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![source.id]);
    remove_temp_path(&preview);
    assert!(protocol::png_preview_for(&mut app, "closed.psd", None).unwrap().is_none());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn utc_stamp_handles_values_beyond_chrono_range() {
    assert!(utc_stamp(u64::MAX).ends_with(" UTC"));
    assert_ne!(utc_stamp(u64::MAX), "Unknown UTC time");
    assert_eq!(utc_stamp(0), "1970-01-01 00:00 UTC");
}

#[test]
fn automation_cannot_confirm_a_denied_telegram_dialog() {
    let services = crate::Services {
        automation_command: Some(Box::new(|id, _| {
            assert_eq!(id, "file.sendTelegram");
            Err("network denied".into())
        })),
        ..crate::Services::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let id = open_dialog(&mut app, None);
    app.automation_input = true;
    assert_eq!(crate::dialogs::confirm(&mut app, id), Err("network denied".into()));
    assert!(app.ui.dialog_mut(id).is_some());
}

#[test]
fn telegram_direct_network_actions_honor_both_automation_gates() {
    for session_gate in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        if session_gate {
            app.session.authorize = Some(|id, _| {
                assert_eq!(id, "file.sendTelegram");
                Err(photocraft_engine::EngineError::Other("network denied".into()))
            });
        } else {
            app.automation_input = true;
            app.services.automation_command = Some(Box::new(|id, _| {
                assert_eq!(id, "file.sendTelegram");
                Err("network denied".into())
            }));
        }
        for result in [
            start_find_chats(&mut app, &token()),
            start_fetch_inbox(&mut app, &token()),
            start_download(&mut app, &token(), "id", "file.png"),
            start_send(&mut app, "missing.png".into(), false, vec!["42".into()], None),
        ] {
            assert!(result.unwrap_err().contains("network denied"));
            assert!(app.telegram_job.is_none());
        }
    }
}

#[test]
fn telegram_answered_save_arms_the_original_document_for_all_save_kinds() {
    for (command, kind) in [("file.save", WriteKind::Save), ("file.saveAs", WriteKind::SaveAs), ("file.saveACopy", WriteKind::SaveCopy)] {
        for background in [false, true] {
            let mut app = PhotocraftApp::new(
                photocraft_engine::Session::new(),
                crate::Services {
                    export: Some(Box::new(|_, _, _| Ok((vec![1], vec![])))),
                    write: Some(Box::new(|_, _| Ok(()))),
                    save_file: Some(Arc::new(|_, _, _, _| Ok(vec![]))),
                    ..Default::default()
                },
            );
            app.session.execute("file.new", json!({"width":2,"height":2,"name":"source"})).unwrap();
            let source = app.active_doc_id().unwrap();
            app.session.execute("file.new", json!({"width":2,"height":2,"name":"other"})).unwrap();
            let other = app.active_doc_id().unwrap();
            app.telegram.send_on_save = true;
            app.telegram.send_on_save_as = true;
            app.telegram.send_on_save_copy = true;
            app.background_jobs = background;
            app.write_answered_save(
                &egui::Context::default(),
                source,
                "source.jpg".into(),
                &crate::ExportSettings::default(),
                command == "file.saveACopy",
                (command, json!({})),
            );
            assert_eq!(app.active_doc_id().unwrap(), other);
            if background {
                assert_eq!(app.jobs.saves.last().unwrap().notify_telegram, Some(kind));
                assert_eq!(app.jobs.saves.last().unwrap().doc, source);
                assert!(!dialog_open(&app));
            } else {
                let dialog = app.ui.dialogs.iter().find(|d| owns(&d.fields)).unwrap();
                assert_eq!(dialog.fields.get("pending"), Some(&json!("source.jpg")));
                assert_eq!(dialog.fields.get("__document"), Some(&json!(source.0)));
            }
        }
    }
}

#[test]
fn telegram_tiff_prompt_arms_save_as_after_confirmation() {
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services { export: Some(Box::new(|_, _, _| Ok((vec![1], vec![])))), write: Some(Box::new(|_, _| Ok(()))), ..Default::default() },
    );
    app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
    app.telegram.send_on_save_as = true;
    crate::tiff_options_ui::park(&mut app, "layers.tif".into()).unwrap();
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    };
    ctx.run_ui(input, |ui| crate::tiff_options_ui::show(&mut app, ui.ctx())).textures_delta.clear();
    assert!(app.tiff_options.is_none());
    assert!(dialog_open(&app));
    assert_eq!(app.session.journal.last().unwrap().0, "file.saveAs");
}
