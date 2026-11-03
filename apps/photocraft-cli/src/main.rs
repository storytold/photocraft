#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() {
    // comfy-photocraft: `photocraft-cli --engine …` is the native engine's worker process, so a
    // headless session has a worker next to it too.
    if std::env::args_os().nth(1).is_some_and(|a| a == photocraft_genai::native::ENGINE_FLAG) {
        let args: Vec<String> = std::env::args_os().skip(2).map(|a| a.to_string_lossy().into_owned()).collect();
        std::process::exit(photocraft_genai::native::engine_main(&args));
    }
    // May re-execute this process with glibc tuned to return freed document memory (#8743).
    photocraft_engine::malloc_tuning::apply();
    // `args_os`, not `args`: a non-Unicode argument (a Latin-1 file name on Linux) must reach clap,
    // not panic before anything is printed (issue #1108).
    let code = photocraft_cli::run(std::env::args_os().skip(1), &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
