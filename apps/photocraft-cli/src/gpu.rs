//! GPU acceleration policy and options for photocraft-cli.

use photocraft_automation::Headless;
use photocraft_io::BandRenderer;

/// Optional GPU composition for supported flat exports and full-size previews.
#[derive(clap::Args)]
pub(crate) struct GpuArgs {
    /// Prefer GPU rendering, falling back to CPU when unavailable or unsupported
    #[arg(long, conflicts_with = "no_gpu")]
    pub(crate) gpu: bool,
    /// Use CPU rendering, overriding PHOTOCRAFT_GPU
    #[arg(long, conflicts_with = "gpu")]
    pub(crate) no_gpu: bool,
}

/// Resolved GPU rendering policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GpuPolicy {
    pub(crate) enabled: bool,
}

impl GpuPolicy {
    /// Resolve policy from arguments and environment.
    ///
    /// Precedence:
    /// - Explicit `--gpu` enables GPU.
    /// - Explicit `--no-gpu` disables GPU.
    /// - Neither flag: `PHOTOCRAFT_GPU` env var (1 = enabled, 0 or empty/unset = disabled).
    ///   Any other nonempty value is rejected.
    pub(crate) fn from_args(args: &GpuArgs) -> crate::Result<Self> {
        if args.gpu {
            return Ok(Self { enabled: true });
        }
        if args.no_gpu {
            return Ok(Self { enabled: false });
        }
        match std::env::var_os("PHOTOCRAFT_GPU") {
            Some(val) => Self::from_environment(&val.to_string_lossy()),
            None => Ok(Self { enabled: false }),
        }
    }

    fn from_environment(value: &str) -> crate::Result<Self> {
        match value.trim() {
            "" | "0" => Ok(Self { enabled: false }),
            "1" => Ok(Self { enabled: true }),
            other => Err(crate::CliError::GpuEnvironment(other.to_owned())),
        }
    }

    /// Apply policy to a headless automation session.
    pub(crate) fn apply_to_headless(&self, h: &mut Headless) {
        #[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
        if self.enabled {
            h.enable_gpu();
        }
        #[cfg(any(not(feature = "gpu"), target_arch = "wasm32"))]
        if self.enabled {
            h.set_renderer(Some(Box::new(DisabledGpuRenderer::new())));
        }
    }

    /// Create an optional boxed renderer under this policy.
    #[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
    pub(crate) fn create_renderer(&self) -> Option<Box<dyn BandRenderer>> {
        if self.enabled { Some(Box::new(photocraft_automation::gpu::GpuRenderer::lazy())) } else { None }
    }

    #[cfg(any(not(feature = "gpu"), target_arch = "wasm32"))]
    pub(crate) fn create_renderer(&self) -> Option<Box<dyn BandRenderer>> {
        if self.enabled { Some(Box::new(DisabledGpuRenderer::new())) } else { None }
    }
}

/// Renderer stub emitted when GPU is requested in builds compiled without `--features gpu`.
#[cfg(any(not(feature = "gpu"), target_arch = "wasm32"))]
struct DisabledGpuRenderer {
    emitted: bool,
}

#[cfg(any(not(feature = "gpu"), target_arch = "wasm32"))]
impl DisabledGpuRenderer {
    fn new() -> Self {
        Self { emitted: false }
    }
}

#[cfg(any(not(feature = "gpu"), target_arch = "wasm32"))]
impl BandRenderer for DisabledGpuRenderer {
    fn render_bands(
        &mut self,
        _doc: &photocraft_doc::Document,
        _rect: photocraft_doc::Rect,
        _band_rows: i32,
        _ctl: &photocraft_raster::Interrupt<'_>,
        _sink: &mut dyn FnMut(photocraft_io::RenderBand) -> Result<(), photocraft_io::IoError>,
    ) -> Result<(), photocraft_io::RenderError> {
        Err(photocraft_io::RenderError::Unavailable("GPU support is not compiled in this build (compile with --features gpu)".into()))
    }

    fn drain_diagnostics(&mut self) -> Vec<String> {
        if self.emitted {
            Vec::new()
        } else {
            self.emitted = true;
            vec!["GPU acceleration requested but this build was compiled without GPU support (compile with --features gpu); falling back to CPU".into()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_policy_keeps_whitespace_and_boolean_semantics() {
        for value in ["", " ", "0", " 0 "] {
            assert_eq!(GpuPolicy::from_environment(value).unwrap(), GpuPolicy { enabled: false });
        }
        for value in ["1", " 1 "] {
            assert_eq!(GpuPolicy::from_environment(value).unwrap(), GpuPolicy { enabled: true });
        }
        let error = GpuPolicy::from_environment(" invalid ").unwrap_err();
        assert!(matches!(error, crate::CliError::GpuEnvironment(ref value) if value == "invalid"));
        assert_eq!(error.to_string(), "invalid value 'invalid' for 'PHOTOCRAFT_GPU': expected '1' or '0'");
    }
}
