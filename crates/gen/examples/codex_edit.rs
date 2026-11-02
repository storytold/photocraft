//! Explicit live subscription smoke test; never run by cargo test.
#[cfg(not(target_arch = "wasm32"))]
use photocraft_gen::{EditRequest, ImageProvider, codex::CodexProvider};

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("usage: codex_edit <canvas.png> <instruction>")?;
    let prompt = args.next().ok_or("missing instruction")?;
    let image = std::fs::canonicalize(input)?;
    let destination = std::env::current_dir()?.join("codex-edit-result.png");
    if destination.exists() {
        return Err("codex-edit-result.png already exists; move it before testing again".into());
    }
    let workspace = tempfile::Builder::new().prefix("photocraft-ai-smoke-").tempdir()?;
    let result = CodexProvider::default().edit(&EditRequest { image: &image, prompt: &prompt, workspace: workspace.path() }, &|| false)?;
    // Copy only our generated image; the runtime owns the original savedPath.
    std::fs::copy(result.saved_path, &destination)?;
    println!("{}", destination.display());
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Codex image editing requires the desktop runtime".into())
}
