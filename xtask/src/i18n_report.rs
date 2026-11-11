use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Key {
    context: String,
    source: String,
}

#[derive(Debug)]
struct Language {
    code: String,
}

/// `cargo xtask i18n-report` (the last `cargo xtask ci` step): per registered language, how many
/// of PhotoCraft's English UI strings have a translation in the craft-i18n checkout
/// `CRAFT_I18N_DIR` names, looked up as the app does (the `craft-i18n` crate: PhotoCraft's rows
/// over the shared ones, a region falling back to its language), the strings it lacks, and how
/// many rows are kept in English (a translation that is the English text itself). It never fails
/// on a missing translation: that string shows in English, and translators fill it in craft-i18n.
/// Without `CRAFT_I18N_DIR` it says so and passes.
pub fn run(root: &Path) -> Result<(), String> {
    let registry_path = root.join("crates/ui-egui/src/i18n.rs");
    let registry = fs::read_to_string(&registry_path).map_err(|e| format!("{}: {e}", registry_path.display()))?;
    let languages = registered_languages(&registry)?;
    let Some(dir) = std::env::var_os("CRAFT_I18N_DIR").filter(|d| !d.is_empty()) else {
        println!("UI translation report: skipped, built without craft-i18n (set CRAFT_I18N_DIR to a craft-i18n checkout)");
        return Ok(());
    };
    let english_keys = source_keys(root)?;
    if english_keys.is_empty() {
        return Err("no source-derived English UI translation keys were found".into());
    }
    let embedded = embedded(&PathBuf::from(dir).join("locales"))?;
    let codes: Vec<&str> = languages.iter().map(|l| l.code.as_str()).collect();
    let (i18n, mut problems) = craft_i18n::I18n::new(embedded, &codes);
    problems.extend(i18n.check_all());
    let mut translations = BTreeMap::new();
    let mut kept = BTreeMap::new();
    for language in &languages {
        let Some(lang) = i18n.from_code(&language.code).filter(|_| language.code != "en") else { continue };
        let found: BTreeSet<Key> = english_keys.iter().filter(|key| translation(&i18n, lang, key).is_some()).cloned().collect();
        let same = english_keys.iter().filter(|key| key.context.is_empty() && translation(&i18n, lang, key) == Some(key.source.as_str())).count();
        translations.insert(language.code.clone(), found);
        kept.insert(language.code.clone(), same);
    }
    print!("{}", format_report(&languages, &english_keys, &translations, &kept));
    if !problems.is_empty() {
        println!("\n{} catalog rows are skipped (they show in English):", problems.len());
        for problem in &problems {
            println!("    {problem}");
        }
    }
    Ok(())
}

/// Checks the language table on its own, so `cargo xtask ci` can fail on it while a coverage report
/// failure stays a warning.
pub fn check_language_table(root: &Path) -> Result<(), String> {
    let registry_path = root.join("crates/ui-egui/src/i18n.rs");
    let registry = fs::read_to_string(&registry_path).map_err(|e| format!("{}: {e}", registry_path.display()))?;
    let doc_path = root.join("docs/localization.md");
    let doc = fs::read_to_string(&doc_path).map_err(|e| format!("{}: {e}", doc_path.display()))?;
    check_documented_languages(&registered_languages(&registry)?, &doc)
}

/// The "Supported languages" table in `docs/localization.md` lists every registered language and
/// nothing else. A language shipped without its row (Turkish, #1373) is otherwise invisible there.
fn check_documented_languages(languages: &[Language], doc: &str) -> Result<(), String> {
    let mut section = doc.lines().skip_while(|line| line.trim_end() != "## Supported languages");
    if section.next().is_none() {
        return Err("docs/localization.md has no `## Supported languages` section".to_string());
    }
    let mut rows = section.take_while(|line| !line.starts_with('#')).skip_while(|line| !line.contains('|')).take_while(|line| line.contains('|'));
    let header = rows.next().map(table_cells).unwrap_or_default();
    let delimiter = rows.next().map(table_cells).unwrap_or_default();
    let delimiter_matches = delimiter.len() == header.len() && delimiter.iter().all(|cell| cell.contains('-') && cell.chars().all(|c| matches!(c, '-' | ':')));
    if header.get(1) != Some(&"Preference code") || !delimiter_matches {
        return Err("docs/localization.md's Supported languages table must start with a `Preference code` header and a matching delimiter row".into());
    }
    let mut documented = BTreeSet::new();
    for line in rows {
        let code = table_cells(line).get(1).map_or("", |cell| cell.trim_matches('`').trim());
        if code.is_empty() {
            return Err(format!("docs/localization.md's Supported languages table has a row with no preference code: {}", line.trim()));
        }
        if !documented.insert(code.to_string()) {
            return Err(format!("docs/localization.md's Supported languages table lists `{code}` more than once"));
        }
    }
    let registered: BTreeSet<String> = languages.iter().map(|language| language.code.clone()).collect();
    let missing: Vec<&str> = registered.difference(&documented).map(String::as_str).collect();
    let extra: Vec<&str> = documented.difference(&registered).map(String::as_str).collect();
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }
    let mut message = String::from("docs/localization.md's Supported languages table is out of date:");
    if !missing.is_empty() {
        message.push_str(&format!("\n  registered but not listed: {}", missing.join(", ")));
    }
    if !extra.is_empty() {
        message.push_str(&format!("\n  listed but not registered: {}", extra.join(", ")));
    }
    Err(message)
}

/// The cells of a GFM table row. `\|` is a pipe inside a cell, not a column break.
fn table_cells(line: &str) -> Vec<&str> {
    let row = line.trim();
    let row = row.strip_prefix('|').unwrap_or(row);
    let row = if row.ends_with('|') && !row.ends_with("\\|") { &row[..row.len() - 1] } else { row };
    let mut cells = Vec::new();
    let (mut start, mut escaped) = (0, false);
    for (index, c) in row.char_indices() {
        if c == '|' && !escaped {
            cells.push(row[start..index].trim());
            start = index + 1;
        }
        escaped = c == '\\' && !escaped;
    }
    cells.push(row[start..].trim());
    cells
}

/// The translation of `key` in `lang`, as the app looks it up: a contextual string falls back to
/// the plain one, a plural to nothing (its English forms show).
fn translation<'a>(i18n: &'a craft_i18n::I18n, lang: craft_i18n::Lang, key: &'a Key) -> Option<&'a str> {
    match key.context.as_str() {
        "" | "@plural" => i18n.get(lang, &key.context, &key.source),
        context => i18n.get(lang, context, &key.source).or_else(|| i18n.get(lang, "", &key.source)),
    }
}

/// The catalogs of every locale under `dir` with a `photocraft.tsv`, as the app's build.rs embeds
/// them: (locale, shared `common.tsv` or "", `photocraft.tsv`). Read once per run, so leaked.
fn embedded(dir: &Path) -> Result<&'static craft_i18n::Embedded, String> {
    let list = fs::read_dir(dir).map_err(|e| format!("{} is not a craft-i18n checkout ({e})", dir.display()))?;
    let mut out: Vec<(&'static str, &'static str, &'static str)> = Vec::new();
    for entry in list {
        // A listing that fails part way would pass off the rest of the locales as missing.
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let Some(code) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        // A locale without the file is skipped (or reads without a shared catalog); a file that is
        // there but can't be read is an error, not a gap in the coverage.
        let Some(app) = read_optional(&path.join("photocraft.tsv"))? else { continue };
        let shared = read_optional(&path.join("common.tsv"))?.unwrap_or_default();
        out.push((Box::leak(code.into_boxed_str()), Box::leak(shared.into_boxed_str()), Box::leak(app.into_boxed_str())));
    }
    if out.is_empty() {
        return Err(format!("{} has no locale with a photocraft.tsv", dir.display()));
    }
    out.sort();
    Ok(Box::leak(out.into_boxed_slice()))
}

/// The file's text; `None` when there is no such file, an error naming it when it can't be read.
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn source_keys(root: &Path) -> Result<BTreeSet<Key>, String> {
    let ui_sources = source_files(&root.join("crates/ui-egui/src"))?;
    let engine_sources = source_files(&root.join("crates/engine/src"))?;
    let mut keys = tl_keys(&ui_sources);
    keys.extend(plural_keys(&ui_sources));

    let catalog_path = root.join("crates/ui-egui/src/menu_catalog.rs");
    let catalog = fs::read_to_string(&catalog_path).map_err(|e| format!("{}: {e}", catalog_path.display()))?;
    keys.extend(menu_catalog_keys(&catalog));

    let ui_commands = ui_sources.iter().find(|(path, _)| path.ends_with("menus.rs")).ok_or_else(|| "could not find crates/ui-egui/src/menus.rs".to_string())?;
    keys.extend(ui_command_keys(&ui_commands.1));
    keys.extend(engine_command_labels(&engine_sources));
    Ok(keys)
}

fn format_report(
    languages: &[Language],
    english_keys: &BTreeSet<Key>,
    translations: &BTreeMap<String, BTreeSet<Key>>,
    kept: &BTreeMap<String, usize>,
) -> String {
    let mut report = String::from("UI translation coverage (translated source keys / source keys; kept: translated as the English itself)\n");
    report.push_str(&format!("{:<12} {:>12} {:>9} {:>6}\n", "language", "translated", "coverage", "kept"));
    // A registered language without a catalog in the checkout isn't a catalog with nothing in it:
    // it is listed apart and left out of the coverage.
    let resolved = |language: &&Language| language.code == "en" || translations.contains_key(&language.code);
    for language in languages.iter().filter(resolved) {
        let translated =
            if language.code == "en" { english_keys.len() } else { translations.get(&language.code).map_or(0, |keys| keys.intersection(english_keys).count()) };
        let coverage = 100.0 * translated as f64 / english_keys.len() as f64;
        let kept = kept.get(&language.code).copied().unwrap_or(0);
        report.push_str(&format!("{:<12} {:>5}/{:<6} {:>8.2}% {:>6}\n", language.code, translated, english_keys.len(), coverage, kept));
    }
    let unresolved: Vec<&str> = languages.iter().filter(|language| !resolved(language)).map(|language| language.code.as_str()).collect();
    if !unresolved.is_empty() {
        report.push_str(&format!("\nRegistered without a photocraft.tsv in the checkout (shown in English, not counted): {}\n", unresolved.join(", ")));
    }
    report.push_str("\nMissing translations (shown in English until translated)\n");
    for language in languages.iter().filter(resolved).filter(|language| language.code != "en") {
        let empty = BTreeSet::new();
        let translated = translations.get(&language.code).unwrap_or(&empty);
        let missing: Vec<&Key> = english_keys.difference(translated).collect();
        if missing.is_empty() {
            report.push_str(&format!("{}: none missing\n", language.code));
            continue;
        }
        report.push_str(&format!("{}: {} missing\n", language.code, missing.len()));
        for key in missing {
            let source = escape_line(&key.source);
            if key.context.is_empty() {
                report.push_str(&format!("    {source}\n"));
            } else {
                report.push_str(&format!("    [{}] {source}\n", escape_line(&key.context)));
            }
        }
    }
    report
}

/// Keeps one key per log line: a source string with a newline or tab prints them escaped.
fn escape_line(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t")
}

fn registered_languages(source: &str) -> Result<Vec<Language>, String> {
    let registry = source
        .split("pub static LANGUAGES")
        .nth(1)
        .ok_or_else(|| "could not find the LANGUAGES registry".to_string())?
        .split("];")
        .next()
        .ok_or_else(|| "could not find the end of the LANGUAGES registry".to_string())?;
    let mut languages = Vec::new();
    let mut codes = HashSet::new();
    for (index, section) in registry.split("LangInfo {").skip(1).enumerate() {
        let block = section.split('}').next().unwrap_or(section);
        let code = quoted_field(block, "code:").ok_or_else(|| format!("LANGUAGES entry {} has no code", index + 1))?;
        if !codes.insert(code.clone()) {
            return Err(format!("duplicate registered language code `{code}`"));
        }
        languages.push(Language { code });
    }
    if !languages.iter().any(|language| language.code == "en") {
        return Err("LANGUAGES registry does not contain English (`en`)".into());
    }
    languages.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(languages)
}

fn quoted_field(block: &str, field: &str) -> Option<String> {
    let start = block.find(field)? + field.len();
    let rest = block.get(start..)?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest.get(..end)?.to_string())
}

fn source_files(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = fs::read_dir(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", path.display()))?;
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "tests") {
                    stack.push(path);
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let stem = path.file_stem().and_then(|name| name.to_str()).unwrap_or_default();
                if stem == "tests" || stem.ends_with("_tests") {
                    continue;
                }
                let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                files.push((path, text));
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn tl_keys(sources: &[(PathBuf, String)]) -> BTreeSet<Key> {
    let mut keys = BTreeSet::new();
    for (path, source) in sources {
        if path.ends_with("ui-egui/src/i18n.rs") {
            continue;
        }
        let code = without_test_modules(source);
        let mut rest = code.as_str();
        while let Some(at) = rest.find("tl!(\"") {
            rest = rest.get(at + 5..).unwrap_or("");
            let bytes = rest.as_bytes();
            let mut end = 0;
            while end < bytes.len() && !(bytes[end] == b'"' && (end == 0 || bytes[end - 1] != b'\\')) {
                end += 1;
            }
            if let Some(raw) = rest.get(..end) {
                let source = unescape_rust_string(raw);
                if rest.get(end + 1..end + 2) == Some(")") {
                    keys.insert(Key { context: String::new(), source });
                }
            }
        }
    }
    keys
}

fn plural_keys(sources: &[(PathBuf, String)]) -> BTreeSet<Key> {
    let mut keys = BTreeSet::new();
    for (path, source) in sources {
        if path.ends_with("ui-egui/src/i18n.rs") {
            continue;
        }
        let code = without_test_modules(source);
        let mut rest = code.as_str();
        while let Some(at) = rest.find("trn(") {
            rest = rest.get(at + 4..).unwrap_or("");
            let Some(end) = call_end(rest) else { break };
            let args = rest.get(..end).unwrap_or("");
            // `trn(lang, n, one, other)`: the forms are the last two literals, so a string in the
            // count expression (`r["files"].len()`) is not mistaken for one.
            let forms = quoted_strings(args);
            if let [.., one, other] = forms.as_slice() {
                keys.insert(Key { context: "@plural".into(), source: format!("{one}|{other}") });
            }
            rest = rest.get(end + 1..).unwrap_or("");
        }
    }
    keys
}

fn menu_catalog_keys(source: &str) -> BTreeSet<Key> {
    let mut keys = BTreeSet::new();
    for line in source.lines().map(str::trim) {
        let Some(entry) = line.strip_prefix("(&[") else { continue };
        let Some((path, item)) = entry.split_once("],") else { continue };
        for segment in quoted_strings(path) {
            keys.insert(Key { context: String::new(), source: segment });
        }
        if let Some(label) = quoted_strings(item).first().filter(|label| label.as_str() != "---") {
            keys.insert(Key { context: String::new(), source: label.clone() });
        }
    }
    keys
}

fn ui_command_keys(source: &str) -> BTreeSet<Key> {
    let mut keys = BTreeSet::new();
    for line in source.lines().map(str::trim) {
        if !line.starts_with("(\"") {
            continue;
        }
        let Some((before_path, path_and_rest)) = line.split_once("&[") else { continue };
        let Some((path, _)) = path_and_rest.split_once(']') else { continue };
        if let Some(label) = quoted_strings(before_path).get(1) {
            keys.insert(Key { context: String::new(), source: label.clone() });
        }
        keys.extend(quoted_strings(path).into_iter().map(|source| Key { context: String::new(), source }));
    }
    keys
}

fn engine_command_labels(sources: &[(PathBuf, String)]) -> BTreeSet<Key> {
    let mut keys = BTreeSet::new();
    let mut command_macros = HashSet::new();
    for (_, source) in sources {
        let code = without_test_modules(source);
        let mut rest = code.as_str();
        while let Some(at) = rest.find("macro_rules!") {
            rest = rest.get(at + "macro_rules!".len()..).unwrap_or("");
            let name = rest.trim_start().split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let next_definition = rest.find("macro_rules!").unwrap_or(rest.len());
            let definition = rest.get(..next_definition).unwrap_or(rest);
            if definition.contains("CommandSpec {") && definition.contains("label:") {
                command_macros.insert(name.to_string());
            }
        }
    }
    for (_, source) in sources {
        let code = without_test_modules(source);
        let mut rest = code.as_str();
        while let Some(at) = rest.find("CommandSpec {") {
            rest = rest.get(at + "CommandSpec {".len()..).unwrap_or("");
            let Some(end) = balanced_end(rest, '{', '}') else { break };
            let Some(body) = rest.get(..end) else { break };
            let mut fields = split_arguments(body).into_iter();
            let label_field = fields.find_map(|field| field.trim().strip_prefix("label:").map(str::trim));
            let menu_field = split_arguments(body).into_iter().find_map(|field| field.trim().strip_prefix("menu:").map(str::trim));
            if let (Some(label_field), Some(menu_field)) = (label_field, menu_field)
                && let Some(label) = quoted_strings(label_field).first()
                && (menu_field.starts_with("&[") || menu_field.starts_with('['))
                && let Some(path) = menu_field.split_once('[').and_then(|(_, rest)| rest.split_once(']').map(|(path, _)| path))
                && !path.trim().is_empty()
            {
                keys.insert(Key { context: String::new(), source: label.clone() });
                keys.extend(quoted_strings(path).into_iter().map(|source| Key { context: String::new(), source }));
            }
            rest = rest.get(end + 1..).unwrap_or("");
        }
        for name in &command_macros {
            let marker = format!("{name}!(");
            let mut rest = code.as_str();
            while let Some(at) = rest.find(&marker) {
                rest = rest.get(at + marker.len()..).unwrap_or("");
                let Some(end) = call_end(rest) else { break };
                let Some(args) = rest.get(..end) else { continue };
                if let Some(label) = menu_command_label(args) {
                    keys.insert(Key { context: String::new(), source: label });
                }
                rest = rest.get(end + 1..).unwrap_or("");
            }
        }
    }
    keys
}

/// Drops the bodies of inline `#[cfg(test)] mod name { … }` modules. A declaration
/// (`#[cfg(test)] mod tests;`) can sit before the rest of a file's code, so only an inline
/// module's own body is skipped.
fn without_test_modules(source: &str) -> String {
    let marker = "#[cfg(test)]";
    let mut code = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(at) = rest.find(marker) {
        let after_marker = rest.get(at + marker.len()..).unwrap_or("");
        let Some(body) = inline_module_body(after_marker) else {
            code.push_str(rest.get(..at + marker.len()).unwrap_or(rest));
            rest = after_marker;
            continue;
        };
        code.push_str(rest.get(..at).unwrap_or(rest));
        let Some(end) = balanced_end(body, '{', '}') else { return code };
        rest = body.get(end + 1..).unwrap_or("");
    }
    code.push_str(rest);
    code
}

/// After a `#[cfg(test)]`: the text following the `{` of an inline `mod name {`, past any further
/// attributes, or `None` for anything else.
fn inline_module_body(mut rest: &str) -> Option<&str> {
    loop {
        rest = rest.trim_start();
        if !rest.starts_with("#[") {
            break;
        }
        let end = balanced_end(rest.get(2..)?, '[', ']')?;
        rest = rest.get(end + 3..)?;
    }
    let rest = rest.strip_prefix("mod")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let name_end = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len());
    if name_end == 0 {
        return None;
    }
    rest.get(name_end..)?.trim_start().strip_prefix('{')
}

fn menu_command_label(args: &str) -> Option<String> {
    let args = split_arguments(args);
    let menu = args.get(2)?.trim();
    if !(menu.starts_with("&[") || menu.starts_with('[')) || quoted_strings(menu).is_empty() {
        return None;
    }
    quoted_strings(args.get(1)?.trim()).first().cloned()
}

fn split_arguments(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, ch) in args.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if let Some(arg) = args.get(start..index) {
                    out.push(arg);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    if let Some(arg) = args.get(start..) {
        out.push(arg);
    }
    out
}

fn call_end(source: &str) -> Option<usize> {
    balanced_end(source, '(', ')')
}

fn balanced_end(source: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 1_usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in source.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            value if value == open => depth += 1,
            value if value == close => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn quoted_strings(source: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find('"') {
        rest = rest.get(at + 1..).unwrap_or("");
        let bytes = rest.as_bytes();
        let mut end = 0;
        while end < bytes.len() && !(bytes[end] == b'"' && (end == 0 || bytes[end - 1] != b'\\')) {
            end += 1;
        }
        if let Some(raw) = rest.get(..end) {
            strings.push(unescape_rust_string(raw));
        }
        rest = rest.get(end.saturating_add(1)..).unwrap_or("");
    }
    strings
}

fn unescape_rust_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANGUAGE_DOC: &str = "# Localisation

## Supported languages

| Language | Preference code | Integer plural forms |
|---|---|---|
| English | `en` | one / other |
| Türkçe | `tr` | one / other |

## Catalogs
| Example | `not-a-language` | ignored outside the section |
";

    fn with_row(row: &str) -> String {
        LANGUAGE_DOC.replace("| Türkçe | `tr` | one / other |\n", &format!("| Türkçe | `tr` | one / other |\n{row}\n"))
    }

    fn documentation_languages() -> [Language; 2] {
        [Language { code: "en".into() }, Language { code: "tr".into() }]
    }

    #[test]
    fn documented_languages_match_the_registry() {
        assert_eq!(check_documented_languages(&documentation_languages(), LANGUAGE_DOC), Ok(()));
    }

    #[test]
    fn documented_languages_reject_a_missing_registered_code() {
        let doc = LANGUAGE_DOC.replace("| Türkçe | `tr` | one / other |\n", "");
        assert_eq!(
            check_documented_languages(&documentation_languages(), &doc),
            Err("docs/localization.md's Supported languages table is out of date:\n  registered but not listed: tr".into())
        );
    }

    #[test]
    fn documented_languages_reject_an_unregistered_code() {
        let doc = with_row("| Klingon | `tlh` | one / other |");
        assert_eq!(
            check_documented_languages(&documentation_languages(), &doc),
            Err("docs/localization.md's Supported languages table is out of date:\n  listed but not registered: tlh".into())
        );
    }

    #[test]
    fn documented_languages_read_codes_without_backticks() {
        let doc = with_row("| Klingon | tlh | one / other |");
        assert_eq!(
            check_documented_languages(&documentation_languages(), &doc),
            Err("docs/localization.md's Supported languages table is out of date:\n  listed but not registered: tlh".into())
        );
        let doc = LANGUAGE_DOC.replace("`tr`", "tr");
        assert_eq!(check_documented_languages(&documentation_languages(), &doc), Ok(()));
    }

    #[test]
    fn documented_languages_reject_a_row_without_a_code() {
        for row in ["| Klingon |  | one / other |", "| Klingon | `` | one / other |", "| Klingon", "|---||---|", "|---||"] {
            let doc = with_row(row);
            assert_eq!(
                check_documented_languages(&documentation_languages(), &doc),
                Err(format!("docs/localization.md's Supported languages table has a row with no preference code: {row}"))
            );
        }
    }

    #[test]
    fn documented_languages_report_a_dash_code_as_unregistered() {
        for code in ["-", ":", "---"] {
            let doc = with_row(&format!("| Klingon | {code} | one / other |"));
            assert_eq!(
                check_documented_languages(&documentation_languages(), &doc),
                Err(format!("docs/localization.md's Supported languages table is out of date:\n  listed but not registered: {code}"))
            );
        }
    }

    #[test]
    fn documented_languages_reject_a_malformed_table_head() {
        let head = "| Language | Preference code | Integer plural forms |\n|---|---|---|\n";
        for replacement in [
            "| Language | Preference code | Integer plural forms |\n|---|---|\n",
            "| Language | Preference code | Integer plural forms |\n|---||---|\n",
            "| Language | Preference code | Integer plural forms |\n|:|:|:|\n",
            "| Language | Code | Integer plural forms |\n|---|---|---|\n",
            "| Language | Preference code | Integer plural forms |\n",
        ] {
            assert_eq!(
                check_documented_languages(&documentation_languages(), &LANGUAGE_DOC.replace(head, replacement)),
                Err("docs/localization.md's Supported languages table must start with a `Preference code` header and a matching delimiter row".into()),
                "{replacement}"
            );
        }
    }

    #[test]
    fn documented_languages_reject_a_delimiter_row_among_the_languages() {
        let doc = with_row("|---|---|---|");
        assert_eq!(
            check_documented_languages(&documentation_languages(), &doc),
            Err("docs/localization.md's Supported languages table is out of date:\n  listed but not registered: ---".into())
        );
    }

    #[test]
    fn documented_languages_reject_a_repeated_code() {
        assert_eq!(
            check_documented_languages(&documentation_languages(), &with_row("| Türkçe again | tr | one / other |")),
            Err("docs/localization.md's Supported languages table lists `tr` more than once".into())
        );
    }

    #[test]
    fn documented_languages_read_only_the_first_table() {
        let separated = LANGUAGE_DOC.replace("| Türkçe | `tr` | one / other |\n", "\n| Türkçe | `tr` | one / other |\n");
        assert_eq!(
            check_documented_languages(&documentation_languages(), &separated),
            Err("docs/localization.md's Supported languages table is out of date:\n  registered but not listed: tr".into())
        );
        let second_table = LANGUAGE_DOC.replace("## Catalogs", "| Form | Meaning |\n|---|---|\n| one | 1 |\n\n## Catalogs");
        assert_eq!(check_documented_languages(&documentation_languages(), &second_table), Ok(()));
    }

    #[test]
    fn documented_languages_keep_an_escaped_pipe_inside_its_cell() {
        let doc = LANGUAGE_DOC.replace("| Türkçe | `tr`", "| Türkçe \\| Turkish | `tr`");
        assert_eq!(check_documented_languages(&documentation_languages(), &doc), Ok(()));
        let doc = with_row("| Klingon | tlh | one \\|");
        assert_eq!(
            check_documented_languages(&documentation_languages(), &doc),
            Err("docs/localization.md's Supported languages table is out of date:\n  listed but not registered: tlh".into())
        );
    }

    #[test]
    fn documented_languages_read_rows_without_a_leading_pipe() {
        for row in [" | Klingon | tlh | one / other |", "\t| Klingon | tlh | one / other |", "Klingon | tlh | one / other"] {
            assert_eq!(
                check_documented_languages(&documentation_languages(), &with_row(row)),
                Err("docs/localization.md's Supported languages table is out of date:\n  listed but not registered: tlh".into()),
                "{row}"
            );
        }
    }

    #[test]
    fn documented_languages_find_the_section_by_its_own_line() {
        let mentioned = format!("See ## Supported languages below.\n{LANGUAGE_DOC}");
        assert_eq!(check_documented_languages(&documentation_languages(), &mentioned), Ok(()));
        assert_eq!(check_documented_languages(&documentation_languages(), &LANGUAGE_DOC.replace('\n', "\r\n")), Ok(()));
    }

    #[test]
    fn documented_languages_require_the_supported_languages_section() {
        assert_eq!(
            check_documented_languages(&documentation_languages(), "# Localisation\n\n## Catalogs\n"),
            Err("docs/localization.md has no `## Supported languages` section".into())
        );
    }

    #[test]
    fn registered_languages_and_report_are_stable() {
        let registry = r#"
            pub static LANGUAGES: [LangInfo; 4] = [
                LangInfo { code: "zh", complete_menus: true },
                LangInfo { code: "de", complete_menus: true },
                LangInfo { code: "en", complete_menus: false },
                // A comment with a } brace.
                LangInfo { code: "fr", complete_menus: true },
            ];
            impl Lang {}
        "#;
        let languages = registered_languages(registry);
        assert!(languages.is_ok());
        let languages = languages.unwrap_or_default();
        let english_keys = BTreeSet::from([
            Key { context: String::new(), source: "Open".into() },
            Key { context: String::new(), source: "Save".into() },
            Key { context: "@plural".into(), source: "{n} layer|{n} layers".into() },
        ]);
        let translations = BTreeMap::from([
            ("fr".to_string(), BTreeSet::from([Key { context: String::new(), source: "Open".into() }])),
            ("zh".to_string(), english_keys.clone()),
        ]);
        let kept = BTreeMap::from([("fr".to_string(), 0), ("zh".to_string(), 1)]);
        assert_eq!(
            format_report(&languages, &english_keys, &translations, &kept),
            "UI translation coverage (translated source keys / source keys; kept: translated as the English itself)\n\
             language       translated  coverage   kept\n\
             en               3/3        100.00%      0\n\
             fr               1/3         33.33%      0\n\
             zh               3/3        100.00%      1\n\
             \n\
             Registered without a photocraft.tsv in the checkout (shown in English, not counted): de\n\
             \n\
             Missing translations (shown in English until translated)\n\
             fr: 2 missing\n\
             \x20   Save\n\
             \x20   [@plural] {n} layer|{n} layers\n\
             zh: none missing\n"
        );
    }

    #[test]
    fn gathers_keys_from_ui_and_engine_sources() {
        let catalog = menu_catalog_keys(
            r#"(&["File", "Export"], "Export As…", Some("Cmd+Shift+S"), "file.exportAs"),
               (&["File"], "---", None, "---"),"#,
        );
        assert!(catalog.contains(&Key { context: String::new(), source: "File".into() }));
        assert!(catalog.contains(&Key { context: String::new(), source: "Export".into() }));
        assert!(catalog.contains(&Key { context: String::new(), source: "Export As…".into() }));
        assert!(!catalog.contains(&Key { context: String::new(), source: "---".into() }));

        let ui = ui_command_keys(
            r#"("view.zoomIn", "Zoom In", &["View"], Some("Cmd+=")),
               ("view.fit", "Fit", &[], None),"#,
        );
        assert!(ui.contains(&Key { context: String::new(), source: "Zoom In".into() }));
        assert!(ui.contains(&Key { context: String::new(), source: "View".into() }));

        let engine = vec![(
            PathBuf::from("commands.rs"),
            r#"
            macro_rules! spec { ($id:literal, $label:literal, $menu:expr) => { CommandSpec { id: $id, label: $label, menu: $menu } }; }
            spec!("file.open", "Open", &["File"]);
            spec!("brush.get", "Get Brush", &[]);
            CommandSpec { id: "layer.new", label: "New Layer", menu: &["Layer"] }
        "#
            .into(),
        )];
        let labels = engine_command_labels(&engine);
        assert!(labels.contains(&Key { context: String::new(), source: "Open".into() }));
        assert!(labels.contains(&Key { context: String::new(), source: "New Layer".into() }));
        assert!(labels.contains(&Key { context: String::new(), source: "Layer".into() }));
        assert!(!labels.contains(&Key { context: String::new(), source: "Get Brush".into() }));
    }

    #[test]
    fn plural_keys_come_from_source_calls() {
        let sources = vec![(PathBuf::from("ui.rs"), r#"trn(lang, count, "{n} layer", "{n} layers");"#.into())];
        assert!(plural_keys(&sources).contains(&Key { context: "@plural".into(), source: "{n} layer|{n} layers".into() }));
        // A string literal in the count expression is not one of the forms.
        let sources = vec![(PathBuf::from("ui.rs"), r#"trn(lang, r["files"].len() as u64, "{n} file", "{n} files");"#.into())];
        let keys = plural_keys(&sources);
        assert!(keys.contains(&Key { context: "@plural".into(), source: "{n} file|{n} files".into() }), "{keys:?}");
        assert_eq!(keys.len(), 1, "{keys:?}");
    }

    #[test]
    fn source_scanner_ignores_test_modules_with_lf_and_crlf() {
        for newline in ["\n", "\r\n"] {
            let source = format!("tl!(\"production key\");{newline}#[cfg(test)]{newline}mod tests {{{newline}tl!(\"test fixture key\");{newline}}}{newline}");
            let sources = vec![(PathBuf::from("ui.rs"), source)];
            let keys = tl_keys(&sources);
            assert!(keys.contains(&Key { context: String::new(), source: "production key".into() }));
            assert!(!keys.contains(&Key { context: String::new(), source: "test fixture key".into() }));
        }
    }

    #[test]
    fn plural_forms_follow_count_expressions_and_dynamic_forms_are_not_literal_keys() {
        let sources = vec![(
            PathBuf::from("ui.rs"),
            r#"trn(lang, r["files"].as_array().map_or(0, Vec::len) as u64, "Saved {n} file", "Saved {n} files");
               trn(lang, count, one, other);"#
                .into(),
        )];
        assert_eq!(plural_keys(&sources), BTreeSet::from([Key { context: "@plural".into(), source: "Saved {n} file|Saved {n} files".into() }]));
    }
}
