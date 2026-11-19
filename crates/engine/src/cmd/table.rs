//! The command table's fragments, one directory per owner: the crate whose items the commands
//! call. Each directory moves whole into its crate; `SPECS` in `cmd.rs` fixes the order.

pub(crate) mod core;
pub(crate) mod image;
pub(crate) mod model;
pub(crate) mod paint;
pub(crate) mod store;
