//! Scoped translations for Codex editing and account controls.
const AI_KEYS: &[(&str, &str)] = &[
    ("", "AI Edit"),
    ("", "AI Edit…"),
    ("", "Cancel sign-in"),
    ("", "Cancelling sign-in…"),
    ("", "ChatGPT plan"),
    ("", "ChatGPT sign-in is required"),
    ("", "Check connection"),
    ("", "Checking Codex connection…"),
    ("", "Codex Account"),
    ("", "Codex account has not been checked"),
    ("", "Codex is using another login method. Sign in with ChatGPT to use your subscription."),
    ("", "Codex was not found. Install and open the Codex desktop app, then check the connection again."),
    ("", "Complete ChatGPT sign-in in your browser"),
    ("", "Connected to ChatGPT"),
    ("", "Connection details"),
    ("", "Copy sign-in link"),
    ("", "Could not open your browser. Try again or copy the sign-in link into your browser."),
    ("", "Could not verify the account. Check your connection or sign in again."),
    ("", "Describe your edit"),
    ("", "Open sign-in page"),
    ("", "Sign in with ChatGPT"),
    ("", "Sign-in cancelled"),
    ("", "Sign-in is managed by the installed Codex runtime and shared with Codex. PhotoCraft does not store tokens."),
    ("", "The visible canvas and instruction will be sent to Codex using your ChatGPT sign-in. The result is fitted to the canvas and added as a new layer."),
    ("", "Use the complete Codex installation; copying codex.exe alone may omit image-editing components."),
    ("@id", "edit.ai.codex"),
];

#[test]
fn every_registered_catalog_covers_codex_editing_and_account_controls() {
    use super::{LANGUAGES, Lang, catalog::parse_entries, tr, tr_id};
    for info in LANGUAGES.iter().filter(|info| info.code != "en") {
        let lang = Lang::from_code(info.code).unwrap();
        let (entries, errors) = parse_entries(&info.text());
        assert!(errors.is_empty(), "{}: {errors:?}", info.code);
        let entries: std::collections::HashMap<(&str, &str), &str> = entries.iter().map(|(ctx, src, tr)| ((ctx.as_str(), src.as_str()), tr.as_str())).collect();
        for &(context, source) in AI_KEYS {
            let expected = entries.get(&(context, source)).unwrap_or_else(|| panic!("{} lacks {context:?} {source:?}", info.code));
            assert_ne!(*expected, source, "{}: untranslated Codex text {source:?}", info.code);
            let rendered = if context == "@id" { tr_id(lang, source, "AI Edit…") } else { tr(lang, source) };
            assert_eq!(rendered, *expected, "{}: {source}", info.code);
        }
    }
}
