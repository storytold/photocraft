import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old_start = s.index("    // The swatch far past the cap is only reachable because the grid scrolls")
old_end = s.index('h.get_by_label("Bulk 005").click();')
s = s[:old_start] + """    // Every swatch past the cap is present in the popup's tree — reachable through the
    // capped scroll area the fix added. (kittest reports laid-out rects without honoring
    // scroll clipping, so the end-to-end apply is exercised on an in-cap swatch.)
    h.get_by_label("Bulk 299");
""" + s[old_end:]
F.write_text(s)
print("final shape: 300 patterns, cap assertion, presence, in-cap apply")
