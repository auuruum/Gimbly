//! CoreVideo buffers and clocks shared by the CMIO sink client and camera
//! extension. IOSurface-backed buffers can cross the CMIO process boundary.

use gimbly_vcam_shm::{Nv12View, OutputFormat, PreferredFormat, pixel};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::*;
use objc2_core_video::*;
use std::ptr::{NonNull, null, null_mut};

pub const DEVICE_ID: &str = "DB0EF305-39CE-4595-BC22-CFB05436C5D7";
pub const NV12: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange;

pub const SIZES: &[(u32, u32)] = &[
    (1920, 1080),
    (1280, 720),
    (3840, 2160),
    (1080, 1920),
    (720, 1280),
];

pub fn max_fps(index: usize) -> u32 {
    if SIZES.get(index) == Some(&(3840, 2160)) {
        30
    } else {
        60
    }
}

pub fn duration(fps: u32) -> CMTime {
    // SAFETY: all callers pass a validated positive frame rate.
    unsafe { CMTime::new(1, fps as i32) }
}

pub fn format_description(w: u32, h: u32) -> Result<CFRetained<CMFormatDescription>, String> {
    let mut raw = null();
    // SAFETY: valid out-pointer for the new description; default allocator.
    let status = unsafe {
        CMVideoFormatDescriptionCreate(
            None,
            NV12,
            w as i32,
            h as i32,
            None,
            NonNull::from(&mut raw),
        )
    };
    owned(status, raw.cast_mut(), "video format")
}

pub fn owned<T: objc2_core_foundation::Type>(
    status: i32,
    ptr: *mut T,
    name: &str,
) -> Result<CFRetained<T>, String> {
    let ptr = NonNull::new(ptr)
        .filter(|_| status == 0)
        .ok_or_else(|| format!("Could not obtain {name} (OSStatus {status})"))?;
    // SAFETY: the successful Create or Copy API transferred this reference.
    Ok(unsafe { CFRetained::from_raw(ptr) })
}

pub struct Pool {
    size: PreferredFormat,
    pool: CFRetained<CVPixelBufferPool>,
    limit: CFRetained<CFDictionary<CFString, CFNumber>>,
    packed: Vec<u8>,
    scaled: Vec<u8>,
}

impl Pool {
    pub fn new(size: PreferredFormat) -> Result<Self, String> {
        // SAFETY: CoreVideo attribute keys and their documented value types.
        let pool = unsafe {
            let empty = CFDictionary::<CFString, CFType>::empty();
            let format = CFNumber::new_i32(NV12 as i32);
            let width = CFNumber::new_i32(size.width as i32);
            let height = CFNumber::new_i32(size.height as i32);
            let options = CFDictionary::<CFString, CFType>::from_slices(
                &[
                    kCVPixelBufferPixelFormatTypeKey,
                    kCVPixelBufferWidthKey,
                    kCVPixelBufferHeightKey,
                    kCVPixelBufferIOSurfacePropertiesKey,
                ],
                &[
                    format.as_ref(),
                    width.as_ref(),
                    height.as_ref(),
                    empty.as_ref(),
                ],
            );
            let mut raw = null_mut();
            let status = CVPixelBufferPool::create(
                None,
                None,
                Some(options.as_opaque()),
                NonNull::from(&mut raw),
            );
            owned(status, raw, "pixel buffer pool")?
        };
        // SAFETY: CoreVideo's allocation threshold is a CFNumber.
        let limit = unsafe {
            CFDictionary::from_slices(
                &[kCVPixelBufferPoolAllocationThresholdKey],
                &[&*CFNumber::new_i32(8)],
            )
        };
        let bytes = size.width as usize * size.height as usize * 3 / 2;
        Ok(Self {
            size,
            pool,
            limit,
            packed: vec![0; bytes],
            scaled: Vec::new(),
        })
    }

    pub fn sample(
        &mut self,
        frame: Option<&Nv12View<'_>>,
        full_range: bool,
    ) -> Option<(CFRetained<CMSampleBuffer>, u64)> {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        if let Some(frame) = frame {
            fit(frame, &mut self.packed, &mut self.scaled, w, h);
            if full_range {
                let (y, uv) = self.packed.split_at_mut(w * h);
                for value in y {
                    *value = (16 + (u16::from(*value) * 219 + 127) / 255) as u8;
                }
                for value in uv {
                    *value = (16 + (u16::from(*value) * 224 + 127) / 255) as u8;
                }
            }
        } else {
            pixel::fill_placeholder(OutputFormat::Nv12, &mut self.packed, w, h);
        }
        // SAFETY: a live pool and a correctly typed auxiliary dictionary.
        let buffer = unsafe {
            let mut raw = null_mut();
            let status = CVPixelBufferPool::create_pixel_buffer_with_aux_attributes(
                None,
                &self.pool,
                Some(self.limit.as_opaque()),
                NonNull::from(&mut raw),
            );
            owned(status, raw, "pixel buffer").ok()?
        };
        write_buffer(&buffer, &self.packed, w, h)?;
        // SAFETY: use CoreMedia's host clock for the sample and CMIO event.
        unsafe {
            let mut description = null();
            let status = CMVideoFormatDescriptionCreateForImageBuffer(
                None,
                &buffer,
                NonNull::from(&mut description),
            );
            let description = owned(status, description.cast_mut(), "sample format").ok()?;
            let pts = CMClock::host_time_clock().time();
            let host_ns = pts
                .convert_scale(1_000_000_000, CMTimeRoundingMethod::Default)
                .value;
            let timing = CMSampleTimingInfo {
                duration: duration(self.size.fps),
                presentationTimeStamp: pts,
                decodeTimeStamp: kCMTimeInvalid,
            };
            let mut sample = null_mut();
            let status = CMSampleBuffer::create_ready_with_image_buffer(
                None,
                &buffer,
                &description,
                NonNull::from(&timing),
                NonNull::from(&mut sample),
            );
            Some((
                owned(status, sample, "video sample").ok()?,
                u64::try_from(host_ns).ok()?,
            ))
        }
    }
}

fn fit(src: &Nv12View<'_>, dst: &mut [u8], scratch: &mut Vec<u8>, w: usize, h: usize) {
    if src.width * h == src.height * w {
        pixel::convert_scaled(src, OutputFormat::Nv12, dst, w, h);
        return;
    }
    let sw = ((src.width * h / src.height).min(w) & !1).max(2);
    let sh = ((src.height * w / src.width).min(h) & !1).max(2);
    let x = ((w - sw) / 2) & !1;
    let y = ((h - sh) / 2) & !1;
    scratch.resize(sw * sh * 3 / 2, 0);
    pixel::convert_scaled(src, OutputFormat::Nv12, scratch, sw, sh);
    let (y_src, uv_src) = scratch.split_at(sw * sh);
    let (y_dst, uv_dst) = dst.split_at_mut(w * h);
    y_dst.fill(16);
    uv_dst.fill(128);
    for (row, data) in y_src.chunks_exact(sw).enumerate() {
        let start = (y + row) * w + x;
        y_dst[start..start + sw].copy_from_slice(data);
    }
    for (row, data) in uv_src.chunks_exact(sw).enumerate() {
        let start = (y / 2 + row) * w + x;
        uv_dst[start..start + sw].copy_from_slice(data);
    }
}

fn write_buffer(buffer: &CVPixelBuffer, bytes: &[u8], w: usize, h: usize) -> Option<()> {
    let lock = CVPixelBufferLockFlags::empty();
    // SAFETY: exclusive write to a new buffer from this pool, before it is
    // wrapped in a sample or shared with clients.
    if unsafe { CVPixelBufferLockBaseAddress(buffer, lock) } != 0 {
        return None;
    }
    let result = (|| {
        if CVPixelBufferGetPlaneCount(buffer) != 2 {
            return None;
        }
        let (y, uv) = bytes.split_at(w * h);
        for (plane, src) in [y, uv].into_iter().enumerate() {
            let ptr = CVPixelBufferGetBaseAddressOfPlane(buffer, plane).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane);
            if ptr.is_null()
                || stride < w
                || stride > 16_384
                || CVPixelBufferGetHeightOfPlane(buffer, plane) < src.len() / w
            {
                return None;
            }
            for (row, src) in src.chunks_exact(w).enumerate() {
                // SAFETY: locked plane, bounded rows, nonoverlapping slices.
                unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), ptr.add(row * stride), w) };
            }
        }
        Some(())
    })();
    // SAFETY: balances this function's successful lock.
    unsafe { CVPixelBufferUnlockBaseAddress(buffer, lock) };
    result
}
