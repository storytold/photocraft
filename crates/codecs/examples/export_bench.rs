//! Reproducible export cost: 24 MP, or each icon's largest legal size.
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = (0..6000usize * 4000 * 4).map(|i| if i % 4 == 3 { 255 } else { ((i / 4 + i % 4 * 73) % 256) as u8 }).collect();
    let image = Image::from_u8(6000, 4000, ChannelLayout::Rgba, data)?;
    for f in [
        Format::Pcx,
        Format::Sgi,
        Format::SunRaster,
        Format::Farbfeld,
        Format::Wbmp,
        Format::Xbm,
        Format::Xpm,
        Format::Cur,
        Format::Icns,
        Format::Gbr,
        Format::GimpPat,
    ] {
        let icon = match f {
            Format::Cur => Some(Image::from_u8(256, 256, ChannelLayout::Rgba, vec![255; 256 * 256 * 4])?),
            Format::Icns => Some(Image::from_u8(1024, 1024, ChannelLayout::Rgba, vec![255; 1024 * 1024 * 4])?),
            _ => None,
        };
        let source = icon.as_ref().unwrap_or(&image);
        let now = std::time::Instant::now();
        let bytes = photocraft_codecs::encode(source, f, &EncodeOptions::default())?;
        println!("{:?}: {}x{}, {} bytes, {:.2} ms", f, source.width(), source.height(), bytes.len(), now.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(())
}
