//! Reviewed FP32 exports. URLs contain immutable revisions; sizes and SHA-256s come from their
//! Hugging Face LFS metadata. Updating a model is a code review, never a remote manifest update.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelId {
    #[serde(rename = "birefnet-hr-matting")]
    BiRefNet,
    #[serde(rename = "sam2.1-large")]
    Sam2,
}

impl ModelId {
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "birefnet-hr-matting" => Some(Self::BiRefNet),
            "sam2.1-large" => Some(Self::Sam2),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::BiRefNet => "birefnet-hr-matting",
            Self::Sam2 => "sam2.1-large",
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::BiRefNet => 0,
            Self::Sam2 => 1,
        }
    }
    pub fn info(self) -> &'static ModelInfo {
        match self {
            Self::BiRefNet => &MODELS[0],
            Self::Sam2 => &MODELS[1],
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub file: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: ModelId,
    pub label: &'static str,
    pub purpose: &'static str,
    pub input_side: usize,
    pub revision: &'static str,
    pub license: &'static str,
    pub upstream: &'static str,
    pub export_source: &'static str,
    pub artifacts: &'static [Artifact],
}

impl ModelInfo {
    pub fn download_bytes(&self) -> u64 {
        self.artifacts.iter().map(|a| a.bytes).sum()
    }
}

pub const MODELS: [ModelInfo; 2] = [
    ModelInfo {
        id: ModelId::BiRefNet,
        label: "BiRefNet HR Matting",
        purpose: "Select Subject and Remove Background; detailed, soft edges",
        input_side: 2048,
        revision: "792518b9d4dfe9793891712677de25e554f948c6",
        license: "MIT",
        upstream: "https://huggingface.co/ZhengPeng7/BiRefNet_HR-matting",
        export_source: "https://huggingface.co/PinkPixel/birefnet-hr-matting-onnx",
        artifacts: &[Artifact {
            file: "model.onnx",
            url: "https://huggingface.co/PinkPixel/birefnet-hr-matting-onnx/resolve/792518b9d4dfe9793891712677de25e554f948c6/model.onnx",
            bytes: 932_150_975,
            sha256: "a548a1f3307250766bef50b174026b3cfa39a695bc1e42ec131c14fe82797ffd",
        }],
    },
    ModelInfo {
        id: ModelId::Sam2,
        label: "SAM 2.1 Large",
        purpose: "Object Selection; drag a box around the object",
        input_side: 1024,
        revision: "3c23431f721e69cae82dbfd0c28fd692cc714021",
        license: "Apache-2.0",
        upstream: "https://huggingface.co/facebook/sam2.1-hiera-large",
        export_source: "https://huggingface.co/onnx-community/sam2.1-hiera-large-ONNX",
        artifacts: &[
            Artifact {
                file: "vision_encoder.onnx",
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-large-ONNX/resolve/3c23431f721e69cae82dbfd0c28fd692cc714021/onnx/vision_encoder.onnx",
                bytes: 1_350_487,
                sha256: "ad3a8161c6bd6d8beea811300c60e20d626035feaa3c19622ace27b22e25bec8",
            },
            Artifact {
                file: "vision_encoder.onnx_data",
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-large-ONNX/resolve/3c23431f721e69cae82dbfd0c28fd692cc714021/onnx/vision_encoder.onnx_data",
                bytes: 888_587_456,
                sha256: "88c501a65b6aaeac99b65a275f3f559727404283930d107d499fda8714ba7e5a",
            },
            Artifact {
                file: "prompt_encoder_mask_decoder.onnx",
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-large-ONNX/resolve/3c23431f721e69cae82dbfd0c28fd692cc714021/onnx/prompt_encoder_mask_decoder.onnx",
                bytes: 213_114,
                sha256: "a21b3b09f6e5f5838d630d92a150914ca58f8f58b603b8f53fc32b03da982203",
            },
            Artifact {
                file: "prompt_encoder_mask_decoder.onnx_data",
                url: "https://huggingface.co/onnx-community/sam2.1-hiera-large-ONNX/resolve/3c23431f721e69cae82dbfd0c28fd692cc714021/onnx/prompt_encoder_mask_decoder.onnx_data",
                bytes: 20_958_208,
                sha256: "bfdcd4f7ee04298a82fe1aaf1bd7b8edf544d36901e1a886f01892fb915bd63c",
            },
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_pinned_and_ids_cannot_escape_storage() {
        for m in MODELS {
            assert_eq!(ModelId::parse(m.id.name()), Some(m.id));
            for a in m.artifacts {
                assert!(a.url.starts_with("https://huggingface.co/"));
                assert!(a.url.contains(m.revision));
                assert!(a.bytes > 0 && a.bytes < 2_000_000_000);
                assert_eq!(a.sha256.len(), 64);
                assert!(a.sha256.bytes().all(|c| c.is_ascii_hexdigit()));
                assert!(!a.file.contains(['/', '\\']));
            }
        }
        for id in ["../model", "sam3", "", "https://example.com/model.onnx"] {
            assert_eq!(ModelId::parse(id), None);
        }
    }
}
