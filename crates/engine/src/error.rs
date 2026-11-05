use super::LayerId;
use crate::command_hints;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum EngineError {
    /// Displays with the nearest registered ids, if any ([`command_hints`], #3674).
    #[error("unknown command `{id}`{hint}", id = .0, hint = command_hints::unknown_command_hint(.0))]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no active document")]
    NoDocument,
    #[error("no such layer {0:?}")]
    NoLayer(LayerId),
    /// [`Self::NoLayer`] as a command reports it: with the layers the document does have, as
    /// ids are not positions (#5058).
    #[error("no such layer {}; {known}", id.0)]
    UnknownLayer { id: LayerId, known: String },
    /// Opening a PSD/PSB file requires explicit high-memory confirmation.
    #[error("opening “{name}” requires explicit memory approval: {required} bytes required (soft limit {soft_limit}, hard limit {hard_limit})")]
    PsdApprovalRequired { name: String, required: u64, soft_limit: u64, hard_limit: u64, available_ram: Option<u64> },
    /// The file's estimated memory cannot be admitted within the hard ceiling, which no approval
    /// can raise. The requirement may equal the ceiling when the estimate saturated.
    #[error("cannot open “{name}”: its estimated memory requirement of {required} bytes cannot be admitted within the {limit} byte hard limit{scope}", scope = build_limit_scope(build_memory_limit()))]
    PsdHardBudget { name: String, required: u64, limit: u64 },
    /// The file behind an opened handle is not the one whose memory estimate was approved.
    /// The document is discarded and is not opened.
    #[error("“{name}” changed since it was checked; open it again to approve the new file")]
    SourceChanged { name: String },
    #[error("{0}")]
    Other(String),
    /// A background job was cancelled (see [`jobs`]); nothing changed.
    #[error("cancelled")]
    Cancelled,
}

/// Where this build's hard memory limit comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildMemoryLimit {
    /// Measured from the computer's memory (64-bit desktop).
    Measured,
    /// The 32-bit desktop build: its cap, or the computer's memory if lower.
    Desktop32Bit,
    /// Fixed by the browser build.
    FixedWeb,
}

/// This build's memory-limit source, from its target.
fn build_memory_limit() -> BuildMemoryLimit {
    memory_limit_for(cfg!(target_arch = "wasm32"), cfg!(target_pointer_width = "32"))
}

/// The limit source for a target. The wasm32 check comes first: wasm32 is also 32-bit.
fn memory_limit_for(is_wasm32: bool, is_32_bit: bool) -> BuildMemoryLimit {
    if is_wasm32 {
        BuildMemoryLimit::FixedWeb
    } else if is_32_bit {
        BuildMemoryLimit::Desktop32Bit
    } else {
        BuildMemoryLimit::Measured
    }
}

/// The text after "hard limit" that says where the limit comes from and what to do.
fn build_limit_scope(limit: BuildMemoryLimit) -> &'static str {
    match limit {
        BuildMemoryLimit::Measured => " on this machine",
        BuildMemoryLimit::Desktop32Bit => {
            " on this machine; 32-bit builds of PhotoCraft can use at most 2 GiB of memory for a file, so the 64-bit build may open it if this computer has more free"
        }
        BuildMemoryLimit::FixedWeb => " of the browser version of PhotoCraft; open it in the desktop app",
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

impl EngineError {
    /// Whether an approval-required error also carries the unknown-capacity warning: capacity was
    /// unknown when the estimate was made and the estimate is above the warning threshold. A
    /// fresh measurement or a known capacity never sets it, and no number is invented for a
    /// capacity that could not be measured.
    pub fn uncertain_capacity_warning(&self) -> bool {
        match self {
            Self::PsdApprovalRequired { required, available_ram, .. } => photocraft_io::memory::uncertain_capacity_warning(*required, *available_ram),
            _ => false,
        }
    }
}

/// Maps an io open/import failure to the engine's typed errors, preserving the memory fields the
/// UI, control channel and agents need to answer or explain it.
pub(crate) fn io_open_error(name: &str, error: photocraft_io::IoError) -> EngineError {
    use photocraft_io::IoError;
    match error {
        IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram } => {
            EngineError::PsdApprovalRequired { name: name.to_string(), required, soft_limit, hard_limit, available_ram }
        }
        IoError::PsdImportBudget { required, limit } => EngineError::PsdHardBudget { name: name.to_string(), required, limit },
        IoError::SourceChanged => EngineError::SourceChanged { name: name.to_string() },
        error => EngineError::Other(format!("{name}: {error}")),
    }
}

impl From<photocraft_raster::AllocationError> for EngineError {
    fn from(error: photocraft_raster::AllocationError) -> Self {
        Self::Other(format!("not enough memory for this operation ({error}); the document was not changed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_budget_scope_text_per_build() {
        assert_eq!(build_limit_scope(BuildMemoryLimit::Measured), " on this machine");
        assert_eq!(
            build_limit_scope(BuildMemoryLimit::Desktop32Bit),
            " on this machine; 32-bit builds of PhotoCraft can use at most 2 GiB of memory for a file, so the 64-bit build may open it if this computer has more free"
        );
        assert_eq!(build_limit_scope(BuildMemoryLimit::FixedWeb), " of the browser version of PhotoCraft; open it in the desktop app");
    }

    #[test]
    fn hard_budget_message_names_file_numbers_and_scope() {
        let text = EngineError::PsdHardBudget { name: "big.psd".into(), required: 9000, limit: 8000 }.to_string();
        for part in ["big.psd", "9000", "8000", "hard limit", build_limit_scope(build_memory_limit())] {
            assert!(text.contains(part), "{text}");
        }
    }

    #[test]
    fn memory_limit_follows_the_target() {
        assert_eq!(memory_limit_for(true, true), BuildMemoryLimit::FixedWeb);
        assert_eq!(memory_limit_for(true, false), BuildMemoryLimit::FixedWeb);
        assert_eq!(memory_limit_for(false, true), BuildMemoryLimit::Desktop32Bit);
        assert_eq!(memory_limit_for(false, false), BuildMemoryLimit::Measured);
    }

    #[test]
    fn two_gib_in_the_text_matches_the_cap() {
        assert_eq!(photocraft_io::memory::NATIVE_32BIT_PEAK_CAP, 2 << 30);
    }
}
