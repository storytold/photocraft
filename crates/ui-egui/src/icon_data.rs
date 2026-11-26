//! The icons the framework's catalog lacks, embedded; sources and licences are listed in `ATTRIBUTION.md`.

use craft::ui::primitives::AppIcons;

/// PhotoCraft's own icon set, registered by `icons::register`. The documents draw with
/// `stroke-width="2"`.
pub static PHOTOCRAFT_ICONS: AppIcons = AppIcons::new("photocraft", Some(2.0), ICONS);

/// Every icon of the set: name and SVG document.
pub static ICONS: &[(&str, &[u8])] = &[
    ("arrow-left-right", include_bytes!("../../../assets/icons/arrow-left-right.svg")),
    ("circle-dashed", include_bytes!("../../../assets/icons/circle-dashed.svg")),
    ("ellipsis", include_bytes!("../../../assets/icons/ellipsis.svg")),
    ("file-plus", include_bytes!("../../../assets/icons/file-plus.svg")),
    ("folder-plus", include_bytes!("../../../assets/icons/folder-plus.svg")),
    ("grid-2x2", include_bytes!("../../../assets/icons/grid-2x2.svg")),
    ("history-brush", include_bytes!("../../../assets/icons/history-brush.svg")),
    ("house", include_bytes!("../../../assets/icons/house.svg")),
    ("link-2", include_bytes!("../../../assets/icons/link-2.svg")),
    ("message-square", include_bytes!("../../../assets/icons/message-square.svg")),
    ("panel-left", include_bytes!("../../../assets/icons/panel-left.svg")),
    ("path-op-combine", include_bytes!("../../../assets/icons/path-op-combine.svg")),
    ("path-op-exclude", include_bytes!("../../../assets/icons/path-op-exclude.svg")),
    ("path-op-intersect", include_bytes!("../../../assets/icons/path-op-intersect.svg")),
    ("path-op-merge", include_bytes!("../../../assets/icons/path-op-merge.svg")),
    ("path-op-new-layer", include_bytes!("../../../assets/icons/path-op-new-layer.svg")),
    ("path-op-subtract", include_bytes!("../../../assets/icons/path-op-subtract.svg")),
    ("pipette", include_bytes!("../../../assets/icons/pipette.svg")),
    ("rectangle-horizontal", include_bytes!("../../../assets/icons/rectangle-horizontal.svg")),
    ("rectangle-vertical", include_bytes!("../../../assets/icons/rectangle-vertical.svg")),
    ("redo-2", include_bytes!("../../../assets/icons/redo-2.svg")),
    ("spirit-level", include_bytes!("../../../assets/icons/spirit-level.svg")),
    ("square-dashed", include_bytes!("../../../assets/icons/square-dashed.svg")),
    ("square-dashed-mouse-pointer", include_bytes!("../../../assets/icons/square-dashed-mouse-pointer.svg")),
    ("triangle-alert", include_bytes!("../../../assets/icons/triangle-alert.svg")),
    ("type", include_bytes!("../../../assets/icons/type.svg")),
    ("vector-shape", include_bytes!("../../../assets/icons/vector-shape.svg")),
    ("x", include_bytes!("../../../assets/icons/x.svg")),
];
