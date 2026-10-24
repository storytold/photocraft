import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()
old = 'panic!("no popup area among {heights:?}")'
assert s.count(old) == 1
s = s.replace(old, 'panic!("DUMP {heights:?}")')
F.write_text(s)
print("dump panic applied")
