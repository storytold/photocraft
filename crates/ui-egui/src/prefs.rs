use craft::ui::runtime::env::UnitKind;
use photocraft_engine::prefs::Unit;

pub mod kys_import;
pub mod prefs_ui;
pub(super) mod pressure_curve_ui;

/// The framework's unit for a length unit of the preferences: the one mapping between the two.
pub(crate) fn unit_kind(u: Unit) -> UnitKind {
    match u {
        Unit::Pixels => UnitKind::Pixels,
        Unit::Inches => UnitKind::Inches,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Points => UnitKind::Points,
        Unit::Picas => UnitKind::Picas,
        Unit::Percent => UnitKind::Percent,
    }
}
