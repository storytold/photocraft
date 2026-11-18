//! The wasmi host: loading, instantiating and calling a plug-in under resource limits.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use wasmi::{
    Config, EnforcedLimits, Engine, Instance, Linker, Memory, Module, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, TypedResumableCall,
    WasmParams, WasmResults,
};
use wasmi_core::LimiterError;

use crate::manifest::Manifest;
use crate::{Error, Result};

/// The plug-in ABI version this host implements (`pc_abi_version` must return it).
pub const ABI_VERSION: i32 = 1;

/// Fuel handed out per slice; between slices the host checks the deadline and the abort flag.
const FUEL_SLICE: u64 = 20_000_000;

/// Resource limits for running plug-ins.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Largest module accepted.
    pub max_module_bytes: usize,
    /// Cap on a plug-in's linear memory.
    pub max_memory_bytes: usize,
    /// Cap on the linear memory actually allocated by all of one run's instances together.
    /// Values below `max_memory_bytes` use that per-instance cap as the effective total, so
    /// at least one instance can run.
    pub max_total_memory_bytes: usize,
    /// Target size of one band of `f32` pixels handed to the plug-in.
    pub band_bytes: usize,
    /// Instructions (fuel) any call may use, plus [`Limits::fuel_per_sample`] per sample for
    /// `pc_filter`.
    pub fuel_base: u64,
    pub fuel_per_sample: u64,
    /// Deepest call nesting.
    pub max_recursion_depth: usize,
    /// Wall-clock budget for one filter run (all bands), in milliseconds. Native builds only:
    /// the web build has no monotonic clock in `std`, so the fuel budget alone bounds it there.
    pub wall_time_ms: u64,
}

impl Default for Limits {
    /// The limits plug-ins run under unless a caller narrows them; see `docs/plugins.md`.
    fn default() -> Self {
        Self {
            max_module_bytes: 32 << 20,
            max_memory_bytes: 512 << 20,
            max_total_memory_bytes: 2 << 30,
            band_bytes: 4 << 20,
            fuel_base: 50_000_000,
            fuel_per_sample: 4_000,
            max_recursion_depth: 1024,
            wall_time_ms: 60_000,
        }
    }
}

impl Limits {
    /// Conservative parallelism when each instance could reach its full memory cap.
    pub fn parallel_bands(&self, threads: usize) -> usize {
        let fit = self.max_total_memory_bytes.checked_div(self.max_memory_bytes).unwrap_or(threads);
        threads.min(fit).max(1)
    }

    /// Schedule against a measured band's peak, while the shared limiter enforces the actual
    /// total if a later band allocates more. One instance may use its full individual cap.
    pub(crate) fn parallel_bands_for(&self, threads: usize, measured_bytes: usize) -> usize {
        let budget = self.max_total_memory_bytes.max(self.max_memory_bytes);
        let fit = budget.checked_div(measured_bytes.max(1)).unwrap_or(1);
        threads.min(fit).max(1)
    }
}

#[derive(Default)]
struct MemoryUsage {
    used: usize,
    peak_instance: usize,
    denials: usize,
}

/// All instances in one filter run charge their allocated linear memory here. A separate budget
/// is used while loading the module; there are no host imports or additional Wasm memories.
pub(crate) struct RunBudget {
    limit: usize,
    usage: Mutex<MemoryUsage>,
}

impl RunBudget {
    pub(crate) fn new(limits: &Limits) -> Arc<Self> {
        Arc::new(Self { limit: limits.max_total_memory_bytes.max(limits.max_memory_bytes), usage: Mutex::new(MemoryUsage::default()) })
    }

    pub(crate) fn snapshot(&self) -> (usize, usize) {
        let usage = self.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        (usage.peak_instance, usage.denials)
    }

    #[cfg(test)]
    pub(crate) fn occupy_for_test(&self, bytes: usize) {
        self.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner).used = bytes;
    }
}

struct BandLimiter {
    limits: StoreLimits,
    budget: Arc<RunBudget>,
    denied: Arc<AtomicBool>,
    charged: usize,
    pending_old: Option<usize>,
}

impl BandLimiter {
    fn new(limits: &Limits, budget: Arc<RunBudget>, denied: Arc<AtomicBool>) -> Self {
        let limits = StoreLimitsBuilder::new().memory_size(limits.max_memory_bytes).memories(1).tables(4).table_elements(100_000).instances(1).build();
        Self { limits, budget, denied, charged: 0, pending_old: None }
    }
}

impl Drop for BandLimiter {
    fn drop(&mut self) {
        let mut usage = self.budget.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // A failed growth is refunded before drop; only completed allocations inform later groups.
        usage.peak_instance = usage.peak_instance.max(self.charged);
        usage.used = usage.used.saturating_sub(self.charged);
    }
}

impl ResourceLimiter for BandLimiter {
    fn memory_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> std::result::Result<bool, LimiterError> {
        if !self.limits.memory_growing(current, desired, maximum)? {
            return Ok(false);
        }
        let mut usage = self.budget.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let delta = desired.saturating_sub(self.charged);
        if usage.used.checked_add(delta).is_none_or(|total| total > self.budget.limit) {
            self.denied.store(true, Ordering::Relaxed);
            usage.denials = usage.denials.saturating_add(1);
            return Ok(false);
        }
        usage.used += delta;
        self.charged = desired;
        self.pending_old = Some(current);
        Ok(true)
    }

    fn memory_grow_failed(&mut self, error: &wasmi::errors::MemoryError) -> std::result::Result<(), LimiterError> {
        if let Some(old) = self.pending_old.take() {
            let mut usage = self.budget.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            usage.used = usage.used.saturating_sub(self.charged.saturating_sub(old));
            self.charged = old;
        }
        self.limits.memory_grow_failed(error)
    }

    fn table_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> std::result::Result<bool, LimiterError> {
        self.limits.table_growing(current, desired, maximum)
    }

    fn table_grow_failed(&mut self, error: &wasmi::errors::TableError) -> std::result::Result<(), LimiterError> {
        self.limits.table_grow_failed(error)
    }

    fn instances(&self) -> usize {
        self.limits.instances()
    }
    fn tables(&self) -> usize {
        self.limits.tables()
    }
    fn memories(&self) -> usize {
        self.limits.memories()
    }
}

/// A loaded, validated plug-in. Cheap to share (`Arc`) and safe to run from several threads:
/// every run gets its own store and instance.
pub struct Plugin {
    manifest: Manifest,
    engine: Engine,
    module: Module,
    limits: Limits,
    size: usize,
    source: Option<String>,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin").field("manifest", &self.manifest).field("size", &self.size).field("source", &self.source).finish()
    }
}

/// When a run must stop: a wall-clock deadline (native) and a flag shared by parallel bands.
#[derive(Clone)]
pub(crate) struct Deadline {
    #[cfg(not(target_arch = "wasm32"))]
    at: std::time::Instant,
    abort: Arc<Mutex<Option<Error>>>,
}

impl Deadline {
    pub(crate) fn new(limits: &Limits) -> Self {
        #[cfg(target_arch = "wasm32")]
        let _ = limits;
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            at: std::time::Instant::now() + std::time::Duration::from_millis(limits.wall_time_ms),
            abort: Arc::new(Mutex::new(None)),
        }
    }
    pub(crate) fn abort(&self, error: Error) {
        // Publish the cause with the cancellation state so siblings cannot replace it with a
        // generic cancellation error, even when results are collected in a different order.
        self.abort.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert(error);
    }
    /// Keep the run's wall-clock limit while isolating cancellation for a retried chunk.
    pub(crate) fn attempt(&self) -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            at: self.at,
            abort: Arc::new(Mutex::new(None)),
        }
    }
    fn check(&self) -> Result<()> {
        if let Some(error) = self.abort.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone() {
            return Err(error);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if std::time::Instant::now() >= self.at {
            let error = Error::Limit("time budget".into());
            self.abort(error.clone());
            return Err(error);
        }
        Ok(())
    }
}

/// One live instance: a store with its own memory limit, and the module's exports.
pub(crate) struct Live {
    store: Store<BandLimiter>,
    instance: Instance,
    memory: Memory,
}

fn wasm_err(e: wasmi::Error) -> Error {
    if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) { Error::Limit("instruction budget".into()) } else { Error::Trap(e.to_string()) }
}

/// Reads an unsigned LEB128 `u32` at the start of `bytes`: the value and its length.
fn leb_u32(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut value = 0u32;
    for (i, &b) in bytes.iter().take(5).enumerate() {
        value |= u32::from(b & 0x7f).checked_shl(7 * i as u32)?;
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

/// Refuses a code section that declares no functions but more bytes than that count. The spec
/// makes the count the whole section then; wasmi 2.0's strict limits divide the section size by
/// the function count before validating it, so such a module (crafted or truncated) panicked.
/// Every code section is checked, since a duplicate one also reaches that division before
/// validation refuses it. Anything else, malformed or not, is left to wasmi.
fn check_code_section(bytes: &[u8]) -> Result<()> {
    if bytes.get(4..8) != Some(&[1, 0, 0, 0][..]) {
        return Ok(());
    }
    let mut pos = 8usize;
    while let Some(&id) = bytes.get(pos) {
        let Some((size, len)) = bytes.get(pos + 1..).and_then(leb_u32) else { return Ok(()) };
        let start = pos + 1 + len;
        if id == 10
            && let Some((0, count_len)) = bytes.get(start..).and_then(leb_u32)
            && size as usize > count_len
        {
            return Err(Error::Module(format!("the code section declares no functions but {size} bytes")));
        }
        pos = start.saturating_add(size as usize);
    }
    Ok(())
}

/// What a file that is not WebAssembly actually is, judged from its magic number, so the install
/// error can name the format the person picked instead of only the one we wanted (public format
/// facts: the PE/COFF, ELF, Mach-O and zip signatures). `None` for anything else. Reads with
/// `starts_with`/`get`, so a file shorter than the signature is fine.
fn format_hint(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"MZ") {
        return Some("a Windows native executable or library (a .dll, or a Photoshop plug-in such as .8bf/.8bt)");
    }
    if bytes.get(..4) == Some(&[0x7f, b'E', b'L', b'F']) {
        return Some("a Linux native library (ELF)");
    }
    if matches!(bytes.get(..4), Some(&[0xce, 0xfa, 0xed, 0xfe] | &[0xcf, 0xfa, 0xed, 0xfe] | &[0xca, 0xfe, 0xba, 0xbe] | &[0xbe, 0xba, 0xfe, 0xca])) {
        return Some("a macOS native binary (Mach-O, thin or fat)");
    }
    if matches!(bytes.get(..4), Some(&[0x50, 0x4b, 0x03, 0x04] | &[0x50, 0x4b, 0x05, 0x06])) {
        return Some("a zip archive");
    }
    None
}

impl Plugin {
    /// Validates and loads a module: checks the ABI version and reads the manifest.
    pub fn load(bytes: &[u8], limits: Limits) -> Result<Plugin> {
        if bytes.len() > limits.max_module_bytes {
            return Err(Error::Module(format!("module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        if !bytes.starts_with(b"\0asm") {
            let msg = "not a WebAssembly binary (missing \\0asm header)";
            return Err(Error::Module(match format_hint(bytes) {
                Some(hint) => format!("{msg}; this file looks like {hint} - PhotoCraft plug-ins are WebAssembly modules (.wasm)"),
                None => msg.to_string(),
            }));
        }
        check_code_section(bytes)?;
        let mut config = Config::default();
        config.consume_fuel(true).enforced_limits(EnforcedLimits::strict()).set_max_recursion_depth(limits.max_recursion_depth.max(16));
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| Error::Module(e.to_string()))?;
        if let Some(imp) = module.imports().next() {
            return Err(Error::Abi(format!(
                "the module imports `{}.{}`, but plug-ins get no host imports (no WASI, files or network)",
                imp.module(),
                imp.name()
            )));
        }
        let mut p = Plugin { manifest: placeholder_manifest(), engine, module, limits, size: bytes.len(), source: None };
        let deadline = Deadline::new(&p.limits);
        let mut live = p.instantiate(&deadline, RunBudget::new(&p.limits), Arc::new(AtomicBool::new(false)))?;
        let version: TypedFunc<(), i32> = live.func("pc_abi_version")?;
        let v = p.call(&mut live, &version, (), p.limits.fuel_base, &deadline)?;
        if v != ABI_VERSION {
            return Err(Error::Abi(format!("pc_abi_version returned {v}; this host implements version {ABI_VERSION}")));
        }
        let manifest: TypedFunc<(), i64> = live.func("pc_manifest")?;
        let packed = p.call(&mut live, &manifest, (), p.limits.fuel_base, &deadline)? as u64;
        let (ptr, len) = ((packed & 0xffff_ffff) as usize, (packed >> 32) as usize);
        if len > crate::manifest::MAX_MANIFEST_BYTES {
            return Err(Error::Manifest(format!("manifest is {len} bytes (limit {})", crate::manifest::MAX_MANIFEST_BYTES)));
        }
        let text = live.memory.data(&live.store).get(ptr..ptr.saturating_add(len)).ok_or_else(|| Error::Abi("pc_manifest points outside memory".into()))?;
        p.manifest = Manifest::parse(text)?;
        // The other exports must exist with the right types.
        live.func::<i32, i32>("pc_alloc")?;
        live.func::<FilterArgs, i32>("pc_filter")?;
        Ok(p)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn id(&self) -> &str {
        &self.manifest.id
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    /// Module size in bytes.
    pub fn size(&self) -> usize {
        self.size
    }
    /// Where the module was loaded from, if it came from a file.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub(crate) fn instantiate(&self, deadline: &Deadline, budget: Arc<RunBudget>, denied: Arc<AtomicBool>) -> Result<Live> {
        deadline.check()?;
        let mut store = Store::new(&self.engine, BandLimiter::new(&self.limits, budget, denied));
        store.limiter(|l| l);
        // The start function (if any) runs on this fuel. It cannot be interrupted to check the
        // deadline the way a call can, so it gets no more than one slice, however much a call may use.
        store.set_fuel(self.limits.fuel_base.min(FUEL_SLICE)).map_err(wasm_err)?;
        let linker = Linker::<BandLimiter>::new(&self.engine);
        let instance = linker.instantiate_and_start(&mut store, &self.module).map_err(wasm_err)?;
        let memory = instance.get_memory(&store, "memory").ok_or_else(|| Error::Abi("the module must export its linear memory as `memory`".into()))?;
        Ok(Live { store, instance, memory })
    }

    /// Calls `f` with at most `fuel` instructions, in slices, checking the deadline between them.
    pub(crate) fn call<P: WasmParams, R: WasmResults>(&self, live: &mut Live, f: &TypedFunc<P, R>, params: P, fuel: u64, deadline: &Deadline) -> Result<R> {
        let mut left = fuel;
        let give = left.min(FUEL_SLICE);
        left -= give;
        live.store.set_fuel(give).map_err(wasm_err)?;
        let mut call = f.call_resumable(&mut live.store, params).map_err(wasm_err)?;
        loop {
            match call {
                TypedResumableCall::Finished(r) => return Ok(r),
                TypedResumableCall::HostTrap(_) => return Err(Error::Trap("unexpected host trap".into())),
                TypedResumableCall::OutOfFuel(inv) => {
                    let need = inv.required_fuel();
                    if left == 0 || need > left {
                        return Err(Error::Limit("instruction budget".into()));
                    }
                    deadline.check()?;
                    let give = left.min(FUEL_SLICE.max(need));
                    left -= give;
                    live.store.set_fuel(give).map_err(wasm_err)?;
                    call = inv.resume(&mut live.store).map_err(wasm_err)?;
                }
            }
        }
    }

    /// Runs `pc_filter` on `data` (interleaved `f32`, `w × h × channels`) in a fresh instance.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn filter_buffer(
        &self,
        data: &mut [f32],
        w: u32,
        h: u32,
        channels: u32,
        format: u32,
        params: &[u8],
        deadline: &Deadline,
        budget: Arc<RunBudget>,
        denied: Arc<AtomicBool>,
    ) -> Result<()> {
        let bytes = data.len().checked_mul(4).filter(|b| *b <= i32::MAX as usize && *b + params.len() <= self.limits.max_memory_bytes);
        let bytes = bytes.ok_or_else(|| Error::Limit("memory budget (band too large)".into()))?;
        let mut live = self.instantiate(deadline, budget, denied)?;
        let alloc: TypedFunc<i32, i32> = live.func("pc_alloc")?;
        let filter: TypedFunc<FilterArgs, i32> = live.func("pc_filter")?;
        let ptr = self.call(&mut live, &alloc, bytes as i32, self.limits.fuel_base, deadline)? as u32 as usize;
        let pptr = self.call(&mut live, &alloc, params.len().max(1) as i32, self.limits.fuel_base, deadline)? as u32 as usize;
        if ptr == 0 || pptr == 0 {
            return Err(Error::Failed("pc_alloc returned 0 (out of memory)".into()));
        }
        let outside = || Error::Abi("pc_alloc returned a block outside memory".into());
        let end = ptr.checked_add(bytes).ok_or_else(outside)?;
        let pend = pptr.checked_add(params.len()).ok_or_else(outside)?;
        {
            let mem = live.memory.data_mut(&mut live.store);
            let dst = mem.get_mut(ptr..end).ok_or_else(outside)?;
            for (d, v) in dst.as_chunks_mut::<4>().0.iter_mut().zip(data.iter()) {
                *d = v.to_le_bytes();
            }
            let pdst = mem.get_mut(pptr..pend).ok_or_else(outside)?;
            pdst.copy_from_slice(params);
        }
        let fuel = self.limits.fuel_base.saturating_add((data.len() as u64).saturating_mul(self.limits.fuel_per_sample));
        let args = (ptr as i32, bytes as i32, w as i32, h as i32, channels as i32, format as i32, pptr as i32, params.len() as i32);
        let rc = self.call(&mut live, &filter, args, fuel, deadline)?;
        if rc != 0 {
            return Err(Error::Failed(format!("pc_filter returned error code {rc}")));
        }
        let mem = live.memory.data(&live.store);
        let src = mem.get(ptr..end).ok_or_else(|| Error::Abi("pixel buffer no longer in memory".into()))?;
        for (v, s) in data.iter_mut().zip(src.as_chunks::<4>().0) {
            *v = f32::from_le_bytes(*s);
        }
        Ok(())
    }
}

/// `pc_filter(buf_ptr, buf_len, width, height, channels, format, params_ptr, params_len)`.
type FilterArgs = (i32, i32, i32, i32, i32, i32, i32, i32);

impl Live {
    fn func<P: WasmParams, R: WasmResults>(&self, name: &str) -> Result<TypedFunc<P, R>> {
        self.instance.get_typed_func::<P, R>(&self.store, name).map_err(|e| Error::Abi(format!("export `{name}`: {e}")))
    }
}

fn placeholder_manifest() -> Manifest {
    Manifest {
        id: String::new(),
        name: String::new(),
        version: String::new(),
        kind: crate::Kind::Filter,
        description: String::new(),
        author: String::new(),
        params: Vec::new(),
        overlap: 0,
        area: crate::Area::Content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_budget_charges_actual_growth_and_releases_on_drop() {
        let limits = Limits { max_memory_bytes: 8 << 20, max_total_memory_bytes: 10 << 20, ..Limits::default() };
        let budget = RunBudget::new(&limits);
        let mut a = BandLimiter::new(&limits, Arc::clone(&budget), Arc::new(AtomicBool::new(false)));
        let mut b = BandLimiter::new(&limits, Arc::clone(&budget), Arc::new(AtomicBool::new(false)));
        assert!(matches!(a.memory_growing(0, 4 << 20, None), Ok(true)));
        assert!(matches!(b.memory_growing(0, 4 << 20, None), Ok(true)));
        assert_eq!(budget.usage.lock().unwrap().used, 8 << 20);
        assert!(matches!(a.memory_growing(4 << 20, 7 << 20, None), Ok(false)));
        assert_eq!(budget.usage.lock().unwrap().used, 8 << 20);
        assert_eq!(budget.snapshot(), (0, 1));
        drop(b);
        assert_eq!(budget.snapshot(), (4 << 20, 1));
        assert!(matches!(a.memory_growing(4 << 20, 7 << 20, None), Ok(true)));
        assert_eq!(budget.usage.lock().unwrap().used, 7 << 20);
        drop(a);
        assert_eq!(budget.usage.lock().unwrap().used, 0);
    }

    #[test]
    fn failed_growth_refunds_reservation_and_instance_cap_still_applies() {
        let limits = Limits { max_memory_bytes: 8 << 20, max_total_memory_bytes: 10 << 20, ..Limits::default() };
        let budget = RunBudget::new(&limits);
        let mut a = BandLimiter::new(&limits, Arc::clone(&budget), Arc::new(AtomicBool::new(false)));
        assert!(matches!(a.memory_growing(0, 2 << 20, None), Ok(true)));
        assert!(matches!(a.memory_growing(2 << 20, 7 << 20, None), Ok(true)));
        a.memory_grow_failed(&wasmi::errors::MemoryError::OutOfSystemMemory).unwrap();
        assert_eq!(budget.usage.lock().unwrap().used, 2 << 20);
        assert_eq!(budget.snapshot().0, 0, "in-flight growth is not a measured peak");
        assert!(matches!(a.memory_growing(2 << 20, 9 << 20, None), Ok(false)));
        assert_eq!(budget.snapshot().1, 0, "individual cap is separate from the run budget");
        drop(a);
        assert_eq!(budget.snapshot().0, 2 << 20, "failed growth does not inflate later group widths");
        assert_eq!(budget.usage.lock().unwrap().used, 0);
    }

    #[test]
    fn observed_footprint_can_fill_all_workers() {
        let limits = Limits::default();
        assert_eq!(limits.parallel_bands(16), 4, "the public conservative estimate remains available");
        assert_eq!(limits.parallel_bands_for(16, 20 << 20), 16);
        assert_eq!(limits.parallel_bands_for(16, 512 << 20), 4);
        let larger_instance = Limits { max_memory_bytes: 1 << 30, ..limits };
        assert_eq!(larger_instance.parallel_bands_for(16, 20 << 20), 16);
    }
}
