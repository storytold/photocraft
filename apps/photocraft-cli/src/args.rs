//! The command line, as clap types. Values are parsed into their types here, so a bad one is a
//! usage error naming its flag (exit 2) and the handlers never see it (#7071). A flag a
//! subcommand doesn't take is a usage error too, so a typo such as `--fromat` can't be silently
//! ignored (#423), and so is a repeated one: `--quality 80 --quality 60` is more likely a mistake
//! than a choice. A value starting with `-` is written `--out=-image.png` (or `./-image.png`), so a
//! value flag never takes the next flag as its value; negative numbers are kept where they mean
//! something.

use std::ffi::OsString;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::parser::ValueSource;
use clap::{ArgAction, ArgGroup, ArgMatches, Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use photocraft_automation::args::{AutomationArgs, ControlArgs, blank_env_is_unset, conflict, control_port, name_env_vars};
use photocraft_codecs::{JpegFormat, TiffCompression};
use photocraft_io::{DdsCompression, DpxBits, TgaBits};
use serde_json::Value;

/// photocraft-cli: headless PhotoCraft
#[derive(Parser)]
#[command(name = "photocraft-cli", version = photocraft_engine::build_info::long_version_static(), arg_required_else_help = true)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    Convert(ConvertArgs),
    Info(InfoArgs),
    Run(RunArgs),
    Batch(BatchArgs),
    Droplet(DropletArgs),
    Commands(CommandsArgs),
    Mcp(McpArgs),
    Serve(ServeArgs),
}

// Explicit decoding policy shared by every file-reading subcommand.
#[derive(Args)]
pub(crate) struct ImportArgs {
    /// Explicit sample byte order for unmarked high-depth XCF7-11; never inferred
    #[arg(long, value_name = "little-endian|big-endian", value_parser = historical_xcf_sample_order)]
    pub(crate) historical_xcf_sample_order: Option<photocraft_io::HistoricalXcfSampleOrder>,
}

fn historical_xcf_sample_order(value: &str) -> Result<photocraft_io::HistoricalXcfSampleOrder, &'static str> {
    match value {
        "little-endian" => Ok(photocraft_io::HistoricalXcfSampleOrder::LittleEndian),
        "big-endian" => Ok(photocraft_io::HistoricalXcfSampleOrder::BigEndian),
        _ => Err("expected little-endian or big-endian"),
    }
}

// How an image is written: shared by `convert`, `run` and `batch`.
#[derive(Args)]
pub(crate) struct ExportArgs {
    /// Output format, overriding the output's extension (png, jpg, tif, webp, …)
    #[arg(long, value_name = "EXT")]
    pub(crate) format: Option<String>,
    /// JPEG or WebP quality, 1-100. A WebP written with a quality is lossy, without one lossless
    // allow_negative_numbers makes `--quality -1` an out-of-range value rather than an unexpected argument.
    #[arg(long, value_name = "1-100", value_parser = clap::value_parser!(u8).range(1..=100), allow_negative_numbers = true)]
    pub(crate) quality: Option<u8>,
    /// Keep the layers in TIFF output (Photoshop layer data); TIFF output is flat without it
    #[arg(long)]
    pub(crate) tiff_layers: bool,
    /// Compress TIFF output: none (the default), lzw or zip
    #[arg(long, value_name = "none|lzw|zip", value_parser = tiff_compression)]
    pub(crate) tiff_compression: Option<TiffCompression>,
    /// A TGA's bits per pixel: 32 writes the alpha channel (or else the transparency) as its
    /// alpha, 24 none. Without it: 32 with an alpha channel or transparency, else 24
    #[arg(long, value_name = "24|32", value_parser = tga_bits)]
    pub(crate) tga_bits: Option<TgaBits>,
    /// A DPX's bits per sample. Without it: 8 for an 8-bit document, else 16
    #[arg(long, value_name = "8|10|12|16", value_parser = dpx_bits)]
    pub(crate) dpx_bits: Option<DpxBits>,
    /// A DDS texture's compression: none (uncompressed, the default), bc1 (DXT1: alpha on or
    /// off), bc2 (DXT3) or bc3 (DXT5)
    #[arg(long, value_name = "none|bc1|bc2|bc3", value_parser = dds_compression)]
    pub(crate) dds_compression: Option<DdsCompression>,
    /// Also write a DDS texture's full mip chain
    #[arg(long)]
    pub(crate) dds_mipmaps: bool,
    /// A JPEG's scans, as JPEG Options picks them: baseline (the default), optimized (Huffman
    /// tables fitted to the image; RGB is written 4:2:0 and keeps the standard tables, so only Gray
    /// and CMYK shrink) or progressive
    #[arg(long, value_name = "baseline|optimized|progressive", value_parser = jpeg_format)]
    pub(crate) jpeg_format: Option<JpegFormat>,
}

/// Convert between formats (.pcraft, .psd, .png, .jpg, .tif, .webp, .exr, …)
///
/// An <OUT> that is the <IN> file is refused, as the result would replace the original;
/// --in-place allows it.
#[derive(Args)]
pub(crate) struct ConvertArgs {
    #[command(flatten)]
    pub(crate) import: ImportArgs,
    /// The file to read
    #[arg(value_name = "IN")]
    pub(crate) input: PathBuf,
    /// The file to write; its extension picks the format unless --format is given
    #[arg(value_name = "OUT")]
    pub(crate) output: PathBuf,
    /// Allow an <OUT> that is the <IN> file, replacing the original
    #[arg(long)]
    pub(crate) in_place: bool,
    #[command(flatten)]
    pub(crate) export: ExportArgs,
}

/// Print the document as JSON (size, mode, depth, layer tree)
#[derive(Args)]
pub(crate) struct InfoArgs {
    #[command(flatten)]
    pub(crate) import: ImportArgs,
    pub(crate) file: PathBuf,
    /// One line of JSON instead of pretty-printed
    #[arg(long)]
    pub(crate) compact: bool,
}

/// Open a file (or start a new document), run engine commands in order, save the result
///
/// Prints each command's JSON result. Give exactly one of <FILE> or --new, and at least one --cmd
/// or --out. An --out that is the <FILE> is refused, as the result would replace the original;
/// --in-place allows it.
#[derive(Args)]
#[command(group(ArgGroup::new("source").args(["file", "new"]).required(true)))]
#[command(group(ArgGroup::new("work").args(["cmd", "out"]).required(true).multiple(true)))]
pub(crate) struct RunArgs {
    #[command(flatten)]
    pub(crate) import: ImportArgs,
    /// The document to open
    pub(crate) file: Option<PathBuf>,
    /// Start from a new document with these JSON parameters instead of opening a file
    #[arg(long, value_name = "JSON", value_parser = json)]
    pub(crate) new: Option<Value>,
    /// An engine command to run; repeat for several, run in order
    #[arg(long, value_name = "ID", action = ArgAction::Append)]
    pub(crate) cmd: Vec<String>,
    /// JSON parameters for the --cmd before it (default {})
    #[arg(long, value_name = "JSON", action = ArgAction::Append, value_parser = json)]
    pub(crate) params: Vec<Value>,
    /// Save the result to this file
    #[arg(long, value_name = "FILE")]
    pub(crate) out: Option<PathBuf>,
    /// Allow an --out that is the <FILE>, replacing the original
    #[arg(long)]
    pub(crate) in_place: bool,
    #[command(flatten)]
    pub(crate) export: ExportArgs,
    /// `--cmd` and `--params` values in command-line order, set by [`parse`].
    #[arg(skip)]
    pub(crate) steps: Vec<Step>,
}

/// One `--cmd` or `--params` of `run`, in command-line order.
pub(crate) enum Step {
    Cmd(String),
    Params(Value),
}

/// Apply an action list to every image in a directory
///
/// A Photoshop Actions file (.atn) is a set: its action --action names plays, as below. JSON steps
/// are [id, params] pairs, {"command": id, "params": {…}} objects or bare ids, as a recorded
/// action or droplet stores them (a list, or wrapped in {"actions": …}, {"steps": …} or a
/// droplet). An Actions set (the Actions panel's actions.json, {"actions": [{"name", "steps"}…]})
/// plays the action --action names (needed when the set holds more than one) with the whole set
/// loaded, so its steps can play other actions of the set.
#[derive(Args)]
pub(crate) struct BatchArgs {
    #[command(flatten)]
    pub(crate) import: ImportArgs,
    /// The action list: actions.json, a droplet, or a Photoshop Actions file (.atn)
    #[arg(long, value_name = "FILE")]
    pub(crate) actions: PathBuf,
    /// Which action of an Actions set to play
    #[arg(long, value_name = "NAME")]
    pub(crate) action: Option<String>,
    /// The folder of images to process
    #[arg(long = "in", value_name = "DIR")]
    pub(crate) in_dir: PathBuf,
    /// The folder to write the results to, as <stem>.<ext>
    #[arg(long, value_name = "DIR")]
    pub(crate) out: PathBuf,
    /// An --out folder that is the --in folder is refused, as the results would replace the
    /// originals; --in-place allows it
    #[arg(long)]
    pub(crate) in_place: bool,
    #[command(flatten)]
    pub(crate) export: ExportArgs,
}

/// Run a droplet (File › Automate › Create Droplet) on images and folders
#[derive(Args)]
pub(crate) struct DropletArgs {
    /// The droplet to run
    #[arg(value_name = "FILE.pcdroplet")]
    pub(crate) file: String,
    /// Images and folders to run it on
    #[arg(value_name = "FILE-OR-DIR", required = true)]
    pub(crate) inputs: Vec<String>,
    /// The folder to write the results to
    #[arg(long, value_name = "DIR")]
    pub(crate) out: Option<String>,
}

/// List the engine command registry
#[derive(Args)]
pub(crate) struct CommandsArgs {
    /// Print the entries as JSON
    #[arg(long)]
    pub(crate) json: bool,
    /// Only the commands matching this text
    #[arg(long, value_name = "TEXT", allow_negative_numbers = true)]
    pub(crate) filter: Option<String>,
}

/// Run the MCP server on stdio (headless engine, or bridge to a running `photocraft --control <port>`)
#[derive(Args)]
pub(crate) struct McpArgs {
    /// Bridge to the PhotoCraft app listening here instead of a headless engine; a bare port means
    /// 127.0.0.1:<PORT>
    #[arg(long, value_name = "127.0.0.1:PORT", env = "PHOTOCRAFT_CONTROL_PORT", value_parser = bridge)]
    pub(crate) bridge: Option<Bridge>,
    /// `--bridge` came from PHOTOCRAFT_CONTROL_PORT, set by [`parse`].
    #[arg(skip)]
    pub(crate) bridge_from_env: bool,
    #[command(flatten)]
    pub(crate) control: ControlArgs,
    #[command(flatten)]
    pub(crate) automation: AutomationArgs,
}

/// Keep one headless session open and answer JSON lines on stdio, or on 127.0.0.1:<PORT>
///
/// Each request is a JSON line {"id", "method", "params"}. Methods: engine.execute,
/// jobs.list/cancel, engine.commands,
/// doc.open/new/place/save/exportLayers/exportDataSets/importDataSets/inspect/render/select/close,
/// session.list, batch, methods (docs/control-protocol.md#headless-server).
#[derive(Args)]
pub(crate) struct ServeArgs {
    /// Listen on 127.0.0.1:<PORT> instead of stdio
    #[arg(long, value_name = "PORT", env = "PHOTOCRAFT_CONTROL_PORT", value_parser = control_port)]
    pub(crate) port: Option<u16>,
    /// `--port` came from PHOTOCRAFT_CONTROL_PORT, set by [`parse`].
    #[arg(skip)]
    pub(crate) port_from_env: bool,
    #[command(flatten)]
    pub(crate) control: ControlArgs,
    #[command(flatten)]
    pub(crate) automation: AutomationArgs,
}

/// Why a value was refused: clap prints it after the flag and its value (exit 2).
#[derive(Debug, thiserror::Error)]
enum ValueError {
    #[error("expected 24 or 32")]
    TgaBits,
    #[error("expected 8, 10, 12 or 16")]
    DpxBits,
    #[error("expected none, bc1, bc2 or bc3")]
    DdsCompression,
    #[error("expected none, lzw or zip")]
    TiffCompression,
    #[error("expected baseline, optimized or progressive")]
    JpegFormat,
    /// Why a `--bridge` value, or a PHOTOCRAFT_CONTROL_PORT value for `mcp`, is refused.
    #[error("expected a port, 0-65535 (--bridge also takes 127.0.0.1:<PORT>)")]
    Bridge,
}

fn tga_bits(s: &str) -> Result<TgaBits, ValueError> {
    s.parse().ok().and_then(TgaBits::from_bits).ok_or(ValueError::TgaBits)
}

fn dpx_bits(s: &str) -> Result<DpxBits, ValueError> {
    s.parse().ok().and_then(DpxBits::from_bits).ok_or(ValueError::DpxBits)
}

fn dds_compression(s: &str) -> Result<DdsCompression, ValueError> {
    DdsCompression::from_name(s).ok_or(ValueError::DdsCompression)
}

fn tiff_compression(s: &str) -> Result<TiffCompression, ValueError> {
    match s {
        "none" => Ok(TiffCompression::None),
        "lzw" => Ok(TiffCompression::Lzw),
        "zip" => Ok(TiffCompression::Deflate),
        _ => Err(ValueError::TiffCompression),
    }
}

fn jpeg_format(s: &str) -> Result<JpegFormat, ValueError> {
    JpegFormat::parse(s).ok_or(ValueError::JpegFormat)
}

/// Where `mcp --bridge` connects. Shown as the address: `127.0.0.1:<PORT>` for a port.
#[derive(Clone)]
pub(crate) enum Bridge {
    /// A bare port: the app on 127.0.0.1.
    Port(u16),
    /// `host:port`, as given. Whether the host is loopback is the bridge's to say.
    Addr(String),
}

impl fmt::Display for Bridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Bridge::Port(port) => SocketAddr::from((Ipv4Addr::LOCALHOST, *port)).fmt(f),
            Bridge::Addr(addr) => f.write_str(addr),
        }
    }
}

/// `mcp --bridge`, or PHOTOCRAFT_CONTROL_PORT, which holds a port.
fn bridge(s: &str) -> Result<Bridge, ValueError> {
    match control_port(s) {
        Ok(port) => Ok(Bridge::Port(port)),
        Err(_) if s.contains(':') => Ok(Bridge::Addr(s.to_owned())),
        Err(_) => Err(ValueError::Bridge),
    }
}

/// `--new` and `--params`: bad JSON is a usage error, before any work. Whether the value is an
/// object is the command's to say.
fn json(s: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(s)
}

/// `--cmd` and `--params` values back in command-line order: clap keeps each flag's values apart,
/// their indices say how they interleave.
fn steps(m: &ArgMatches) -> Vec<Step> {
    let mut steps: Vec<(usize, Step)> = Vec::new();
    if let (Some(at), Ok(Some(ids))) = (m.indices_of("cmd"), m.try_get_many::<String>("cmd")) {
        steps.extend(at.zip(ids).map(|(i, id)| (i, Step::Cmd(id.clone()))));
    }
    if let (Some(at), Ok(Some(params))) = (m.indices_of("params"), m.try_get_many::<Value>("params")) {
        steps.extend(at.zip(params).map(|(i, p)| (i, Step::Params(p.clone()))));
    }
    steps.sort_by_key(|s| s.0);
    steps.into_iter().map(|(_, s)| s).collect()
}

/// Parse the arguments (without the binary name). clap reads them as `OsString`s: a path need not
/// be valid Unicode (#1108), text must. A flag left out falls back to its environment variable,
/// unless that is empty, and a bad value from one is a usage error naming the variable (#2491).
pub(crate) fn parse(args: &[OsString]) -> Result<Cli, clap::Error> {
    let mut cmd = Cli::command().mut_subcommands(|sub| sub.mut_args(blank_env_is_unset));
    let argv = std::iter::once(OsString::from("photocraft-cli")).chain(args.iter().cloned());
    let matches = match cmd.try_get_matches_from_mut(argv) {
        // Help where a flag's value should be (`--out --help`) is still help: clap reports the
        // missing value first.
        Err(e) if e.kind() == ErrorKind::InvalidValue && args.iter().take_while(|a| *a != "--").any(|a| *a == "--help" || *a == "-h") => {
            let help = [OsString::from("photocraft-cli"), args.first().cloned().unwrap_or_default(), OsString::from("--help")];
            return Err(Cli::command().try_get_matches_from(help).err().unwrap_or(e));
        }
        matches => matches.map_err(|e| name_env_vars(e, &cmd, args))?,
    };
    let mut cli = Cli::from_arg_matches(&matches)?;
    let from_env = |m: &ArgMatches, id: &str| m.value_source(id) == Some(ValueSource::EnvVariable);
    match (&mut cli.command, matches.subcommand()) {
        (Command::Run(r), Some((_, m))) => r.steps = steps(m),
        (Command::Mcp(a), Some((name, m))) => {
            a.bridge_from_env = from_env(m, "bridge");
            // The variable holds a port, as for the desktop app's --control: only the flag takes
            // host:port, which `bridge` lets through.
            if a.bridge_from_env
                && let Some(Bridge::Addr(v)) = &a.bridge
            {
                return Err(cmd.error(ErrorKind::ValueValidation, format!("invalid value '{v}' for 'PHOTOCRAFT_CONTROL_PORT': {}", ValueError::Bridge)));
            }
            if let Some(sub) = cmd.find_subcommand_mut(name) {
                // Only the bridge reads a token, so a stale exported pair doesn't stop a headless
                // run.
                if a.bridge.is_some() {
                    conflict(sub, m, "control_token", "control_token_file")?;
                }
                // The roots are the headless engine's: the variable switching to the bridge would
                // drop them.
                if a.bridge_from_env {
                    conflict(sub, m, "automation_read_root", "bridge")?;
                    conflict(sub, m, "automation_write_root", "bridge")?;
                }
            }
            check_server(cmd.find_subcommand_mut(name), m, "bridge")?;
        }
        (Command::Serve(a), Some((name, m))) => {
            a.port_from_env = from_env(m, "port");
            // Only the TCP server reads a token, so a stale exported pair doesn't stop stdio.
            if a.port.is_some()
                && let Some(sub) = cmd.find_subcommand_mut(name)
            {
                conflict(sub, m, "control_token", "control_token_file")?;
            }
            check_server(cmd.find_subcommand_mut(name), m, "port")?;
        }
        _ => {}
    }
    Ok(cli)
}

/// Refuse `mcp` and `serve` launch options that would otherwise be silently ignored (#8035): a
/// token flag without the bridge or port that reads it (flag or variable; a token variable alone
/// may be a stale export), and bridge roots, which belong to the desktop process: accepting them
/// here promises authority the bridge cannot set.
fn check_server(sub: Option<&mut clap::Command>, m: &ArgMatches, transport: &str) -> Result<(), clap::Error> {
    let Some(sub) = sub else { return Ok(()) };
    for (id, flag) in [("control_token", "--control-token"), ("control_token_file", "--control-token-file")] {
        if m.value_source(id) == Some(ValueSource::CommandLine) && !m.contains_id(transport) {
            return Err(sub.error(ErrorKind::MissingRequiredArgument, format!("{flag} requires --{transport}")));
        }
    }
    if transport == "bridge" && m.contains_id(transport) && (m.contains_id("automation_read_root") || m.contains_id("automation_write_root")) {
        let message = "configure automation roots on the desktop app; --automation-read-root and --automation-write-root require headless MCP mode";
        return Err(sub.error(ErrorKind::ArgumentConflict, message));
    }
    Ok(())
}
