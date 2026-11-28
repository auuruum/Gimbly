//! The user's own Sleep Background pictures: PNG or JPEG files that Gimbly
//! shows instead of the camera's picture while it sleeps.

use eframe::egui::IconData;
use std::sync::Arc;
use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
use zune_jpeg::JpegDecoder;

/// At most this many pictures can be added.
pub const MAX: usize = 8;
/// Larger files are refused before they are read.
const MAX_FILE: u64 = 64 << 20;
/// Larger images are refused before they are decoded.
const MAX_SIDE: usize = 16_384;
/// Kept pictures are scaled down to fit this; the video is at most 4K.
const KEEP: (usize, usize) = (3840, 2160);

/// Reads and decodes a picture file into RGBA.
pub fn load(path: &str) -> Result<Arc<IconData>, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("Could not read {path}: {e}"))?
        .len();
    if size > MAX_FILE {
        return Err("The picture is larger than 64 MB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read {path}: {e}"))?;
    decode(&bytes).map(Arc::new)
}

/// PNG or JPEG bytes to RGBA, scaled down to [`KEEP`].
pub fn decode(bytes: &[u8]) -> Result<IconData, String> {
    let (rgba, w, h) = if bytes.starts_with(b"\x89PNG") {
        decode_png(bytes)?
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        decode_jpeg(bytes)?
    } else {
        return Err("Only PNG and JPEG pictures are supported".into());
    };
    let (rgba, w, h) = shrink(&rgba, (w, h), KEEP);
    Ok(IconData {
        rgba,
        width: w as u32,
        height: h as u32,
    })
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    let bad = |e: png::DecodingError| format!("The PNG file could not be read: {e}");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(bad)?;
    let (w, h) = (reader.info().width as usize, reader.info().height as usize);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err("The picture is too large".into());
    }
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("The picture is too large")?
    ];
    let info = reader.next_frame(&mut buf).map_err(bad)?;
    let pixels = w * h;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf.get(..pixels * 4).map(<[u8]>::to_vec),
        png::ColorType::Rgb => buf.get(..pixels * 3).map(|b| {
            b.as_chunks::<3>()
                .0
                .iter()
                .flat_map(|[r, g, b]| [*r, *g, *b, 255])
                .collect()
        }),
        png::ColorType::GrayscaleAlpha => buf.get(..pixels * 2).map(|b| {
            b.as_chunks::<2>()
                .0
                .iter()
                .flat_map(|[l, a]| [*l, *l, *l, *a])
                .collect()
        }),
        png::ColorType::Grayscale => buf
            .get(..pixels)
            .map(|b| b.iter().flat_map(|l| [*l, *l, *l, 255]).collect()),
        png::ColorType::Indexed => None,
    };
    rgba.map(|rgba| (rgba, w, h))
        .ok_or_else(|| "The PNG file could not be read".into())
}

fn decode_jpeg(bytes: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    let options = DecoderOptions::default()
        .set_max_width(MAX_SIDE)
        .set_max_height(MAX_SIDE)
        .jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let bad = |e| format!("The JPEG file could not be read: {e:?}");
    decoder.decode_headers().map_err(bad)?;
    let info = decoder.info().ok_or("The JPEG file could not be read")?;
    let (w, h) = (usize::from(info.width), usize::from(info.height));
    let rgba = decoder.decode().map_err(bad)?;
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return Err("The JPEG file could not be read".into());
    }
    Ok((rgba, w, h))
}

/// Scales RGBA down to fit `max` by averaging boxes of pixels; never up.
pub fn shrink(rgba: &[u8], (w, h): (usize, usize), max: (usize, usize)) -> (Vec<u8>, usize, usize) {
    let step = w
        .div_ceil(max.0.max(1))
        .max(h.div_ceil(max.1.max(1)))
        .max(1);
    if step == 1 || rgba.len() < w * h * 4 {
        return (rgba.to_vec(), w, h);
    }
    let (out_w, out_h) = ((w / step).max(1), (h / step).max(1));
    let mut out = Vec::with_capacity(out_w * out_h * 4);
    for oy in 0..out_h {
        for ox in 0..out_w {
            let mut sum = [0u32; 4];
            for y in oy * step..((oy + 1) * step).min(h) {
                for x in ox * step..((ox + 1) * step).min(w) {
                    let at = (y * w + x) * 4;
                    for (s, c) in sum.iter_mut().zip(&rgba[at..at + 4]) {
                        *s += u32::from(*c);
                    }
                }
            }
            let n = (step * step) as u32;
            out.extend(sum.map(|s| (s / n) as u8));
        }
    }
    (out, out_w, out_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn decodes_rgb_and_grey_png() {
        let rgb = decode(&png(2, 1, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6])).unwrap();
        assert_eq!((rgb.width, rgb.height), (2, 1));
        assert_eq!(rgb.rgba, [1, 2, 3, 255, 4, 5, 6, 255]);
        let grey = decode(&png(1, 1, png::ColorType::Grayscale, &[9])).unwrap();
        assert_eq!(grey.rgba, [9, 9, 9, 255]);
    }

    #[test]
    fn refuses_other_and_broken_files() {
        assert!(decode(b"GIF89a").is_err());
        assert!(decode(b"").is_err());
        assert!(decode(&[0xff, 0xd8, 0xff, 0x00]).is_err());
        let mut broken = png(4, 4, png::ColorType::Rgba, &[7; 64]);
        broken.truncate(broken.len() / 2);
        assert!(decode(&broken).is_err());
    }

    #[test]
    fn shrinks_by_averaging_and_never_grows() {
        let rgba = [0, 0, 0, 255, 200, 100, 50, 255].repeat(2);
        let (out, w, h) = shrink(&rgba, (2, 2), (1, 1));
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, [100, 50, 25, 255]);
        let (same, w, h) = shrink(&rgba, (2, 2), (10, 10));
        assert_eq!((w, h, same.len()), (2, 2, 16));
    }
}
