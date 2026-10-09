//! Data-parallel helpers: rayon on native targets, sequential on wasm.

/// Calls `f(index, chunk)` for each `chunk`-sized piece of `buf`.
pub(crate) fn chunks_mut<T: Send>(buf: &mut [T], chunk: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    let chunk = chunk.max(1);
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    {
        use rayon::prelude::*;
        buf.par_chunks_mut(chunk).enumerate().for_each(|(i, c)| f(i, c));
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
    buf.chunks_mut(chunk).enumerate().for_each(|(i, c)| f(i, c));
}

/// Maps `f` over `0..n`, keeping order.
pub(crate) fn map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(f).collect()
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
    (0..n).map(f).collect()
}

/// Rows per parallel band for an image `width` samples wide (about 64 K samples a band).
pub(crate) fn band_rows(width: usize) -> usize {
    (65536 / width.max(1)).clamp(1, 256)
}
