//! Localized content-review notices over the provider's safe diagnostic vocabulary.
use photocraft_engine::api_error::{ReviewKind, review_error};

use crate::{PhotocraftApp, i18n, notices};

pub(crate) fn report(app: &mut PhotocraftApp, command: &str, error: &str) -> bool {
    if !matches!(command, "edit.ai.api" | "edit.ai.codex") {
        return false;
    }
    let Some(rejected) = review_error(error) else { return false };
    let title = if rejected.kind == ReviewKind::ServiceUnavailable {
        "The image service's content review is unavailable"
    } else {
        "AI edit did not pass the service's content review"
    };
    let (message, action) = explanation(rejected.kind);
    let lines = vec![message.into(), action.into(), "This edit did not change existing layers or add a new layer.".into(), "Error code: {code}".into()];
    app.ui.status = format!("{}: {}", i18n::t(title), i18n::t(message));
    app.ui.status_error = true;
    // Store source templates so already-visible notices follow language changes too.
    let id = notices::post(app, title, lines, true, None);
    if let Some(notice) = app.ui.notices.iter_mut().find(|notice| notice.id == id) {
        notice.args.push(("code".into(), rejected.code.into()));
    }
    true
}

fn explanation(kind: ReviewKind) -> (&'static str, &'static str) {
    use ReviewKind::*;
    match kind {
        InputText => ("Your editing instruction did not pass the service's content review.", "Revise the instruction and try again manually."),
        ReferenceImage => {
            ("Your reference image did not pass the service's content review.", "Choose a reference image that meets the service's requirements.")
        }
        GeneratedImage => {
            ("The generated image did not pass the service's content review.", "Adjust the editing request or contact the service provider for details.")
        }
        InputTextPolicy => (
            "Your instruction triggered the service's content rules, which may include copyright restrictions.",
            "Check that your editing request meets the service's content requirements.",
        ),
        ReferenceImagePolicy => (
            "Your reference image triggered the service's content rules, which may include copyright restrictions.",
            "Choose a reference image that meets the service's requirements.",
        ),
        ReferenceImagePrivacy => {
            ("Your reference image did not pass the service's privacy review.", "Choose a reference image that meets the service's requirements.")
        }
        GeneratedImagePolicy => (
            "The generated image triggered the service's content rules, which may include copyright restrictions.",
            "Adjust the editing request or contact the service provider for details.",
        ),
        GeneratedImageImpersonation => (
            "The generated image triggered the service's impersonation restrictions.",
            "Adjust the editing request or contact the service provider for details.",
        ),
        Unspecified => (
            "The service rejected this edit without identifying the content-review stage.",
            "Check your editing content or ask the service provider for details.",
        ),
        ServiceUnavailable => (
            "The service's content review failed, so this edit could not be completed.",
            "Try again manually later. Contact the service provider if the problem continues.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use photocraft_engine::Session;

    fn rendered(app: &PhotocraftApp, lang: &str) -> String {
        i18n::with_language(Lang::from_code(lang).unwrap(), || {
            let n = app.ui.notices.last().unwrap();
            std::iter::once(n.text(&n.title)).chain(n.lines.iter().map(|line| n.text(line))).collect::<Vec<_>>().join("\n")
        })
    }

    #[test]
    fn localized_review_notices_explain_each_stage_and_preserve_safe_codes() {
        for (code, expected) in [
            ("InputTextRiskDetection", "编辑指令未通过"),
            ("InputImageRiskDetection", "参考图片未通过"),
            ("OutputImageRiskDetection", "生成结果未通过"),
            ("InputTextSensitiveContentDetected.PolicyViolation", "可能包含版权"),
            ("InputImageSensitiveContentDetected.PolicyViolation", "可能包含版权"),
            ("OutputImageSensitiveContentDetected.PolicyViolation", "可能包含版权"),
            ("InputImageSensitiveContentDetected.PrivacyInformation", "隐私审核"),
            ("OutputImageSensitiveContentDetected.DeepFake", "仿冒内容限制"),
            ("ContentRejected", "未说明具体"),
            ("ContentSecurityDetectionError", "审核服务发生错误"),
        ] {
            let mut app = PhotocraftApp::new(Session::new(), Default::default());
            let error = format!("Image API HTTP 400. {code}; parameter: PRIVATE_KEY. PRIVATE_PROMPT");
            assert!(report(&mut app, "edit.ai.api", &error));
            let text = rendered(&app, "zh-hans");
            assert!(text.contains(expected), "{text}");
            assert!(text.contains(code), "{text}");
            assert!(text.contains("本次未改动已有图层，也未添加新图层"), "{text}");
            assert!(!text.contains("PRIVATE"), "{text}");
            let json = serde_json::to_string(&app.ui.notices).unwrap();
            assert!(!json.contains("PRIVATE"));
        }
    }

    #[test]
    fn visible_notice_follows_language_changes_and_fallbacks_remain_untouched() {
        let mut app = PhotocraftApp::new(Session::new(), Default::default());
        assert!(report(&mut app, "edit.ai.codex", "Image service rejected the request. InputTextRiskDetection. Message"));
        assert!(rendered(&app, "zh-hans").contains("编辑指令未通过"));
        assert!(rendered(&app, "zh-hant").contains("編輯指令未通過"));
        assert!(rendered(&app, "en").contains("Your editing instruction"));
        let before = app.ui.notices.clone();
        let status = app.ui.status.clone();
        assert!(!report(&mut app, "file.open", "Image API HTTP 400. InputTextRiskDetection. Message"));
        assert!(!report(&mut app, "edit.ai.api", "Image API HTTP 400. InvalidParameter; parameter: size. Message"));
        assert!(!report(&mut app, "edit.ai.api", "Could not open InputTextRiskDetection.png"));
        assert_eq!(app.ui.notices, before);
        assert_eq!(app.ui.status, status);
    }

    #[test]
    fn visible_review_notice_switches_across_all_languages_and_keeps_the_code() {
        let mut app = PhotocraftApp::new(Session::new(), Default::default());
        let code = "InputImageSensitiveContentDetected.PrivacyInformation";
        assert!(report(&mut app, "edit.ai.api", &format!("Image API HTTP 400. {code}. PRIVATE_BODY")));
        for lang in Lang::all() {
            let text = rendered(&app, lang.code());
            assert!(text.contains(i18n::tr(lang, "Your reference image did not pass the service's privacy review.")), "{}: {text}", lang.code());
            assert!(text.contains(code), "{}: {text}", lang.code());
            assert!(!text.contains("PRIVATE"));
        }
        assert_eq!(app.ui.notices.len(), 1, "language changes do not duplicate notices");
    }
}
