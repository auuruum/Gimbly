//! YUV frame helpers shared by the producer and the virtual camera.
//!
//! Scaling is nearest-neighbour: it only runs when a consumer asks for a size
//! different from the capture size, and it keeps per-frame cost predictable.

/// Borrowed NV12 frame: a full-resolution luma plane followed by an
/// interleaved, half-resolution UV plane.
#[derive(Clone, Copy, Debug)]
pub struct Nv12View<'a> {
    pub width: usize,
    pub height: usize,
    pub y: &'a [u8],
    pub y_stride: usize,
    pub uv: &'a [u8],
    pub uv_stride: usize,
}

impl<'a> Nv12View<'a> {
    /// Splits a tightly packed NV12 buffer. Returns `None` when the buffer is
    /// too small or the size is odd.
    pub fn packed(buf: &'a [u8], width: usize, height: usize) -> Option<Self> {
        Self::with_stride(buf, width, height, width, height)
    }

    /// Splits an NV12 buffer whose rows are `stride` bytes apart and whose UV
    /// plane starts after `alloc_height` luma rows (decoders often pad the
    /// height, e.g. 1088 rows for a 1080-row picture).
    pub fn with_stride(
        buf: &'a [u8],
        width: usize,
        height: usize,
        stride: usize,
        alloc_height: usize,
    ) -> Option<Self> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return None;
        }
        if stride < width || alloc_height < height {
            return None;
        }
        let uv_start = stride.checked_mul(alloc_height)?;
        let uv_len = stride.checked_mul(height / 2)?;
        if buf.len() < uv_start.checked_add(uv_len)? {
            return None;
        }
        Some(Self {
            width,
            height,
            y: &buf[..uv_start],
            y_stride: stride,
            uv: &buf[uv_start..],
            uv_stride: stride,
        })
    }

    /// The columns `x .. x + width` of the frame, without copying. Both must
    /// be even (NV12 shares one chroma pair between two columns).
    pub fn crop_columns(&self, x: usize, width: usize) -> Option<Self> {
        if width == 0 || !x.is_multiple_of(2) || !width.is_multiple_of(2) {
            return None;
        }
        if x.checked_add(width)? > self.width {
            return None;
        }
        Some(Self {
            width,
            y: &self.y[x..],
            uv: &self.uv[x..],
            ..*self
        })
    }

    /// The centred 9:16 part of the frame (unchanged if it is already that
    /// narrow).
    pub fn crop_to_portrait(&self) -> Option<Self> {
        let width = (self.height * 9 / 16) & !1;
        if width == 0 || width >= self.width {
            return Some(*self);
        }
        let x = ((self.width - width) / 2) & !1;
        self.crop_columns(x, width)
    }

    fn y_row(&self, row: usize) -> &[u8] {
        &self.y[row * self.y_stride..row * self.y_stride + self.width]
    }

    fn uv_row(&self, row: usize) -> &[u8] {
        &self.uv[row * self.uv_stride..row * self.uv_stride + self.width]
    }
}

/// Pixel formats the virtual camera can hand to applications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Nv12,
    I420,
    Yuy2,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 3] = [OutputFormat::Nv12, OutputFormat::I420, OutputFormat::Yuy2];

    pub fn bits_per_pixel(self) -> u16 {
        match self {
            OutputFormat::Nv12 | OutputFormat::I420 => 12,
            OutputFormat::Yuy2 => 16,
        }
    }

    pub fn frame_bytes(self, width: usize, height: usize) -> usize {
        width * height * self.bits_per_pixel() as usize / 8
    }
}

/// Converts `src` into `fmt` at `dw` x `dh`, scaling when the sizes differ.
/// `dst` must hold at least `fmt.frame_bytes(dw, dh)` bytes, and `dw`/`dh` must
/// be even.
pub fn convert_scaled(src: &Nv12View<'_>, fmt: OutputFormat, dst: &mut [u8], dw: usize, dh: usize) {
    debug_assert!(dw.is_multiple_of(2) && dh.is_multiple_of(2));
    let need = fmt.frame_bytes(dw, dh);
    if dst.len() < need || dw == 0 || dh == 0 {
        return;
    }
    // Source column for each destination column, and source chroma pair for
    // each destination chroma column.
    let x_map: Vec<usize> = (0..dw).map(|x| x * src.width / dw).collect();
    let cx_map: Vec<usize> = (0..dw / 2)
        .map(|cx| (cx * src.width / dw).min(src.width / 2 - 1))
        .collect();
    let same_width = dw == src.width;
    let src_row = |y: usize| y * src.height / dh;
    let src_crow = |cy: usize| (cy * src.height / dh).min(src.height / 2 - 1);

    let (luma, chroma) = dst[..need].split_at_mut(dw * dh);
    match fmt {
        OutputFormat::Nv12 | OutputFormat::I420 => {
            for (y, out) in luma.chunks_exact_mut(dw).enumerate() {
                let row = src.y_row(src_row(y));
                if same_width {
                    out.copy_from_slice(row);
                } else {
                    for (o, &sx) in out.iter_mut().zip(&x_map) {
                        *o = row[sx];
                    }
                }
            }
            if fmt == OutputFormat::Nv12 {
                for (cy, out) in chroma.chunks_exact_mut(dw).enumerate() {
                    let row = src.uv_row(src_crow(cy));
                    if same_width {
                        out.copy_from_slice(row);
                    } else {
                        for (pair, &sx) in out.as_chunks_mut::<2>().0.iter_mut().zip(&cx_map) {
                            pair[0] = row[sx * 2];
                            pair[1] = row[sx * 2 + 1];
                        }
                    }
                }
            } else {
                let (u_plane, v_plane) = chroma.split_at_mut(dw * dh / 4);
                for (cy, (u_out, v_out)) in u_plane
                    .chunks_exact_mut(dw / 2)
                    .zip(v_plane.chunks_exact_mut(dw / 2))
                    .enumerate()
                {
                    let row = src.uv_row(src_crow(cy));
                    for ((u, v), &sx) in u_out.iter_mut().zip(v_out.iter_mut()).zip(&cx_map) {
                        *u = row[sx * 2];
                        *v = row[sx * 2 + 1];
                    }
                }
            }
        }
        OutputFormat::Yuy2 => {
            let out = &mut dst[..need];
            for (y, line) in out.chunks_exact_mut(dw * 2).enumerate() {
                let yrow = src.y_row(src_row(y));
                let uvrow = src.uv_row(src_crow(y / 2));
                for (cx, quad) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let x0 = cx * 2;
                    let sc = cx_map[cx];
                    quad[0] = yrow[x_map[x0]];
                    quad[1] = uvrow[sc * 2];
                    quad[2] = yrow[x_map[x0 + 1]];
                    quad[3] = uvrow[sc * 2 + 1];
                }
            }
        }
    }
}

/// Fills `dst` with a neutral dark frame, shown while no producer is running.
/// Rotation in clockwise quarter turns plus a left-right mirror (applied
/// after the rotation), as chosen in the preview window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Transform {
    quarter_turns: u8,
    pub mirror: bool,
}

impl Transform {
    /// `degrees` is rounded down to a multiple of 90.
    pub fn new(degrees: u32, mirror: bool) -> Self {
        Self {
            quarter_turns: (degrees / 90 % 4) as u8,
            mirror,
        }
    }

    pub fn degrees(self) -> u32 {
        self.quarter_turns as u32 * 90
    }

    pub fn clockwise(self) -> Self {
        Self {
            quarter_turns: (self.quarter_turns + 1) % 4,
            ..self
        }
    }

    pub fn counter_clockwise(self) -> Self {
        Self {
            quarter_turns: (self.quarter_turns + 3) % 4,
            ..self
        }
    }

    pub fn is_identity(self) -> bool {
        self == Self::default()
    }

    /// Whether the picture lies on its side (90° or 270°).
    pub fn is_sideways(self) -> bool {
        self.quarter_turns % 2 == 1
    }
}

/// Where the transformed picture sits inside a frame of the source's size:
/// `(x, y, width, height)`, all even. Sideways pictures are fitted inside
/// with black bars, so consumers keep the size they negotiated.
pub fn transformed_area(width: usize, height: usize, t: Transform) -> (usize, usize, usize, usize) {
    if !t.is_sideways() {
        return (0, 0, width, height);
    }
    // The rotated picture is height x width; scale it to fit.
    let (rw, rh) = (height, width);
    let (pw, ph) = if rw * height > rh * width {
        (width, (rh * width / rw) & !1)
    } else {
        ((rw * height / rh) & !1, height)
    };
    let (pw, ph) = (pw.max(2), ph.max(2));
    (((width - pw) / 2) & !1, ((height - ph) / 2) & !1, pw, ph)
}

/// Writes `src` rotated and mirrored by `t` into `dst` as a packed NV12
/// frame of the same size and returns a view of it. Nearest-neighbour
/// sampling; bars are video-range black.
pub fn transform_into<'a>(src: &Nv12View<'_>, t: Transform, dst: &'a mut Vec<u8>) -> Nv12View<'a> {
    let (w, h) = (src.width, src.height);
    dst.clear();
    dst.resize(w * h, 16);
    dst.resize(w * h * 3 / 2, 128);
    let (x0, y0, pw, ph) = transformed_area(w, h, t);
    let (luma, chroma) = dst.split_at_mut(w * h);

    // Nearest source index along an axis of `len_src` samples for each of
    // `len_out` display positions, optionally counted from the far end.
    let along = |len_out: usize, len_src: usize, flip: bool| -> Vec<usize> {
        (0..len_out)
            .map(|i| {
                let v = ((2 * i + 1) * len_src / (2 * len_out)).min(len_src - 1);
                if flip { len_src - 1 - v } else { v }
            })
            .collect()
    };
    // A display column walks along a source row (even turns) or a source
    // column (odd turns); display rows take the other axis.
    let m = t.mirror;
    let (sideways, cols, rows) = match t.quarter_turns {
        0 => (false, along(pw, w, m), along(ph, h, false)),
        1 => (true, along(pw, h, !m), along(ph, w, false)),
        2 => (false, along(pw, w, !m), along(ph, h, true)),
        _ => (true, along(pw, h, m), along(ph, w, true)),
    };
    // Split each source offset into a column part and a row part.
    let (col_y, row_y): (Vec<usize>, Vec<usize>) = if sideways {
        (
            cols.iter().map(|&sy| sy * src.y_stride).collect(),
            rows.clone(),
        )
    } else {
        (
            cols.clone(),
            rows.iter().map(|&sy| sy * src.y_stride).collect(),
        )
    };
    let (col_uv, row_uv): (Vec<usize>, Vec<usize>) = if sideways {
        (
            cols.iter()
                .step_by(2)
                .map(|&sy| sy / 2 * src.uv_stride)
                .collect(),
            rows.iter().step_by(2).map(|&sx| sx & !1).collect(),
        )
    } else {
        (
            cols.iter().step_by(2).map(|&sx| sx & !1).collect(),
            rows.iter()
                .step_by(2)
                .map(|&sy| sy / 2 * src.uv_stride)
                .collect(),
        )
    };

    for (y, &row) in row_y.iter().enumerate() {
        let out = &mut luma[(y0 + y) * w + x0..][..pw];
        for (o, &col) in out.iter_mut().zip(&col_y) {
            *o = src.y[row + col];
        }
    }
    for (cy, &row) in row_uv.iter().enumerate() {
        let out = &mut chroma[(y0 / 2 + cy) * w + x0..][..pw];
        for (pair, &col) in out.as_chunks_mut::<2>().0.iter_mut().zip(&col_uv) {
            pair[0] = src.uv[row + col];
            pair[1] = src.uv[row + col + 1];
        }
    }
    Nv12View {
        width: w,
        height: h,
        y: luma,
        y_stride: w,
        uv: chroma,
        uv_stride: w,
    }
}

pub fn fill_placeholder(fmt: OutputFormat, dst: &mut [u8], dw: usize, dh: usize) {
    const Y: u8 = 32;
    const C: u8 = 128;
    let need = fmt.frame_bytes(dw, dh);
    if dst.len() < need {
        return;
    }
    match fmt {
        OutputFormat::Nv12 | OutputFormat::I420 => {
            dst[..dw * dh].fill(Y);
            dst[dw * dh..need].fill(C);
        }
        OutputFormat::Yuy2 => {
            for quad in dst[..need].as_chunks_mut::<4>().0 {
                quad.copy_from_slice(&[Y, C, Y, C]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x2 NV12 frame with distinct luma values and one UV pair per 2x2 block.
    fn sample() -> Vec<u8> {
        vec![
            10, 11, 12, 13, //
            20, 21, 22, 23, //
            100, 200, 101, 201, // UV for the two 2x2 blocks
        ]
    }

    #[test]
    fn packed_view_rejects_bad_sizes() {
        let buf = sample();
        assert!(Nv12View::packed(&buf, 4, 2).is_some());
        assert!(Nv12View::packed(&buf, 3, 2).is_none());
        assert!(Nv12View::packed(&buf, 4, 4).is_none());
        assert!(Nv12View::packed(&buf[..10], 4, 2).is_none());
    }

    #[test]
    fn padded_stride_and_height_are_respected() {
        // 4x2 picture stored with stride 6 and 4 allocated luma rows.
        let mut buf = vec![0u8; 6 * 4 + 6];
        buf[..4].copy_from_slice(&[10, 11, 12, 13]);
        buf[6..10].copy_from_slice(&[20, 21, 22, 23]);
        buf[24..28].copy_from_slice(&[100, 200, 101, 201]);
        let view = Nv12View::with_stride(&buf, 4, 2, 6, 4).unwrap();
        let mut out = vec![0u8; OutputFormat::Nv12.frame_bytes(4, 2)];
        convert_scaled(&view, OutputFormat::Nv12, &mut out, 4, 2);
        assert_eq!(out, sample());
    }

    #[test]
    fn same_size_nv12_is_identity() {
        let buf = sample();
        let view = Nv12View::packed(&buf, 4, 2).unwrap();
        let mut out = vec![0u8; OutputFormat::Nv12.frame_bytes(4, 2)];
        convert_scaled(&view, OutputFormat::Nv12, &mut out, 4, 2);
        assert_eq!(out, buf);
    }

    #[test]
    fn i420_splits_planes() {
        let buf = sample();
        let view = Nv12View::packed(&buf, 4, 2).unwrap();
        let mut out = vec![0u8; OutputFormat::I420.frame_bytes(4, 2)];
        convert_scaled(&view, OutputFormat::I420, &mut out, 4, 2);
        assert_eq!(
            out,
            vec![10, 11, 12, 13, 20, 21, 22, 23, 100, 101, 200, 201]
        );
    }

    #[test]
    fn yuy2_interleaves() {
        let buf = sample();
        let view = Nv12View::packed(&buf, 4, 2).unwrap();
        let mut out = vec![0u8; OutputFormat::Yuy2.frame_bytes(4, 2)];
        convert_scaled(&view, OutputFormat::Yuy2, &mut out, 4, 2);
        assert_eq!(
            out,
            vec![
                10, 100, 11, 200, 12, 101, 13, 201, 20, 100, 21, 200, 22, 101, 23, 201
            ]
        );
    }

    #[test]
    fn downscale_picks_nearest_samples() {
        let buf = sample();
        let view = Nv12View::packed(&buf, 4, 2).unwrap();
        let mut out = vec![0u8; OutputFormat::Nv12.frame_bytes(2, 2)];
        convert_scaled(&view, OutputFormat::Nv12, &mut out, 2, 2);
        assert_eq!(out, vec![10, 12, 20, 22, 100, 200]);
    }

    #[test]
    fn upscale_stays_in_bounds() {
        let buf = sample();
        let view = Nv12View::packed(&buf, 4, 2).unwrap();
        for fmt in OutputFormat::ALL {
            let mut out = vec![0u8; fmt.frame_bytes(8, 6)];
            convert_scaled(&view, fmt, &mut out, 8, 6);
            assert!(out.iter().all(|&b| b != 0));
        }
    }

    #[test]
    fn placeholder_is_neutral() {
        for fmt in OutputFormat::ALL {
            let mut out = vec![0u8; fmt.frame_bytes(4, 2)];
            fill_placeholder(fmt, &mut out, 4, 2);
            assert!(out.iter().all(|&b| b == 32 || b == 128));
        }
    }

    #[test]
    fn crop_columns_selects_the_middle() {
        let buf = sample();
        let view = Nv12View::packed(&buf, 4, 2).unwrap();
        let right = view.crop_columns(2, 2).unwrap();
        let mut out = vec![0u8; OutputFormat::Nv12.frame_bytes(2, 2)];
        convert_scaled(&right, OutputFormat::Nv12, &mut out, 2, 2);
        assert_eq!(out, vec![12, 13, 22, 23, 101, 201]);
        assert!(view.crop_columns(1, 2).is_none());
        assert!(view.crop_columns(2, 4).is_none());
    }

    #[test]
    fn portrait_crop_is_centred_9_by_16() {
        // 32x16 frame: column index in luma, chroma marks the pair index.
        let (w, h) = (32, 16);
        let mut buf = vec![0u8; w * h * 3 / 2];
        for row in 0..h {
            for x in 0..w {
                buf[row * w + x] = x as u8;
            }
        }
        let view = Nv12View::packed(&buf, w, h).unwrap();
        let portrait = view.crop_to_portrait().unwrap();
        assert_eq!((portrait.width, portrait.height), (8, 16));
        let mut out = vec![0u8; OutputFormat::Nv12.frame_bytes(8, 16)];
        convert_scaled(&portrait, OutputFormat::Nv12, &mut out, 8, 16);
        assert_eq!(&out[..8], &[12, 13, 14, 15, 16, 17, 18, 19]);
        // Already narrow frames are left alone.
        let narrow = Nv12View::packed(&buf[..8 * 16 * 3 / 2], 8, 16).unwrap();
        assert_eq!(narrow.crop_to_portrait().unwrap().width, 8);
    }

    /// Luma rows of a packed frame.
    fn luma_rows(view: &Nv12View<'_>) -> Vec<Vec<u8>> {
        (0..view.height).map(|y| view.y_row(y).to_vec()).collect()
    }

    #[test]
    fn identity_and_half_turn_keep_every_pixel() {
        let buf = sample();
        let src = Nv12View::packed(&buf, 4, 2).unwrap();
        let mut out = Vec::new();
        let same = transform_into(&src, Transform::default(), &mut out);
        assert_eq!(luma_rows(&same), [[10, 11, 12, 13], [20, 21, 22, 23]]);

        let mut out = Vec::new();
        let flipped = transform_into(&src, Transform::new(180, false), &mut out);
        assert_eq!(luma_rows(&flipped), [[23, 22, 21, 20], [13, 12, 11, 10]]);
        // The UV pairs of the two blocks swap places too.
        assert_eq!(&flipped.uv[..4], &[101, 201, 100, 200]);
    }

    #[test]
    fn mirror_reverses_columns() {
        let buf = sample();
        let src = Nv12View::packed(&buf, 4, 2).unwrap();
        let mut out = Vec::new();
        let mirrored = transform_into(&src, Transform::new(0, true), &mut out);
        assert_eq!(luma_rows(&mirrored), [[13, 12, 11, 10], [23, 22, 21, 20]]);
    }

    #[test]
    fn quarter_turn_keeps_the_size_and_adds_bars() {
        // 8x4 frame whose luma value is its column number.
        let (w, h) = (8, 4);
        let mut buf: Vec<u8> = (0..h).flat_map(|_| 0..w as u8).collect();
        buf.resize(w * h * 3 / 2, 128);
        let src = Nv12View::packed(&buf, w, h).unwrap();
        assert_eq!(
            transformed_area(w, h, Transform::new(90, false)),
            (2, 0, 2, 4)
        );

        let mut out = Vec::new();
        let turned = transform_into(&src, Transform::new(90, false), &mut out);
        assert_eq!((turned.width, turned.height), (w, h));
        let rows = luma_rows(&turned);
        // Black bars left and right of the 2-wide picture.
        assert!(rows.iter().all(|r| r[..2] == [16, 16] && r[4..] == [16; 4]));
        // Turned clockwise, the source's left edge is now at the top.
        assert!(rows[0][2] < rows[3][2]);
    }

    #[test]
    fn transform_steps_wrap_around() {
        let t = Transform::default();
        assert_eq!(t.clockwise().clockwise().clockwise().clockwise(), t);
        assert_eq!(t.counter_clockwise().degrees(), 270);
        assert!(t.clockwise().is_sideways());
        assert_eq!(Transform::new(450, true), Transform::new(90, true));
    }
}
