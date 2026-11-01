//! Scoped image editing and account vocabulary.
const AI_KEYS: &[(&str, &str)] = &[
    ("", "Adjust the editing request or contact the service provider for details."),
    ("", "AI Edit"),
    ("", "AI edit did not pass the service's content review"),
    ("", "AI Edit via API"),
    ("", "AI Edit…"),
    ("", "AI Provider Settings"),
    ("", "API address"),
    ("", "API address and protocol"),
    ("", "API configured. Saving settings does not generate an image."),
    ("", "API key"),
    ("", "API key configured — leave empty to keep it"),
    ("", "API key removed"),
    ("", "API protocol"),
    ("", "API settings"),
    ("", "API settings saved"),
    ("", "Cancel sign-in"),
    ("", "Cancelling sign-in…"),
    ("", "ChatGPT plan"),
    ("", "ChatGPT sign-in is required"),
    ("", "Check connection"),
    ("", "Check that your editing request meets the service's content requirements."),
    ("", "Check your editing content or ask the service provider for details."),
    ("", "Checking Codex connection…"),
    ("", "Choose a reference image that meets the service's requirements."),
    ("", "Codex (ChatGPT subscription)"),
    ("", "Codex Account"),
    ("", "Codex account has not been checked"),
    ("", "Codex is using another login method. Sign in with ChatGPT to use your subscription."),
    ("", "Codex was not found. Install and open the Codex desktop app, then check the connection again."),
    ("", "Complete ChatGPT sign-in in your browser"),
    ("", "Connected to ChatGPT"),
    ("", "Connection details"),
    ("", "Copy sign-in link"),
    ("", "Could not load API settings"),
    ("", "Could not open your browser. Try again or copy the sign-in link into your browser."),
    ("", "Could not verify the account. Check your connection or sign in again."),
    ("", "Custom services must support the selected image-editing protocol. Changing the address requires a key for that address."),
    ("", "Describe your edit"),
    ("", "Doubao / Seedream"),
    ("", "Enter a valid API key without whitespace"),
    ("", "Error code: {code}"),
    ("", "Image service"),
    ("", "Model ID"),
    ("", "Open sign-in page"),
    ("", "Remember API key in system credential store"),
    ("", "Remove API key"),
    ("", "Revise the instruction and try again manually."),
    ("", "Save API settings"),
    ("", "Save your API settings before editing."),
    ("", "Sign in with ChatGPT"),
    ("", "Sign-in cancelled"),
    ("", "Sign-in is managed by the installed Codex runtime and shared with Codex. PhotoCraft does not store tokens."),
    ("", "The API key is available for this session only."),
    ("", "The generated image did not pass the service's content review."),
    ("", "The generated image triggered the service's content rules, which may include copyright restrictions."),
    ("", "The generated image triggered the service's impersonation restrictions."),
    ("", "The image service's content review is unavailable"),
    ("", "The result is fitted to the canvas and added as a new layer. Your existing layers are preserved."),
    ("", "The service rejected this edit without identifying the content-review stage."),
    ("", "The service's content review failed, so this edit could not be completed."),
    ("", "The visible canvas and instruction will be sent to Codex using your ChatGPT sign-in. The result is fitted to the canvas and added as a new layer."),
    ("", "This API sends the visible canvas and instruction to the configured service. Usage may incur charges billed by that service."),
    ("", "This edit did not change existing layers or add a new layer."),
    ("", "Try again manually later. Contact the service provider if the problem continues."),
    ("", "Use the complete Codex installation; copying codex.exe alone may omit image-editing components."),
    ("", "Your editing instruction did not pass the service's content review."),
    ("", "Your instruction triggered the service's content rules, which may include copyright restrictions."),
    ("", "Your reference image did not pass the service's content review."),
    ("", "Your reference image did not pass the service's privacy review."),
    ("", "Your reference image triggered the service's content rules, which may include copyright restrictions."),
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
            if source != "Doubao / Seedream" {
                assert_ne!(*expected, source, "{}: untranslated AI text {source:?}", info.code);
            }
            let rendered = if context == "@id" { tr_id(lang, source, "AI Edit…") } else { tr(lang, source) };
            assert_eq!(rendered, *expected, "{}: {source}", info.code);
        }
    }
}
