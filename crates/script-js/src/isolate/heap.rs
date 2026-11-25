//! The script host's heap cap, the same on every platform: the executable that serves runs
//! installs [`HostAllocator`] as its global allocator, and a run's memory limit caps it, so an
//! allocation past the limit fails the moment it is asked for (see `limits` for what each
//! desktop system adds).

/// The allocator an executable that serves runs installs as its global allocator
/// ([`host_allocator`]), so a run can cap the heap. Uncapped, it passes every allocation to the
/// system's.
pub type HostAllocator = cap::Cap<std::alloc::System>;

/// A [`HostAllocator`] with no limit, for `#[global_allocator]`.
pub const fn host_allocator() -> HostAllocator {
    cap::Cap::new(std::alloc::System, usize::MAX)
}

/// Cap `heap` at `bytes`.
///
/// # Errors
/// The heap already holds more than that.
pub fn cap_heap(heap: &HostAllocator, bytes: u64) -> Result<(), String> {
    let budget = usize::try_from(bytes).unwrap_or(usize::MAX);
    heap.set_limit(budget).map_err(|()| format!("the host already uses more than {} MiB", budget >> 20))
}
