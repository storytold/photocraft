//! Feature carrier for the Linux `test (without heif)` job.
//!
//! This crate has no behaviour of its own. Its manifest pins third-party features
//! to the set the workspace gate compiles with `--features heif`, so
//! `cargo nextest run -p photocraft-io` can reuse that cache. photocraft-io
//! depends on it only as a dev-dependency.

// The dependencies exist to unify features, not to be called from this crate.
#![allow(unused_crate_dependencies)]

#[cfg(test)]
mod tests {
    #[test]
    fn feature_carrier_links() {}
}
