//! What a panel may ask the host to run by id (`doc.command`), and what it may not.
//!
//! The allow-list is **here**, in the host, not in the manifest: a manifest is written by whoever
//! ships the plug-in, so it can ask for anything at all, and it is the user's copy of PhotoCraft
//! that decides. A panel's manifest only ever *requests*; [`granted`] is what it actually got, and
//! `init.grants.commands` hands that list to the panel so it can offer what it can do and say so
//! when it cannot.
//!
//! Two gates, and both must pass:
//!
//! 1. the command is in [`PANEL_COMMANDS`] — a short, reviewed list, not a namespace glob, so a
//!    command added to the engine tomorrow is not reachable by a panel until someone decides it is;
//! 2. the manifest asks for the document capability that command needs (`read` to look, `addLayers`
//!    to change).
//!
//! Everything a panel could use to reach outside the document is absent by construction: no `file.*`
//! (open, save, export, batch — a panel must not read or write the user's disk), no `plugin.*` (no
//! self-install or self-restart), no `edit.preferences` (no settings), no `actions.*` (a recorded
//! action can run any command at all, which would walk around this list), no `script.*`, and nothing
//! at all from the shell's own `view.*`/`window.*`.
//!
//! The list is what lettering a page takes — type made, styled and read back — and nothing more.
//! Growing it is a deliberate act with a comment saying why.

use photocraft_plugins::{Capabilities, DocumentCap};

/// The commands a panel may run, and the document capability each one needs.
///
/// Reading the document (`doc.info`) covers the layer stack, the selection bounds and the history, so
/// the reading half of lettering needs no command at all. What is left is making type, and reading a
/// layer's back: the five below and nothing else — a command nothing calls is permission surface
/// nobody has had to justify.
///
/// `type.edit` is the widest of the five, because it can rewrite what a layer says as well as shape
/// it. It is the one that cannot be left out: it is what fits a text box around type that has just
/// been laid out, re-shapes a layer on Align, moves the type into place, sets a style's
/// anti-aliasing and steps a layer's size. Granting it is a decision, which is why the list is short
/// enough to read.
const PANEL_COMMANDS: &[(&str, DocumentCap)] = &[
    // The type the letterer is placing: what it says, and how it looks.
    ("type.create", DocumentCap::AddLayers),
    ("type.edit", DocumentCap::AddLayers),
    ("type.setStyle", DocumentCap::AddLayers),
    // Reading it back: which font, which size, what it says — and which fonts the host has, for the
    // style editor's two dropdowns.
    ("type.info", DocumentCap::Read),
    ("type.fonts", DocumentCap::Read),
];

/// The capability `command` needs, if a panel may run it at all.
pub(super) fn needs(command: &str) -> Option<DocumentCap> {
    PANEL_COMMANDS.iter().find(|(id, _)| *id == command).map(|(_, cap)| *cap)
}

/// May this panel run `command`? `Err` is the reason it is answered with, verbatim.
pub(super) fn check(caps: &Capabilities, command: &str) -> std::result::Result<(), String> {
    let Some(cap) = needs(command) else {
        return Err(format!("this host does not let a panel run `{command}` (see `init.grants.commands`)"));
    };
    if !caps.document.contains(&cap) {
        let what = match cap {
            DocumentCap::Read => "read",
            DocumentCap::AddLayers => "addLayers",
        };
        return Err(format!("`{command}` needs the manifest's `capabilities.document` to include `{what}`"));
    }
    Ok(())
}

/// The commands this panel may actually run, in the order the host lists them: its manifest's
/// declared capabilities intersected with [`PANEL_COMMANDS`]. Sent in `init.grants.commands`.
pub(super) fn granted(caps: &Capabilities) -> Vec<&'static str> {
    PANEL_COMMANDS.iter().filter(|(_, cap)| caps.document.contains(cap)).map(|(id, _)| *id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(document: &[DocumentCap]) -> Capabilities {
        Capabilities { document: document.to_vec(), ..Capabilities::default() }
    }

    #[test]
    fn a_read_only_panel_may_look_but_not_change() {
        let read = caps(&[DocumentCap::Read]);
        assert!(check(&read, "type.info").is_ok());
        assert!(check(&read, "type.fonts").is_ok());
        let why = check(&read, "type.create").unwrap_err();
        assert!(why.contains("addLayers"), "{why}");
        assert_eq!(granted(&read), ["type.info", "type.fonts"]);
    }

    #[test]
    fn a_panel_that_asks_for_nothing_may_do_nothing() {
        assert_eq!(granted(&caps(&[])), Vec::<&str>::new());
        assert!(check(&caps(&[]), "type.info").is_err(), "`doc.info` is the capability-free way to read");
    }

    #[test]
    fn adding_layers_takes_the_whole_writing_set_with_it() {
        let both = caps(&[DocumentCap::Read, DocumentCap::AddLayers]);
        for id in ["type.create", "type.edit", "type.setStyle", "type.info"] {
            assert!(check(&both, id).is_ok(), "{id}");
        }
        assert_eq!(granted(&both).len(), PANEL_COMMANDS.len());
    }

    /// The list is short on purpose, and this is the test that keeps it that way: every row has to be
    /// a command lettering actually emits. A row nothing calls is permission surface nobody decided
    /// on, and a missing row is a feature that never worked.
    #[test]
    fn the_list_is_exactly_what_lettering_needs() {
        let mut ids: Vec<&str> = PANEL_COMMANDS.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["type.create", "type.edit", "type.fonts", "type.info", "type.setStyle"]);
    }

    /// The point of the table: a panel cannot reach the file system, the settings, the plug-in
    /// registry, or a recorded action, however it writes its manifest.
    #[test]
    fn nothing_outside_the_document_is_reachable() {
        let everything = caps(&[DocumentCap::Read, DocumentCap::AddLayers]);
        for id in [
            "file.open",
            "file.save",
            "file.export.exportAs",
            "file.automate.batch",
            "edit.preferences",
            "plugin.install",
            "plugin.remove",
            "plugin.panel.event",
            "actions.play",
            "actions.record",
            "script.play",
            "window.new",
            "view.zoomIn",
            "image.size",
            "edit.undo",
            "type.rasterize",
            "layer.delete",
        ] {
            let why = check(&everything, id).unwrap_err();
            assert!(why.contains("does not let a panel run"), "{id}: {why}");
        }
    }

    /// Every id the table grants must be a real command: a typo here would silently grant nothing,
    /// and a panel would report a feature that never worked.
    #[test]
    fn every_granted_command_exists() {
        for (id, _) in PANEL_COMMANDS {
            assert!(crate::commands::find(id).is_some(), "`{id}` is not in the command registry");
        }
    }

    /// Ids are unique: two rows for one command would let the second silently contradict the first.
    #[test]
    fn no_command_is_listed_twice() {
        let mut ids: Vec<&str> = PANEL_COMMANDS.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "a command is listed twice");
    }
}
