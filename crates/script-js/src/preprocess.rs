//! Source preprocessing: `#`-directives at the start of a line, at brace depth 0.
//!
//! `#target`, `#targetengine`, `#strict` and `#script` lines are blanked (kept as empty lines so
//! error line numbers still match the file). `#include` and `#includepath`, and their comment
//! spellings `//@include` and `//@includepath`, are reported as unsupported rather than silently
//! dropped. Leading byte-order marks are stripped; UTF-16 text (which reaches us as replacement
//! characters and NULs) is a named error.
//!
//! A line is a directive only when it starts at brace (bracket) depth 0, outside a string,
//! template literal or block comment, and reads `#name` for a known name followed by whitespace
//! or the end of the line, and not by `=` or `(`. So a bare `#target` field inside a class body,
//! `#target = 1;`, and `#include` inside a multiline template stay script. Lines end at LF,
//! CRLF, a lone CR, U+2028 or U+2029, as in JavaScript. The scan ([`crate::scan`]) also rejects
//! brackets nested deeper than the parser can take.
//!
//! A regular expression literal is told from division by the token before the `/`. One right
//! after a `)` (as in `if (x) /re/.test(s)`) is read as division, so a quote or backtick inside
//! it can shift the scan: a later directive is then left in (a syntax error), or a
//! directive-looking line inside a template literal is blanked.

use crate::scan::{self, Scanner, lines};

const DIRECTIVES: [&str; 6] = ["target", "targetengine", "strict", "script", "include", "includepath"];

/// The source to evaluate.
///
/// # Errors
/// A message with the 1-based line of an unsupported directive.
pub fn preprocess(source: &str) -> Result<String, String> {
    preprocess_with(source, true)
}

/// [`preprocess`], refusing brackets nested deeper than the parser can take only when `guard`
/// is set (see [`crate::RunOptions::unguarded_nesting`]).
///
/// # Errors
/// As [`preprocess`].
pub fn preprocess_with(source: &str, guard: bool) -> Result<String, String> {
    // Editors that re-save a file can stack byte-order marks.
    let source = source.trim_start_matches('\u{feff}');
    if is_utf16(source) {
        return Err("the file is UTF-16 text (it starts with a UTF-16 byte-order mark); save it as UTF-8".into());
    }
    let mut out = String::with_capacity(source.len());
    // Unguarded, the scan still has to follow every bracket to tell where directives can be.
    let mut scan = if guard { Scanner::default() } else { Scanner::unbounded() };
    for (n, (line, end)) in lines(source).enumerate() {
        if scan.at_top_level()
            && let Some(d) = comment_include(line)
        {
            return Err(format!("line {}: //@{d} is not supported yet", n + 1));
        }
        match scan.at_top_level().then(|| directive(line)).flatten() {
            Some(d @ ("include" | "includepath")) => return Err(format!("line {}: #{d} is not supported yet", n + 1)),
            Some(_) => {}
            None => {
                // U+2028 and U+2029 end a line but not a string literal.
                if scan.line(line, matches!(end, "\u{2028}" | "\u{2029}")) {
                    return Err(format!("line {}: SyntaxError: {}", n + 1, scan::TOO_DEEP));
                }
                out.push_str(line);
            }
        }
        out.push_str(end);
    }
    // The single reading above finds directives; the parser needs every reading checked.
    if guard && let Err(r) = scan::check_nesting(&out) {
        return Err(format!("line {}: SyntaxError: {}", r.line, r.why));
    }
    Ok(out)
}

/// A UTF-16 byte-order mark decoded as UTF-8 with replacements: two U+FFFD, then the NUL half of
/// the first UTF-16 character (or nothing). Text in a single-byte encoding has no NULs.
fn is_utf16(source: &str) -> bool {
    let Some(rest) = source.strip_prefix("\u{FFFD}\u{FFFD}") else { return false };
    rest.is_empty() || rest.chars().take(2).any(|c| c == '\0')
}

/// The name of a `//@include` or `//@includepath` line.
fn comment_include(line: &str) -> Option<&'static str> {
    let rest = line.trim_start_matches(scan::space).strip_prefix("//@")?;
    let name = ["includepath", "include"].into_iter().find(|d| rest.starts_with(d))?;
    let after = &rest[name.len()..];
    (after.is_empty() || after.starts_with(scan::space)).then_some(name)
}

/// The directive name when `line` is one.
fn directive(line: &str) -> Option<&'static str> {
    // White space as script reads it, U+FEFF included.
    let rest = line.trim_start_matches(scan::space).strip_prefix('#')?;
    let name = DIRECTIVES.into_iter().filter(|d| rest.starts_with(d)).max_by_key(|d| d.len())?;
    let after = &rest[name.len()..];
    let separated = after.is_empty() || after.starts_with(scan::space);
    let assigns_or_calls = after.trim_start_matches(scan::space).starts_with(['=', '(']);
    (separated && !assigns_or_calls).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::{preprocess, preprocess_with};

    #[test]
    fn unguarded_directives_after_deep_nesting_are_still_found() {
        let deep = format!("var a = {}0{};", "[".repeat(100), "]".repeat(100));
        let src = format!("{deep}\n#target photoshop\nvar b = 1;");
        assert!(preprocess_with(&src, true).is_err());
        let out = preprocess_with(&src, false).unwrap();
        assert_eq!(out, format!("{deep}\n\nvar b = 1;"));
        let inc = format!("{deep}\n#include \"x.jsx\"");
        assert!(preprocess_with(&inc, false).unwrap_err().contains("#include"));
    }

    #[test]
    fn blanks_known_directives_and_keeps_line_numbers() {
        let out = preprocess("\u{feff}#target photocraft\r\n#targetengine main\nvar a = 1;\n").unwrap();
        assert_eq!(out, "\r\n\nvar a = 1;\n");
    }

    #[test]
    fn include_is_reported_with_its_line() {
        assert_eq!(preprocess("var a;\n#include \"x.jsx\"\n").unwrap_err(), "line 2: #include is not supported yet");
    }

    #[test]
    fn private_fields_are_not_directives() {
        assert_eq!(preprocess("  #x = 1;\n").unwrap(), "  #x = 1;\n");
        let class = "class A {\n  #target = 1;\n  #strict;\n  #script() {}\n  #include (a) {}\n}\n";
        assert_eq!(preprocess(class).unwrap(), class);
    }

    #[test]
    fn a_bare_private_field_named_like_a_directive_is_kept() {
        for src in ["class A {\n  #target\n  get() { return this.#target; }\n}\n", "class A {\n #include\n}\n"] {
            assert_eq!(preprocess(src).unwrap(), src, "{src:?}");
        }
    }

    #[test]
    fn a_lone_cr_ends_a_line() {
        assert_eq!(preprocess("#target x\r$.writeln('ran');\r").unwrap(), "\r$.writeln('ran');\r");
        assert_eq!(preprocess("#target x\r\n#strict on\rvar a;\n#include y").unwrap_err(), "line 4: #include is not supported yet");
        assert_eq!(preprocess("#strict\n#target").unwrap(), "\n");
        // U+FEFF is white space to script, before a directive's value as anywhere.
        assert_eq!(preprocess("var a;\n#target\u{feff}photocraft\n").unwrap(), "var a;\n\n");
        assert!(preprocess("var a;\n//@include\u{feff}x.jsx\n").unwrap_err().contains("include"));
    }

    #[test]
    fn directives_inside_templates_comments_and_strings_stay_script() {
        for src in [
            "var t = `\n#include example\n`;\n",
            "var t = `a${ `\n#include x\n` }b`;\n",
            "var t = `${ { a: 1 }.a\n#include x\n}`;\n",
            "/*\n#include x\n*/\n",
            "var s = 'a\\\n#include x';\n",
            "var s = 'a\\\r\n#include x';\n",
        ] {
            assert_eq!(preprocess(src).unwrap(), src, "{src:?}");
        }
    }

    #[test]
    fn directives_after_closed_tokens_are_found() {
        assert_eq!(preprocess("var t = `\n`;\n#target x\nvar a;").unwrap(), "var t = `\n`;\n\nvar a;");
        // A line comment after `o.` leaves `return` a property name, so `/` divides: what
        // follows is a class whose `#target` field stays, or a template whose text stays.
        for member in ["o. // the name follows\nreturn / new (class {\n#target x\n})", "x = o. // the name follows\nreturn / `\n#target x\n`;"] {
            assert_eq!(preprocess(member).unwrap(), member);
        }
        assert_eq!(preprocess("// `\n#target x\n").unwrap(), "// `\n\n");
        assert_eq!(preprocess("var s = '`';\n#target x\n").unwrap(), "var s = '`';\n\n");
        assert_eq!(preprocess("/* `\n*/ var a;\n#target x\n").unwrap(), "/* `\n*/ var a;\n\n");
        assert_eq!(preprocess("var s = 'unterminated\n#target x\n").unwrap(), "var s = 'unterminated\n\n");
    }

    #[test]
    fn line_and_paragraph_separators_end_lines() {
        assert_eq!(preprocess("#target x\u{2028}$.writeln('ran');\u{2029}").unwrap(), "\u{2028}$.writeln('ran');\u{2029}");
        assert_eq!(preprocess("var a;\u{2028}#include y").unwrap_err(), "line 2: #include is not supported yet");
        // Inside a string literal they are ordinary characters (ES2019), so the string goes on.
        let src = "var s = 'a\u{2028}#target x';\n";
        assert_eq!(preprocess(src).unwrap(), src);
    }

    #[test]
    fn a_backtick_in_a_regex_keeps_template_lines() {
        let src = "var r = /`/;\nvar t = `\n#target y\n`;\n";
        assert_eq!(preprocess(src).unwrap(), src);
    }

    #[test]
    fn comment_includes_are_reported() {
        assert_eq!(preprocess("//@include \"lib.jsx\"\nvar a;").unwrap_err(), "line 1: //@include is not supported yet");
        assert_eq!(preprocess("var a;\r  //@includepath \"~/lib\"").unwrap_err(), "line 2: //@includepath is not supported yet");
        for src in ["// @include \"x\"\n", "//@target photocraft\n", "/*\n//@include x\n*/\n", "var t = `\n//@include x\n`;\n", "//@includes\n"] {
            assert_eq!(preprocess(src).unwrap(), src, "{src:?}");
        }
    }

    #[test]
    fn every_leading_bom_is_stripped() {
        assert_eq!(preprocess("\u{feff}\u{feff}\u{feff}#target x\nvar a;").unwrap(), "\nvar a;");
    }

    #[test]
    fn utf16_text_is_a_named_error() {
        for bytes in [&[0xFF, 0xFE, b'#', 0, b't', 0][..], &[0xFE, 0xFF, 0, b'#', 0, b't'][..], &[0xFF, 0xFE][..]] {
            let err = preprocess(&String::from_utf8_lossy(bytes)).unwrap_err();
            assert_eq!(err, "the file is UTF-16 text (it starts with a UTF-16 byte-order mark); save it as UTF-8");
        }
    }

    #[test]
    fn a_quote_inside_a_regex_does_not_hide_a_directive() {
        assert_eq!(preprocess("var r = /'/;\n#target x\nvar a;").unwrap(), "var r = /'/;\n\nvar a;");
        assert_eq!(preprocess("var r = /`/g;\n#include x").unwrap_err(), "line 2: #include is not supported yet");
        // Division is not a regex.
        assert_eq!(preprocess("var a = 4 / 2, b = '/';\n#target x\n").unwrap(), "var a = 4 / 2, b = '/';\n\n");
    }
}
