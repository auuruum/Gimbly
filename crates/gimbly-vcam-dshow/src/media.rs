//! DirectShow media types offered by the virtual camera output pin.

use gimbly_vcam_shm::{MAX_DIMENSION, MAX_PIXELS, OutputFormat, PreferredFormat};
use std::mem::{ManuallyDrop, size_of};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::BITMAPINFOHEADER;
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, FORMAT_VideoInfo, MEDIASUBTYPE_I420, MEDIASUBTYPE_NV12, MEDIASUBTYPE_YUY2,
    MEDIATYPE_Video, VIDEOINFOHEADER,
};
use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};
use windows_core::GUID;

pub const DEFAULT_FPS: u32 = 30;
pub const MAX_FPS: u32 = 60;
/// 100-ns units per second (DirectShow reference time).
pub const UNITS: i64 = 10_000_000;
/// Standard sizes offered next to the preferred one, largest first.
const LANDSCAPE_SIZES: [(usize, usize); 4] = [(3840, 2160), (1920, 1080), (1280, 720), (640, 360)];
const PORTRAIT_SIZES: [(usize, usize); 3] = [(1080, 1920), (720, 1280), (360, 640)];
const PIXELS: [OutputFormat; 3] = [OutputFormat::Nv12, OutputFormat::Yuy2, OutputFormat::I420];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoFormat {
    pub pixel: OutputFormat,
    pub width: usize,
    pub height: usize,
    pub fps: u32,
}

impl Default for VideoFormat {
    fn default() -> Self {
        Self {
            pixel: OutputFormat::Nv12,
            width: 1920,
            height: 1080,
            fps: DEFAULT_FPS,
        }
    }
}

impl VideoFormat {
    /// Every format the pin offers: the size chosen in Gimbly's settings
    /// first, then the other standard sizes of the same orientation, each in
    /// NV12, YUY2 and I420, all at the preferred frame rate.
    pub fn catalog(preferred: Option<PreferredFormat>) -> Vec<VideoFormat> {
        let (first, fps) = match preferred {
            Some(p) => ((p.width as usize, p.height as usize), p.fps.min(MAX_FPS)),
            None => ((1920, 1080), DEFAULT_FPS),
        };
        let standard: &[(usize, usize)] = if first.1 > first.0 {
            &PORTRAIT_SIZES
        } else {
            &LANDSCAPE_SIZES
        };
        let sizes = std::iter::once(first).chain(standard.iter().copied().filter(|&s| s != first));
        sizes
            .flat_map(|(width, height)| {
                PIXELS.iter().map(move |&pixel| VideoFormat {
                    pixel,
                    width,
                    height,
                    fps,
                })
            })
            .collect()
    }

    pub fn frame_bytes(&self) -> usize {
        self.pixel.frame_bytes(self.width, self.height)
    }

    pub fn frame_duration(&self) -> i64 {
        UNITS / self.fps as i64
    }

    /// Same pixel format and size; frame rate may differ.
    pub fn same_picture(&self, other: &VideoFormat) -> bool {
        self.pixel == other.pixel && self.width == other.width && self.height == other.height
    }
}

fn subtype(pixel: OutputFormat) -> GUID {
    match pixel {
        OutputFormat::Nv12 => MEDIASUBTYPE_NV12,
        OutputFormat::Yuy2 => MEDIASUBTYPE_YUY2,
        OutputFormat::I420 => MEDIASUBTYPE_I420,
    }
}

fn pixel_from_subtype(guid: &GUID) -> Option<OutputFormat> {
    PIXELS.into_iter().find(|&p| subtype(p) == *guid)
}

pub fn video_info(f: VideoFormat) -> VIDEOINFOHEADER {
    let bytes = f.frame_bytes() as u32;
    let full = RECT {
        left: 0,
        top: 0,
        right: f.width as i32,
        bottom: f.height as i32,
    };
    VIDEOINFOHEADER {
        rcSource: full,
        rcTarget: full,
        dwBitRate: bytes.saturating_mul(8).saturating_mul(f.fps),
        dwBitErrorRate: 0,
        AvgTimePerFrame: f.frame_duration(),
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: f.width as i32,
            biHeight: f.height as i32,
            biPlanes: 1,
            biBitCount: f.pixel.bits_per_pixel(),
            // YUV FOURCCs equal data1 of their subtype GUID.
            biCompression: subtype(f.pixel).data1,
            biSizeImage: bytes,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
    }
}

/// Builds a media type whose format block is allocated with CoTaskMemAlloc.
/// Release it with [`free_contents`] (or hand it to a caller that does).
pub fn new_media_type(f: VideoFormat) -> AM_MEDIA_TYPE {
    let info = video_info(f);
    // SAFETY: allocation of a plain-old-data block that we fully initialise.
    let block = unsafe { CoTaskMemAlloc(size_of::<VIDEOINFOHEADER>()) } as *mut VIDEOINFOHEADER;
    let (pb_format, cb_format) = if block.is_null() {
        (std::ptr::null_mut(), 0)
    } else {
        // SAFETY: freshly allocated and large enough.
        unsafe { block.write(info) };
        (block as *mut u8, size_of::<VIDEOINFOHEADER>() as u32)
    };
    AM_MEDIA_TYPE {
        majortype: MEDIATYPE_Video,
        subtype: subtype(f.pixel),
        bFixedSizeSamples: true.into(),
        bTemporalCompression: false.into(),
        lSampleSize: f.frame_bytes() as u32,
        formattype: FORMAT_VideoInfo,
        pUnk: ManuallyDrop::new(None),
        cbFormat: cb_format,
        pbFormat: pb_format,
    }
}

/// Allocates an `AM_MEDIA_TYPE` (and its format block) for a caller that
/// frees it with `DeleteMediaType`.
pub fn alloc_media_type(f: VideoFormat) -> *mut AM_MEDIA_TYPE {
    // SAFETY: the allocation is checked and fully written before use.
    unsafe {
        let mt = CoTaskMemAlloc(size_of::<AM_MEDIA_TYPE>()) as *mut AM_MEDIA_TYPE;
        if !mt.is_null() {
            mt.write(new_media_type(f));
        }
        mt
    }
}

/// Frees the format block and interface held by `mt` (like `FreeMediaType`).
///
/// # Safety
/// `mt.pbFormat` must be null or allocated with CoTaskMemAlloc.
pub unsafe fn free_contents(mt: &mut AM_MEDIA_TYPE) {
    if !mt.pbFormat.is_null() {
        // SAFETY: guaranteed by the caller.
        unsafe { CoTaskMemFree(Some(mt.pbFormat as *const _)) };
        mt.pbFormat = std::ptr::null_mut();
        mt.cbFormat = 0;
    }
    // SAFETY: releases the optional interface exactly once.
    unsafe { ManuallyDrop::drop(&mut mt.pUnk) };
    mt.pUnk = ManuallyDrop::new(None);
}

/// Parses a fully specified video media type into one of our formats.
///
/// # Safety
/// `mt` must be null or point to a valid `AM_MEDIA_TYPE`.
pub unsafe fn parse(mt: *const AM_MEDIA_TYPE) -> Option<VideoFormat> {
    // SAFETY: guaranteed by the caller.
    let mt = unsafe { mt.as_ref()? };
    if mt.majortype != MEDIATYPE_Video || mt.formattype != FORMAT_VideoInfo {
        return None;
    }
    let pixel = pixel_from_subtype(&mt.subtype)?;
    if mt.pbFormat.is_null() || (mt.cbFormat as usize) < size_of::<VIDEOINFOHEADER>() {
        return None;
    }
    // SAFETY: the block is at least VIDEOINFOHEADER-sized.
    let info = unsafe { std::ptr::read_unaligned(mt.pbFormat as *const VIDEOINFOHEADER) };
    let width = usize::try_from(info.bmiHeader.biWidth).ok()?;
    let height = info.bmiHeader.biHeight.unsigned_abs() as usize;
    let sane = (2..=MAX_DIMENSION).contains(&width)
        && (2..=MAX_DIMENSION).contains(&height)
        && width * height <= MAX_PIXELS
        && width.is_multiple_of(2)
        && height.is_multiple_of(2);
    if !sane {
        return None;
    }
    let fps = if info.AvgTimePerFrame > 0 {
        ((UNITS + info.AvgTimePerFrame / 2) / info.AvgTimePerFrame).clamp(1, MAX_FPS as i64) as u32
    } else {
        DEFAULT_FPS
    };
    Some(VideoFormat {
        pixel,
        width,
        height,
        fps,
    })
}

/// True when a possibly partial media type (GUID_NULL fields act as
/// wildcards, as in `IPin::Connect`) admits `f`.
///
/// # Safety
/// `mt` must point to a valid `AM_MEDIA_TYPE`.
pub unsafe fn admits(mt: &AM_MEDIA_TYPE, f: VideoFormat) -> bool {
    let wild = GUID::zeroed();
    (mt.majortype == wild || mt.majortype == MEDIATYPE_Video)
        && (mt.subtype == wild || mt.subtype == subtype(f.pixel))
        && (mt.formattype == wild || mt.formattype == FORMAT_VideoInfo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_types_round_trip() {
        let portrait = PreferredFormat::validated(1080, 1920, 60);
        for f in VideoFormat::catalog(None)
            .into_iter()
            .chain(VideoFormat::catalog(portrait))
        {
            let mut mt = new_media_type(f);
            assert_eq!(unsafe { parse(&mt) }, Some(f));
            assert_eq!(mt.lSampleSize as usize, f.frame_bytes());
            unsafe { free_contents(&mut mt) };
            assert!(mt.pbFormat.is_null());
        }
    }

    #[test]
    fn catalog_puts_the_preferred_format_first() {
        let default = VideoFormat::catalog(None);
        assert_eq!(default[0], VideoFormat::default());
        assert_eq!(default.len(), 4 * 3);

        let uhd = VideoFormat::catalog(PreferredFormat::validated(3840, 2160, 30));
        assert_eq!((uhd[0].width, uhd[0].height, uhd[0].fps), (3840, 2160, 30));
        assert_eq!(uhd.len(), 4 * 3, "4K is a standard size, not duplicated");

        let fhd60 = VideoFormat::catalog(PreferredFormat::validated(1920, 1080, 60));
        assert!(fhd60.iter().all(|f| f.fps == 60));

        let portrait = VideoFormat::catalog(PreferredFormat::validated(1080, 1920, 30));
        assert!(portrait.iter().all(|f| f.height > f.width));
        assert_eq!((portrait[0].width, portrait[0].height), (1080, 1920));

        let custom = VideoFormat::catalog(PreferredFormat::validated(608, 1080, 30));
        assert_eq!((custom[0].width, custom[0].height), (608, 1080));
        assert_eq!(custom.len(), 4 * 3);
    }

    #[test]
    fn rejects_unusable_sizes_and_types() {
        let mut mt = new_media_type(VideoFormat::default());
        let info = mt.pbFormat as *mut VIDEOINFOHEADER;
        unsafe { (*info).bmiHeader.biWidth = 1001 };
        assert_eq!(unsafe { parse(&mt) }, None);
        unsafe { (*info).bmiHeader.biWidth = 8000 };
        assert_eq!(unsafe { parse(&mt) }, None);
        unsafe { (*info).bmiHeader.biWidth = 1920 };
        mt.subtype = GUID::zeroed();
        assert_eq!(unsafe { parse(&mt) }, None);
        unsafe { free_contents(&mut mt) };
    }

    #[test]
    fn frame_rate_comes_from_avg_time_per_frame() {
        let mut mt = new_media_type(VideoFormat::default());
        let info = mt.pbFormat as *mut VIDEOINFOHEADER;
        unsafe { (*info).AvgTimePerFrame = 666_666 };
        assert_eq!(unsafe { parse(&mt) }.map(|f| f.fps), Some(15));
        unsafe { (*info).AvgTimePerFrame = 0 };
        assert_eq!(unsafe { parse(&mt) }.map(|f| f.fps), Some(DEFAULT_FPS));
        unsafe { free_contents(&mut mt) };
    }

    #[test]
    fn partial_types_act_as_wildcards() {
        let mut mt: AM_MEDIA_TYPE = unsafe { std::mem::zeroed() };
        let nv12 = VideoFormat::default();
        let yuy2 = VideoFormat {
            pixel: OutputFormat::Yuy2,
            ..nv12
        };
        assert!(unsafe { admits(&mt, nv12) });
        mt.majortype = MEDIATYPE_Video;
        mt.subtype = MEDIASUBTYPE_YUY2;
        assert!(unsafe { admits(&mt, yuy2) });
        assert!(!unsafe { admits(&mt, nv12) });
    }
}
