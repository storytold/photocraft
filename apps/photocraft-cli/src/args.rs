use std::io::Write;

use crate::{
    R,
    automation::{commands, mcp, serve},
    batch::{batch, droplet},
    document::{convert, info, run_cmds},
};

pub const USAGE: &str = "\
photocraft-cli: headless PhotoCraft

USAGE:
  photocraft-cli convert <in> <out> [--format <ext>] [--quality <1-100>] [--tiff-layers] [--tga-bits <24|32>] [--dpx-bits <8|10|12|16>]
      Convert between formats (.pcraft, .psd, .png, .jpg, .tif, .webp, .exr, …).
      TIFF output is flat unless --tiff-layers keeps the layers (Photoshop layer data).
      --quality sets the JPEG or WebP quality; a WebP written with a quality is lossy, without one lossless.
      --tga-bits sets a TGA's bits per pixel: 32 writes the alpha channel (or else the transparency) as
      its alpha, 24 none. Without it: 32 with an alpha channel or transparency, else 24.
      --dpx-bits sets a DPX's bits per sample. Without it: 8 for an 8-bit document, else 16.
  photocraft-cli info <file> [--compact]
      Print the document as JSON (size, mode, depth, layer tree).
  photocraft-cli run (<file> | --new <json>) --cmd <id> [--params <json>] [--cmd …] [--out <file>] [--format <ext>] [--quality <1-100>] [--tiff-layers] [--tga-bits <24|32>] [--dpx-bits <8|10|12|16>]
      Open a file, run engine commands in order, save the result. Each --params
      applies to the preceding --cmd. Prints each command's JSON result.
  photocraft-cli batch --actions <actions.json|set.atn> [--action <name>] --in <dir> --out <dir> [--format <ext>] [--quality <1-100>] [--in-place] [--tiff-layers] [--tga-bits <24|32>] [--dpx-bits <8|10|12|16>]
      Apply an action list to every image in a directory. A Photoshop Actions file (.atn) is a
      set: its action --action names plays, as below. JSON steps are [id, params] pairs,
      {\"command\": id, \"params\": {…}} objects or bare ids, as a recorded action or droplet stores them
      (a list, or wrapped in {\"actions\": …}, {\"steps\": …} or a droplet). An Actions set (the Actions
      panel's actions.json, {\"actions\": [{\"name\", \"steps\"}…]}) plays the action --action names (needed
      when the set holds more than one) with the whole set loaded, so its steps can play other actions
      of the set. An --out folder that is the --in folder is refused, as the results would replace
      the originals; --in-place allows it.
  photocraft-cli droplet <file.pcdroplet> <file-or-dir>… [--out <dir>]
      Run a droplet (File › Automate › Create Droplet) on images and folders.
  photocraft-cli commands [--json] [--filter <text>]
      List the engine command registry.
  photocraft-cli mcp [--bridge <127.0.0.1:port>] [--control-token <64-hex> | --control-token-file <path>]
      [--automation-read-root <dir>] [--automation-write-root <dir>]
      Run the MCP server on stdio (headless engine, or bridge to a running `photocraft --control <port>`).
  photocraft-cli serve [--port <port>] [--control-token <64-hex> | --control-token-file <path>]
      [--automation-read-root <dir>] [--automation-write-root <dir>]
      Keep one headless session open and answer JSON lines ({\"id\",\"method\",\"params\"}) on stdio,
      or on 127.0.0.1:<port>. Methods: engine.execute, jobs.list/cancel, engine.commands,
      doc.open/new/place/save/exportLayers/exportDataSets/importDataSets/inspect/render/select/close, session.list, batch, methods
      (docs/control-protocol.md#headless-server).

  photocraft-cli <subcommand> --help (or -h) prints this text. A flag the subcommand doesn't take is
  an error.
";

pub(crate) struct Args {
    pub(crate) positional: Vec<String>,
    pub(crate) flags: Vec<(String, Option<String>)>,
}

/// A subcommand: the flags it reads (taking a value, or given bare) and what it runs. A flag
/// outside these is a usage error, so a typo such as `--fromat` can't be silently ignored (#423).
pub(crate) struct Subcommand {
    pub(crate) name: &'static str,
    pub(crate) values: &'static [&'static str],
    pub(crate) bare: &'static [&'static str],
    pub(crate) run: fn(&Args, &mut dyn Write, &mut dyn Write) -> R,
}

pub(crate) const SUBCOMMANDS: &[Subcommand] = &[
    Subcommand { name: "convert", values: &["--format", "--quality", "--tga-bits", "--dpx-bits"], bare: &["--tiff-layers"], run: convert },
    Subcommand { name: "info", values: &[], bare: &["--compact"], run: |a, out, _| info(a, out) },
    Subcommand {
        name: "run",
        values: &["--new", "--cmd", "--params", "--out", "--format", "--quality", "--tga-bits", "--dpx-bits"],
        bare: &["--tiff-layers"],
        run: run_cmds,
    },
    Subcommand {
        name: "batch",
        values: &["--actions", "--action", "--in", "--out", "--format", "--quality", "--tga-bits", "--dpx-bits"],
        bare: &["--in-place", "--tiff-layers"],
        run: batch,
    },
    Subcommand { name: "droplet", values: &["--out"], bare: &[], run: droplet },
    Subcommand { name: "commands", values: &["--filter"], bare: &["--json"], run: |a, out, _| commands(a, out) },
    Subcommand {
        name: "mcp",
        values: &["--bridge", "--control-token", "--control-token-file", "--automation-read-root", "--automation-write-root"],
        bare: &[],
        run: |a, _, _| mcp(a),
    },
    Subcommand {
        name: "serve",
        values: &["--port", "--control-token", "--control-token-file", "--automation-read-root", "--automation-write-root"],
        bare: &[],
        run: |a, _, err| serve(a, err),
    },
];

/// The subcommand's arguments, or `None` when they ask for help (`--help` or `-h`).
pub(crate) fn parse(sub: &Subcommand, args: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args { positional: Vec::new(), flags: Vec::new() };
    let unknown = |flag: &str| format!("unknown flag {flag} for {}", sub.name);
    let mut i = 0;
    while let Some(s) = args.get(i) {
        if s == "--help" || s == "-h" {
            return Ok(None);
        }
        if let Some((k, v)) = s.split_once('=').filter(|(k, _)| k.starts_with("--")) {
            if sub.bare.contains(&k) {
                return Err(format!("{k} takes no value"));
            }
            if !sub.values.contains(&k) {
                return Err(unknown(k));
            }
            a.flags.push((k.to_owned(), Some(v.to_owned())));
        } else if sub.values.contains(&s.as_str()) {
            let v = args.get(i + 1).ok_or_else(|| format!("{s} needs a value"))?;
            a.flags.push((s.clone(), Some(v.clone())));
            i += 1;
        } else if sub.bare.contains(&s.as_str()) {
            a.flags.push((s.clone(), None));
        } else if s.starts_with("--") {
            return Err(unknown(s));
        } else {
            a.positional.push(s.clone());
        }
        i += 1;
    }
    Ok(Some(a))
}

impl Args {
    pub(crate) fn get(&self, k: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(f, _)| f == k).and_then(|(_, v)| v.as_deref())
    }
    pub(crate) fn has(&self, k: &str) -> bool {
        self.flags.iter().any(|(f, _)| f == k)
    }
}
