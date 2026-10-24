//! Import-only preparation for the finite G001 native candidates.
//! Local implementation verification is distinct from saved-source acceptance.
//! No source graph, operation callback or evaluator enters the native document.
use super::{Argument, ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeMapping, NativeStage, NativeStructure, StoredEffect};

const MAX_BYTES: usize = 32768;
const MAX_TOKENS: usize = 2048;
const MAX_NODES: usize = 64;
const MAX_ARGUMENTS: usize = 256;
const MAX_FILTERS: usize = 64;
const MAX_DEPTH: usize = 8;
const GRAPH_NOTICE: &str = "Bounded source syntax was lowered once to backward-indexed ordinary native stages and ordinary native layer joins. No source node names, text, callback or evaluator remains in executable data. F64 literals retain their precision until individual native controls convert them; native document profile/depth/alpha/compositing rules apply. The node vocabulary is the frozen182 identities/five aliases plus explicitly audited stock graph declarations, native compositors, generators and object/tone/placement adapters; unavailable resource/object values, unfinished binary-input adapters and historical/default provenance remain explicitly inventoried domains";

type Result<T> = std::result::Result<T, MappingFailure>;

use super::graph_schema::Kind;
fn operation(token: &str) -> Option<&'static str> {
    super::graph_porter::operation(token)
        .or_else(|| super::graph_composites::operation(token))
        .or_else(|| super::graph_schema::operation(token))
        .or_else(|| super::graph_tone::operation(token))
        .or_else(|| super::graph_objects::operation(token))
        .or_else(|| super::graph_core::operation(token))
        .or_else(|| super::graph_math::operation(token))
        .or_else(|| super::graph_resources::operation(token))
        .or_else(|| super::graph_displacement::operation(token))
        .or_else(|| super::graph_stock::operation(token))
}
fn field(operation: &str, name: &str) -> Option<Kind> {
    super::graph_porter::field(operation, name)
        .or_else(|| super::graph_composites::field(operation, name))
        .or_else(|| super::graph_schema::field(operation, name))
        .or_else(|| super::graph_tone::field(operation, name))
        .or_else(|| super::graph_objects::field(operation, name))
        .or_else(|| super::graph_core::field(operation, name))
        .or_else(|| super::graph_math::field(operation, name))
        .or_else(|| super::graph_resources::field(operation, name))
        .or_else(|| super::graph_displacement::field(operation, name))
        .or_else(|| super::graph_stock::field(operation, name))
}

// Public GEGL0.4.72 gegl-color.h:57-64 defines CSS hexadecimal forms
// and the sixteen HTML4 colors, separately from linear-light rgb[a](...).
fn color(value: &str) -> Result<([f32; 4], bool)> {
    let (components, alpha) = if let Some(v) = value.strip_prefix("rgba(").and_then(|v| v.strip_suffix(')')) {
        (Some(v), true)
    } else if let Some(v) = value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        (Some(v), false)
    } else {
        (None, false)
    };
    if let Some(components) = components {
        let mut rgba = [0., 0., 0., 1.];
        let mut count = 0;
        for (index, component) in components.split(',').enumerate() {
            let out = rgba.get_mut(index).ok_or("Too many graph color components")?;
            let value = component.trim().parse::<f64>().map_err(|_| "Invalid graph linear color component")?;
            if !value.is_finite() || !(value as f32).is_finite() || (index == 3 && !(0.0..=1.0).contains(&value)) {
                return Err("Graph color exceeds finite native F32/alpha domain".into());
            }
            *out = value as f32;
            count += 1;
        }
        if count != if alpha { 4 } else { 3 } {
            return Err("Graph rgb/rgba component count is invalid".into());
        }
        return Ok((rgba, true));
    }
    if let Some(hex) = value.strip_prefix('#') {
        let mut result = [0, 0, 0, 255];
        match hex.len() {
            3 | 4 => {
                for (out, digit) in result.iter_mut().zip(hex.bytes()) {
                    *out = char::from(digit).to_digit(16).ok_or("Invalid graph hexadecimal color")? as u8 * 17;
                }
            }
            6 | 8 => {
                for (index, out) in result.iter_mut().take(hex.len() / 2).enumerate() {
                    *out = hex.get(index * 2..index * 2 + 2).and_then(|v| u8::from_str_radix(v, 16).ok()).ok_or("Invalid graph hexadecimal color")?;
                }
            }
            _ => return Err("Graph CSS hexadecimal color has an invalid length".into()),
        }
        return Ok((result.map(|v| f32::from(v) / 255.), false));
    }
    let rgb: [u8; 3] = match value.to_ascii_lowercase().as_str() {
        "black" => [0, 0, 0],
        "silver" => [192, 192, 192],
        "gray" => [128, 128, 128],
        "white" => [255, 255, 255],
        "maroon" => [128, 0, 0],
        "red" => [255, 0, 0],
        "purple" => [128, 0, 128],
        "fuchsia" => [255, 0, 255],
        "green" => [0, 128, 0],
        "lime" => [0, 255, 0],
        "olive" => [128, 128, 0],
        "yellow" => [255, 255, 0],
        "navy" => [0, 0, 128],
        "blue" => [0, 0, 255],
        "teal" => [0, 128, 128],
        "aqua" => [0, 255, 255],
        _ => return Err("Graph color is not a documented CSS hex/HTML4 literal; linear numeric color lowering requires its explicit typed adapter".into()),
    };
    Ok(([rgb[0], rgb[1], rgb[2], 255].map(|v| f32::from(v) / 255.), false))
}

// GEGL's public chain entry uses g_shell_parse_argv. Apply only its inert
// quote/backslash rules here: no variables, commands, paths or expressions run.
fn tokens(input: &str) -> Result<Vec<String>> {
    if input.len() > MAX_BYTES {
        return Err("Finite native graph candidate requires at most32768 UTF-8 bytes".into());
    }
    let mut result = Vec::new();
    let mut cursor = 0;
    let next = |at: usize| input.get(at..).and_then(|s| s.chars().next());
    while let Some(ch) = next(cursor) {
        if ch.is_ascii_whitespace() {
            cursor += ch.len_utf8();
            continue;
        }
        if ch == '#' && (cursor == 0 || input.as_bytes().get(cursor - 1).is_some_and(u8::is_ascii_whitespace)) {
            while next(cursor).is_some_and(|c| c != '\n') {
                cursor += next(cursor).ok_or("Invalid graph comment boundary")?.len_utf8();
            }
            continue;
        }
        if result.len() == MAX_TOKENS {
            return Err("Finite native graph candidate exceeds2048 lexical items".into());
        }
        if matches!(ch, '=' | '[' | ']') {
            result.push(ch.to_string());
            cursor += 1;
            continue;
        }
        let mut word = String::new();
        while let Some(ch) = next(cursor) {
            if ch.is_ascii_whitespace() || matches!(ch, '=' | '[' | ']') {
                break;
            }
            if ch == '\0' {
                return Err("Native graph strings cannot contain NUL".into());
            }
            if ch == '\'' || ch == '"' {
                let quote = ch;
                cursor += 1;
                loop {
                    let ch = next(cursor).ok_or("Unterminated native graph quoted literal")?;
                    cursor += ch.len_utf8();
                    if ch == quote {
                        break;
                    }
                    if ch == '\0' {
                        return Err("Native graph strings cannot contain NUL".into());
                    }
                    if quote == '"' && ch == '\\' {
                        let escaped = next(cursor).ok_or("Unterminated native graph quoted escape")?;
                        if matches!(escaped, '$' | '`' | '"' | '\\' | '\n') {
                            cursor += escaped.len_utf8();
                            if escaped != '\n' {
                                word.push(escaped);
                            }
                        } else {
                            // Shell double quotes preserve backslash before other bytes;
                            // in particular, backslash-n is not a C newline escape.
                            word.push('\\');
                        }
                    } else {
                        word.push(ch);
                    }
                }
            } else if ch == '\\' {
                cursor += 1;
                let escaped = next(cursor).ok_or("Unterminated native graph literal escape")?;
                cursor += escaped.len_utf8();
                if escaped == '\0' {
                    return Err("Native graph strings cannot contain NUL".into());
                }
                if escaped != '\n' {
                    word.push(escaped);
                }
            } else {
                word.push(ch);
                cursor += ch.len_utf8();
            }
        }
        result.push(word);
    }
    Ok(result)
}
#[derive(Debug)]
enum NodeValue<'a> {
    Number(f64),
    Integer(i64),
    Boolean(bool),
    Choice(i32),
    Text(&'a str),
    Color { rgba: [f32; 4], linear: bool },
    Configuration(&'a str),
}
#[derive(Debug)]
struct NodeArgument<'a> {
    name: &'a str,
    value: NodeValue<'a>,
}
#[derive(Debug)]
struct Node<'a> {
    operation: &'static str,
    arguments: Vec<NodeArgument<'a>>,
}
struct Decoder<'a> {
    tokens: Vec<&'a str>,
    cursor: usize,
    nodes: usize,
    assignments: usize,
    labels: Vec<(&'a str, usize)>,
    steps: Vec<NativeStage>,
    filters: usize,
    remaining_generation: usize,
    remaining_metadata: usize,
    notes: Vec<String>,
}
impl<'a> Decoder<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.cursor).copied()
    }
    fn take(&mut self) -> Result<&'a str> {
        let value = self.peek().ok_or("Incomplete finite native graph")?;
        self.cursor += 1;
        Ok(value)
    }
    fn expect(&mut self, expected: &str) -> Result<()> {
        if self.take()? == expected { Ok(()) } else { Err(format!("Finite native graph expected {expected}").into()) }
    }
    fn label(&mut self) -> Result<&'a str> {
        let label = self.take()?;
        if label.is_empty() || label.len() > 128 || label.chars().any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '=' | '[' | ']')) {
            return Err("Native graph label must have1..128 UTF-8 bytes without whitespace, controls or structural delimiters".into());
        }
        Ok(label)
    }
    fn read_node(&mut self, context: &MappingContext<'_>) -> Result<Node<'a>> {
        context.interrupt.check().map_err(|e| e.to_string())?;
        let token = self.take()?;
        let operation = operation(token).ok_or_else(|| format!("Graph node {token} has no frozen native conversion"))?;
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err("Native graph exceeds64 source nodes".into());
        }
        let mut arguments = Vec::new();
        while self.tokens.get(self.cursor + 1) == Some(&"=") {
            if matches!(self.peek(), Some("id" | "ref" | "aux"))
                || (operation == "gegl:piecewise-blend" && self.peek().and_then(super::graph_resources::level_pad).is_some())
            {
                break;
            }
            let name = self.take()?;
            self.expect("=")?;
            let value = self.take()?;
            self.assignments += 1;
            if self.assignments > MAX_ARGUMENTS || arguments.iter().any(|arg: &NodeArgument<'_>| arg.name == name) {
                return Err("Finite native graph has duplicate or excessive scalar assignments".into());
            }
            let kind = field(operation, name).ok_or_else(|| format!("Unmapped finite graph property {operation}:{name}"))?;
            let value = match kind {
                Kind::Number { min: low, max: high } => {
                    let number = value.parse::<f64>().map_err(|_| "Native graph scalar is not a decimal number")?;
                    if !number.is_finite() || !(low..=high).contains(&number) {
                        return Err("Native graph scalar exceeds its finite source/native parameter domain".into());
                    }
                    // Importer-only exact text literal, never a fabricated XCF Float tag.
                    // Individual native converters validate their actual control precision.
                    NodeValue::Number(number)
                }
                Kind::Integer { min: low, max: high } => {
                    let number = value.parse::<i64>().map_err(|_| "Native graph property requires an exact integer")?;
                    if !(low..=high).contains(&number) {
                        return Err("Native graph integer is outside its source domain".into());
                    }
                    NodeValue::Integer(number)
                }
                Kind::Boolean => NodeValue::Boolean(match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err("Native graph boolean must be true or false".into()),
                }),
                Kind::Choice(names) => {
                    let value = names
                        .iter()
                        .find(|(name, _)| *name == value)
                        .map(|(_, id)| *id)
                        .or_else(|| value.parse::<i32>().ok().filter(|id| names.iter().any(|(_, known)| known == id)))
                        .ok_or("Unknown native graph enum nickname/value")?;
                    NodeValue::Choice(value)
                }
                Kind::Text => NodeValue::Text(value),
                Kind::Color => {
                    let (rgba, linear) = color(value)?;
                    NodeValue::Color { rgba, linear }
                }
                Kind::Configuration => NodeValue::Configuration(value),
                Kind::Unavailable(reason) => {
                    return Err(format!("Graph property {operation}:{name} requires unavailable typed source payload: {reason}").into());
                }
            };
            arguments.push(NodeArgument { name, value });
        }
        Ok(Node { operation, arguments })
    }
    fn next_context<'c>(&self, context: &MappingContext<'c>) -> MappingContext<'c> {
        MappingContext {
            mode: context.mode,
            linear: context.linear,
            owner_bounds: context.owner_bounds,
            native_filter_bounds: context.native_filter_bounds,
            target_profile: context.target_profile.clone(),
            resolution_dpi: context.resolution_dpi,
            remaining_generation_bytes: self.remaining_generation,
            remaining_metadata_bytes: self.remaining_metadata,
            interrupt: context.interrupt,
        }
    }
    fn admit(&mut self, mapping: &NativeMapping, context: &MappingContext<'_>) -> Result<()> {
        let (resource, metadata) =
            crate::xcf::native_effects::retained_mapping_bytes(mapping, &context.interrupt).map_err(|e| MappingFailure::resource(e.to_string()))?;
        self.remaining_generation = self
            .remaining_generation
            .checked_sub(resource)
            .ok_or_else(|| MappingFailure::resource("Retained graph stages exceed the aggregate generation allowance"))?;
        self.remaining_metadata = self
            .remaining_metadata
            .checked_sub(metadata)
            .ok_or_else(|| MappingFailure::resource("Retained graph stages exceed the aggregate metadata allowance"))?;
        Ok(())
    }
    fn push_flat_mapping(&mut self, input: usize, mapping: NativeMapping, context: &MappingContext<'_>) -> Result<usize> {
        if matches!(&mapping.structure, Some(NativeStructure::Stages { .. })) {
            return Err("Nested native construction plans are not admitted".into());
        }
        self.filters = self.filters.checked_add(mapping.filters.len()).ok_or("Native graph filter count overflow")?;
        if self.filters > MAX_FILTERS {
            return Err("Native graph exceeds64 emitted top-level filters".into());
        }
        self.admit(&mapping, context)?;
        self.steps.push(NativeStage::Apply { input, mapping: Box::new(mapping) });
        if self.steps.len() > MAX_NODES {
            return Err("Native graph exceeds64 construction steps".into());
        }
        Ok(self.steps.len())
    }

    fn admit_deferred(&mut self, step: &NativeStage) -> Result<()> {
        if let NativeStage::Displace { metadata_limit, .. } = step {
            self.remaining_metadata = self
                .remaining_metadata
                .checked_sub(*metadata_limit)
                .ok_or_else(|| MappingFailure::resource("Native dynamic displacement exceeds the aggregate metadata allowance"))?;
        }
        Ok(())
    }

    fn push_mapping(&mut self, input: usize, mut mapping: NativeMapping, context: &MappingContext<'_>) -> Result<usize> {
        if !matches!(&mapping.structure, Some(NativeStructure::Stages { .. })) {
            return self.push_flat_mapping(input, mapping, context);
        }
        if !mapping.filters.is_empty()
            || !mapping.patterns.is_empty()
            || !mapping.layer_effects.is_empty()
            || mapping.working_rgb
            || mapping.quarter_turns != 0
            || mapping.rotation_degrees != 0.
        {
            return Err("Native child construction has outer data requiring an explicit wrapper; no filters, styles or orientation were discarded".into());
        }
        let Some(NativeStructure::Stages { steps, output }) = mapping.structure.take() else {
            return Err("Native child construction is absent".into());
        };
        super::validate_stages(&steps, output)?;
        if self.steps.len().checked_add(steps.len()).is_none_or(|v| v > MAX_NODES) {
            return Err("Flattened native construction exceeds64 global steps".into());
        }
        self.admit(&mapping, context)?;
        self.notes.extend(mapping.notes);
        let mut indexes = Vec::with_capacity(steps.len() + 1);
        indexes.push(input);
        for step in steps {
            context.interrupt.check().map_err(|e| e.to_string())?;
            let result = match step {
                NativeStage::Apply { input, mapping } => {
                    let actual = *indexes.get(input).ok_or("Native child input has no earlier result")?;
                    self.push_flat_mapping(actual, *mapping, context)?
                }
                NativeStage::Join { below, above, blend, opacity } => {
                    let below = *indexes.get(below).ok_or("Native child bottom input has no earlier result")?;
                    let above = *indexes.get(above).ok_or("Native child top input has no earlier result")?;
                    self.steps.push(NativeStage::Join { below, above, blend, opacity });
                    self.steps.len()
                }
                NativeStage::Knockout { below, above, opacity } => {
                    let below = *indexes.get(below).ok_or("Native child knockout bottom input has no earlier result")?;
                    let above = *indexes.get(above).ok_or("Native child knockout top input has no earlier result")?;
                    self.steps.push(NativeStage::Knockout { below, above, opacity });
                    self.steps.len()
                }
                NativeStage::Channel { input, component } => {
                    let input = *indexes.get(input).ok_or("Native child channel input has no earlier result")?;
                    self.steps.push(NativeStage::Channel { input, component });
                    self.steps.len()
                }
                NativeStage::Mask { input, mask, component, invert, scale } => {
                    let input = *indexes.get(input).ok_or("Native child masked input has no earlier result")?;
                    let mask = *indexes.get(mask).ok_or("Native child mask input has no earlier result")?;
                    self.steps.push(NativeStage::Mask { input, mask, component, invert, scale });
                    self.steps.len()
                }
                NativeStage::Displace { input, map, coordinates, scale, wrap, metadata_limit } => {
                    let input = *indexes.get(input).ok_or("Native child displaced input has no earlier result")?;
                    let map = *indexes.get(map).ok_or("Native child displacement map has no earlier result")?;
                    let stage = NativeStage::Displace { input, map, coordinates, scale, wrap, metadata_limit };
                    self.admit_deferred(&stage)?;
                    self.steps.push(stage);
                    self.steps.len()
                }
                NativeStage::Align { input, target, horizontal, vertical, margin_x, margin_y, snap } => {
                    let input = *indexes.get(input).ok_or("Native child alignment input has no earlier result")?;
                    let target = *indexes.get(target).ok_or("Native child alignment target has no earlier result")?;
                    self.steps.push(NativeStage::Align { input, target, horizontal, vertical, margin_x, margin_y, snap });
                    self.steps.len()
                }
            };
            indexes.push(result);
        }
        indexes.get(output).copied().ok_or_else(|| "Native child output is invalid".into())
    }

    /// Helpers emit already-native global references, never source node names.
    fn append_native(&mut self, steps: Vec<NativeStage>, output: usize, notes: Vec<String>, context: &MappingContext<'_>) -> Result<usize> {
        if self.steps.len().checked_add(steps.len()).is_none_or(|count| count > MAX_NODES) {
            return Err("Native auxiliary lowering exceeds64 global construction steps".into());
        }
        let notice = NativeMapping { notes, ..Default::default() };
        self.admit(&notice, context)?;
        self.notes.extend(notice.notes);
        for step in steps {
            context.interrupt.check().map_err(|error| error.to_string())?;
            match step {
                NativeStage::Apply { input, mapping } => {
                    self.push_flat_mapping(input, *mapping, context)?;
                }
                other => {
                    self.admit_deferred(&other)?;
                    self.steps.push(other);
                }
            }
        }
        super::validate_stages(&self.steps, output)?;
        Ok(output)
    }

    fn auxiliary(&mut self, pad: &str, depth: usize, context: &MappingContext<'_>) -> Result<usize> {
        self.expect(pad)?;
        self.expect("=")?;
        self.expect("[")?;
        let output = self.block(None, depth + 1, context)?;
        self.expect("]")?;
        Ok(output)
    }

    fn block(&mut self, mut current: Option<usize>, depth: usize, context: &MappingContext<'_>) -> Result<usize> {
        if depth > MAX_DEPTH {
            return Err("Finite native graph exceeds8 auxiliary nesting levels".into());
        }
        while let Some(token) = self.peek() {
            context.interrupt.check().map_err(|e| e.to_string())?;
            if token == "]" {
                if depth == 0 {
                    return Err("Unmatched native graph closing bracket".into());
                }
                break;
            }
            match token {
                "id" => {
                    self.take()?;
                    self.expect("=")?;
                    let label = self.label()?;
                    let input = current.ok_or("Graph label has no actual native input")?;
                    if self.labels.len() >= MAX_NODES || self.labels.iter().any(|(name, _)| *name == label) {
                        return Err("Native graph has duplicate or excessive labels".into());
                    }
                    self.labels.push((label, input));
                }
                "ref" => {
                    self.take()?;
                    self.expect("=")?;
                    let label = self.label()?;
                    current = Some(
                        self.labels
                            .iter()
                            .find(|(name, _)| *name == label)
                            .map(|(_, index)| *index)
                            .ok_or("Native graph reference is missing, forward or cyclic")?,
                    );
                }
                value if super::graph_porter::operation(value).is_some() => {
                    let source = self.read_node(context)?;
                    let main = current.ok_or("Native mask composer lacks its actual main input")?;
                    if self.peek() != Some("aux") {
                        return Err(MappingFailure::resource("Native mask composer requires its actual auxiliary input"));
                    }
                    let aux = self.auxiliary("aux", depth, context)?;
                    let native_context = self.next_context(context);
                    let next = self.steps.len() + 1;
                    let lowered = with_effect(source, &native_context, |effect| super::graph_porter::lower(effect, &native_context, main, aux, next))?;
                    current = Some(self.append_native(lowered.steps, lowered.output, lowered.notes, context)?);
                }
                value if super::graph_composites::operation(value).is_some() => {
                    let source = self.read_node(context)?;
                    let below = current.ok_or("Native binary graph node lacks its actual main input")?;
                    if self.peek() != Some("aux") {
                        if super::graph_math::operation(source.operation).is_some() {
                            let mapping = node(source, &self.next_context(context))?;
                            current = Some(self.push_mapping(below, mapping, context)?);
                            continue;
                        }
                        return Err(MappingFailure::resource("Native binary graph node requires its actual explicit auxiliary input"));
                    }
                    let mut above = self.auxiliary("aux", depth, context)?;
                    let binary_context = self.next_context(context);
                    let binary = with_effect(source, &binary_context, |effect| super::graph_composites::map(effect, &binary_context))?;
                    let notice = NativeMapping { notes: binary.notes, ..Default::default() };
                    self.admit(&notice, context)?;
                    self.notes.extend(notice.notes);
                    if let Some(auxiliary) = binary.auxiliary {
                        above = self.push_mapping(above, auxiliary, context)?;
                    }
                    if let Some(auxiliary) = binary.select {
                        current = Some(if auxiliary { above } else { below });
                    } else {
                        let (below, above) = if binary.swap { (above, below) } else { (below, above) };
                        self.steps.push(if binary.knockout {
                            NativeStage::Knockout { below, above, opacity: binary.opacity }
                        } else {
                            NativeStage::Join { below, above, blend: binary.blend, opacity: binary.opacity }
                        });
                        current = Some(self.steps.len());
                    }
                }
                value if super::graph_resources::operation(value) == Some("gegl:piecewise-blend") => {
                    let source = self.read_node(context)?;
                    let input = current.ok_or("Piecewise graph node lacks its actual main input")?;
                    let mut pads = Vec::new();
                    while let Some(pad) = self.peek().filter(|name| super::graph_resources::level_pad(name).is_some()) {
                        let level = super::graph_resources::level_pad(pad).ok_or("Unknown declared level pad")?;
                        if pads.iter().any(|(known, _)| *known == level) {
                            return Err("Piecewise graph repeats an actual level pad".into());
                        }
                        let value = self.auxiliary(pad, depth, context)?;
                        pads.push((level, value));
                    }
                    let native_context = self.next_context(context);
                    let next = self.steps.len() + 1;
                    let (steps, output, notes) =
                        with_effect(source, &native_context, |effect| super::graph_resources::piecewise(effect, &native_context, input, &pads, next))?;
                    current = Some(self.append_native(steps, output, notes, context)?);
                }
                value if super::graph_displacement::operation(value).is_some() => {
                    let source = self.read_node(context)?;
                    let input = current.ok_or("Displacement node lacks its actual main input")?;
                    if self.peek() != Some("aux") {
                        return Err(MappingFailure::resource("Displacement node requires its actual native map input"));
                    }
                    let aux = self.auxiliary("aux", depth, context)?;
                    let native_context = self.next_context(context);
                    let next = self.steps.len() + 1;
                    let (steps, output, notes) =
                        with_effect(source, &native_context, |effect| super::graph_displacement::lower(effect, &native_context, input, aux, next))?;
                    current = Some(self.append_native(steps, output, notes, context)?);
                }
                _ => {
                    let source = self.read_node(context)?;
                    if self.peek() == Some("aux") && matches!(source.operation, "gegl:opacity" | "gegl:threshold" | "gegl:border-align" | "gegl:weighted-blend")
                    {
                        let input = current.ok_or("Auxiliary native node lacks its actual main input")?;
                        let aux = self.auxiliary("aux", depth, context)?;
                        let native_context = self.next_context(context);
                        let next = self.steps.len() + 1;
                        let (steps, output, notes) =
                            with_effect(source, &native_context, |effect| auxiliary_native(effect, &native_context, input, aux, next))?;
                        current = Some(self.append_native(steps, output, notes, context)?);
                        continue;
                    }
                    if source.operation == "gegl:weighted-blend" {
                        return Err(MappingFailure::resource("Weighted blend requires its actual auxiliary input"));
                    }
                    if source.operation == "gegl:clone" && self.peek() == Some("ref") {
                        self.take()?;
                        self.expect("=")?;
                        let label = self.label()?;
                        current = Some(
                            self.labels
                                .iter()
                                .find(|(name, _)| *name == label)
                                .map(|(_, index)| *index)
                                .ok_or("Graph clone reference has no actual earlier native input")?,
                        );
                        continue;
                    }
                    let input = if super::graph_stock::generator(source.operation) || super::graph_objects::generator(source.operation) {
                        current.unwrap_or(0)
                    } else {
                        current.ok_or("Auxiliary graph node requires an explicit actual input reference")?
                    };
                    let mapping = node(source, &self.next_context(context))?;
                    current = Some(self.push_mapping(input, mapping, context)?);
                }
            }
            if self.steps.len() > MAX_NODES {
                return Err("Native graph exceeds64 native construction steps".into());
            }
        }
        current.ok_or_else(|| "Native auxiliary graph has no resolved output".into())
    }
}

fn native_push(steps: &mut Vec<NativeStage>, next: usize, stage: NativeStage) -> Result<usize> {
    let output = next.checked_add(steps.len()).filter(|value| *value <= MAX_NODES).ok_or("Auxiliary native construction exceeds64 stages")?;
    steps.push(stage);
    Ok(output)
}
fn native_apply(steps: &mut Vec<NativeStage>, next: usize, input: usize, mapping: NativeMapping) -> Result<usize> {
    native_push(steps, next, NativeStage::Apply { input, mapping: Box::new(mapping) })
}
fn native_gain(gain: f64, offset: f64, context: &MappingContext<'_>) -> NativeMapping {
    NativeMapping {
        working_rgb: context.mode == photocraft_color::ColorMode::Grayscale,
        filters: vec![super::native_filter("image.adjustments.channelMixer", serde_json::json!({"monochrome":true,"gray":[100.*gain,0.,0.,100.*offset]}))],
        ..Default::default()
    }
}
fn native_interval(low: f64, high: f64) -> NativeMapping {
    let points = (0..256).map(|i| if (low..=high).contains(&(f64::from(i) / 255.)) { 255 } else { 0 }).collect::<Vec<_>>();
    NativeMapping { filters: vec![super::native_filter("image.adjustments.curves", serde_json::json!({"pencil":true,"points":points}))], ..Default::default() }
}
fn auxiliary_native(e: &StoredEffect<'_>, c: &MappingContext<'_>, main: usize, aux: usize, next: usize) -> Result<(Vec<NativeStage>, usize, Vec<String>)> {
    use super::NativeMaskComponent::{Alpha, Composite};
    use photocraft_color::BlendMode;
    if c.remaining_metadata_bytes < 32768 || c.remaining_generation_bytes < 65536 {
        return Err(MappingFailure::resource("Auxiliary native construction exceeds remaining preparation allowance"));
    }
    c.interrupt.check().map_err(|error| error.to_string())?;
    let mut steps = Vec::new();
    let output = match e.operation {
        "gegl:opacity" => {
            let value = e.number("value")?.unwrap_or(1.);
            let masked = native_push(&mut steps, next, NativeStage::Mask { input: main, mask: aux, component: Composite, invert: false, scale: 1. })?;
            let values = (0..256).map(|i| (f64::from(i) / 255. * value).clamp(0., 1.)).collect::<Vec<_>>();
            native_apply(
                &mut steps,
                next,
                masked,
                NativeMapping { filters: vec![super::native_filter("filter.other.alphaCurve", serde_json::json!({"values":values}))], ..Default::default() },
            )?
        }
        "gegl:border-align" => native_push(
            &mut steps,
            next,
            NativeStage::Align {
                input: main,
                target: aux,
                horizontal: e.number("x")?.unwrap_or(0.5),
                vertical: e.number("y")?.unwrap_or(0.5),
                margin_x: e.number("horizontal-margin")?.unwrap_or(0.),
                margin_y: e.number("vertical-margin")?.unwrap_or(0.),
                snap: e.boolean("snap-integer")?.unwrap_or(true),
            },
        )?,
        "gegl:threshold" => {
            let high = e.number("high")?.unwrap_or(1.);
            let main_field = native_push(&mut steps, next, NativeStage::Channel { input: main, component: Composite })?;
            let aux_field = native_push(&mut steps, next, NativeStage::Channel { input: aux, component: Composite })?;
            let inverted = native_apply(&mut steps, next, aux_field, native_gain(-0.5, 0.75, c))?;
            let difference =
                native_push(&mut steps, next, NativeStage::Join { below: main_field, above: inverted, blend: BlendMode::LinearLight, opacity: 1. })?;
            let compared = native_apply(&mut steps, next, difference, native_interval(0.5, 1.))?;
            let high_mask = native_apply(&mut steps, next, main_field, native_interval(0., high))?;
            let selected = native_push(&mut steps, next, NativeStage::Join { below: compared, above: high_mask, blend: BlendMode::Multiply, opacity: 1. })?;
            native_push(&mut steps, next, NativeStage::Mask { input: selected, mask: main, component: Alpha, invert: false, scale: 1. })?
        }
        "gegl:weighted-blend" => {
            let main_alpha = native_push(&mut steps, next, NativeStage::Channel { input: main, component: Alpha })?;
            let aux_alpha = native_push(&mut steps, next, NativeStage::Channel { input: aux, component: Alpha })?;
            let main_half = native_apply(&mut steps, next, main_alpha, native_gain(0.5, 0., c))?;
            let aux_half = native_apply(&mut steps, next, aux_alpha, native_gain(0.5, 0., c))?;
            let sum_half = native_push(&mut steps, next, NativeStage::Join { below: main_half, above: aux_half, blend: BlendMode::LinearDodge, opacity: 1. })?;
            let ratio = native_push(&mut steps, next, NativeStage::Join { below: aux_half, above: sum_half, blend: BlendMode::Divide, opacity: 1. })?;
            let opaque =
                || NativeMapping { filters: vec![super::native_filter("filter.other.setAlpha", serde_json::json!({"value":1.}))], ..Default::default() };
            let main_color = native_apply(&mut steps, next, main, opaque())?;
            let aux_color = native_apply(&mut steps, next, aux, opaque())?;
            let weighted = native_push(&mut steps, next, NativeStage::Mask { input: aux_color, mask: ratio, component: Composite, invert: false, scale: 1. })?;
            let result = native_push(&mut steps, next, NativeStage::Join { below: main_color, above: weighted, blend: BlendMode::Normal, opacity: 1. })?;
            let coverage = native_apply(&mut steps, next, sum_half, native_gain(2., 0., c))?;
            native_push(&mut steps, next, NativeStage::Mask { input: result, mask: coverage, component: Composite, invert: false, scale: 1. })?
        }
        _ => return Err("Unknown exact auxiliary native adapter".into()),
    };
    let policy = match e.operation {
        "gegl:opacity" => {
            "Actual auxiliary Composite channel becomes a static native pixel mask, followed by ordinary Alpha Curve for the source global opacity. Native composite mean, clamped coverage, 256-sample interpolation and original color/alpha rules apply."
        }
        "gegl:threshold" => {
            "Actual native Composite fields are compared with editable native Channel Mixer, Linear Light and pencil Curves, intersected with the high bound, then masked by original main alpha once. The scalar low bound is inactive with actual auxiliary threshold. Native arithmetic-mean channels, clipping, profile/depth and 256-sample interpolation supersede source luminance/threshold kernels."
        }
        "gegl:border-align" => {
            "Actual prepared native raster content extents determine one ordinary affine placement. Native extents, margins and integer rounding apply; no dynamic source bounding-box evaluator is retained."
        }
        _ => {
            "Actual alpha channels become native half-scale fields; Add/Divide constructs auxiliary normalized weights, opaque native colors mix through a static mask, and clamped summed-alpha coverage is restored. Native profile/depth/zero-division/interpolation rules and summed coverage are an explicit approximation of source alpha-weighted blending, not its processing kernel."
        }
    };
    Ok((steps, output, vec![policy.into()]))
}

fn with_effect<T>(node: Node<'_>, context: &MappingContext<'_>, convert: impl FnOnce(&StoredEffect<'_>) -> Result<T>) -> Result<T> {
    let mut budget = context.remaining_metadata_bytes;
    let arguments = node
        .arguments
        .iter()
        .map(|arg| {
            Ok(Argument {
                name: arg.name,
                value: match &arg.value {
                    NodeValue::Number(value) => ArgumentValue::Literal(*value),
                    NodeValue::Integer(value) => {
                        if let Ok(value) = i32::try_from(*value) {
                            ArgumentValue::Int(value)
                        } else if let Ok(value) = u32::try_from(*value) {
                            ArgumentValue::UInt(value)
                        } else {
                            return Err("Exact graph integer exceeds existing native importer scalar representation".into());
                        }
                    }
                    NodeValue::Boolean(value) => ArgumentValue::Bool(*value),
                    NodeValue::Choice(value) => ArgumentValue::Enum(*value),
                    NodeValue::Text(value) => ArgumentValue::String(value),
                    NodeValue::Color { rgba, linear } => ArgumentValue::LiteralColor { rgba: *rgba, linear: *linear },
                    NodeValue::Configuration(raw) => ArgumentValue::Config(super::parse_config(raw, &mut budget)?),
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let effect = StoredEffect {
        name: "",
        icon: "",
        operation: node.operation,
        version: None,
        arguments,
        visible: true,
        opacity: 1.,
        mode: 0,
        blend_space: 0,
        composite_space: 0,
        composite_mode: 0,
        clip: false,
        region: 0,
        mask_offset: 0,
        record_offset: 0,
        raw: &[],
        unknown_properties: Vec::new(),
        errors: Vec::new(),
    };
    convert(&effect)
}
fn node(node: Node<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    with_effect(node, context, |effect| {
        super::graph_tone::map(effect, context)
            .or_else(|| super::graph_objects::map(effect, context))
            .or_else(|| super::graph_core::map(effect, context))
            .or_else(|| super::graph_math::map(effect, context))
            .or_else(|| super::graph_resources::map(effect, context))
            .or_else(|| super::graph_stock::map(effect, context))
            .or_else(|| super::tone::map(effect, context))
            .or_else(|| super::curves_and_quantization::map(effect, context))
            .or_else(|| super::color::map(effect, context))
            .or_else(|| super::resources::map(effect, context))
            .or_else(|| super::spatial::map(effect, context))
            .or_else(|| super::residual_tone::map(effect, context))
            .or_else(|| super::residual_geometry::map(effect, context))
            .or_else(|| super::residual_spatial::map(effect, context))
            .or_else(|| super::core_composites::map(effect, context))
            .or_else(|| super::residual_transform::map(effect, context))
            .or_else(|| super::residual_coverage::map(effect, context))
            .or_else(|| super::residual_render::map(effect, context))
            .ok_or_else(|| MappingFailure::from(format!("Frozen graph node {} has an unfinished native conversion route", effect.operation)))?
    })
}

fn decoding_scratch(input: &str) -> usize {
    MAX_TOKENS * (std::mem::size_of::<String>() + std::mem::size_of::<&str>())
        + MAX_ARGUMENTS * 2 * std::mem::size_of::<NodeArgument<'_>>()
        + MAX_NODES * 2 * (std::mem::size_of::<Node<'_>>() + std::mem::size_of::<NativeStage>())
        // Inert unquoting never grows source bytes; this also covers bounded
        // String capacity rounding and temporary property/config preparation.
        + input.len().saturating_mul(32)
        + GRAPH_NOTICE.len()
}
fn lower(input: &str, context: &MappingContext<'_>) -> Result<NativeMapping> {
    context.interrupt.check().map_err(|e| e.to_string())?;
    let scratch = decoding_scratch(input);
    if scratch > context.remaining_generation_bytes || input.len() > context.remaining_metadata_bytes {
        return Err(MappingFailure::resource("Native graph decoding exceeds the remaining shared allowance"));
    }
    let token_storage = tokens(input)?;
    let mut decoder = Decoder {
        tokens: token_storage.iter().map(String::as_str).collect(),
        cursor: 0,
        nodes: 0,
        assignments: 0,
        labels: Vec::new(),
        steps: Vec::new(),
        filters: 0,
        remaining_generation: context.remaining_generation_bytes - scratch,
        notes: Vec::new(),
        remaining_metadata: context
            .remaining_metadata_bytes
            .checked_sub(GRAPH_NOTICE.len() + 4)
            .ok_or_else(|| MappingFailure::resource("Native graph notice exceeds remaining metadata allowance"))?,
    };
    if decoder.tokens.is_empty() {
        return Err("Native graph has no mapped input or operation".into());
    }
    let output = decoder.block(Some(0), 0, context)?;
    if decoder.peek().is_some() {
        return Err("Native graph has trailing unsupported syntax".into());
    }
    let linear = output == decoder.steps.len()
        && decoder.steps.iter().enumerate().all(|(index, step)| match step {
            NativeStage::Apply { input, mapping } => {
                *input == index
                    && mapping.structure.is_none()
                    && mapping.patterns.is_empty()
                    && mapping.layer_effects.is_empty()
                    && !mapping.working_rgb
                    && mapping.quarter_turns == 0
                    && mapping.rotation_degrees == 0.
            }
            NativeStage::Join { .. }
            | NativeStage::Knockout { .. }
            | NativeStage::Channel { .. }
            | NativeStage::Mask { .. }
            | NativeStage::Displace { .. }
            | NativeStage::Align { .. } => false,
        });
    let mut mapping = if decoder.steps.len() == 1 && output == 1 && matches!(decoder.steps.first(), Some(NativeStage::Apply { input: 0, .. })) {
        let Some(NativeStage::Apply { mapping, .. }) = decoder.steps.pop() else { return Err("Native graph stage disappeared".into()) };
        *mapping
    } else if linear {
        let mut result = NativeMapping::default();
        for step in decoder.steps {
            let NativeStage::Apply { mapping, .. } = step else { return Err("Native graph linear stage is invalid".into()) };
            result.filters.extend(mapping.filters);
            result.notes.extend(mapping.notes);
        }
        result
    } else {
        let notes = decoder
            .steps
            .iter_mut()
            .flat_map(|step| match step {
                NativeStage::Apply { mapping, .. } => std::mem::take(&mut mapping.notes),
                NativeStage::Join { .. }
                | NativeStage::Knockout { .. }
                | NativeStage::Channel { .. }
                | NativeStage::Mask { .. }
                | NativeStage::Displace { .. }
                | NativeStage::Align { .. } => Vec::new(),
            })
            .collect();
        NativeMapping { structure: Some(NativeStructure::Stages { steps: decoder.steps, output }), notes, ..Default::default() }
    };
    mapping.notes.extend(decoder.notes);
    mapping.notes.push(GRAPH_NOTICE.into());
    Ok(mapping)
}

pub(super) fn map(effect: &StoredEffect<'_>, context: &MappingContext<'_>) -> MappingResult {
    if effect.operation != "gegl:gegl" {
        return None;
    }
    Some((|| {
        if effect.version.is_some_and(|v| v != "0:0") || !effect.errors.is_empty() {
            return Err("Finite native graph version or stored record errors are not mapped".into());
        }
        for arg in &effect.arguments {
            if !matches!(arg.name, "string" | "error") {
                return Err(format!("Unmapped graph argument {}", arg.name).into());
            }
            effect.argument(arg.name)?;
        }
        match effect.argument("error")? {
            None | Some(ArgumentValue::String("")) => {}
            _ => return Err("Source graph reports an error or has invalid error metadata".into()),
        }
        let Some(ArgumentValue::String(input)) = effect.argument("string")? else {
            return Err("Finite native graph needs one typed string".into());
        };
        lower(input, context)
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::{Interrupt, Surface};
    use serde_json::json;

    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::new(0, 0, 4, 2),
            native_filter_bounds: Rect::new(0, 0, 4, 2),
            target_profile: None,
            resolution_dpi: 72.,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: Interrupt::NONE,
        }
    }
    fn run(input: &str, c: &MappingContext<'_>) -> Result<NativeMapping> {
        map(&StoredEffect::test("gegl:gegl", vec![Argument { name: "string", value: ArgumentValue::String(input) }]), c).unwrap()
    }
    #[test]
    fn isolated_and_noncommuting_scalar_chains_emit_independently_specified_native_filters() {
        let c = context();
        let single = run("\n# original source syntax\ngegl:invert-linear", &c).unwrap();
        assert_eq!(single.filters, vec![super::super::native_filter("image.adjustments.invert", json!({}))]);
        let chain = run("invert-gamma exposure exposure=1 black-level=0", &c).unwrap();
        assert_eq!(
            chain.filters,
            vec![
                super::super::native_filter("image.adjustments.invert", json!({})),
                super::super::native_filter("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.}))
            ]
        );
        let rectangle = c.owner_bounds;
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            source.write_region(rectangle, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]);
            let mut actual = Document::new("Mapped native chain", Size::new(4, 2), ColorMode::Grayscale, sample);
            actual.layers.push(Layer::new("Original", LayerContent::Raster(source.clone())));
            for filter in &chain.filters {
                let adjustment = crate::xcf::coverage::adjustment(&filter.command, &filter.params).unwrap();
                actual.layers.push(Layer::new("Native adjustment", LayerContent::Adjustment(adjustment)));
            }
            let mut expected = Document::new("Independent native chain", Size::new(4, 2), ColorMode::Grayscale, sample);
            expected.layers.push(Layer::new("Original", LayerContent::Raster(source)));
            expected.layers.push(Layer::new("Invert", LayerContent::Adjustment(Adjustment::Invert)));
            expected.layers.push(Layer::new("Exposure", LayerContent::Adjustment(Adjustment::Exposure { exposure: 1., offset: 0., gamma: 1. })));
            assert_eq!(photocraft_compose::flatten(&actual).px, photocraft_compose::flatten(&expected).px, "{sample:?}");
            let mut reversed = expected.clone();
            reversed.layers.swap(1, 2);
            assert_ne!(photocraft_compose::flatten(&actual).px, photocraft_compose::flatten(&reversed).px, "ordering oracle:{sample:?}");
        }
    }
    #[test]
    fn one_explicit_snapshot_fork_preserves_native_branch_and_postfilter_order() {
        for label in ["in", "source_27"] {
            let input = format!("id={label} exposure exposure=-1 over aux=[ref={label} invert-linear] exposure exposure=1");
            let mapping = run(&input, &context()).unwrap();
            let Some(NativeStructure::Stages { steps, output }) = mapping.structure else { panic!("native stages") };
            assert_eq!(output, 4);
            let NativeStage::Apply { input: 0, mapping: bottom } = &steps[0] else { panic!("bottom") };
            let NativeStage::Apply { input: 0, mapping: top } = &steps[1] else { panic!("top") };
            assert!(matches!(steps[2], NativeStage::Join { below: 1, above: 2, blend: photocraft_color::BlendMode::Normal, opacity: 1. }));
            let NativeStage::Apply { input: 3, mapping: post } = &steps[3] else { panic!("post") };
            assert_eq!(bottom.filters, vec![super::super::native_filter("image.adjustments.exposure", json!({"exposure":-1.,"offset":0.,"gamma":1.}))]);
            assert_eq!(top.filters, vec![super::super::native_filter("image.adjustments.invert", json!({}))]);
            assert_eq!(post.filters, vec![super::super::native_filter("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.}))]);
        }
    }
    #[test]
    fn twelve_frozen_nodes_have_typed_scalar_routes_and_structures_are_never_discarded() {
        for input in [
            "invert-gamma",
            "invert-linear",
            "brightness-contrast brightness=.2 contrast=1.2",
            "exposure exposure=.5",
            "levels in-low=.1 in-high=.9",
            "box-blur radius=2",
            "gaussian-blur std-dev-x=2 std-dev-y=3 filter=fir abyss-policy=clamp clip-extent=true",
            "median-blur radius=-2 neighborhood=diamond percentile=20 alpha-percentile=70 high-precision=false",
            "saturation scale=1.2 colorspace=Native",
            "unsharp-mask std-dev=2 scale=.5 threshold=0",
            "value-invert",
            "rgb-clip clip-low=true low-limit=.2",
        ] {
            run(input, &context()).unwrap_or_else(|e| panic!("{input}:{}", e.message));
        }
        let Some(NativeStructure::Stages { steps, output: 2 }) = run("rgb-clip invert-linear", &context()).unwrap().structure else {
            panic!("complete structural composition")
        };
        assert!(matches!(&steps[0],NativeStage::Apply{input:0,mapping} if mapping.structure.is_some()));
        assert!(matches!(&steps[1],NativeStage::Apply{input:1,mapping} if mapping.filters[0].command=="image.adjustments.invert"));
        assert!(run("rgb-clip", &context()).unwrap().structure.is_some());
    }
    #[test]
    fn hostile_unknown_incomplete_and_resource_syntax_is_rejected_as_a_whole() {
        for input in [
            "",
            "invert-linear load path=secret",
            "invert-linear arbitrary=1",
            "exposure exposure=NaN",
            "exposure exposure=1e309",
            "exposure exposure=1 exposure=2",
            "box-blur radius=1.5",
            "gaussian-blur filter=unknown",
            "rgb-clip clip-low=1",
            "id=in over aux=[ref=other invert-linear]",
            "over aux=[invert-linear]",
            "id=in over aux=[ref=in invert-linear",
            "id=in id=in invert-linear",
            "ref=future invert-linear id=future",
            "invert-linear ]",
            "invert-linear; exposure",
            "exposure exposure=1rel",
            "exposure exposure=$(1)",
            "gegl:gegl string=invert-linear",
            "id=in over aux=[]",
        ] {
            assert!(run(input, &context()).is_err(), "{input}");
        }
        assert!(run(&"invert-linear ".repeat(65), &context()).is_err());
        assert!(run(&format!("id=a {}invert-linear{}", "over aux=[ref=a ".repeat(9), "]".repeat(9)), &context()).is_err());
        assert!(run(&" ".repeat(MAX_BYTES + 1), &context()).is_err());
        let mut c = context();
        c.remaining_generation_bytes = 0;
        assert!(run("invert-linear", &c).is_err());
        let mut effect = StoredEffect::test(
            "gegl:gegl",
            vec![
                Argument { name: "string", value: ArgumentValue::String("invert-linear") },
                Argument { name: "error", value: ArgumentValue::String("source failure") },
            ],
        );
        assert!(map(&effect, &context()).unwrap().is_err());
        if let Some(argument) = effect.arguments.get_mut(1) {
            argument.value = ArgumentValue::String("");
        }
        assert!(map(&effect, &context()).unwrap().is_ok(), "independently saved graphs carry an empty typed error string");
        effect.arguments.pop();
        effect.version = Some("future");
        assert!(map(&effect, &context()).unwrap().is_err());
    }

    #[test]
    fn graph_f64_literals_reach_native_controls_without_fake_wire_rounding_and_cancel_is_observed() {
        let exact = 0.12345678901234567f64;
        let mapping = run("exposure exposure=0.12345678901234567", &context()).unwrap();
        assert_eq!(mapping.filters[0].params["exposure"].as_f64().unwrap().to_bits(), exact.to_bits());
        assert_ne!(exact.to_bits(), f64::from(exact as f32).to_bits());
        let cancel = || true;
        let progress = |_| {};
        let c = MappingContext { interrupt: Interrupt::new(&cancel, &progress), ..context() };
        assert!(run("invert-linear", &c).is_err());
        let c = MappingContext { remaining_metadata_bytes: 1, ..context() };
        assert!(run("invert-linear", &c).is_err());
    }

    #[test]
    fn intermediate_snapshot_references_nested_forks_and_full_structural_nodes_lower_to_native_only_indexes() {
        let mapping=run("id=original exposure exposure=-1 id=dark over aux=[ref=original invert-linear id=negative over aux=[ref=dark gaussian-blur std-dev-x=0 std-dev-y=0]] ref=negative rgb-clip",&context()).unwrap();
        let Some(NativeStructure::Stages { steps, output }) = mapping.structure else { panic!("native plan") };
        assert_eq!(output, 6);
        assert!(matches!(&steps[0], NativeStage::Apply { input: 0, .. }));
        assert!(matches!(&steps[1], NativeStage::Apply { input: 0, .. }));
        assert!(matches!(&steps[2], NativeStage::Apply { input: 1, .. }));
        assert!(matches!(&steps[3], NativeStage::Join { below: 2, above: 3, .. }));
        assert!(matches!(&steps[4], NativeStage::Join { below: 1, above: 4, .. }));
        assert!(matches!(&steps[5],NativeStage::Apply{input:2,mapping} if mapping.structure.is_some()));
        for (index, step) in steps.iter().enumerate() {
            match step {
                NativeStage::Apply { input, mapping } => {
                    assert!(*input <= index);
                    assert!(!matches!(&mapping.structure, Some(NativeStructure::Stages { .. })));
                }
                NativeStage::Join { below, above, .. } | NativeStage::Knockout { below, above, .. } => {
                    assert!(*below <= index);
                    assert!(*above <= index);
                }
                NativeStage::Channel { input, .. } => assert!(*input <= index),
                NativeStage::Mask { input, mask, .. } => {
                    assert!(*input <= index);
                    assert!(*mask <= index);
                }
                NativeStage::Displace { input, map, .. } => {
                    assert!(*input <= index);
                    assert!(*map <= index);
                }
                NativeStage::Align { input, target, .. } => {
                    assert!(*input <= index);
                    assert!(*target <= index);
                }
            }
        }
        assert!(run("id=in over aux=[ref=in invert-linear] over aux=[ref=in invert-linear]", &context()).is_ok());
    }

    #[test]
    fn frozen_schema_fields_preserve_real_enum_ids_config_structure_and_no_proxy_flattening() {
        let config = run("gimp:brightness-contrast config='(brightness 0.25) (contrast -0.5)'", &context()).unwrap();
        assert_eq!(config.filters[0].command, "image.adjustments.brightnessContrast");
        assert!(run("gimp:brightness-contrast brightness=0.25", &context()).is_err());
        assert!(run("gaussian-blur filter=fir abyss-policy=white", &context()).is_ok());
        assert!(run("gaussian-blur filter=99", &context()).is_err());
        let stages = run("absolute invert-linear", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, .. }) = stages.structure else { panic!("native style stage") };
        assert!(matches!(&steps[0],NativeStage::Apply{mapping,..} if !mapping.layer_effects.is_empty()));
        let missing = run("gimp:normal", &context()).unwrap_err();
        assert!(missing.message.contains("auxiliary"));
        assert!(run("id=in gimp:normal aux=[ref=in invert-linear]", &context()).is_ok());
    }

    #[test]
    fn documented_color_literals_and_utf8_text_are_typed_data_with_explicit_transfer_and_precision() {
        assert_eq!(color("#3af8").unwrap(), ([51. / 255., 170. / 255., 1., 136. / 255.], false));
        assert_eq!(color("#12345678").unwrap(), ([18. / 255., 52. / 255., 86. / 255., 120. / 255.], false));
        assert_eq!(color("Lime").unwrap(), ([0., 1., 0., 1.], false));
        assert_eq!(color("rgba(0.25, -0.5, 2.0, 0.75)").unwrap(), ([0.25, -0.5, 2., 0.75], true));
        assert!(color("rgb(1e100,0,0)").is_err());
        assert!(color("rgba(0,0,0,2)").is_err());
        assert_eq!(tokens("label='héλlo 漢字'").unwrap(), vec!["label", "=", "héλlo 漢字"]);
        let literal = run("color-overlay unreviewed-paint='rgba(0.25, 0.5, 2.0, 0.75)'", &context());
        // The exact declared property name matters; never accept a random paint field.
        assert!(literal.is_err());
        let overlay = run("color-overlay value='rgba(0.25, 0.5, 2.0, 0.75)'", &context()).unwrap();
        let photocraft_doc::effects::Effect::ColorOverlay { color, common } = &overlay.layer_effects[0] else { panic!("native Color Overlay") };
        let mut expected = [0.; 3];
        photocraft_cms::Transform::new(
            photocraft_cms::Builtin::LinearSrgb.profile(),
            photocraft_cms::Builtin::Srgb.profile(),
            photocraft_cms::Intent::RelativeColorimetric,
            false,
        )
        .unwrap()
        .convert_f32(&[0.25, 0.5, 2.], 3, &mut expected, 3, false);
        assert_eq!(&color.c[..3], &expected);
        assert_eq!(common.opacity, 0.75);
        let checker = run("checkerboard x=2 y=3 color1='#3af8' color2='rgb(0.25, 0.5, 2.0)'", &context()).unwrap();
        assert!(checker.structure.is_some());
    }

    #[test]
    fn generation_is_reduced_before_the_second_pattern_converter_and_failures_are_transactional() {
        let c = context();
        let child = node(
            Node {
                operation: "gegl:checkerboard",
                arguments: vec![NodeArgument { name: "x", value: NodeValue::Integer(64) }, NodeArgument { name: "y", value: NodeValue::Integer(64) }],
            },
            &c,
        )
        .unwrap();
        let (resource, _) = crate::xcf::native_effects::retained_mapping_bytes(&child, &Interrupt::NONE).unwrap();
        let single = "checkerboard x=64 y=64";
        let pair = "checkerboard x=64 y=64 checkerboard x=64 y=64";
        let limited = MappingContext { remaining_generation_bytes: decoding_scratch(pair) + resource + resource / 2, ..context() };
        let before = (limited.remaining_generation_bytes, limited.remaining_metadata_bytes);
        assert!(run(single, &limited).is_ok());
        let error = run(pair, &limited).unwrap_err();
        // This error originates in the second generator's preallocation check;
        // post-generation aggregate admission has a different explicit message.
        assert!(error.message.contains("remaining generation allowance"), "{}", error.message);
        assert_eq!((limited.remaining_generation_bytes, limited.remaining_metadata_bytes), before);
        let child = node(Node { operation: "gegl:invert-linear", arguments: vec![] }, &context()).unwrap();
        let (_, metadata) = crate::xcf::native_effects::retained_mapping_bytes(&child, &Interrupt::NONE).unwrap();
        let limited = MappingContext { remaining_metadata_bytes: GRAPH_NOTICE.len() + 4 + metadata + metadata / 2, ..context() };
        assert!(run("invert-linear", &limited).is_ok());
        assert!(run("invert-linear invert-linear", &limited).unwrap_err().message.contains("aggregate metadata allowance"));
    }

    #[test]
    fn actual_aux_native_opacity_swap_and_source_destination_selection_are_not_missing_resources() {
        let result = run("id=in gimp:normal opacity=0.4 aux=[ref=in invert-linear]", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, output: 2 }) = result.structure else { panic!("normal native join") };
        assert!(matches!(&steps[1],NativeStage::Join{below:0,above:1,blend:photocraft_color::BlendMode::Normal,opacity} if (*opacity-0.4).abs()<1e-7));
        let result = run("id=in gimp:behind opacity=0.4 aux=[ref=in invert-linear]", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, output: 3 }) = result.structure else { panic!("behind native join") };
        let NativeStage::Apply { input: 1, mapping } = &steps[1] else { panic!("actual aux opacity") };
        assert_eq!(mapping.filters[0].command, "filter.other.alphaCurve");
        assert!(matches!(&steps[2], NativeStage::Join { below: 2, above: 0, opacity: 1., .. }));
        let result = run("id=in svg:src aux=[ref=in invert-linear]", &context()).unwrap();
        assert_eq!(result.filters, vec![super::super::native_filter("image.adjustments.invert", json!({}))]);
        let result = run("id=in svg:dst aux=[ref=in invert-linear]", &context()).unwrap();
        assert!(matches!(result.structure, Some(NativeStructure::Stages { output: 0, .. })));
        let result = run("id=in gimp:erase opacity=0.4 aux=[ref=in invert-linear]", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, output: 2 }) = result.structure else { panic!("actual erase native knockout") };
        assert!(matches!(&steps[0],NativeStage::Apply{input:0,mapping} if mapping.filters[0].command == "image.adjustments.invert"));
        assert!(matches!(&steps[1],NativeStage::Knockout{below:0,above:1,opacity} if (*opacity-0.4).abs()<1e-7));
    }

    #[test]
    fn actual_native_auxiliary_roles_preserve_bindings_and_share_deferred_map_allowance() {
        use super::super::NativeMaskComponent;
        let mapping = run("id=in opacity value=0.5 aux=[ref=in invert-linear]", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, output: 3 }) = mapping.structure else { panic!("actual opacity") };
        assert!(matches!(&steps[1], NativeStage::Mask { input: 0, mask: 1, component: NativeMaskComponent::Composite, invert: false, scale: 1. }));
        assert!(matches!(&steps[2],NativeStage::Apply{input:2,mapping} if mapping.filters[0].command=="filter.other.alphaCurve"));
        let mapping = run("id=in threshold high=0.8 aux=[ref=in invert-linear]", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, output }) = mapping.structure else { panic!("actual threshold") };
        assert!(matches!(steps.last(), Some(NativeStage::Mask { mask: 0, component: NativeMaskComponent::Alpha, .. })));
        super::super::validate_stages(&steps, output).unwrap();
        let mapping = run("id=in weighted-blend aux=[ref=in invert-linear]", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, output }) = mapping.structure else { panic!("actual weighted inputs") };
        assert!(matches!(&steps[1], NativeStage::Channel { input: 0, component: NativeMaskComponent::Alpha }));
        assert!(matches!(&steps[2], NativeStage::Channel { input: 1, component: NativeMaskComponent::Alpha }));
        super::super::validate_stages(&steps, output).unwrap();
        let mapping = run("id=in border-align x=1 horizontal-margin=2 aux=[ref=in invert-linear]", &context()).unwrap();
        assert!(
            matches!(mapping.structure,Some(NativeStructure::Stages{steps,..}) if matches!(&steps[1],NativeStage::Align{input:0,target:1,horizontal:1.,margin_x:2.,..}))
        );
        let mapping = run("id=in piecewise-blend levels=2 aux1=[ref=in] aux2=[ref=in invert-linear]", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, output }) = mapping.structure else { panic!("actual piecewise levels") };
        super::super::validate_stages(&steps, output).unwrap();
        for graph in [
            "weighted-blend",
            "map-relative",
            "piecewise-blend levels=2 aux1=[color value=red]",
            "piecewise-blend levels=1 aux1=[color] aux1=[color]",
            "piecewise-blend levels=1 aux17=[color]",
            "id=in opacity auxiliary=[ref=in]",
            "id=in piecewise-blend levels=1 aux1=[ref=missing]",
        ] {
            assert!(run(graph, &context()).is_err(), "{graph}");
        }
        let mut limited = context();
        limited.remaining_metadata_bytes = 25000;
        run("id=in map-relative aux=[ref=in]", &limited).unwrap();
        assert!(run("id=in map-relative aux=[ref=in] map-relative aux=[ref=in]", &limited).is_err());
        assert_eq!(limited.remaining_metadata_bytes, 25000);
        for graph in ["id=in src-in aux=[ref=in]", "id=in gimp:anti-erase aux=[ref=in]", "id=in map-absolute aux=[ref=in]"] {
            let mapping = run(graph, &context()).unwrap();
            if let Some(NativeStructure::Stages { steps, output }) = mapping.structure {
                super::super::validate_stages(&steps, output).unwrap();
            }
        }
    }

    #[test]
    fn shell_quoted_literals_are_inert_and_utf8_labels_preserve_actual_references() {
        // Public GLib shell_unquote rules, not C escapes or expression expansion.
        assert_eq!(tokens(r#"text='one\n$HOME'"#).unwrap(), vec!["text", "=", r"one\n$HOME"]);
        assert_eq!(tokens(r#"text="one\"two\\three\n\$HOME\`tick""#).unwrap(), vec!["text", "=", "one\"two\\three\\n$HOME`tick"]);
        assert_eq!(tokens(r#"text='foo'\''bar' word=foo\ bar"#).unwrap(), vec!["text", "=", "foo'bar", "word", "=", "foo bar"]);
        assert_eq!(tokens("text=\"a\\\nb\"").unwrap(), vec!["text", "=", "ab"]);
        assert_eq!(tokens("text=\"a\nb\"").unwrap(), vec!["text", "=", "a\nb"]);
        assert_eq!(tokens("color=#12ab34 # inert comment\ninvert-linear").unwrap(), vec!["color", "=", "#12ab34", "invert-linear"]);
        let mapping = run(r#"text string="quote\" slash\\ literal\n $HOME `cmd`""#, &context()).unwrap();
        let Some(NativeStructure::Replace { layer, .. }) = mapping.structure else { panic!("native text") };
        let LayerContent::Text(text) = layer.content else { panic!("editable text") };
        assert_eq!(text.text, "quote\" slash\\ literal\\n $HOME `cmd`");
        for label in ["café-漢字", "a.b:label", &"é".repeat(64)] {
            let graph = format!("id='{label}' over aux=[ref='{label}' invert-linear]");
            let mapping = run(&graph, &context()).unwrap();
            assert!(matches!(mapping.structure, Some(NativeStructure::Stages { output: 2, .. })));
        }
        for label in ["", "bad space", "bad\nline", "bad=delimiter", "bad[delimiter", &"é".repeat(65)] {
            assert!(run(&format!("id='{label}' invert-linear"), &context()).is_err());
        }
        for invalid in ["text='unclosed", "text=\"unclosed", "text=trailing\\", "text='nul\0byte'"] {
            assert!(tokens(invalid).is_err());
        }
    }

    #[test]
    fn arithmetic_uses_actual_pad_presence_without_changing_scalar_or_auxiliary_identity() {
        use photocraft_color::BlendMode;
        for (operation, blend) in
            [("add", BlendMode::LinearDodge), ("subtract", BlendMode::Subtract), ("multiply", BlendMode::Multiply), ("divide", BlendMode::Divide)]
        {
            let scalar = run(&format!("{operation} value=0.5"), &context()).unwrap();
            assert!(scalar.structure.is_none());
            assert!(!scalar.filters.is_empty());
            let mapping = run(&format!("id=in {operation} value=0.5 aux=[ref=in invert-linear]"), &context()).unwrap();
            let Some(NativeStructure::Stages { steps, output: 2 }) = mapping.structure else { panic!("actual arithmetic auxiliary plan") };
            assert!(matches!(&steps[0], NativeStage::Apply { input: 0, mapping } if mapping.filters[0].command == "image.adjustments.invert"));
            assert!(matches!(&steps[1], NativeStage::Join { below: 0, above: 1, blend: actual, opacity: 1. } if *actual == blend));
        }
        let scalar = run("gamma value=2", &context()).unwrap();
        assert_eq!(scalar.filters[0].command, "image.adjustments.exposure");
        let error = run("id=in gamma aux=[ref=in invert-linear]", &context()).unwrap_err();
        assert_ne!(error.reason, photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable);
    }

    #[test]
    fn native_child_plans_rebase_input_and_output_without_nested_stages_or_losing_outer_data() {
        let mapping = run("invert-linear path d='M0,0 L4,0 L0,2 z' fill=red invert-linear", &context()).unwrap();
        let Some(NativeStructure::Stages { steps, output: 4 }) = mapping.structure else { panic!("flattened native child") };
        assert!(matches!(&steps[0], NativeStage::Apply { input: 0, .. }));
        assert!(matches!(&steps[1],NativeStage::Apply{input:1,mapping} if matches!(&mapping.structure,Some(NativeStructure::Replace{..}))));
        assert!(matches!(&steps[2], NativeStage::Join { below: 1, above: 2, .. }));
        assert!(matches!(&steps[3], NativeStage::Apply { input: 3, .. }));
        super::super::validate_stages(&steps, 4).unwrap();
        let c = context();
        let mut decoder = Decoder {
            tokens: vec![],
            cursor: 0,
            nodes: 0,
            assignments: 0,
            labels: vec![],
            steps: vec![],
            filters: 0,
            remaining_generation: c.remaining_generation_bytes,
            remaining_metadata: c.remaining_metadata_bytes,
            notes: vec![],
        };
        let invalid = NativeMapping {
            filters: vec![super::super::native_filter("image.adjustments.invert", json!({}))],
            structure: Some(NativeStructure::Stages { steps: vec![], output: 0 }),
            ..Default::default()
        };
        assert!(decoder.push_mapping(0, invalid, &c).unwrap_err().message.contains("outer data"));
        assert!(decoder.steps.is_empty());
        let invert = || NativeMapping { filters: vec![super::super::native_filter("image.adjustments.invert", json!({}))], ..Default::default() };
        let input = decoder.push_mapping(0, invert(), &c).unwrap();
        let child = NativeMapping {
            structure: Some(NativeStructure::Stages {
                steps: vec![NativeStage::Apply { input: 0, mapping: Box::new(invert()) }, NativeStage::Knockout { below: 0, above: 1, opacity: 0.3 }],
                output: 2,
            }),
            ..Default::default()
        };
        assert_eq!(decoder.push_mapping(input, child, &c).unwrap(), 3);
        assert!(matches!(&decoder.steps[1], NativeStage::Apply { input: 1, .. }));
        assert!(matches!(&decoder.steps[2], NativeStage::Knockout { below: 1, above: 2, opacity: 0.3 }));
        super::super::validate_stages(&decoder.steps, 3).unwrap();
    }
}
