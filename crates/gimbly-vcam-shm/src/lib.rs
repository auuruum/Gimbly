//! Frame transport between the Gimbly app (producer) and the virtual
//! camera driver (consumer).
//!
//! The producer owns a named shared-memory region that holds a small header
//! and [`SLOT_COUNT`] NV12 frame slots. Each slot is guarded by a sequence
//! counter (a seqlock): the producer makes the counter odd while it writes and
//! even when the slot is complete, and the consumer discards a copy whose
//! counter changed while it was reading. The producer never waits for
//! consumers, so a stalled video app cannot stall the camera pipeline.
//!
//! The layout logic in this module is platform independent; mapping the
//! region into a process is done by the platform module (currently Windows).

pub mod pixel;

#[cfg(windows)]
mod windows_config;
#[cfg(windows)]
mod windows_map;
#[cfg(windows)]
pub use windows_config::{read_preferred_format, write_preferred_format};
#[cfg(windows)]
pub use windows_map::{SharedMemory, now_ms};

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

pub use pixel::{Nv12View, OutputFormat, Transform};

/// `"OBTV"` in little-endian byte order.
pub const MAGIC: u32 = u32::from_le_bytes(*b"OBTV");
/// Bumped whenever the header or slot layout changes.
pub const VERSION: u32 = 2;
/// Largest side of a frame the transport carries.
pub const MAX_DIMENSION: usize = 3840;
/// Largest frame area (4K UHD). Larger captures are downscaled by the
/// producer before publishing.
pub const MAX_PIXELS: usize = 3840 * 2160;
pub const SLOT_COUNT: usize = 3;
pub const SLOT_BYTES: usize = MAX_PIXELS * 3 / 2;
/// A consumer treats the producer as gone when no frame arrived for this long.
pub const STALE_AFTER_MS: u64 = 1000;
/// Name of the Windows file mapping. `Local\` keeps it inside the user's
/// session. The suffix follows [`VERSION`] so old and new builds never share
/// a mapping.
pub const MAPPING_NAME: &str = "Local\\GimblyVirtualCamera.v2";

/// Format the virtual camera offers first, chosen in Gimbly's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreferredFormat {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl PreferredFormat {
    /// Accepts only sane values, so a damaged setting cannot break consumers.
    pub fn validated(width: u32, height: u32, fps: u32) -> Option<Self> {
        let ok = (2..=MAX_DIMENSION as u32).contains(&width)
            && (2..=MAX_DIMENSION as u32).contains(&height)
            && (width as usize) * (height as usize) <= MAX_PIXELS
            && width.is_multiple_of(2)
            && height.is_multiple_of(2)
            && (1..=120).contains(&fps);
        ok.then_some(Self { width, height, fps })
    }
}

/// Device name applications show for the virtual camera.
pub const FRIENDLY_NAME: &str = "Gimbly Virtual Camera";
/// COM class ID of the Windows DirectShow source filter (generated for this
/// project).
pub const FILTER_CLSID: u128 = 0xD172DFA9_7D74_48C3_9F18_7DE6D2955D71;
/// File name of the Windows virtual camera DLL, shipped next to the app.
pub const FILTER_DLL_NAME: &str = "gimbly_vcam_dshow.dll";

/// Frame slots start at this offset so they stay page aligned.
pub const DATA_OFFSET: usize = 4096;
pub const REGION_BYTES: usize = DATA_OFFSET + SLOT_COUNT * SLOT_BYTES;

#[repr(C)]
pub struct Header {
    magic: AtomicU32,
    version: AtomicU32,
    slot_count: AtomicU32,
    slot_bytes: AtomicU32,
    /// Producer clock (milliseconds) at the last publish.
    heartbeat_ms: AtomicU64,
    /// `frame_number << 8 | slot_index` of the newest complete frame.
    latest: AtomicU64,
    slots: [SlotHeader; SLOT_COUNT],
}

#[repr(C)]
struct SlotHeader {
    seq: AtomicU64,
    width: AtomicU32,
    height: AtomicU32,
}

const _: () = assert!(std::mem::size_of::<Header>() <= DATA_OFFSET);

/// Metadata of a frame copied out of the region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub width: usize,
    pub height: usize,
    pub frame_number: u64,
}

/// A mapped region of at least [`REGION_BYTES`] bytes.
///
/// The region is shared with other processes, so all header fields are
/// atomics. Frame bytes are plain memory protected by the slot seqlock.
pub struct Region {
    base: *mut u8,
}

// SAFETY: the region is only accessed through atomics and seqlock-validated
// copies; the pointer itself can move between threads.
unsafe impl Send for Region {}
unsafe impl Sync for Region {}

impl Region {
    /// # Safety
    /// `base` must be valid for reads (and writes, if [`Region::publish`] or
    /// [`Region::init`] is used) of [`REGION_BYTES`] bytes, aligned to 8 bytes,
    /// and stay mapped for the lifetime of the returned value.
    pub unsafe fn from_raw(base: *mut u8) -> Self {
        Self { base }
    }

    fn header(&self) -> &Header {
        // SAFETY: guaranteed by `from_raw`; Header only contains atomics.
        unsafe { &*(self.base as *const Header) }
    }

    fn slot_ptr(&self, slot: usize) -> *mut u8 {
        debug_assert!(slot < SLOT_COUNT);
        // SAFETY: in bounds of the region by construction.
        unsafe { self.base.add(DATA_OFFSET + slot * SLOT_BYTES) }
    }

    /// Prepares a freshly created region. Only the producer calls this.
    pub fn init(&self) {
        let h = self.header();
        h.version.store(VERSION, Ordering::Relaxed);
        h.slot_count.store(SLOT_COUNT as u32, Ordering::Relaxed);
        h.slot_bytes.store(SLOT_BYTES as u32, Ordering::Relaxed);
        h.heartbeat_ms.store(0, Ordering::Relaxed);
        h.latest.store(0, Ordering::Relaxed);
        for slot in &h.slots {
            slot.seq.store(0, Ordering::Relaxed);
        }
        h.magic.store(MAGIC, Ordering::Release);
    }

    /// True when the header was written by a compatible producer.
    pub fn is_compatible(&self) -> bool {
        let h = self.header();
        h.magic.load(Ordering::Acquire) == MAGIC
            && h.version.load(Ordering::Relaxed) == VERSION
            && h.slot_count.load(Ordering::Relaxed) == SLOT_COUNT as u32
            && h.slot_bytes.load(Ordering::Relaxed) == SLOT_BYTES as u32
    }

    /// Writes `frame` into the slot that is not the newest one and makes it
    /// the newest. Frames beyond [`MAX_DIMENSION`] or [`MAX_PIXELS`] are
    /// scaled down to fit, keeping the aspect ratio.
    pub fn publish(&self, frame: &Nv12View<'_>, now_ms: u64) -> u64 {
        let h = self.header();
        let latest = h.latest.load(Ordering::Relaxed);
        let slot = ((latest & 0xff) as usize + 1) % SLOT_COUNT;
        let frame_number = (latest >> 8) + 1;
        let (width, height) = fit_within(frame.width, frame.height);

        let header = &h.slots[slot];
        let seq = header.seq.load(Ordering::Relaxed);
        header.seq.store(seq.wrapping_add(1) | 1, Ordering::Relaxed);
        fence(Ordering::Release);

        // SAFETY: the slot holds SLOT_BYTES bytes and width * height is at
        // most MAX_PIXELS, so the NV12 frame fits as well.
        let dst =
            unsafe { std::slice::from_raw_parts_mut(self.slot_ptr(slot), width * height * 3 / 2) };
        pixel::convert_scaled(frame, OutputFormat::Nv12, dst, width, height);
        header.width.store(width as u32, Ordering::Relaxed);
        header.height.store(height as u32, Ordering::Relaxed);

        header
            .seq
            .store((seq.wrapping_add(1) | 1).wrapping_add(1), Ordering::Release);
        h.latest
            .store(frame_number << 8 | slot as u64, Ordering::Release);
        h.heartbeat_ms.store(now_ms, Ordering::Release);
        frame_number
    }

    /// Marks the producer as gone so consumers switch to their placeholder
    /// immediately instead of waiting for [`STALE_AFTER_MS`].
    pub fn mark_stopped(&self) {
        self.header().heartbeat_ms.store(0, Ordering::Release);
    }

    /// Copies the newest frame into `dst` (resized as needed). Returns `None`
    /// when there is no compatible producer, it is stale, or the slot was
    /// overwritten during the copy.
    pub fn read_latest(&self, dst: &mut Vec<u8>, now_ms: u64) -> Option<FrameInfo> {
        if !self.is_compatible() {
            return None;
        }
        let h = self.header();
        let heartbeat = h.heartbeat_ms.load(Ordering::Acquire);
        if heartbeat == 0 || now_ms.saturating_sub(heartbeat) > STALE_AFTER_MS {
            return None;
        }
        let latest = h.latest.load(Ordering::Acquire);
        let frame_number = latest >> 8;
        if frame_number == 0 {
            return None;
        }
        let slot = (latest & 0xff) as usize;
        if slot >= SLOT_COUNT {
            return None;
        }
        let header = &h.slots[slot];
        let seq_before = header.seq.load(Ordering::Acquire);
        if seq_before & 1 == 1 {
            return None;
        }
        let width = header.width.load(Ordering::Relaxed) as usize;
        let height = header.height.load(Ordering::Relaxed) as usize;
        if width == 0
            || height == 0
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || width * height > MAX_PIXELS
        {
            return None;
        }
        let len = width * height * 3 / 2;
        dst.resize(len, 0);
        // SAFETY: the slot is SLOT_BYTES long and len <= SLOT_BYTES. A torn
        // copy is detected by the sequence check below and discarded.
        unsafe { std::ptr::copy_nonoverlapping(self.slot_ptr(slot), dst.as_mut_ptr(), len) };
        fence(Ordering::Acquire);
        if header.seq.load(Ordering::Relaxed) != seq_before {
            return None;
        }
        Some(FrameInfo {
            width,
            height,
            frame_number,
        })
    }
}

/// Largest even size within the transport limits that keeps the aspect ratio.
pub fn fit_within(width: usize, height: usize) -> (usize, usize) {
    if width <= MAX_DIMENSION && height <= MAX_DIMENSION && width * height <= MAX_PIXELS {
        return (width & !1, height & !1);
    }
    let (w, h) = (width as f64, height as f64);
    let scale = (MAX_DIMENSION as f64 / w)
        .min(MAX_DIMENSION as f64 / h)
        .min((MAX_PIXELS as f64 / (w * h)).sqrt());
    let w = ((w * scale) as usize).min(MAX_DIMENSION) & !1;
    let h = ((h * scale) as usize).min(MAX_DIMENSION) & !1;
    (w.max(2), h.max(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region_buffer() -> Vec<u64> {
        vec![0u64; REGION_BYTES / 8]
    }

    fn solid_nv12(width: usize, height: usize, y: u8, u: u8, v: u8) -> Vec<u8> {
        let mut buf = vec![y; width * height * 3 / 2];
        for pair in buf[width * height..].as_chunks_mut::<2>().0 {
            pair[0] = u;
            pair[1] = v;
        }
        buf
    }

    #[test]
    fn empty_region_has_no_frame() {
        let mut buf = region_buffer();
        let region = unsafe { Region::from_raw(buf.as_mut_ptr().cast()) };
        let mut out = Vec::new();
        assert!(!region.is_compatible());
        assert_eq!(region.read_latest(&mut out, 10), None);
        region.init();
        assert!(region.is_compatible());
        assert_eq!(region.read_latest(&mut out, 10), None);
    }

    #[test]
    fn publish_then_read_round_trips() {
        let mut buf = region_buffer();
        let region = unsafe { Region::from_raw(buf.as_mut_ptr().cast()) };
        region.init();
        let (w, h) = (64, 32);
        let frame = solid_nv12(w, h, 100, 90, 200);
        let view = Nv12View::packed(&frame, w, h).unwrap();
        assert_eq!(region.publish(&view, 1_000), 1);
        assert_eq!(region.publish(&view, 1_010), 2);

        let mut out = Vec::new();
        let info = region.read_latest(&mut out, 1_020).unwrap();
        assert_eq!(
            info,
            FrameInfo {
                width: w,
                height: h,
                frame_number: 2
            }
        );
        assert_eq!(out, frame);
    }

    #[test]
    fn stale_or_stopped_producer_yields_nothing() {
        let mut buf = region_buffer();
        let region = unsafe { Region::from_raw(buf.as_mut_ptr().cast()) };
        region.init();
        let frame = solid_nv12(16, 16, 1, 2, 3);
        region.publish(&Nv12View::packed(&frame, 16, 16).unwrap(), 5_000);
        let mut out = Vec::new();
        assert!(
            region
                .read_latest(&mut out, 5_000 + STALE_AFTER_MS)
                .is_some()
        );
        assert!(
            region
                .read_latest(&mut out, 5_001 + STALE_AFTER_MS)
                .is_none()
        );
        region.mark_stopped();
        assert!(region.read_latest(&mut out, 5_000).is_none());
    }

    #[test]
    fn torn_slot_is_rejected() {
        let mut buf = region_buffer();
        let region = unsafe { Region::from_raw(buf.as_mut_ptr().cast()) };
        region.init();
        let frame = solid_nv12(16, 16, 1, 2, 3);
        region.publish(&Nv12View::packed(&frame, 16, 16).unwrap(), 100);
        // Simulate a producer that is mid-write on the newest slot.
        let slot = (region.header().latest.load(Ordering::Relaxed) & 0xff) as usize;
        region.header().slots[slot]
            .seq
            .fetch_add(1, Ordering::Relaxed);
        let mut out = Vec::new();
        assert!(region.read_latest(&mut out, 100).is_none());
    }

    #[test]
    fn large_frames_are_fitted() {
        assert_eq!(fit_within(1920, 1080), (1920, 1080));
        assert_eq!(fit_within(3840, 2160), (3840, 2160));
        assert_eq!(fit_within(1215, 2160), (1214, 2160));
        assert_eq!(fit_within(1281, 721), (1280, 720));
        for (w, h) in [(4000, 3000), (7680, 4320), (5000, 400)] {
            let (fw, fh) = fit_within(w, h);
            assert!(fw <= MAX_DIMENSION && fh <= MAX_DIMENSION && fw * fh <= MAX_PIXELS);
            assert!(fw % 2 == 0 && fh % 2 == 0);
        }
    }

    #[test]
    fn preferred_format_is_validated() {
        assert!(PreferredFormat::validated(1920, 1080, 30).is_some());
        assert!(PreferredFormat::validated(1080, 1920, 60).is_some());
        assert!(PreferredFormat::validated(3840, 2160, 30).is_some());
        assert!(PreferredFormat::validated(7680, 4320, 30).is_none());
        assert!(PreferredFormat::validated(1919, 1080, 30).is_none());
        assert!(PreferredFormat::validated(1920, 1080, 0).is_none());
    }
}
