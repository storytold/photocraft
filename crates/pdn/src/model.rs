use crate::chunks;
use crate::error::Error;
use crate::header;
use crate::nrbf::{self, Value};

/// Paint.NET layer blend modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Normal = 0,
    Multiply = 1,
    Additive = 2,
    ColorBurn = 3,
    ColorDodge = 4,
    Reflect = 5,
    Glow = 6,
    Overlay = 7,
    Difference = 8,
    Negation = 9,
    Lighten = 10,
    Darken = 11,
    Screen = 12,
    Xor = 13,
}

impl BlendMode {
    pub fn from_u32(val: u32) -> Self {
        match val {
            0 => Self::Normal,
            1 => Self::Multiply,
            2 => Self::Additive,
            3 => Self::ColorBurn,
            4 => Self::ColorDodge,
            5 => Self::Reflect,
            6 => Self::Glow,
            7 => Self::Overlay,
            8 => Self::Difference,
            9 => Self::Negation,
            10 => Self::Lighten,
            11 => Self::Darken,
            12 => Self::Screen,
            13 => Self::Xor,
            _ => Self::Normal,
        }
    }

    pub fn from_class_name(name: &str) -> Self {
        if name.contains("Multiply") {
            Self::Multiply
        } else if name.contains("Additive") {
            Self::Additive
        } else if name.contains("ColorBurn") {
            Self::ColorBurn
        } else if name.contains("ColorDodge") {
            Self::ColorDodge
        } else if name.contains("Reflect") {
            Self::Reflect
        } else if name.contains("Glow") {
            Self::Glow
        } else if name.contains("Overlay") {
            Self::Overlay
        } else if name.contains("Difference") {
            Self::Difference
        } else if name.contains("Negation") {
            Self::Negation
        } else if name.contains("Lighten") {
            Self::Lighten
        } else if name.contains("Darken") {
            Self::Darken
        } else if name.contains("Screen") {
            Self::Screen
        } else if name.contains("Xor") || name.contains("XOR") {
            Self::Xor
        } else {
            Self::Normal
        }
    }
}

/// A decoded Paint.NET layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub is_background: bool,
    pub opacity: u8,
    pub blend_mode: BlendMode,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub rgba_pixels: Vec<u8>,
}

/// A decoded Paint.NET document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub width: u32,
    pub height: u32,
    pub version: Option<String>,
    pub layers: Vec<Layer>,
    pub warnings: Vec<String>,
    pub thumbnail_png: Option<Vec<u8>>,
}

/// Safety limits for parsing untrusted PDN files.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_header_size: usize,
    pub max_dimension: u32,
    pub max_layers: usize,
    pub max_memory_per_layer: usize,
    pub max_total_memory: usize,
    pub max_objects: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 512 << 20,       // 512 MiB
            max_header_size: 10 << 20,      // 10 MiB
            max_dimension: 65_536,          // 65k pixels side
            max_layers: 1_024,              // 1024 layers
            max_memory_per_layer: 512 << 20,// 512 MiB per layer buffer
            max_total_memory: 1 << 30,      // 1 GiB total pixel buffers
            max_objects: 100_000,           // 100k NRBF objects
        }
    }
}

/// Reads a PDN document using default limits.
pub fn read(bytes: &[u8], limits: Limits) -> Result<Document, Error> {
    read_with_limits(bytes, limits)
}

/// Reads a PDN document with the given safety limits.
pub fn read_with_limits(bytes: &[u8], limits: Limits) -> Result<Document, Error> {
    if bytes.len() > limits.max_file_size {
        return Err(Error::Limit("file size exceeds maximum allowed limit"));
    }

    let (header_info, remaining) = header::parse_header(bytes, limits.max_header_size)?;
    let (ctx, mut chunk_stream) = nrbf::parse_nrbf(remaining, limits.max_objects)?;

    let root_val = ctx
        .get_object(ctx.root_id)
        .ok_or(Error::Malformed("root document object missing in NRBF stream"))?;
    let root = root_val
        .as_class()
        .ok_or(Error::Malformed("root document is not a class instance"))?;

    let width = root
        .find("width")
        .and_then(Value::as_i32)
        .and_then(|w| u32::try_from(w).ok())
        .or(header_info.width)
        .ok_or(Error::Malformed("missing document width"))?;

    let height = root
        .find("height")
        .and_then(Value::as_i32)
        .and_then(|h| u32::try_from(h).ok())
        .or(header_info.height)
        .ok_or(Error::Malformed("missing document height"))?;

    if width == 0 || height == 0 {
        return Err(Error::Malformed("document dimensions cannot be zero"));
    }
    if width > limits.max_dimension || height > limits.max_dimension {
        return Err(Error::Limit("document dimensions exceed maximum allowed size"));
    }

    let layers_val = root
        .find("layers")
        .and_then(|v| ctx.resolve(v))
        .ok_or(Error::Malformed("missing layers collection in document"))?;
    let layer_list = layers_val
        .as_class()
        .ok_or(Error::Malformed("layers field is not a class instance"))?;

    let layer_count = layer_list
        .find("size")
        .and_then(Value::as_i32)
        .and_then(|s| usize::try_from(s).ok())
        .or(header_info.layers)
        .unwrap_or(0);

    if layer_count > limits.max_layers {
        return Err(Error::Limit("layer count exceeds maximum allowed limit"));
    }

    let items_val = layer_list
        .find("items")
        .and_then(|v| ctx.resolve(v))
        .ok_or(Error::Malformed("missing items array in layers collection"))?;
    let items_array = items_val
        .as_array()
        .ok_or(Error::Malformed("items field is not an array"))?;

    let mut layers = Vec::with_capacity(layer_count);
    let mut total_memory: usize = 0;
    let mut warnings = Vec::new();

    for index in 0..layer_count {
        let layer_val = items_array
            .get(index)
            .and_then(|v| ctx.resolve(v))
            .ok_or(Error::Malformed("layer array element missing"))?;
        let layer_obj = layer_val
            .as_class()
            .ok_or(Error::Malformed("layer element is not a class instance"))?;

        let layer_props = layer_obj
            .find("Layer_properties")
            .or_else(|| layer_obj.find("properties"))
            .and_then(|v| ctx.resolve(v))
            .and_then(Value::as_class);

        let name = layer_props
            .and_then(|p| p.find("name"))
            .and_then(|v| ctx.resolve(v))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("Layer {}", index + 1));

        let visible = layer_props
            .and_then(|p| p.find("visible"))
            .and_then(|v| ctx.resolve(v))
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let is_background = layer_props
            .and_then(|p| p.find("isBackground"))
            .and_then(|v| ctx.resolve(v))
            .and_then(Value::as_bool)
            .unwrap_or(index == 0);

        let opacity = layer_props
            .and_then(|p| p.find("opacity"))
            .and_then(|v| ctx.resolve(v))
            .and_then(Value::as_i32)
            .map(|o| o.clamp(0, 255) as u8)
            .unwrap_or(255);

        let blend_mode = extract_blend_mode(&ctx, layer_obj, layer_props);

        let surface_val = layer_obj
            .find("surface")
            .and_then(|v| ctx.resolve(v))
            .ok_or(Error::Malformed("missing surface in layer"))?;
        let surf_obj = surface_val
            .as_class()
            .ok_or(Error::Malformed("surface is not a class instance"))?;

        let layer_w = surf_obj
            .find("width")
            .and_then(Value::as_i32)
            .and_then(|w| u32::try_from(w).ok())
            .unwrap_or(width);

        let layer_h = surf_obj
            .find("height")
            .and_then(Value::as_i32)
            .and_then(|h| u32::try_from(h).ok())
            .unwrap_or(height);

        let stride = surf_obj
            .find("stride")
            .and_then(Value::as_i32)
            .and_then(|s| u32::try_from(s).ok())
            .unwrap_or(layer_w.saturating_mul(4));

        let scan0_val = surf_obj
            .find("scan0")
            .and_then(|v| ctx.resolve(v))
            .ok_or(Error::Malformed("missing scan0 in surface"))?;
        let scan0_obj = scan0_val
            .as_class()
            .ok_or(Error::Malformed("scan0 is not a class instance"))?;

        let length = scan0_obj
            .find("length64")
            .and_then(Value::as_i64)
            .or_else(|| scan0_obj.find("length").and_then(Value::as_i64))
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or((layer_h as usize).saturating_mul(stride as usize));

        total_memory = total_memory
            .checked_add(length)
            .ok_or(Error::Limit("total pixel memory overflow"))?;
        if total_memory > limits.max_total_memory {
            return Err(Error::Limit("total pixel memory exceeds safety limit"));
        }

        let raw_pixels = chunks::read_layer_chunks(&mut chunk_stream, length, limits.max_memory_per_layer)?;
        let rgba_pixels = chunks::convert_to_rgba8(&raw_pixels, layer_w, layer_h, stride)?;

        layers.push(Layer {
            name,
            visible,
            is_background,
            opacity,
            blend_mode,
            width: layer_w,
            height: layer_h,
            stride,
            rgba_pixels,
        });
    }

    if let Some(v) = &header_info.version {
        warnings.push(format!("Saved with Paint.NET version {v}"));
    }

    Ok(Document {
        width,
        height,
        version: header_info.version,
        layers,
        warnings,
        thumbnail_png: header_info.thumbnail_png,
    })
}

fn extract_blend_mode(
    ctx: &nrbf::NrbfContext,
    layer_obj: &nrbf::ClassInstance,
    layer_props: Option<&nrbf::ClassInstance>,
) -> BlendMode {
    if let Some(props) = layer_props
        && let Some(bm_val) = props.find("blendMode").and_then(|v| ctx.resolve(v))
    {
        if let Some(val_i32) = bm_val.as_i32() {
            return BlendMode::from_u32(val_i32 as u32);
        }
        if let Some(bm_class) = bm_val.as_class()
            && let Some(val_i32) = bm_class.find("value__").and_then(Value::as_i32)
        {
            return BlendMode::from_u32(val_i32 as u32);
        }
    }

    let b_props = layer_obj
        .find("properties")
        .and_then(|v| ctx.resolve(v))
        .and_then(Value::as_class);

    if let Some(bp) = b_props
        && let Some(op_val) = bp.find("blendOp").and_then(|v| ctx.resolve(v))
        && let Some(op_class) = op_val.as_class()
    {
        return BlendMode::from_class_name(&op_class.class_name);
    }

    BlendMode::Normal
}
