//! One filter invocation per process. Native code never enters the editor.
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(windows)]
mod memory;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod pixels;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod properties;
#[cfg(windows)]
mod record;
#[cfg(windows)]
mod response_pipe;
#[path = "../../../crates/plugins/src/native_8bf/wire.rs"]
mod wire;

fn main() {
    if let Err(e) = run() {
        eprintln!("PhotoCraft 8BF worker: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (request, pixels) = wire::read_frame::<wire::Request>(std::io::stdin().lock())?;
    if request.pixel_len()? != pixels.len() {
        return Err("8BF input buffer does not match its declared format and dimensions".into());
    }
    #[cfg(windows)]
    let output = response_pipe::reserve()?;
    #[cfg(not(windows))]
    let output = std::io::stdout();
    #[cfg(windows)]
    let result = native::run(&request, pixels);
    #[cfg(not(windows))]
    let result: Result<Vec<u8>, String> = {
        let _ = pixels;
        Err("This worker must be built for Windows; Linux launches it through Wine".into())
    };
    let (status, message, out) = match result {
        Ok(out) => ("ok", String::new(), out),
        Err(message) if message == "cancelled" => ("cancelled", message, Vec::new()),
        Err(message) => ("error", message, Vec::new()),
    };
    wire::write_frame(
        output,
        &wire::Response { status: status.into(), message, width: request.width, height: request.height, channels: request.channels, depth: request.depth },
        &out,
    )?;
    Ok(())
}
