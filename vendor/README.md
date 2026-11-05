# Vendored dependencies

## 1. edit-session 0.1.0-alpha.1

Edit Selected Guides uses this dependency-free draft state machine to validate a proposal against the guide and document revision before committing.
The snapshot comes from [upstream commit `28620b8264660ea84a182110cf5894da90281c42`](https://github.com/PerchAnton/edit-session/tree/28620b8264660ea84a182110cf5894da90281c42). The only local change to `src/lib.rs` adds the PhotoCraft Clippy deny attribute. The lifecycle tests and license files match upstream byte for byte. The manifest inherits PhotoCraft's workspace lints and links to this ledger. Local rustfmt settings retain upstream formatting. The snapshot is dual-licensed under [MIT](edit-session/LICENSE-MIT) or [Apache-2.0](edit-session/LICENSE-APACHE).

| Vendored file | SHA-256 |
| --- | --- |
| [`src/lib.rs`](edit-session/src/lib.rs) | `d92acb859aed63692bbbc0a8fce2e74aba9a6773d96c8d2e3cf5818e2cde6858` |
| [`tests/lifecycle.rs`](edit-session/tests/lifecycle.rs) | `e1cd12aa2527e58bd4d4a60f0956524ead0543d671ee7b051c1f923a97ec4bff` |
| [`LICENSE-APACHE`](edit-session/LICENSE-APACHE) | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| [`LICENSE-MIT`](edit-session/LICENSE-MIT) | `665a942f9317a358930b470ad522a22a58ff6c346db0395463ec84dd3e19d775` |

The upstream `src/lib.rs` SHA-256 before the lint attribute is `a71c518bfb706abb44c9565a5993dc9c24e65edaea25a426a8747e77832dafde`.

When updating the snapshot, review the upstream diff, update the revision and hashes here, and run `cargo test --locked -p edit-session` and the guide dialog tests.
