//! The command table's fragments, one directory per owner: the crate whose items the commands
//! call. Each directory moves whole into its crate; `TABLES` in `install.rs` fixes the order.

pub mod core;
pub mod image;
pub mod model;
pub mod store;
