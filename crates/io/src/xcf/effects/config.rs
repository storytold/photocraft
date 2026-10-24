//! A bounded reader for configuration lists, never an evaluator.

use super::{Config, ConfigExpr};

const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 8192;
const MAX_TEXT: usize = 1 << 20;

pub(in crate::xcf) fn parse_config<'a>(raw: &'a str, budget: &mut usize) -> std::result::Result<Config<'a>, String> {
    if raw.len() > MAX_TEXT {
        return Err("effect configuration exceeds 1 MiB".into());
    }
    let mut reader = Reader { raw, pos: 0, nodes: 0, budget };
    let forms = reader.sequence(0, false)?;
    if forms.iter().any(|form| !matches!(form, ConfigExpr::List(_))) {
        return Err("effect configuration must contain named lists".into());
    }
    Ok(Config { raw, forms })
}

struct Reader<'a, 'b> {
    raw: &'a str,
    pos: usize,
    nodes: usize,
    budget: &'b mut usize,
}

impl<'a> Reader<'a, '_> {
    fn charge(&mut self, count: usize) -> std::result::Result<(), String> {
        *self.budget = self.budget.checked_sub(count).ok_or_else(|| "effect configuration exceeds document metadata allowance".to_string())?;
        Ok(())
    }

    fn peek(&self) -> Option<char> {
        self.raw.get(self.pos..)?.chars().next()
    }

    fn advance(&mut self, character: char) {
        self.pos = self.pos.saturating_add(character.len_utf8());
    }

    fn whitespace(&mut self) {
        loop {
            match self.peek() {
                Some(character) if character.is_ascii_whitespace() => self.advance(character),
                Some('#') => {
                    while let Some(character) = self.peek() {
                        self.advance(character);
                        if character == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn sequence(&mut self, depth: usize, nested: bool) -> std::result::Result<Vec<ConfigExpr<'a>>, String> {
        if depth > MAX_DEPTH {
            return Err("effect configuration nesting exceeds 32".into());
        }
        let mut items = Vec::new();
        loop {
            self.whitespace();
            match self.peek() {
                None if nested => return Err("effect configuration has an unterminated list".into()),
                None => return Ok(items),
                Some(')') if nested => {
                    self.advance(')');
                    return Ok(items);
                }
                Some(')') => return Err("effect configuration has an unmatched closing parenthesis".into()),
                _ => {}
            }
            self.nodes = self.nodes.checked_add(1).ok_or_else(|| "effect configuration node count overflow".to_string())?;
            if self.nodes > MAX_NODES {
                return Err("effect configuration exceeds 8,192 syntax nodes".into());
            }
            // Charge before Vec growth, including a conservative capacity allowance.
            self.charge(std::mem::size_of::<ConfigExpr<'a>>().saturating_mul(2))?;
            let item = match self.peek() {
                Some('(') => {
                    self.advance('(');
                    ConfigExpr::List(self.sequence(depth + 1, true)?)
                }
                Some('"') => self.string()?,
                Some(_) => self.atom()?,
                None => return Err("effect configuration ended unexpectedly".into()),
            };
            items.push(item);
        }
    }

    fn atom(&mut self) -> std::result::Result<ConfigExpr<'a>, String> {
        let start = self.pos;
        while let Some(character) = self.peek() {
            if character.is_ascii_whitespace() || matches!(character, '(' | ')' | '#' | '"') {
                break;
            }
            if character.is_control() {
                return Err("effect configuration has a control character in an atom".into());
            }
            self.advance(character);
        }
        let atom = self.raw.get(start..self.pos).ok_or_else(|| "effect configuration atom bounds".to_string())?;
        if atom.is_empty() {
            return Err("effect configuration has an empty atom".into());
        }
        if let Ok(number) = atom.parse::<f64>() {
            if !number.is_finite() {
                return Err("effect configuration has a nonfinite number".into());
            }
            return Ok(ConfigExpr::Number(number));
        }
        Ok(ConfigExpr::Symbol(atom))
    }

    fn string(&mut self) -> std::result::Result<ConfigExpr<'a>, String> {
        self.advance('"');
        let mut text = Vec::new();
        loop {
            let character = self.peek().ok_or_else(|| "effect configuration has an unterminated string".to_string())?;
            self.advance(character);
            if character == '"' {
                return Ok(match String::from_utf8(text) {
                    Ok(text) if !text.contains('\0') => ConfigExpr::String(text),
                    Ok(text) => ConfigExpr::Bytes(text.into_bytes()),
                    Err(error) => ConfigExpr::Bytes(error.into_bytes()),
                });
            }
            let character = if character == '\\' {
                let escape = self.peek().ok_or_else(|| "effect configuration has an incomplete escape".to_string())?;
                self.advance(escape);
                match escape {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'b' => '\u{0008}',
                    'f' => '\u{000c}',
                    '"' => '"',
                    '\\' => '\\',
                    '0'..='7' => {
                        let mut value = escape.to_digit(8).unwrap_or(0);
                        for _ in 0..2 {
                            if let Some(next @ '0'..='7') = self.peek() {
                                self.advance(next);
                                value = value.saturating_mul(8).saturating_add(next.to_digit(8).unwrap_or(0));
                            } else {
                                break;
                            }
                        }
                        let byte = u8::try_from(value).map_err(|_| "effect configuration octal escape exceeds one byte".to_string())?;
                        self.charge(2)?;
                        text.push(byte);
                        continue;
                    }
                    _ => return Err("effect configuration uses an unsupported string escape".into()),
                }
            } else {
                character
            };
            self.charge(character.len_utf8().saturating_mul(2))?;
            let mut encoded = [0; 4];
            text.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_repeated_fields_and_unicode_text_are_data() {
        let raw = "# retained syntax\n(channel red) (gamma 1.25) (channel blue) (gamma 2) (label \"café\\n(x)\")";
        let config = parse_config(raw, &mut (1 << 20)).unwrap();
        assert_eq!(config.fields("channel").map(|value| value[0].as_symbol()).collect::<Vec<_>>(), vec![Some("red"), Some("blue")]);
        assert_eq!(config.fields("gamma").map(|value| value[0].as_number()).collect::<Vec<_>>(), vec![Some(1.25), Some(2.0)]);
        assert!(matches!(&config.forms[4], ConfigExpr::List(list) if matches!(&list[1], ConfigExpr::String(text) if text == "café\n(x)")));
        let escaped = parse_config("(label \"caf\\303\\251\")", &mut (1 << 20)).unwrap();
        assert!(matches!(&escaped.forms[0], ConfigExpr::List(list) if matches!(&list[1], ConfigExpr::String(text) if text == "café")));
    }

    #[test]
    fn malformed_nonfinite_and_bounded_syntax_fails_without_evaluation() {
        for raw in ["(", ")", "(x \"unterminated)", "(x NaN)", "(x inf)", "(x \"\\q\")", "atom"] {
            assert!(parse_config(raw, &mut (1 << 20)).is_err(), "{raw}");
        }
        let deep = format!("{}{}", "(".repeat(34), ")".repeat(34));
        assert!(parse_config(&deep, &mut (1 << 20)).is_err());
        let many = format!("(x {})", "0 ".repeat(8192));
        assert!(parse_config(&many, &mut (1 << 20)).is_err());
        assert!(parse_config("(x 1)", &mut 1).is_err());
    }

    #[test]
    fn quoted_binary_color_and_profile_payloads_are_bytes() {
        let config = parse_config("(color \"\\100\\200\\377\\000\")", &mut (1 << 20)).unwrap();
        assert!(matches!(&config.forms[0], ConfigExpr::List(list) if matches!(&list[1], ConfigExpr::Bytes(bytes) if bytes == &[64,128,255,0])));
    }
}
