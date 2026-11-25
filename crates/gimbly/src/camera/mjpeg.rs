//! Motion JPEG frames to NV12 for the preview, snapshots and the virtual
//! camera. Pure Rust (zune-jpeg), so every platform decodes the same way.

use gimbly_vcam_shm::Nv12View;
use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
use zune_jpeg::JpegDecoder;

/// Decodes a `w` x `h` JPEG into `packed` NV12; `None` for any other size
/// or a damaged frame.
pub(super) fn decode<'a>(
    data: &[u8],
    w: usize,
    h: usize,
    decoded: &mut Vec<u8>,
    packed: &'a mut Vec<u8>,
) -> Option<Nv12View<'a>> {
    let options = DecoderOptions::default()
        .set_max_width(w)
        .set_max_height(h)
        .jpeg_set_out_colorspace(ColorSpace::YCbCr);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(data), options);
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    if (usize::from(info.width), usize::from(info.height)) != (w, h) {
        return None;
    }
    let size = w * h;
    if decoder.output_buffer_size()? != size * 3 {
        return None;
    }
    decoded.resize(size * 3, 0);
    decoder.decode_into(decoded).ok()?;
    packed.resize(size * 3 / 2, 0);
    let (y, uv) = packed.split_at_mut(size);
    for (dst, pixel) in y.iter_mut().zip(decoded.as_chunks::<3>().0) {
        *dst = pixel[0];
    }
    for (rows, dst) in decoded.chunks_exact(w * 6).zip(uv.chunks_exact_mut(w)) {
        let (top, bottom) = rows.split_at(w * 3);
        for ((a, b), uv) in top
            .as_chunks::<6>()
            .0
            .iter()
            .zip(bottom.as_chunks::<6>().0)
            .zip(dst.as_chunks_mut::<2>().0)
        {
            for (out, c) in uv.iter_mut().zip(1..=2) {
                *out = ((u16::from(a[c])
                    + u16::from(a[c + 3])
                    + u16::from(b[c])
                    + u16::from(b[c + 3])
                    + 2)
                    / 4) as u8;
            }
        }
    }
    Nv12View::packed(packed, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_truncated_frames() {
        let mut packed = Vec::new();
        assert!(decode(&[0xff, 0xd8], 2, 2, &mut Vec::new(), &mut packed).is_none());
        assert!(decode(&[], 2, 2, &mut Vec::new(), &mut packed).is_none());
    }
}
