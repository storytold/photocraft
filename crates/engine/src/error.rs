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
    /// can raise. The requirement may equal the ceiling when the estimate saturated. The ceiling is
    /// measured from this machine's memory, so the message points at where the app's own limit is
    /// set, and only a limit of 0 - the figure a memory reading reports when it finds no free memory
    /// at all - says anything about the reading itself (#11544).
    #[error(
        "cannot open “{name}”: its estimated memory requirement of {required} bytes exceeds the {limit} byte memory limit on this machine{advice}",
        advice = memory_limit_advice(*limit)
    )]
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

pub type Result<T> = std::result::Result<T, EngineError>;

/// What the limit in [`EngineError::PsdHardBudget`] means for the user.
///
/// A positive limit is a figure worth acting on: an import above it does not fit the memory this
/// machine has, and the app's own limit is a setting. Nothing is claimed about the reading. A limit
/// of 0 is a different story - only a memory reading that found no free memory at all produces it -
/// so the message says what the number is, instead of letting "0 bytes" read as "this machine has
/// no memory" (#11544, where every open on a working Windows 11 host failed that way).
fn memory_limit_advice(limit: u64) -> &'static str {
    if limit == 0 {
        " (the system memory reading found no free memory on this machine, so no import can be admitted; the app's own memory limit is set in Preferences › Performance)"
    } else {
        " (the limit is measured from this machine's available memory; the app's own memory limit is set in Preferences › Performance)"
    }
}

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
