//! The script Worker's stack in its memory: 64 MiB, as the desktop script host's, where
//! WebAssembly's default is 1 MiB. The engine's frames also use the browser's own call stack,
//! which this doesn't change, so a script still nests less deeply in the browser than on the
//! desktop; past either, the run ends with a message. The page's own binary keeps the default.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|arch| arch == "wasm32") {
        println!("cargo:rustc-link-arg-bin=photocraft-script-worker=-zstack-size=67108864");
    }
}
