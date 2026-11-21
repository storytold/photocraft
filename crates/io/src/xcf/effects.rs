//! Bounded stored effect records; execution follows in later slices.

mod config;

mod envelope;

mod resources;

pub(super) use config::parse_config;

#[cfg(test)]
pub(super) use envelope::POINTER_READS;
#[cfg(test)]
pub(super) use envelope::decode;
pub(super) use envelope::{DecodeWork, decode_with_work, read_pointers, read_pointers_with_work};

pub(super) use resources::literal_rgb;

/// One parsed effect; file order and argument order remain explicit.
#[derive(Debug)]
pub(super) struct StoredEffect<'a> {
    pub name: &'a str,
    pub icon: &'a str,
    pub operation: &'a str,
    pub version: Option<&'a str>,
    pub arguments: Vec<Argument<'a>>,
    pub visible: bool,
    pub opacity: f32,
    pub mode: i32,
    pub blend_space: i32,
    pub composite_space: i32,
    pub composite_mode: i32,
    pub clip: bool,
    pub region: u32,
    pub mask_offset: usize,
    pub record_offset: usize,
    pub raw: &'a [u8],
    pub unknown_properties: Vec<SourceProperty<'a>>,
    pub errors: Vec<String>,
}

#[derive(Debug)]
pub(super) struct SourceProperty<'a> {
    pub offset: usize,
    pub kind: u32,
    pub bytes: &'a [u8],
}

#[derive(Debug)]
pub(super) struct Argument<'a> {
    pub name: &'a str,
    pub value: ArgumentValue<'a>,
}

#[derive(Debug)]
pub(super) enum ArgumentValue<'a> {
    Int(i32),
    UInt(u32),
    Bool(bool),
    Float(f32),
    /// Portable graph text colors, with an explicit source transfer encoding.
    /// Never constructed by the binary XCF argument decoder.
    LiteralColor {
        rgba: [f32; 4],
        linear: bool,
    },
    String(&'a str),
    Enum(i32),
    Config(Config<'a>),
    Color(Option<ColorLiteral<'a>>),
    Unknown {
        kind: u32,
        bytes: &'a [u8],
    },
    Invalid {
        kind: u32,
        bytes: &'a [u8],
        message: String,
    },
}

/// Typed source color bytes and their own profile. No implicit document ICC.
#[derive(Debug)]
pub(super) struct ColorLiteral<'a> {
    pub encoding: &'a str,
    pub pixels: &'a [u8],
    pub profile: &'a [u8],
}

/// A bounded configuration syntax tree. It contains no commands or evaluation API.
#[derive(Debug)]
pub(super) struct Config<'a> {
    pub raw: &'a str,
    pub forms: Vec<ConfigExpr<'a>>,
}

#[derive(Debug)]
pub(super) enum ConfigExpr<'a> {
    Symbol(&'a str),
    String(String),
    /// GimpConfig quotes may carry escaped binary color/profile data. These are
    /// bytes, not source text or executable instructions.
    Bytes(Vec<u8>),
    Number(f64),
    List(Vec<ConfigExpr<'a>>),
}

impl<'a> ConfigExpr<'a> {
    /// Return only a parsed symbol, leaving other configuration value kinds distinguishable.
    pub fn as_symbol(&self) -> Option<&'a str> {
        if let Self::Symbol(value) = self { Some(value) } else { None }
    }
    /// Return only a numeric configuration atom without coercing strings or symbols.
    pub fn as_number(&self) -> Option<f64> {
        if let Self::Number(value) = self { Some(*value) } else { None }
    }
    /// Borrow an ordered configuration list without flattening repeated source groups.
    pub fn as_list(&self) -> Option<&[Self]> {
        if let Self::List(value) = self { Some(value) } else { None }
    }
    /// Read the leading symbol of a configuration list when both shapes are valid.
    pub fn head(&self) -> Option<&'a str> {
        self.as_list()?.first()?.as_symbol()
    }
}

impl<'a> Config<'a> {
    /// Repeated groups remain ordered, including channel/curve/levels groups.
    pub fn fields(&self, name: &str) -> impl Iterator<Item = &[ConfigExpr<'a>]> {
        self.forms.iter().filter_map(move |form| {
            let list = form.as_list()?;
            if list.first()?.as_symbol() == Some(name) { list.get(1..) } else { None }
        })
    }
}

impl<'a> StoredEffect<'a> {
    /// Resolve a unique named stored argument; duplicate source fields are invalid instead of silently overwritten.
    pub fn argument(&self, name: &str) -> std::result::Result<Option<&ArgumentValue<'a>>, String> {
        let mut matches = self.arguments.iter().filter(|arg| arg.name == name);
        let first = matches.next().map(|arg| &arg.value);
        if matches.next().is_some() {
            return Err(format!("duplicate stored effect argument {name}"));
        }
        Ok(first)
    }
    /// Read admitted stored numeric variants without accepting nonnumeric argument kinds.
    pub fn number(&self, name: &str) -> std::result::Result<Option<f64>, String> {
        match self.argument(name)? {
            None => Ok(None),
            Some(ArgumentValue::Float(value)) => Ok(Some(f64::from(*value))),
            Some(ArgumentValue::Int(value)) => Ok(Some(f64::from(*value))),
            Some(ArgumentValue::UInt(value)) => Ok(Some(f64::from(*value))),
            _ => Err(format!("effect argument {name} is not a number")),
        }
    }
    /// Read an optional typed source boolean, distinguishing absence from a mismatched argument type.
    pub fn boolean(&self, name: &str) -> std::result::Result<Option<bool>, String> {
        match self.argument(name)? {
            None => Ok(None),
            Some(ArgumentValue::Bool(value)) => Ok(Some(*value)),
            _ => Err(format!("effect argument {name} is not a boolean")),
        }
    }
    /// Read an optional stored enum value without reinterpreting unrelated integer or string fields.
    pub fn enum_value(&self, name: &str) -> std::result::Result<Option<i32>, String> {
        match self.argument(name)? {
            None => Ok(None),
            Some(ArgumentValue::Enum(value)) => Ok(Some(*value)),
            _ => Err(format!("effect argument {name} is not an enum")),
        }
    }
    /// Borrow a typed configuration argument while rejecting other stored value kinds.
    pub fn config(&self, name: &str) -> std::result::Result<Option<&Config<'a>>, String> {
        match self.argument(name)? {
            None => Ok(None),
            Some(ArgumentValue::Config(value)) => Ok(Some(value)),
            _ => Err(format!("effect argument {name} is not configuration text")),
        }
    }
}
