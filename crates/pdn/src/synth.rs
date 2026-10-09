use std::io::Write;

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::model::BlendMode;

/// Synthetic layer specification for tests.
pub struct SynthLayer {
    pub name: String,
    pub visible: bool,
    pub is_background: bool,
    pub opacity: u8,
    pub blend_mode: BlendMode,
    pub rgba_pixels: Vec<u8>,
}

/// Builds a valid PDN file in memory.
pub fn build_pdn(
    width: u32,
    height: u32,
    layers: &[SynthLayer],
    compress: bool,
) -> Vec<u8> {
    let mut out = Vec::new();

    // 1. Magic: PDN3
    out.extend_from_slice(b"PDN3");

    // 2. XML Header
    let xml = format!(
        "<pdnImage width=\"{width}\" height=\"{height}\" layers=\"{}\" savedWithVersion=\"4.21.6589.7045\"><custom><thumb png=\"\" /></custom></pdnImage>",
        layers.len()
    );
    let xml_len = xml.len();
    out.push((xml_len & 0xff) as u8);
    out.push(((xml_len >> 8) & 0xff) as u8);
    out.push(((xml_len >> 16) & 0xff) as u8);
    out.extend_from_slice(xml.as_bytes());

    // 3. Indicator bytes: 0x00, 0x01
    out.push(0x00);
    out.push(0x01);

    // 4. NRBF Stream
    // SerializedStreamHeader (record 0)
    out.push(0x00);
    out.extend_from_slice(&1i32.to_le_bytes()); // root_id = 1
    out.extend_from_slice(&(-1i32).to_le_bytes()); // header_id = -1
    out.extend_from_slice(&1i32.to_le_bytes()); // major = 1
    out.extend_from_slice(&0i32.to_le_bytes()); // minor = 0

    // BinaryLibrary (record 12)
    out.push(12);
    out.extend_from_slice(&2i32.to_le_bytes()); // library_id = 2
    write_string(&mut out, "PaintDotNet.Data, Version=4.0.0.0, Culture=neutral, PublicKeyToken=null");

    // PaintDotNet.Document (record 5: ClassWithMembersAndTypes, id = 1)
    out.push(5);
    out.extend_from_slice(&1i32.to_le_bytes()); // object_id = 1
    write_string(&mut out, "PaintDotNet.Document");
    out.extend_from_slice(&4i32.to_le_bytes()); // member_count = 4
    write_string(&mut out, "isDisposed");
    write_string(&mut out, "layers");
    write_string(&mut out, "width");
    write_string(&mut out, "height");
    // Binary types:
    // isDisposed: Primitive (0)
    // layers: Class (4)
    // width: Primitive (0)
    // height: Primitive (0)
    out.push(0); // Primitive
    out.push(4); // Class
    out.push(0); // Primitive
    out.push(0); // Primitive
    // Additional infos:
    out.push(1); // Boolean
    write_string(&mut out, "PaintDotNet.LayerList");
    out.extend_from_slice(&2i32.to_le_bytes()); // library_id = 2
    out.push(8); // Int32
    out.push(8); // Int32
    out.extend_from_slice(&2i32.to_le_bytes()); // library_id = 2 for Document

    // Member values for Document:
    // isDisposed
    out.push(0); // false

    // layers: inline Record 5 for LayerList (id = 3)
    out.push(5);
    out.extend_from_slice(&3i32.to_le_bytes()); // object_id = 3
    write_string(&mut out, "PaintDotNet.LayerList");
    out.extend_from_slice(&2i32.to_le_bytes()); // member_count = 2
    write_string(&mut out, "ArrayList__size");
    write_string(&mut out, "ArrayList__items");
    out.push(0); // Primitive
    out.push(5); // ObjectArray
    out.push(8); // Int32
    out.extend_from_slice(&2i32.to_le_bytes()); // library_id = 2

    // Member values for LayerList:
    out.extend_from_slice(&(layers.len() as i32).to_le_bytes()); // ArrayList__size

    // ArrayList__items: Record 16 (ArraySingleObject, id = 4)
    out.push(16);
    out.extend_from_slice(&4i32.to_le_bytes()); // object_id = 4
    out.extend_from_slice(&(layers.len() as i32).to_le_bytes()); // length

    // Elements of items array: each layer is a BitmapLayer
    for (i, layer) in layers.iter().enumerate() {
        let base_id = 100 + (i as i32) * 10;
        let layer_id = base_id;
        let props_id = base_id + 1;
        let name_id = base_id + 2;
        let surf_id = base_id + 3;
        let scan0_id = base_id + 4;

        // BitmapLayer: Record 5 (ClassWithMembersAndTypes)
        out.push(5);
        out.extend_from_slice(&layer_id.to_le_bytes());
        write_string(&mut out, "PaintDotNet.BitmapLayer");
        out.extend_from_slice(&4i32.to_le_bytes());
        write_string(&mut out, "Layer_width");
        write_string(&mut out, "Layer_height");
        write_string(&mut out, "Layer_properties");
        write_string(&mut out, "surface");
        out.push(0); // Primitive
        out.push(0); // Primitive
        out.push(4); // Class
        out.push(4); // Class
        out.push(8); // Int32
        out.push(8); // Int32
        write_string(&mut out, "PaintDotNet.Layer.LayerProperties");
        out.extend_from_slice(&2i32.to_le_bytes());
        write_string(&mut out, "PaintDotNet.Surface");
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes()); // lib id for BitmapLayer

        // Members of BitmapLayer:
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(height as i32).to_le_bytes());

        // Layer_properties: inline Record 5
        out.push(5);
        out.extend_from_slice(&props_id.to_le_bytes());
        write_string(&mut out, "PaintDotNet.Layer.LayerProperties");
        out.extend_from_slice(&5i32.to_le_bytes());
        write_string(&mut out, "name");
        write_string(&mut out, "visible");
        write_string(&mut out, "isBackground");
        write_string(&mut out, "opacity");
        write_string(&mut out, "blendMode");
        out.push(1); // String
        out.push(0); // Primitive
        out.push(0); // Primitive
        out.push(0); // Primitive
        out.push(0); // Primitive
        out.push(1); // Boolean
        out.push(1); // Boolean
        out.push(2); // Byte
        out.push(8); // Int32
        out.extend_from_slice(&2i32.to_le_bytes()); // lib id

        // Members of Layer_properties:
        // name: Record 6 (BinaryObjectString)
        out.push(6);
        out.extend_from_slice(&name_id.to_le_bytes());
        write_string(&mut out, &layer.name);

        out.push(if layer.visible { 1 } else { 0 });
        out.push(if layer.is_background { 1 } else { 0 });
        out.push(layer.opacity);
        out.extend_from_slice(&(layer.blend_mode as i32).to_le_bytes());

        // surface: inline Record 5
        out.push(5);
        out.extend_from_slice(&surf_id.to_le_bytes());
        write_string(&mut out, "PaintDotNet.Surface");
        out.extend_from_slice(&4i32.to_le_bytes());
        write_string(&mut out, "width");
        write_string(&mut out, "height");
        write_string(&mut out, "stride");
        write_string(&mut out, "scan0");
        out.push(0); // Primitive
        out.push(0); // Primitive
        out.push(0); // Primitive
        out.push(4); // Class
        out.push(8); // Int32
        out.push(8); // Int32
        out.push(8); // Int32
        write_string(&mut out, "PaintDotNet.MemoryBlock");
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes()); // lib id

        // Members of surface:
        let stride = width * 4;
        let length64 = (height as i64) * (stride as i64);
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(height as i32).to_le_bytes());
        out.extend_from_slice(&(stride as i32).to_le_bytes());

        // scan0: inline Record 5
        out.push(5);
        out.extend_from_slice(&scan0_id.to_le_bytes());
        write_string(&mut out, "PaintDotNet.MemoryBlock");
        out.extend_from_slice(&1i32.to_le_bytes());
        write_string(&mut out, "length64");
        out.push(0); // Primitive
        out.push(9); // Int64
        out.extend_from_slice(&2i32.to_le_bytes()); // lib id

        // Members of scan0:
        out.extend_from_slice(&length64.to_le_bytes());
    }

    // width and height members of Document:
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());

    // MessageEnd (record 11)
    out.push(11);

    // 5. Deferred chunks for each layer
    for layer in layers {
        let stride = width * 4;
        let length = (height as usize) * (stride as usize);

        // Convert RGBA to BGRA
        let mut bgra = vec![0u8; length];
        for (src, dst) in layer.rgba_pixels.as_chunks::<4>().0.iter().zip(bgra.as_chunks_mut::<4>().0.iter_mut()) {
            dst[0] = src[2]; // B
            dst[1] = src[1]; // G
            dst[2] = src[0]; // R
            dst[3] = src[3]; // A
        }

        let chunk_size = 65536usize;
        let format_version = if compress { 0u8 } else { 1u8 };
        out.push(format_version);
        out.extend_from_slice(&(chunk_size as u32).to_be_bytes());

        let chunk_count = length.div_ceil(chunk_size);
        for c in 0..chunk_count {
            let offset = c * chunk_size;
            let actual = std::cmp::min(chunk_size, length - offset);
            let slice = &bgra[offset..offset + actual];

            out.extend_from_slice(&(c as u32).to_be_bytes());

            if compress {
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                let _ = encoder.write_all(slice);
                let compressed = encoder.finish().unwrap_or_default();
                out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
                out.extend_from_slice(&compressed);
            } else {
                out.extend_from_slice(&(actual as u32).to_be_bytes());
                out.extend_from_slice(slice);
            }
        }
    }

    out
}

fn write_7bit_int(buf: &mut Vec<u8>, mut val: usize) {
    while val >= 0x80 {
        buf.push(((val & 0x7f) as u8) | 0x80);
        val >>= 7;
    }
    buf.push(val as u8);
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_7bit_int(buf, s.len());
    buf.extend_from_slice(s.as_bytes());
}
