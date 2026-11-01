//! Safe, platform-neutral content-review diagnostics shared by transports and frontends.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewKind {
    InputText,
    ReferenceImage,
    GeneratedImage,
    InputTextPolicy,
    ReferenceImagePolicy,
    ReferenceImagePrivacy,
    GeneratedImagePolicy,
    GeneratedImageImpersonation,
    Unspecified,
    ServiceUnavailable,
}

impl ReviewKind {
    /// Stable English transport text; frontends can present localized, actionable notices.
    pub fn description(self) -> &'static str {
        match self {
            Self::InputText => "The provider rejected the input text under its content rules",
            Self::ReferenceImage => "The provider rejected the reference image under its content rules",
            Self::GeneratedImage => "The provider rejected the generated image under its content rules",
            Self::InputTextPolicy => "The provider's policy check rejected the input text",
            Self::ReferenceImagePolicy => "The provider's policy check rejected the reference image",
            Self::ReferenceImagePrivacy => "The provider's privacy check rejected the reference image",
            Self::GeneratedImagePolicy | Self::GeneratedImageImpersonation => "The provider's policy check rejected the generated image",
            Self::Unspecified => "The provider rejected the input or output under its content rules",
            Self::ServiceUnavailable => "The provider's content review service failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewRejection {
    /// Always a literal from the allowlist, never a borrowed provider value.
    pub code: &'static str,
    pub kind: ReviewKind,
}

pub fn review_code(code: &str) -> Option<ReviewRejection> {
    if code.len() > 128 {
        return None;
    }
    use ReviewKind::*;
    let (code, kind) = match code {
        "InputTextSensitiveContentDetected.PolicyViolation" => ("InputTextSensitiveContentDetected.PolicyViolation", InputTextPolicy),
        "InputImageSensitiveContentDetected.PolicyViolation" => ("InputImageSensitiveContentDetected.PolicyViolation", ReferenceImagePolicy),
        "InputImageSensitiveContentDetected.PrivacyInformation" => ("InputImageSensitiveContentDetected.PrivacyInformation", ReferenceImagePrivacy),
        "OutputImageSensitiveContentDetected.PolicyViolation" => ("OutputImageSensitiveContentDetected.PolicyViolation", GeneratedImagePolicy),
        "OutputImageSensitiveContentDetected.DeepFake" => ("OutputImageSensitiveContentDetected.DeepFake", GeneratedImageImpersonation),
        code => match code.split('.').next()? {
            "SensitiveContentDetected" => ("SensitiveContentDetected", InputText),
            "InputTextSensitiveContentDetected" => ("InputTextSensitiveContentDetected", InputText),
            "InputTextRiskDetection" => ("InputTextRiskDetection", InputText),
            "InputImageSensitiveContentDetected" => ("InputImageSensitiveContentDetected", ReferenceImage),
            "InputImageRiskDetection" => ("InputImageRiskDetection", ReferenceImage),
            "OutputImageSensitiveContentDetected" => ("OutputImageSensitiveContentDetected", GeneratedImage),
            "OutputImageRiskDetection" => ("OutputImageRiskDetection", GeneratedImage),
            "ContentSecurityDetectionError" => ("ContentSecurityDetectionError", ServiceUnavailable),
            // Compatibility with notices produced by the first diagnostic prototype.
            "ContentRejected" => ("ContentRejected", Unspecified),
            _ => return None,
        },
    };
    Some(ReviewRejection { code, kind })
}

/// Decode our bounded error envelope after engine/job transport has formatted it as text.
/// Never infer content review from a substring in a filename, a prompt or an unrelated error.
pub fn review_error(error: &str) -> Option<ReviewRejection> {
    if error.len() > 4096 {
        return None;
    }
    let detail = if let Some(rest) = error.strip_prefix("Image API HTTP ") {
        let (status, detail) = rest.split_once(". ")?;
        if status.len() != 3 || !(400..=599).contains(&status.parse::<u16>().ok()?) {
            return None;
        }
        detail
    } else {
        error.strip_prefix("Image service rejected the request. ").or_else(|| error.strip_prefix("Image service reported a failed image. "))?
    };
    let code = detail.split_whitespace().next()?.trim_end_matches(['.', ';']);
    review_code(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_review_survives_http_and_embedded_error_envelopes() {
        for prefix in ["Image API HTTP 400. ", "Image API HTTP 403. ", "Image service rejected the request. ", "Image service reported a failed image. "] {
            for suffix in [". Message", "; parameter: image. Message"] {
                let error = format!("{prefix}InputImageSensitiveContentDetected.PrivacyInformation{suffix}");
                let rejected = review_error(&error).unwrap();
                assert_eq!(rejected.kind, ReviewKind::ReferenceImagePrivacy);
                assert_eq!(rejected.code, "InputImageSensitiveContentDetected.PrivacyInformation");
            }
        }
        let rejected = review_error("Image API HTTP 400. InputTextRiskDetection.PRIVATE_KEY. PRIVATE_PROMPT").unwrap();
        assert_eq!(rejected.code, "InputTextRiskDetection");
        assert_eq!(rejected.kind, ReviewKind::InputText);
        assert_eq!(review_error("Image API HTTP 400. ContentRejected. Message").unwrap().kind, ReviewKind::Unspecified);
    }

    #[test]
    fn unrelated_errors_and_untrusted_diagnostics_are_not_promoted_to_review_failures() {
        for error in [
            "Could not open InputTextRiskDetection.png",
            "Image API HTTP 400. PRIVATE_KEY. InputTextRiskDetection",
            "Image API HTTP 400. InvalidParameter; parameter: image. InputTextRiskDetection",
            "Image API HTTP 200. InputTextRiskDetection. Message",
            "Image API HTTP 4000. InputTextRiskDetection. Message",
            "Image API HTTP abc. InputTextRiskDetection. Message",
            "Image API HTTP 400. ",
        ] {
            assert_eq!(review_error(error), None, "{error}");
        }
        assert_eq!(review_error(&"InputTextRiskDetection".repeat(500)), None);
        assert_eq!(review_code(&"InputTextRiskDetection".repeat(100)), None);
    }
}
