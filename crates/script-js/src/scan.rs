//! A light line-by-line scan of script source: where strings, template literals, comments and
//! regular expression literals start and end, and how deeply brackets nest.
//!
//! [`crate::preprocess`] uses it to find directives only outside those tokens. The nesting check
//! guards the parser: Boa 0.22 parses and compiles nested brackets by native recursion with no
//! depth limit, so `[[[…]]]` or `(((…)))` a few hundred levels deep overflows even the 16 MiB
//! script thread (measured: 400 levels parse, 700 abort, in a release build; a debug build
//! aborts at about 70 on a 2 MiB thread). [`MAX_NESTING`] keeps well inside that. `(`, `[`, `{`
//! and `${` all count as a level. Long chains without brackets (`!!!…`, `a ? b : c ? …`,
//! `if (a) if (b) …`, `a = b = …`) also recurse in the parser, but only fail thousands deep; this
//! scan does not limit them.
//!
//! It is not a lexer, and the guard is not complete. Whether a `/` starts a regular expression or
//! divides is the parser's to know (`if (x) /re/` against `(a) / b`); this scan guesses from the
//! token before it. A `/` after a name, a number, `]` or a property divides; after an operator, an
//! opening bracket or a keyword such as `return` it starts a regular expression. Where the guess
//! can go either way (after `)`, `}`, and after `of`, `yield` and `await`, which can also be
//! names), [`check_nesting`] follows the other reading too, and fails when either nests too
//! deeply. A second reading still inside a regular expression at the end of its line is not
//! script and is dropped; one line may start at most [`MAX_FORKS`] of them.
//!
//! What a crafted script can still do is fool the guess elsewhere and hide brackets inside what
//! the scan takes for a string or a regular expression. On the desktop the script host process
//! is the protection that holds: a parser stack overflow ends that process, and the run fails
//! with a named error; a run in the app's own process keeps the guess.
//! [`Scanner::line`], used to find directives,
//! keeps the main reading only.

/// Deepest bracket nesting a script may use.
pub const MAX_NESTING: usize = 64;

/// The message for nesting past [`MAX_NESTING`].
pub const TOO_DEEP: &str = "brackets are nested more than 64 levels deep";

/// The message for a line whose second readings cost more than [`SECOND_PASSES`] passes over it.
pub const TOO_COMPLEX: &str = "a line has too many divisions after brackets to check; split it";

const _: () = assert!(MAX_NESTING == 64, "keep TOO_DEEP in step with MAX_NESTING");

/// Second readings one line may start (after merging those that start at the same place in the
/// same state).
pub const MAX_FORKS: usize = 4096;

/// How many passes over a line its second readings may make in all. Most end within a few
/// characters, where they meet the main reading again; past this the line is refused.
pub const SECOND_PASSES: usize = 64;

/// Readings [`check_nesting`] carries from one line to the next, after merging those that end a
/// line in the same state. Past this the source is refused rather than checked partly.
const MAX_READINGS: usize = 32;

/// Keywords after which a `/` starts a regular expression (unless they are a property).
const BEFORE_REGEX: [&str; 18] = [
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
    "extends",
    // Statements that may end at a line break: a `/` after them starts the next statement.
    "debugger",
    "break",
    "continue",
];

/// Statements whose label, if any, may end them at a line break (`break outer` then a `/`
/// starting the next line is a regular expression), or be followed by a division.
const BEFORE_LABEL: [&str; 2] = ["break", "continue"];

/// Of [`BEFORE_REGEX`], those a script may also use as a variable name (`var of = 4; of / 2`).
const ALSO_NAMES: [&str; 3] = ["of", "yield", "await"];

/// Each line with its terminator (`"\n"`, `"\r\n"`, `"\r"`, U+2028, U+2029, or `""` at the end).
pub fn lines(source: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut rest = source;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let (line, end, next) = match rest.char_indices().find(|&(_, c)| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')) {
            Some((i, _)) if rest[i..].starts_with("\r\n") => (&rest[..i], &rest[i..i + 2], &rest[i + 2..]),
            Some((i, c)) => (&rest[..i], &rest[i..i + c.len_utf8()], &rest[i + c.len_utf8()..]),
            None => (rest, "", ""),
        };
        rest = next;
        Some((line, end))
    })
}

/// Why [`check_nesting`] refused a source, and the 1-based line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    pub line: usize,
    /// [`TOO_DEEP`] or [`TOO_COMPLEX`].
    pub why: &'static str,
}

/// `Ok` when no reading of the source nests brackets deeper than [`MAX_NESTING`] (see the module
/// docs for what the readings are).
///
/// # Errors
/// Where, and why, the source is refused.
pub fn check_nesting(source: &str) -> Result<(), Refused> {
    let mut readings = vec![Scanner::default()];
    for (n, (line, end)) in lines(source).enumerate() {
        let refuse = |why| Err(Refused { line: n + 1, why });
        let string_goes_on = matches!(end, "\u{2028}" | "\u{2029}");
        // The main reading goes first, so second readings can be compared with where it stood.
        readings.sort_by_key(|r| r.second);
        let carried = readings.iter().any(|r| r.second);
        let mut work: std::collections::VecDeque<(Scanner, usize)> = readings.into_iter().map(|s| (s, 0)).collect();
        let mut done: Vec<Scanner> = Vec::new();
        let mut check = Check {
            forks: Vec::new(),
            trail: Vec::new(),
            // Second readings carried over from the line before start at its beginning.
            record_until: if carried { TRAIL_WINDOW } else { 0 },
            steps: 0,
            budget: SECOND_PASSES.saturating_mul(line.len().saturating_add(1)),
            base: 0,
        };
        // Second readings started on this line, by where they start and their state: one that
        // starts where another did, in the same state but no deeper, adds nothing (what follows
        // adds the same depth to both).
        let mut started: std::collections::HashMap<(usize, u64), usize> = std::collections::HashMap::new();
        while let Some((mut scan, from)) = work.pop_front() {
            check.base = from;
            match scan.scan(line.get(from..).unwrap_or_default(), from == 0, string_goes_on, Some(&mut check)) {
                Scan::Deep => return refuse(TOO_DEEP),
                Scan::Complex => return refuse(TOO_COMPLEX),
                Scan::Dropped => {}
                Scan::End => merge_into(&mut done, scan),
            }
            for (fork, at) in std::mem::take(&mut check.forks) {
                let at = from + at;
                let deepest = started.entry((at, fork.fingerprint())).or_insert(usize::MAX);
                if *deepest != usize::MAX && *deepest >= fork.depth {
                    continue;
                }
                *deepest = fork.depth;
                if started.len() > MAX_FORKS {
                    return refuse(TOO_COMPLEX);
                }
                work.push_back((fork, at));
            }
        }
        if done.len() > MAX_READINGS {
            return refuse(TOO_COMPLEX);
        }
        readings = done;
    }
    Ok(())
}

/// What [`check_nesting`] keeps while it scans one line.
struct Check {
    /// Second readings the scan started, with the offset (in the scanned slice) where they go on.
    forks: Vec<(Scanner, usize)>,
    /// Where the main reading stood after the sync points of the line (see [`SYNC`]): offset,
    /// [`Scanner::fingerprint`] and depth, by offset. Kept only up to [`TRAIL_WINDOW`] bytes
    /// after a place where second readings start, and at most [`TRAIL_MAX`] of them: a second
    /// reading that lives on past that is not dropped early, only counted against the budget.
    trail: Vec<(usize, u64, usize)>,
    /// The main reading records its sync points before this offset.
    record_until: usize,
    /// Characters second readings have scanned on this line, and how many they may.
    steps: usize,
    budget: usize,
    /// Where in the line the current scan started.
    base: usize,
}

/// Characters after which a second reading is compared with the main reading: when it stands
/// where the main reading stood, in the same state and no deeper, it is dropped, since the main
/// reading counts all it would.
const SYNC: [char; 9] = [';', ',', '(', ')', '[', ']', '{', '}', ':'];
/// How far after a fork the main reading's sync points are kept.
const TRAIL_WINDOW: usize = 1 << 16;
/// Most sync points kept for one line.
const TRAIL_MAX: usize = 1 << 20;

/// Add a reading that ended its line, merged with one in the same state. The main reading stays
/// the main one, and the deeper depth is kept (what follows adds the same depth to both).
fn merge_into(done: &mut Vec<Scanner>, r: Scanner) {
    match done.iter_mut().find(|o| o.same_state(&r)) {
        Some(o) => {
            o.second &= r.second;
            o.depth = o.depth.max(r.depth);
        }
        None => done.push(r),
    }
}

/// How a scan of (the rest of) a line ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    End,
    /// Brackets nested deeper than [`MAX_NESTING`].
    Deep,
    /// A second reading that is not script, or that the main reading covers.
    Dropped,
    /// Past the line's budget for second readings.
    Complex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum Mode {
    #[default]
    Code,
    Str(char),
    Template,
    BlockComment,
    /// A regular expression literal; `true` inside a `[...]` class.
    Regex(bool),
}

/// Where the scan stands at the end of the lines seen so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanner {
    mode: Mode,
    /// One entry per open `${`: the `{` depth inside that substitution.
    substitutions: Vec<usize>,
    /// Open brackets, `${` included.
    depth: usize,
    /// A `/` here would start a regular expression rather than divide.
    regex_ok: bool,
    /// The guess for a `/` here can go either way: [`check_nesting`] follows the other reading.
    ambiguous: bool,
    /// The last token was `.` or `?.`: the next name is a property, even one spelled like a
    /// keyword (`o.return / 2` divides).
    member: bool,
    /// The last token was `break` or `continue`: a name next may be its label (see
    /// [`BEFORE_LABEL`]).
    label: bool,
    /// A second reading (see [`check_nesting`]), dropped when it is not script.
    second: bool,
    /// Past the first line (a `#!` line at the very start is a comment).
    started: bool,
    /// Any depth is fine: only directives are being found (see [`Self::unbounded`]).
    unbounded: bool,
}

impl Default for Scanner {
    fn default() -> Self {
        Self {
            mode: Mode::Code,
            substitutions: Vec::new(),
            depth: 0,
            regex_ok: true,
            ambiguous: false,
            member: false,
            label: false,
            second: false,
            started: false,
            unbounded: false,
        }
    }
}

impl Scanner {
    /// A scanner that follows brackets nested to any depth, for a parser that takes them all.
    pub fn unbounded() -> Self {
        Self { unbounded: true, ..Self::default() }
    }
}

/// A character that continues a name: what `char::is_alphanumeric` accepts, `_` and `$`, and any
/// other non-ASCII character that is not a space (`·`, joiners, combining marks).
fn name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$' || (!c.is_ascii() && !space(c))
}

/// White space as script reads it: what `char::is_whitespace` accepts, and U+FEFF.
fn space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// The name that starts with `first`, without its `\u` escapes, and whether it is plain (no
/// escape and no `#`), the only kind that can be a keyword.
fn read_name(first: char, chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> (String, bool) {
    let mut word = String::new();
    let mut plain = first != '#';
    let mut pending = Some(first);
    loop {
        let ch = match pending.take() {
            Some(ch) => ch,
            None => match chars.next_if(|&(_, n)| name_char(n) || n == '\\') {
                Some((_, n)) => n,
                None => break,
            },
        };
        if ch != '\\' {
            word.push(ch);
            continue;
        }
        // `\u0061` or `\u{61}`: part of the name, and never a keyword.
        plain = false;
        if chars.next_if(|&(_, n)| n == 'u').is_none() {
            continue;
        }
        if chars.next_if(|&(_, n)| n == '{').is_some() {
            while chars.next_if(|&(_, n)| n != '}').is_some() {}
            chars.next();
        } else {
            for _ in 0..4 {
                if chars.next_if(|&(_, n)| n.is_ascii_hexdigit()).is_none() {
                    break;
                }
            }
        }
    }
    (word, plain)
}

impl Scanner {
    /// The same state but for the depth (and which reading it is).
    /// A hash of what [`Self::same_state`] compares.
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.mode, &self.substitutions, self.regex_ok, self.ambiguous, self.member, self.label, self.started).hash(&mut h);
        h.finish()
    }

    fn same_state(&self, o: &Scanner) -> bool {
        self.mode == o.mode
            && self.substitutions == o.substitutions
            && self.regex_ok == o.regex_ok
            && self.ambiguous == o.ambiguous
            && self.member == o.member
            && self.label == o.label
            && self.started == o.started
    }

    /// Outside every string, template, substitution and comment, and not inside any bracket.
    /// Directives are recognised only here, so a bare `#target` field in a class body stays script.
    pub fn at_top_level(&self) -> bool {
        self.mode == Mode::Code && self.substitutions.is_empty() && self.depth == 0
    }

    fn open(&mut self) -> bool {
        self.depth += 1;
        self.regex_ok = true;
        !self.unbounded && self.depth > MAX_NESTING
    }

    /// Scan one line; `true` when brackets nest deeper than [`MAX_NESTING`] on it.
    /// `string_goes_on`: the line ended at U+2028/U+2029, which a string literal may contain.
    /// Keeps the main reading only.
    pub fn line(&mut self, line: &str, string_goes_on: bool) -> bool {
        self.scan(line, true, string_goes_on, None) == Scan::Deep
    }

    /// One character inside a string, template, comment or regular expression, for
    /// [`Self::scan`]. `true` when a template substitution it opens nests too deeply.
    fn inside_token(&mut self, c: char, next: Option<char>, chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, escaped: &mut bool) -> bool {
        match self.mode {
            Mode::Code => {}
            Mode::Str(q) => match c {
                '\\' => *escaped = chars.next().is_none(),
                _ if c == q => {
                    self.mode = Mode::Code;
                    self.regex_ok = false;
                }
                _ => {}
            },
            Mode::Template => match c {
                '\\' => {
                    chars.next();
                }
                '`' => {
                    self.mode = Mode::Code;
                    self.regex_ok = false;
                }
                '$' if next == Some('{') => {
                    chars.next();
                    self.substitutions.push(0);
                    self.mode = Mode::Code;
                    if self.open() {
                        return true;
                    }
                }
                _ => {}
            },
            Mode::BlockComment => {
                if c == '*' && next == Some('/') {
                    chars.next();
                    self.mode = Mode::Code;
                }
            }
            Mode::Regex(class) => match c {
                '\\' => {
                    chars.next();
                }
                '[' => self.mode = Mode::Regex(true),
                ']' => self.mode = Mode::Regex(false),
                '/' if !class => {
                    self.mode = Mode::Code;
                    self.regex_ok = false;
                }
                _ => {}
            },
        }
        false
    }

    /// A `/` that is not a comment: a regular expression where one may start, else a division.
    /// After an `ambiguous` token the other reading goes to `check` as a fork at `after` (in
    /// the scanned slice). False when the line has too many forks to check.
    #[must_use]
    fn slash(&mut self, after: usize, ambiguous: bool, check: Option<&mut Check>) -> bool {
        if ambiguous && let Some(check) = check {
            if check.forks.len() >= MAX_FORKS {
                return false;
            }
            if !self.second {
                // Keep where the main reading stands for a while, to compare the fork with.
                check.record_until = check.record_until.max(check.base + after + TRAIL_WINDOW);
            }
            let mut other = self.clone();
            other.second = true;
            if self.regex_ok {
                other.regex_ok = true;
            } else {
                other.mode = Mode::Regex(false);
            }
            check.forks.push((other, after));
        }
        if self.regex_ok {
            self.mode = Mode::Regex(false);
        } else {
            self.regex_ok = true;
        }
        true
    }

    /// [`Self::line`] from the start of a line (`line_start`) or from where a second reading
    /// goes on. With `forks`, an ambiguous `/` also hands its other reading to the caller, with
    /// the byte offset where it goes on.
    fn scan(&mut self, line: &str, line_start: bool, string_goes_on: bool, mut check: Option<&mut Check>) -> Scan {
        if !std::mem::replace(&mut self.started, true) && line_start && line.starts_with("#!") {
            return Scan::End;
        }
        let mut chars = line.char_indices().peekable();
        let mut escaped = false;
        // Only spaces and comments so far on this line: `-->` here is a comment.
        let mut bare = line_start;
        // The last token was a whole decimal number with nothing after it yet: a `.` right here
        // is its decimal point (`1. / 2` divides).
        let mut after_integer = false;
        while let Some((at, c)) = chars.next() {
            escaped = false;
            let next = chars.peek().map(|&(_, n)| n);
            let integer = std::mem::take(&mut after_integer);
            if self.second
                && let Some(check) = check.as_deref_mut()
            {
                check.steps += 1;
                if check.steps > check.budget {
                    return Scan::Complex;
                }
            }
            match self.mode {
                Mode::Code => {
                    // Cleared by every token; whitespace and comments keep them.
                    let ambiguous = std::mem::take(&mut self.ambiguous);
                    let member = std::mem::take(&mut self.member);
                    let label = std::mem::take(&mut self.label);
                    let rest = line.get(at..).unwrap_or_default();
                    let was_bare = std::mem::replace(&mut bare, space(c) || (c == '/' && next == Some('*')));
                    match c {
                        '/' if next == Some('/') => break,
                        '/' if next == Some('*') => {
                            chars.next();
                            self.mode = Mode::BlockComment;
                            self.ambiguous = ambiguous;
                            self.member = member;
                            self.label = label;
                            bare = was_bare;
                        }
                        '/' => {
                            if !self.slash(at + 1, ambiguous, check.as_deref_mut()) {
                                return Scan::Complex;
                            }
                        }
                        // HTML-like comments, which scripts (not modules) allow.
                        '<' if rest.starts_with("<!--") => break,
                        '-' if was_bare && rest.starts_with("-->") => break,
                        '\'' | '"' => self.mode = Mode::Str(c),
                        '`' => self.mode = Mode::Template,
                        '(' | '[' => {
                            if self.open() {
                                return Scan::Deep;
                            }
                        }
                        '{' => {
                            if let Some(depth) = self.substitutions.last_mut() {
                                *depth += 1;
                            }
                            if self.open() {
                                return Scan::Deep;
                            }
                        }
                        ')' | ']' => {
                            self.depth = self.depth.saturating_sub(1);
                            self.regex_ok = false;
                            self.ambiguous = c == ')';
                        }
                        '}' => {
                            self.depth = self.depth.saturating_sub(1);
                            self.regex_ok = false;
                            match self.substitutions.last_mut() {
                                Some(0) => {
                                    self.substitutions.pop();
                                    self.mode = Mode::Template;
                                }
                                Some(depth) => {
                                    *depth -= 1;
                                    self.ambiguous = true;
                                }
                                None => self.ambiguous = true,
                            }
                        }
                        c if space(c) => {
                            self.ambiguous = ambiguous;
                            self.member = member;
                            self.label = label;
                            bare = was_bare;
                        }
                        // A name: letters and the like, a `#private` name, `\u` escapes.
                        c if name_char(c) || c == '\\' || (c == '#' && next.is_some_and(|n| name_char(n) || n == '\\')) => {
                            let (word, plain) = read_name(c, &mut chars);
                            let keyword = plain && !member && BEFORE_REGEX.contains(&word.as_str());
                            self.regex_ok = keyword;
                            // `of`, `yield` and `await` may be names too, and a label may end its
                            // statement: follow both readings.
                            self.ambiguous = (keyword && ALSO_NAMES.contains(&word.as_str())) || (label && !keyword);
                            self.label = keyword && BEFORE_LABEL.contains(&word.as_str());
                            after_integer = plain && word.starts_with(|d: char| d.is_ascii_digit()) && word.chars().all(|d| d.is_ascii_digit() || d == '_');
                        }
                        // `a++ / b` divides: `++` after an operand is postfix and keeps the
                        // division reading (and the ambiguity of a `)` before it). After an
                        // operator it is prefix, and an operand follows.
                        '+' | '-' if next == Some(c) => {
                            chars.next();
                            self.ambiguous = ambiguous;
                        }
                        // `1.` is a number (its fraction, if any, is the next word): `1. / 2`
                        // divides, and a `.` after `1.5` reaches a property.
                        // `1..x`: the first `.` is the decimal point, the second reaches `x`.
                        '.' if integer => {
                            self.regex_ok = false;
                            // The fraction and exponent belong to the number too.
                            while chars.next_if(|&(_, n)| n.is_ascii_alphanumeric() || n == '_').is_some() {}
                        }
                        // `...` spreads an expression; `.` and `?.` reach a property.
                        '.' if next == Some('.') => {
                            chars.next();
                            if chars.peek().map(|&(_, n)| n) == Some('.') {
                                chars.next();
                            }
                            self.regex_ok = true;
                        }
                        '.' => {
                            self.regex_ok = true;
                            self.member = true;
                        }
                        _ => self.regex_ok = true,
                    }
                    if self.mode == Mode::Code
                        && SYNC.contains(&c)
                        && let Some(check) = check.as_deref_mut()
                    {
                        let here = check.base + at + c.len_utf8();
                        if !self.second {
                            if here < check.record_until && check.trail.len() < TRAIL_MAX {
                                check.trail.push((here, self.fingerprint(), self.depth));
                            }
                        } else if let Ok(i) = check.trail.binary_search_by_key(&here, |t| t.0)
                            && check.trail[i].1 == self.fingerprint()
                            && check.trail[i].2 >= self.depth
                        {
                            return Scan::Dropped;
                        }
                    }
                }
                _ => {
                    if self.inside_token(c, next, &mut chars, &mut escaped) {
                        return Scan::Deep;
                    }
                }
            }
        }
        // A regular expression literal cannot span lines: a second reading that ends a line in
        // one is not script. The main reading goes on as code.
        if matches!(self.mode, Mode::Regex(_)) && self.second {
            return Scan::Dropped;
        }
        // A string ends with its line unless the line ends in a backslash (a continuation) or in
        // U+2028/U+2029.
        if matches!(self.mode, Mode::Str(_)) && !escaped && !string_goes_on || matches!(self.mode, Mode::Regex(_)) {
            self.mode = Mode::Code;
        }
        Scan::End
    }
}

// `std::time::Instant` has no clock on wasm32-unknown-unknown.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{MAX_NESTING, Refused, TOO_COMPLEX, TOO_DEEP, check_nesting};

    fn too_deep(line: usize) -> Result<(), Refused> {
        Err(Refused { line, why: TOO_DEEP })
    }

    #[test]
    fn counts_every_bracket_kind_and_reports_the_line() {
        let ok = format!("{}{}", "([{".repeat(MAX_NESTING / 3), "}])".repeat(MAX_NESTING / 3));
        assert_eq!(check_nesting(&ok), Ok(()));
        assert_eq!(check_nesting(&"[".repeat(MAX_NESTING + 1)), too_deep(1));
        assert_eq!(check_nesting(&format!("a\r{}\n{}", "(".repeat(MAX_NESTING), "`${")), too_deep(3));
        // Closing brackets free their level again.
        assert_eq!(check_nesting(&"(1)".repeat(10 * MAX_NESTING)), Ok(()));
    }

    #[test]
    fn brackets_inside_tokens_do_not_count() {
        let many = "(".repeat(2 * MAX_NESTING);
        for src in [
            format!("'{many}'"),
            format!("\"{many}\""),
            format!("`{many}\n{many}`"),
            format!("// {many}"),
            format!("/* {many}\n{many} */"),
            format!("var r = /{}/;", "\\(".repeat(2 * MAX_NESTING)),
            format!("x = /[/(]{}/;", "\\(".repeat(2 * MAX_NESTING)),
            format!("function f() {{ return /[(]{}/ }}", "\\(".repeat(2 * MAX_NESTING)),
        ] {
            assert_eq!(check_nesting(&src), Ok(()), "{src}");
        }
        // A division, not a regex: the brackets after it count.
        assert_eq!(check_nesting(&format!("a / b; c = {many}")), too_deep(1));
    }

    #[test]
    fn a_slash_after_a_closing_brace_does_not_hide_brackets() {
        // `{}` is an object literal here, so the `/` divides; the brackets after it count.
        let deep = format!("const x = {{}} / {}1{} / 1;", "(".repeat(10_000), ")".repeat(10_000));
        assert_eq!(check_nesting(&deep), too_deep(1));
        // A block, then a regular expression: read as a division, which only miscounts this line.
        assert_eq!(check_nesting("if (x) {} /ab/.test(s)"), Ok(()));
        // Brackets on later lines are still counted.
        assert_eq!(check_nesting(&format!("if (x) {{}} /ab/.test(s)\n{}", "[".repeat(MAX_NESTING + 1))), too_deep(2));
    }

    #[test]
    fn a_quote_or_backtick_after_an_ambiguous_slash_does_not_hide_later_lines() {
        let deep = format!("var a = {}1{};", "(".repeat(MAX_NESTING + 1), ")".repeat(MAX_NESTING + 1));
        // Read as a division, the backtick opens a template that would swallow line 2; read as a
        // regular expression, line 2 is code. Both readings are followed.
        for first in ["if (true) {} /`/;", "if (x) /`/.test(s);", "if (x) {} /'/;", "if (x) /\"/;", "if (x) {} /* c */ /`/;"] {
            assert_eq!(check_nesting(&format!("{first}\n{deep}")), too_deep(2), "{first}");
        }
        // The same holds when the division reading is the one that counts the brackets.
        assert_eq!(check_nesting(&format!("if (x) {{}} /`/;\nvar b = `${{{}`;", "(".repeat(MAX_NESTING))), too_deep(2));
        // Ordinary code with both readings stays within the limit.
        let ok = "if (a) {} /b/.test(c);\n(x) / 2;\nvar o = ({}) / 3;\nvar t = `a${b}c`;\nfunction f() { return [1, (2), {a: 3}] }";
        assert_eq!(check_nesting(ok), Ok(()));
        // Readings that end a line alike are merged, so many such slashes stay cheap.
        assert_eq!(check_nesting(&"(a) / (b) / (c);\n".repeat(5000)), Ok(()));
    }

    #[test]
    fn a_slash_after_a_postfix_operator_or_a_keyword_named_property_is_also_read_as_division() {
        let deep = |prefix: &str| format!("{prefix} {}1{} / 1;", "(".repeat(20_000), ")".repeat(20_000));
        for prefix in [
            "var n = 1. /",
            "x = 0x1F + 2. /",
            "var a=1; a++ /",
            "var a=1; a-- /",
            "var o={return:1}; o.return /",
            "var o={typeof:1}; o.typeof /",
            "x = {in: 2}.in /",
            "o?.void /",
            "(a)++ /",
            "o. /* c */ return /",
        ] {
            assert_eq!(check_nesting(&deep(prefix)), too_deep(1), "{prefix}");
        }
        // The regular-expression readings still work.
        let ok = "function f(s) { return /\\(/.test(s) }
var n = 0; n++;
if (typeof /x/ === 'object') n--;
var o = {return: 1}; var q = o.return / 2;";
        assert_eq!(check_nesting(ok), Ok(()));
        // A regular expression after `return` holding brackets does not count them.
        let re = format!("function f(s) {{ return /{}/.test(s) }}", "\\(".repeat(MAX_NESTING + 1));
        assert_eq!(check_nesting(&re), Ok(()));
    }

    /// The reviewer's reproducers and their kin: whatever comes before a `/`, deep brackets after
    /// it are counted, because both readings are followed.
    #[test]
    fn no_token_before_a_slash_hides_brackets() {
        let deep = |prefix: &str| format!("{prefix} {}1{} / 1;", "(".repeat(20_000), ")".repeat(20_000));
        for prefix in [
            "x = 1.5.return /",
            "x = 0x1F.in /",
            "x = 1e5.of /",
            "x = 1n.do /",
            "var of = 4; of /",
            "var yield = 4; yield /",
            "var await = 4; await /",
            "class A { #in = 4; m() { return this.#in /",
            "a\\u{24}in /",
            "a\u{b7}in /",
            "a\u{200c}in /",
            "a\u{301}in /",
            "\u{2118}in /",
            "if (x) {} /",
            "x = `${a}` /",
            "x = 'q' /",
            "x = /re/g /",
        ] {
            assert_eq!(check_nesting(&deep(prefix)), too_deep(1), "{prefix}");
        }
    }

    #[test]
    fn ordinary_regular_expressions_hold_up_over_many_lines() {
        let line = "s = s.replace(/\\(/g, '[').replace(/[(]/g, '').split(/[^)]+/); t = /a{2,3}\"/.test(u) / 2; v = w / x / y;\n";
        assert_eq!(check_nesting(&line.repeat(500)), Ok(()));
        let html = "<!-- an old comment, don't mind it (((\nx = 1;\n--> closing (((\n";
        assert_eq!(check_nesting(&html.repeat(100)), Ok(()));
        // One long minified line with many slashes stays quick.
        let minified = "a=b/c;d=/x\\//g.test(e)?f/2:g;".repeat(20_000);
        let start = std::time::Instant::now();
        assert_eq!(check_nesting(&minified), Ok(()));
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
    }

    #[test]
    fn ordinary_scripts_with_divisions_after_brackets_are_not_refused() {
        let lines: String = (0..70).map(|i| format!("var a{i} = Math.round(w / 2) / 2; x{i} = (a.width / 2) / scale;\n")).collect();
        assert_eq!(check_nesting(&lines), Ok(()));
        assert_eq!(check_nesting(&"t=/[/]/;\n".repeat(64)), Ok(()));
        assert_eq!(check_nesting(&"s=s.replace(/[/]/g,'')/1;".repeat(22)), Ok(()));
        let mixed = "if (/^\\d+$/.test(v)) { n = parseInt(v) / 100; }\nvar parts = s.split(/=/), half = (w + 1) / 2;\nclass A { #x = 1; get x() { return this.#x / 2; } }\nasync function* g() { yield /a/; await /b/; }\nfor (var of of list) total += of / 2;\n";
        assert_eq!(check_nesting(&mixed.repeat(30)), Ok(()));
    }

    #[test]
    fn hashbang_and_html_comments_are_comments() {
        let deep = format!("var a = {}1{};", "(".repeat(100), ")".repeat(100));
        assert_eq!(check_nesting(&format!("#!node `\n{deep}")), too_deep(2), "a backtick in the #! line opens nothing");
        assert_eq!(check_nesting(&format!("/*\n*/ --> `\n{deep}")), too_deep(3), "--> after a comment that spanned lines is a comment");
        assert_eq!(check_nesting("#!/bin/sh it's\nx = 1;"), Ok(()));
        assert_eq!(check_nesting("x = 1;\n#!not first `\ny = 2;"), Ok(()), "only the first line can be a #! line; the rest is the guess");
    }

    #[test]
    fn a_line_whose_second_readings_never_meet_the_main_one_is_refused_not_checked_slowly() {
        // After `)` the regex reading of `/'/` and the division reading part ways for good: the
        // division reading is inside a string to the end of the line.
        let line = "if (x) /'/ + ((((1))));".repeat(8 * super::SECOND_PASSES);
        let start = std::time::Instant::now();
        assert_eq!(check_nesting(&line), Err(Refused { line: 1, why: TOO_COMPLEX }));
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
    }

    /// From the fourth review: the main reading's sync points are kept only near forks, so a
    /// long line costs little memory, and forks far into it are still compared and dropped.
    #[test]
    fn long_lines_with_forks_far_apart_are_checked() {
        let filler = ";".repeat(3 * super::TRAIL_WINDOW);
        let line = format!("a = (b) / 2;{filler}c = (d) / 3;{filler}e = (f) / [1][0];");
        assert_eq!(check_nesting(&line), Ok(()));
        let deep = format!("{filler}g = (h) / {}1{}", "(".repeat(100), ")".repeat(100));
        assert_eq!(check_nesting(&deep), Err(Refused { line: 1, why: TOO_DEEP }));
    }

    #[test]
    fn a_line_with_too_many_forks_is_refused_quickly() {
        let line = "(a) / 1 + ".repeat(4 * super::MAX_FORKS) + "1";
        let start = std::time::Instant::now();
        assert_eq!(check_nesting(&line), Err(Refused { line: 1, why: TOO_COMPLEX }));
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
    }

    /// From the third review: what the guess knew at the end of a line still steers a `/` that
    /// starts the next one; U+FEFF is white space; `1..in` reaches a property.
    #[test]
    fn a_slash_starting_a_line_a_bom_and_a_double_dot_keep_their_reading() {
        let deep = format!("{}1{}", "(".repeat(20_000), ")".repeat(20_000));
        let cases = [
            (format!("var a = 2;\nvar x = a\n/ {deep} / 1;"), 3),
            (format!("var x = 1, a = 2, y;\nif (x) /'/; y = a\n/ {deep} / 1;"), 3),
            (format!("x = (a)\n/ {deep} / 1;"), 2),
            (format!("x = /*\n*/ a\n/ {deep} / 1;"), 3),
            (format!("function f(){{ return\u{feff}/'/ }}; var x = {deep};"), 1),
            (format!("var x = 1;\nif (x)\u{feff}/'/; var y = {deep};"), 2),
            (format!("var x = 1..in / {deep} / 1;"), 1),
            (format!("var x = 1..return / {deep} / 1;"), 1),
        ];
        for (src, line) in cases {
            assert_eq!(check_nesting(&src), too_deep(line), "{}", src.chars().take(60).collect::<String>());
        }
    }

    #[test]
    fn minified_lines_with_many_divisions_after_brackets_are_checked() {
        let a = "function c(a,b){return Math.floor((a+b)/2)}".repeat(200);
        let b = "var d=app.activeDocument,w=d.width,h=d.height,l=d.activeLayer;l.translate((w-l.bounds[2])/2,(h-l.bounds[3])/2);".repeat(100);
        for src in [a, b] {
            let start = std::time::Instant::now();
            assert_eq!(check_nesting(&src), Ok(()));
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
        }
    }
}
