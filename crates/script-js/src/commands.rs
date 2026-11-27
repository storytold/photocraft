//! The session commands scripts can reach, with the parameters each may take.
//!
//! Deny by default: a global that needs another command adds it here. The same check runs on
//! every command a run sends, in process and in the script host's supervisor, so a host that
//! forges a request can't reach anything a script couldn't.
//!
//! The table holds the reads behind `app.documents` and `app.activeDocument`.

use serde_json::Value;

/// How a command parameter must look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Param {
    /// A tab index or a document serial.
    Index,
    Bool,
}

/// Required, or may be left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Required,
    Optional,
}

use Need::{Optional, Required};
use Param::{Bool, Index};

/// Largest index or serial.
const MAX_INDEX: u64 = 1 << 53;

type Spec = (&'static str, &'static [(&'static str, Param, Need)]);

/// The commands, with every parameter each may take.
pub const COMMANDS: &[Spec] = &[
    ("session.inspect", &[]),
    ("document.inspect", &[("document", Index, Optional), ("serial", Index, Optional), ("history", Bool, Optional), ("layers", Bool, Optional)]),
    ("document.activate", &[("document", Index, Optional), ("serial", Index, Optional)]),
];

/// The command ids, in [`COMMANDS`] order.
pub fn allowed() -> impl Iterator<Item = &'static str> {
    COMMANDS.iter().map(|(id, _)| *id)
}

/// The start of `s`, at most `max` bytes, cut at a character boundary.
pub fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

fn param_ok(rule: Param, v: &Value) -> bool {
    match rule {
        Index => v.as_u64().is_some_and(|i| i <= MAX_INDEX),
        Bool => v.is_boolean(),
    }
}

/// Check a command request against [`COMMANDS`]: a known command, an object of parameters,
/// every required one there, none other, each the shape its rule says.
///
/// # Errors
/// What is wrong with it, for the failure message.
pub fn check_command(id: &str, params: &Value) -> Result<(), String> {
    let Some((_, allowed)) = COMMANDS.iter().find(|(name, _)| *name == id) else {
        return Err(format!("the command {:?} is not available to scripts", cut(id, 80)));
    };
    let Value::Object(map) = params else { return Err(format!("{id}: the parameters are not an object")) };
    for key in map.keys() {
        if !allowed.iter().any(|(k, _, _)| k == key) {
            return Err(format!("{id}: the parameter {:?} is not one it takes", cut(key, 80)));
        }
    }
    for (key, rule, need) in *allowed {
        let ok = match (map.get(*key), need) {
            (None, Required) => false,
            (None, Optional) => true,
            (Some(v), _) => param_ok(*rule, v),
        };
        if !ok {
            return Err(format!("{id}: the parameter {key:?} is missing or invalid"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn known_commands_with_their_parameters_pass() {
        for (id, p) in [
            ("session.inspect", json!({})),
            ("document.activate", json!({"document": 0})),
            ("document.activate", json!({"serial": 7})),
            ("document.inspect", json!({"serial": 7, "history": false, "layers": false})),
        ] {
            assert_eq!(check_command(id, &p), Ok(()), "{id} {p}");
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for (id, p) in [
            ("file.open", json!({"path": "/etc/passwd"})),
            ("session.inspect", json!(null)),
            ("session.inspect", json!({"coalesce": "x"})),
            ("session.inspect", json!({"onDocument": 1})),
            ("document.inspect", json!({"serial": "7"})),
            ("document.inspect", json!({"serial": -1})),
            ("document.inspect", json!({"history": 1})),
            ("document.activate", json!({"offset": 1})),
        ] {
            assert!(check_command(id, &p).is_err(), "{id} {p}");
        }
    }
}
