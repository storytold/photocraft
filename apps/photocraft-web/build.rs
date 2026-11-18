//! The script Worker's stack: scripts nest as deep as on the desktop, whose script host gives a run
//! a 64 MiB stack. WebAssembly's default stack is 1 MiB, which a deeply nested script overflows
//! (a trap that ends the run). The page's own binary keeps the default.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|arch| arch == "wasm32") {
        println!("cargo:rustc-link-arg-bin=photocraft-script-worker=-zstack-size=67108864");
    }
}
