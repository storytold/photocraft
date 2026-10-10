//! Bounded arithmetic parser shared by every editable numeric widget.
//! No scripting, allocations derived from results, or non-finite values.

pub(crate) fn parse(text: &str) -> Option<f64> {
    if text.len() > 1024 {
        return None;
    }
    let normalized = text.replace('−', "-");
    let mut parser = Parser { rest: &normalized };
    let value = parser.expression(0, 0)?;
    parser.space();
    (parser.rest.is_empty() && value.is_finite()).then_some(value)
}

struct Parser<'a> {
    rest: &'a str,
}
impl Parser<'_> {
    fn space(&mut self) {
        self.rest = self.rest.trim_start();
    }
    fn take(&mut self, token: &str) -> bool {
        self.space();
        if let Some(rest) = self.rest.strip_prefix(token) {
            self.rest = rest;
            true
        } else {
            false
        }
    }
    fn expression(&mut self, minimum: u8, depth: u8) -> Option<f64> {
        if depth >= 32 {
            return None;
        }
        self.space();
        let mut left = if self.take("+") {
            self.expression(3, depth + 1)?
        } else if self.take("-") {
            -self.expression(3, depth + 1)?
        } else if self.take("(") {
            let value = self.expression(0, depth + 1)?;
            if !self.take(")") {
                return None;
            }
            value
        } else if self.take("pi") {
            std::f64::consts::PI
        } else if self.take("tau") {
            std::f64::consts::TAU
        } else {
            self.number()?
        };
        loop {
            self.space();
            let (token, precedence, right_minimum) = if self.rest.starts_with("**") {
                ("**", 3, 3)
            } else {
                match self.rest.chars().next() {
                    Some('+') => ("+", 1, 2),
                    Some('-') => ("-", 1, 2),
                    Some('*') => ("*", 2, 3),
                    Some('/') => ("/", 2, 3),
                    Some('%') => ("%", 2, 3),
                    Some('^') => ("^", 3, 3),
                    _ => break,
                }
            };
            if precedence < minimum {
                break;
            }
            if !self.take(token) {
                return None;
            }
            let right = self.expression(right_minimum, depth + 1)?;
            left = match token {
                "+" => left + right,
                "-" => left - right,
                "*" => left * right,
                "/" if right != 0.0 => left / right,
                "%" if right != 0.0 => left % right,
                "^" | "**" => left.powf(right),
                _ => return None,
            };
            if !left.is_finite() {
                return None;
            }
        }
        left.is_finite().then_some(left)
    }
    fn number(&mut self) -> Option<f64> {
        let mut length = 0;
        let mut exponent = false;
        for (offset, ch) in self.rest.char_indices() {
            let accepted = ch.is_ascii_digit()
                || ch == '.'
                || (!exponent && matches!(ch, 'e' | 'E'))
                || (matches!(ch, '+' | '-') && self.rest.get(..offset).is_some_and(|s| s.ends_with(['e', 'E'])));
            if !accepted {
                break;
            }
            exponent |= matches!(ch, 'e' | 'E');
            length = offset + ch.len_utf8();
        }
        let value = self.rest.get(..length)?.parse::<f64>().ok()?;
        self.rest = self.rest.get(length..)?;
        value.is_finite().then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    #[test]
    fn arithmetic() {
        for (text, expected) in [
            ("1920/2", 960.0),
            ("30+15", 45.0),
            ("2+3*4", 14.0),
            ("(2+3)*4", 20.0),
            ("-2^2", -4.0),
            ("2**3**2", 512.0),
            ("2^-2", 0.25),
            ("10%3", 1.0),
            ("1e-3 * 1000", 1.0),
            ("−12 + .5", -11.5),
            ("pi*2", std::f64::consts::TAU),
            ("tau/2", std::f64::consts::PI),
        ] {
            assert_eq!(parse(text), Some(expected), "{text}");
        }
    }
    #[test]
    fn rejects_invalid_and_hostile_input() {
        for text in ["", "1/0", "0/0", "1%0", "NaN", "inf", "1e999", "2+", "(2*3", "2 3", "pi junk", "sqrt(4)", "2^(1e300)", "你好"] {
            assert_eq!(parse(text), None, "{text}");
        }
        assert_eq!(parse(&"(".repeat(1000)), None);
        assert_eq!(parse(&"1".repeat(1025)), None);
    }
}

#[cfg(test)]
mod widget_tests {
    use egui_kittest::{Harness, kittest::Queryable};
    #[test]
    fn typed_expression_commits_clamps_and_rejects_invalid() {
        let mut h = Harness::builder().build_ui_state(
            |ui, value| {
                ui.add(egui::DragValue::new(value).range(0.0..=1000.0).custom_parser(super::parse));
            },
            100.0_f64,
        );
        for (text, expected) in [("1920/2", 960.0), ("2000*2", 1000.0), ("1/0", 1000.0)] {
            let center = h.get_by_role(egui::accesskit::Role::SpinButton).rect().center();
            h.hover_at(center);
            h.run_steps(1);
            h.drag_at(center);
            h.run_steps(1);
            h.drop_at(center);
            h.run();
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.run();
            h.event(egui::Event::Text(text.to_owned()));
            h.run();
            h.key_press(egui::Key::Enter);
            h.run();
            assert_eq!(*h.state(), expected);
        }
    }
}
