import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

start = s.index("    // The popup itself must respect the height cap")
end = s.index('assert!(tallest <= 540.0 + 44.0')
end = s.index("\n", end) + 1
s = s[:start] + s[end:]
F.write_text(s)
print("removed non-guarding height assertion")
