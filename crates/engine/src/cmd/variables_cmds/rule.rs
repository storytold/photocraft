//! The rule language of variable bindings: a small, side-effect-free expression evaluated against
//! one data set. A rule computes a layer's visibility, text or replacement-image path from any of
//! the document's variables.
//!
//! ```text
//! Type == "gear"                                   visibility from a comparison
//! Type == "gear" and (Rarity == "epic" or Level >= 50)
//! not empty(Badge)
//! "{Name} - Lv {Level}"                            text template: `{var}` inside a string
//! if(Rarity == "epic", "★ {Name}", Name)           conditional text
//! switch(Type, "gear", "icons/gear.png", "potion", "icons/potion.png", "icons/other.png")
//! ```
//!
//! Values are text, numbers or booleans. CSV cells are text; a comparison is numeric when both
//! sides read as numbers, otherwise text compared ignoring case and surrounding whitespace.
//! A variable missing from the data set is empty text. Names with spaces are written `{Card Name}`.
//! Operators: `or` `||`, `and` `&&`, `not` `!`, `==` `=` `!=` `<>` `<` `<=` `>` `>=`, `+` (adds
//! numbers, else joins text), `&` (joins text), `-` `*` `/`. Functions: `if(cond, then, else?)`,
//! `switch(value, match, result, …, default?)`, `empty(x)`, `contains(a, b)`, `startsWith(a, b)`,
//! `endsWith(a, b)`, `upper(x)`, `lower(x)`, `trim(x)`.

/// Longest rule source accepted, in bytes.
const MAX_LEN: usize = 4096;
/// Deepest nesting of parentheses, calls and operators.
const MAX_DEPTH: usize = 64;

/// A parsed rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule(Expr);

/// A rule's result.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Text(String),
    Num(f64),
    Bool(bool),
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Lit(Val),
    Var(String),
    /// A string with `{var}` holes: literal and variable parts, in order.
    Template(Vec<Part>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Lit(String),
    Var(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Join,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Func {
    If,
    Switch,
    Empty,
    Contains,
    StartsWith,
    EndsWith,
    Upper,
    Lower,
    Trim,
}

impl Func {
    fn from_name(n: &str) -> Option<Self> {
        Some(match n.to_ascii_lowercase().as_str() {
            "if" => Self::If,
            "switch" => Self::Switch,
            "empty" => Self::Empty,
            "contains" => Self::Contains,
            "startswith" => Self::StartsWith,
            "endswith" => Self::EndsWith,
            "upper" => Self::Upper,
            "lower" => Self::Lower,
            "trim" => Self::Trim,
            _ => return None,
        })
    }

    /// Accepted argument counts (inclusive).
    fn arity(self) -> (usize, usize) {
        match self {
            Self::If => (2, 3),
            Self::Switch => (3, usize::MAX),
            Self::Contains | Self::StartsWith | Self::EndsWith => (2, 2),
            Self::Empty | Self::Upper | Self::Lower | Self::Trim => (1, 1),
        }
    }
}

// --------------------------------------------------------------------------- values

impl Val {
    /// The value as text (numbers without a trailing `.0`).
    pub fn text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Bool(b) => b.to_string(),
            Self::Num(n) => fmt_num(*n),
        }
    }

    /// Truthiness: false, 0, and the text "", "false", "0", "no", "off", "hidden" are false.
    pub fn truthy(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Num(n) => *n != 0.0 && !n.is_nan(),
            Self::Text(s) => !matches!(s.trim().to_lowercase().as_str(), "" | "false" | "0" | "no" | "off" | "hidden"),
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(*n),
            Self::Text(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
            Self::Bool(_) => None,
        }
    }

    /// Text normalised for comparison: trimmed, lower case.
    fn key(&self) -> String {
        self.text().trim().to_lowercase()
    }
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else if n.is_finite() {
        let s = format!("{n:.10}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        n.to_string()
    }
}

fn equal(a: &Val, b: &Val) -> bool {
    if matches!(a, Val::Bool(_)) || matches!(b, Val::Bool(_)) {
        return a.truthy() == b.truthy();
    }
    match (a.num(), b.num()) {
        (Some(x), Some(y)) => x == y,
        _ => a.key() == b.key(),
    }
}

fn order(a: &Val, b: &Val) -> std::cmp::Ordering {
    match (a.num(), b.num()) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.key().cmp(&b.key()),
    }
}

// --------------------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(Vec<Part>),
    Ident(String),
    /// `{name with spaces}`.
    Braced(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut it = src.char_indices().peekable();
    while let Some(&(i, c)) = it.peek() {
        if c.is_whitespace() {
            it.next();
            continue;
        }
        match c {
            '(' | ')' | ',' => {
                it.next();
                out.push(match c {
                    '(' => Tok::LParen,
                    ')' => Tok::RParen,
                    _ => Tok::Comma,
                });
            }
            '"' | '\'' => {
                it.next();
                out.push(Tok::Str(lex_string(&mut it, c)?));
            }
            '{' => {
                it.next();
                let mut name = String::new();
                loop {
                    match it.next() {
                        Some((_, '}')) => break,
                        Some((_, ch)) => name.push(ch),
                        None => return Err("unclosed `{` in a variable name".into()),
                    }
                }
                let name = name.trim();
                if name.is_empty() {
                    return Err("empty `{}` variable name".into());
                }
                out.push(Tok::Braced(name.to_string()));
            }
            c if c.is_ascii_digit() || (c == '.' && src.get(i + 1..).and_then(|r| r.chars().next()).is_some_and(|d| d.is_ascii_digit())) => {
                let mut s = String::new();
                while let Some(&(_, d)) = it.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        s.push(d);
                        it.next();
                    } else {
                        break;
                    }
                }
                out.push(Tok::Num(s.parse().map_err(|_| format!("bad number `{s}`"))?));
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut s = String::new();
                while let Some(&(_, d)) = it.peek() {
                    if d.is_alphanumeric() || d == '_' {
                        s.push(d);
                        it.next();
                    } else {
                        break;
                    }
                }
                out.push(Tok::Ident(s));
            }
            _ => {
                let rest = src.get(i..).unwrap_or("");
                let op = ["==", "!=", "<>", "<=", ">=", "&&", "||", "=", "<", ">", "!", "+", "-", "*", "/", "&"]
                    .into_iter()
                    .find(|op| rest.starts_with(op))
                    .ok_or_else(|| format!("unexpected `{c}`"))?;
                for _ in 0..op.len() {
                    it.next();
                }
                out.push(Tok::Op(op));
            }
        }
    }
    Ok(out)
}

/// The rest of a string literal opened by `quote`: escapes `\"` `\'` `\\` `\n` `\{` `\}`, `{{`/`}}`
/// for literal braces, and `{name}` holes.
fn lex_string(it: &mut std::iter::Peekable<std::str::CharIndices<'_>>, quote: char) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut lit = String::new();
    loop {
        let Some((_, c)) = it.next() else { return Err("unclosed string".into()) };
        match c {
            c if c == quote => break,
            '\\' => match it.next() {
                Some((_, 'n')) => lit.push('\n'),
                Some((_, 't')) => lit.push('\t'),
                Some((_, e)) => lit.push(e),
                None => return Err("unclosed string".into()),
            },
            '{' if it.peek().map(|p| p.1) == Some('{') => {
                it.next();
                lit.push('{');
            }
            '}' if it.peek().map(|p| p.1) == Some('}') => {
                it.next();
                lit.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match it.next() {
                        Some((_, '}')) => break,
                        Some((_, ch)) if ch == quote => return Err("unclosed `{` in a string (write `{{` for a literal brace)".into()),
                        Some((_, ch)) => name.push(ch),
                        None => return Err("unclosed string".into()),
                    }
                }
                let name = name.trim();
                if name.is_empty() {
                    return Err("empty `{}` in a string (write `{{}}` for literal braces)".into());
                }
                if !lit.is_empty() {
                    parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
                parts.push(Part::Var(name.to_string()));
            }
            c => lit.push(c),
        }
    }
    if !lit.is_empty() || parts.is_empty() {
        parts.push(Part::Lit(lit));
    }
    Ok(parts)
}

// --------------------------------------------------------------------------- parser

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos = self.pos.saturating_add(1);
        t
    }

    /// True (and consumed) when the next token is one of the operator spellings or keywords.
    fn eat(&mut self, ops: &[&str], words: &[&str]) -> bool {
        let hit = match self.peek() {
            Some(Tok::Op(o)) => ops.contains(o),
            Some(Tok::Ident(w)) => words.iter().any(|k| w.eq_ignore_ascii_case(k)),
            _ => false,
        };
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(format!("rule nests deeper than {MAX_DEPTH} levels")) } else { Ok(()) }
    }

    fn or(&mut self) -> Result<Expr, String> {
        self.enter()?;
        let mut l = self.and()?;
        while self.eat(&["||"], &["or"]) {
            l = Expr::Bin(Op::Or, Box::new(l), Box::new(self.and()?));
        }
        self.depth -= 1;
        Ok(l)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.not()?;
        while self.eat(&["&&"], &["and"]) {
            l = Expr::Bin(Op::And, Box::new(l), Box::new(self.not()?));
        }
        Ok(l)
    }

    fn not(&mut self) -> Result<Expr, String> {
        if self.eat(&["!"], &["not"]) {
            self.enter()?;
            let e = Expr::Not(Box::new(self.not()?));
            self.depth -= 1;
            return Ok(e);
        }
        self.cmp()
    }

    fn cmp(&mut self) -> Result<Expr, String> {
        let l = self.add()?;
        let op = match self.peek() {
            Some(Tok::Op("==" | "=")) => Op::Eq,
            Some(Tok::Op("!=" | "<>")) => Op::Ne,
            Some(Tok::Op("<")) => Op::Lt,
            Some(Tok::Op("<=")) => Op::Le,
            Some(Tok::Op(">")) => Op::Gt,
            Some(Tok::Op(">=")) => Op::Ge,
            _ => return Ok(l),
        };
        self.pos += 1;
        Ok(Expr::Bin(op, Box::new(l), Box::new(self.add()?)))
    }

    fn add(&mut self) -> Result<Expr, String> {
        let mut l = self.mul()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op("+")) => Op::Add,
                Some(Tok::Op("-")) => Op::Sub,
                Some(Tok::Op("&")) => Op::Join,
                _ => return Ok(l),
            };
            self.pos += 1;
            l = Expr::Bin(op, Box::new(l), Box::new(self.mul()?));
        }
    }

    fn mul(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op("*")) => Op::Mul,
                Some(Tok::Op("/")) => Op::Div,
                _ => return Ok(l),
            };
            self.pos += 1;
            l = Expr::Bin(op, Box::new(l), Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat(&["-"], &[]) {
            self.enter()?;
            let e = Expr::Neg(Box::new(self.unary()?));
            self.depth -= 1;
            return Ok(e);
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.bump() {
            Some(Tok::Num(n)) => Ok(Expr::Lit(Val::Num(n))),
            Some(Tok::Str(parts)) => Ok(match parts.as_slice() {
                [Part::Lit(s)] => Expr::Lit(Val::Text(s.clone())),
                _ => Expr::Template(parts),
            }),
            Some(Tok::Braced(n)) => Ok(Expr::Var(n)),
            Some(Tok::Ident(w)) if w.eq_ignore_ascii_case("true") => Ok(Expr::Lit(Val::Bool(true))),
            Some(Tok::Ident(w)) if w.eq_ignore_ascii_case("false") => Ok(Expr::Lit(Val::Bool(false))),
            Some(Tok::Ident(w)) if self.peek() == Some(&Tok::LParen) => {
                let f = Func::from_name(&w).ok_or_else(|| format!("unknown function `{w}`"))?;
                self.pos += 1;
                let mut args = Vec::new();
                if self.peek() == Some(&Tok::RParen) {
                    self.pos += 1;
                } else {
                    loop {
                        args.push(self.or()?);
                        match self.bump() {
                            Some(Tok::Comma) => {}
                            Some(Tok::RParen) => break,
                            _ => return Err(format!("expected `,` or `)` in `{w}(…)`")),
                        }
                    }
                }
                let (lo, hi) = f.arity();
                if args.len() < lo || args.len() > hi {
                    return Err(format!("`{w}` takes {}", if lo == hi { format!("{lo} arguments") } else if hi == usize::MAX { format!("at least {lo} arguments") } else { format!("{lo} to {hi} arguments") }));
                }
                Ok(Expr::Call(f, args))
            }
            Some(Tok::Ident(w)) => {
                if ["and", "or", "not"].iter().any(|k| w.eq_ignore_ascii_case(k)) {
                    return Err(format!("expected a value before `{w}`"));
                }
                Ok(Expr::Var(w))
            }
            Some(Tok::LParen) => {
                let e = self.or()?;
                match self.bump() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err("missing `)`".into()),
                }
            }
            Some(t) => Err(format!("unexpected {}", describe(&t))),
            None => Err("rule ends early".into()),
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Op(o) => format!("`{o}`"),
        Tok::RParen => "`)`".into(),
        Tok::Comma => "`,`".into(),
        _ => "value".into(),
    }
}

impl Rule {
    /// Parse a rule, or explain what is wrong with it.
    pub fn parse(src: &str) -> Result<Self, String> {
        if src.len() > MAX_LEN {
            return Err(format!("rule is longer than {MAX_LEN} bytes"));
        }
        if src.trim().is_empty() {
            return Err("rule is empty".into());
        }
        let toks = lex(src)?;
        let mut p = Parser { toks, pos: 0, depth: 0 };
        let e = p.or()?;
        if let Some(t) = p.peek() {
            return Err(format!("unexpected {} after the end of the rule", describe(t)));
        }
        Ok(Self(e))
    }

    /// The variables the rule reads, in first-use order, without duplicates.
    pub fn variables(&self) -> Vec<String> {
        fn walk(e: &Expr, out: &mut Vec<String>) {
            let mut add = |n: &String| {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            };
            match e {
                Expr::Lit(_) => {}
                Expr::Var(n) => add(n),
                Expr::Template(parts) => {
                    for p in parts {
                        if let Part::Var(n) = p {
                            add(n);
                        }
                    }
                }
                Expr::Not(a) | Expr::Neg(a) => walk(a, out),
                Expr::Bin(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Expr::Call(_, args) => args.iter().for_each(|a| walk(a, out)),
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &mut out);
        out
    }

    /// Evaluate against a data set: `lookup` returns a variable's value (None → empty text).
    pub fn eval(&self, lookup: &dyn Fn(&str) -> Option<Val>) -> Result<Val, String> {
        eval(&self.0, lookup)
    }
}

fn var(name: &str, lookup: &dyn Fn(&str) -> Option<Val>) -> Val {
    lookup(name).unwrap_or_else(|| Val::Text(String::new()))
}

fn eval(e: &Expr, lookup: &dyn Fn(&str) -> Option<Val>) -> Result<Val, String> {
    Ok(match e {
        Expr::Lit(v) => v.clone(),
        Expr::Var(n) => var(n, lookup),
        Expr::Template(parts) => Val::Text(
            parts
                .iter()
                .map(|p| match p {
                    Part::Lit(s) => s.clone(),
                    Part::Var(n) => var(n, lookup).text(),
                })
                .collect(),
        ),
        Expr::Not(a) => Val::Bool(!eval(a, lookup)?.truthy()),
        Expr::Neg(a) => Val::Num(-eval(a, lookup)?.num().ok_or("`-` needs a number")?),
        Expr::Bin(Op::Or, a, b) => Val::Bool(eval(a, lookup)?.truthy() || eval(b, lookup)?.truthy()),
        Expr::Bin(Op::And, a, b) => Val::Bool(eval(a, lookup)?.truthy() && eval(b, lookup)?.truthy()),
        Expr::Bin(op, a, b) => {
            let (x, y) = (eval(a, lookup)?, eval(b, lookup)?);
            use std::cmp::Ordering::*;
            match op {
                Op::Eq => Val::Bool(equal(&x, &y)),
                Op::Ne => Val::Bool(!equal(&x, &y)),
                Op::Lt => Val::Bool(order(&x, &y) == Less),
                Op::Le => Val::Bool(order(&x, &y) != Greater),
                Op::Gt => Val::Bool(order(&x, &y) == Greater),
                Op::Ge => Val::Bool(order(&x, &y) != Less),
                Op::Join => Val::Text(x.text() + &y.text()),
                Op::Add => match (x.num(), y.num()) {
                    (Some(m), Some(n)) => Val::Num(m + n),
                    _ => Val::Text(x.text() + &y.text()),
                },
                Op::Sub | Op::Mul | Op::Div => {
                    let sym = match op {
                        Op::Sub => "-",
                        Op::Mul => "*",
                        _ => "/",
                    };
                    let (Some(m), Some(n)) = (x.num(), y.num()) else {
                        return Err(format!("`{sym}` needs numbers, got `{}` and `{}`", x.text(), y.text()));
                    };
                    match op {
                        Op::Sub => Val::Num(m - n),
                        Op::Mul => Val::Num(m * n),
                        _ if n == 0.0 => return Err("division by zero".into()),
                        _ => Val::Num(m / n),
                    }
                }
                Op::Or | Op::And => Val::Bool(false), // handled above
            }
        }
        Expr::Call(f, args) => {
            let arg = |i: usize| -> Result<Val, String> { args.get(i).map(|a| eval(a, lookup)).unwrap_or_else(|| Ok(Val::Text(String::new()))) };
            match f {
                Func::If => {
                    if arg(0)?.truthy() {
                        arg(1)?
                    } else {
                        arg(2)?
                    }
                }
                Func::Switch => {
                    let x = arg(0)?;
                    let rest = args.get(1..).unwrap_or(&[]);
                    let mut pairs = rest.chunks_exact(2);
                    for pair in pairs.by_ref() {
                        if let [k, v] = pair
                            && equal(&x, &eval(k, lookup)?)
                        {
                            return eval(v, lookup);
                        }
                    }
                    match pairs.remainder() {
                        [default] => eval(default, lookup)?,
                        _ => Val::Text(String::new()),
                    }
                }
                Func::Empty => Val::Bool(arg(0)?.text().trim().is_empty()),
                Func::Contains => Val::Bool(arg(0)?.key().contains(&arg(1)?.key())),
                Func::StartsWith => Val::Bool(arg(0)?.key().starts_with(&arg(1)?.key())),
                Func::EndsWith => Val::Bool(arg(0)?.key().ends_with(&arg(1)?.key())),
                Func::Upper => Val::Text(arg(0)?.text().to_uppercase()),
                Func::Lower => Val::Text(arg(0)?.text().to_lowercase()),
                Func::Trim => Val::Text(arg(0)?.text().trim().to_string()),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, vars: &[(&str, &str)]) -> Result<Val, String> {
        let owned: Vec<(String, String)> = vars.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        Rule::parse(src)?.eval(&|n| owned.iter().find(|(k, _)| k == n).map(|(_, v)| Val::Text(v.clone())))
    }

    fn text(src: &str, vars: &[(&str, &str)]) -> String {
        run(src, vars).unwrap().text()
    }

    fn truth(src: &str, vars: &[(&str, &str)]) -> bool {
        run(src, vars).unwrap().truthy()
    }

    #[test]
    fn comparisons_ignore_case_and_whitespace() {
        assert!(truth("Type == \"gear\"", &[("Type", " Gear ")]));
        assert!(truth("Type = 'GEAR'", &[("Type", "gear")]));
        assert!(!truth("Type != \"gear\"", &[("Type", "gear")]));
        assert!(truth("Type <> \"gear\"", &[("Type", "potion")]));
    }

    #[test]
    fn numbers_compare_numerically() {
        assert!(truth("Level >= 50", &[("Level", "50")]));
        assert!(truth("Level > 9", &[("Level", "10")]), "10 > 9 as numbers, not text");
        assert!(truth("Level == 7", &[("Level", "7.0")]));
        assert!(truth("Name < \"b\"", &[("Name", "Apple")]));
    }

    #[test]
    fn logic_and_precedence() {
        let row = [("Type", "gear"), ("Rarity", "common"), ("Level", "60")];
        assert!(truth("Type == \"gear\" and (Rarity == \"epic\" or Level >= 50)", &row));
        assert!(!truth("Type == \"gear\" && Rarity == \"epic\" || Level < 10", &row));
        assert!(truth("not empty(Type)", &row));
        assert!(truth("!empty(Type) && !(Level < 10)", &row));
        assert!(truth("empty(Missing)", &row), "a missing variable is empty");
    }

    #[test]
    fn templates_and_names_with_spaces() {
        let row = [("Name", "Sword"), ("Level", "3"), ("Card Name", "Blade")];
        assert_eq!(text("\"{Name} - Lv {Level}\"", &row), "Sword - Lv 3");
        assert_eq!(text("\"{ Card Name }!\"", &row), "Blade!");
        assert_eq!(text("{Card Name} & \" x\" & Level", &row), "Blade x3");
        assert_eq!(text("\"{{literal}}\"", &row), "{literal}");
        assert_eq!(text("\"say \\\"hi\\\"\"", &row), "say \"hi\"");
    }

    #[test]
    fn conditional_text_and_lookup() {
        assert_eq!(text("if(Rarity == \"epic\", \"★ {Name}\", Name)", &[("Rarity", "Epic"), ("Name", "Axe")]), "★ Axe");
        assert_eq!(text("if(Rarity == \"epic\", \"★ {Name}\", Name)", &[("Rarity", "rare"), ("Name", "Axe")]), "Axe");
        assert_eq!(text("if(false, \"x\")", &[]), "");
        let sw = "switch(Type, \"gear\", \"icons/gear.png\", \"potion\", \"icons/potion.png\", \"icons/other.png\")";
        assert_eq!(text(sw, &[("Type", "POTION")]), "icons/potion.png");
        assert_eq!(text(sw, &[("Type", "rock")]), "icons/other.png");
        assert_eq!(text("switch(Type, \"a\", \"1\")", &[("Type", "b")]), "", "no default → empty");
    }

    #[test]
    fn arithmetic_and_text_functions() {
        assert_eq!(text("Level + 1", &[("Level", "4")]), "5");
        assert_eq!(text("\"Lv \" + Level", &[("Level", "4")]), "Lv 4");
        assert_eq!(text("Price * 2 / 4 - 0.5", &[("Price", "3")]), "1");
        assert_eq!(text("0.1 + 0.2", &[]), "0.3");
        assert_eq!(text("-Level", &[("Level", "2")]), "-2");
        assert_eq!(text("upper(trim(Name))", &[("Name", "  axe ")]), "AXE");
        assert_eq!(text("lower(Name)", &[("Name", "AXE")]), "axe");
        assert!(truth("contains(Tags, \"FIRE\") and startsWith(Tags, \"hot\") and endsWith(Tags, \"x\")", &[("Tags", "hot fire box")]));
    }

    #[test]
    fn truthiness_of_text() {
        for f in ["", " ", "false", "FALSE", "0", "no", "off", "hidden"] {
            assert!(!truth("V", &[("V", f)]), "{f:?}");
        }
        for t in ["true", "1", "yes", "anything"] {
            assert!(truth("V", &[("V", t)]), "{t:?}");
        }
        assert!(truth("V == true", &[("V", "yes")]));
    }

    #[test]
    fn identifiers_are_listed() {
        let r = Rule::parse("if(Type == \"gear\", \"{Name} {Card Name}\", {Card Name}) & Type").unwrap();
        assert_eq!(r.variables(), vec!["Type", "Name", "Card Name"]);
    }

    #[test]
    fn bad_rules_are_errors_not_panics() {
        let deep = "(".repeat(500) + "1" + &")".repeat(500);
        let nots = "not ".repeat(500) + "x";
        let negs = "-".repeat(500) + "1";
        let long = "a".repeat(MAX_LEN + 1);
        for src in [
            "", "   ", "Type ==", "== 1", "(a", "a)", "\"open", "'", "{open", "{}", "\"{}\"", "\"{open\"", "nope(1)", "if(1)", "upper(1, 2)",
            "switch(a, b)", "a b", "1..2", "@", "and", "a and", "f(", "if(1,", "\\", "é é", deep.as_str(), nots.as_str(), negs.as_str(), long.as_str(),
        ] {
            assert!(Rule::parse(src).is_err(), "{src:?} should not parse");
        }
        assert!(run("1 / 0", &[]).is_err());
        assert!(run("Name * 2", &[("Name", "x")]).is_err());
        assert!(run("-Name", &[("Name", "x")]).is_err());
        // Every prefix of a valid rule either parses or errors (no panics at char boundaries).
        let src = "if(Rarity == \"épique ★\", \"{Name} ✓\", 'x') & {Card Name} >= 1.5";
        for (i, _) in src.char_indices() {
            let _ = src.get(..i).map(Rule::parse);
        }
    }
}
