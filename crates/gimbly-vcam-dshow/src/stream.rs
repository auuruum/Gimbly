//! Delivery thread: paces frames at the negotiated rate, copies the newest
//! frame published by Gimbly (or a neutral placeholder) into a sample and
//! pushes it downstream.

use crate::filter::Shared;
use crate::media::VideoFormat;
use gimbly_vcam_shm::{Nv12View, SharedMemory, now_ms, pixel};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Media::DirectShow::{IMediaSample, IMemAllocator, IMemInputPin, State_Running};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

/// How often to retry opening the shared frame region while Gimbly is not
/// running.
const REOPEN_INTERVAL: Duration = Duration::from_secs(1);

pub struct Handle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Handle {
    pub fn stop_and_join(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// Moves COM pointers into the delivery thread.
struct SendPtrs(IMemAllocator, IMemInputPin);
// SAFETY: allocator and input pin are free-threaded DirectShow objects.
unsafe impl Send for SendPtrs {}

pub fn start(
    shared: Arc<Shared>,
    allocator: IMemAllocator,
    input: IMemInputPin,
    format: VideoFormat,
) -> Handle {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let ptrs = SendPtrs(allocator, input);
    let thread = std::thread::Builder::new()
        .name("gimbly-vcam".into())
        .spawn(move || {
            let ptrs = ptrs;
            // SAFETY: balanced below on the same thread.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            deliver(&shared, &ptrs.0, &ptrs.1, format, &flag);
            drop(ptrs);
            if com {
                // SAFETY: matches the successful CoInitializeEx.
                unsafe { CoUninitialize() };
            }
        })
        .ok();
    Handle { stop, thread }
}

struct Source {
    shm: Option<SharedMemory>,
    last_attempt: Option<Instant>,
    frame: Vec<u8>,
}

impl Source {
    /// Writes the newest published frame into `dst`, or a placeholder.
    fn fill(&mut self, format: VideoFormat, dst: &mut [u8]) {
        if self.shm.is_none()
            && self
                .last_attempt
                .is_none_or(|t| t.elapsed() >= REOPEN_INTERVAL)
        {
            self.last_attempt = Some(Instant::now());
            self.shm = SharedMemory::open().ok();
        }
        let frame = self
            .shm
            .as_ref()
            .and_then(|shm| shm.region().read_latest(&mut self.frame, now_ms()));
        match frame.and_then(|info| Nv12View::packed(&self.frame, info.width, info.height)) {
            Some(view) => {
                pixel::convert_scaled(&view, format.pixel, dst, format.width, format.height)
            }
            None => pixel::fill_placeholder(format.pixel, dst, format.width, format.height),
        }
    }
}

fn deliver(
    shared: &Shared,
    allocator: &IMemAllocator,
    input: &IMemInputPin,
    format: VideoFormat,
    stop: &AtomicBool,
) {
    let period = Duration::from_nanos(1_000_000_000 / format.fps as u64);
    let duration = format.frame_duration();
    let bytes = format.frame_bytes();
    let mut source = Source {
        shm: None,
        last_attempt: None,
        frame: Vec::new(),
    };
    let mut next_due = Instant::now();
    let mut frame_number: i64 = 0;
    let mut first = true;

    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        if next_due > now {
            std::thread::sleep(next_due - now);
        }
        next_due += period;
        if next_due + period < Instant::now() {
            // We fell behind (e.g. the consumer blocked); do not burst.
            next_due = Instant::now() + period;
        }

        let (state, start_time, clock) = {
            let inner = shared.lock();
            (inner.state, inner.start_time, inner.clock.clone())
        };
        if state != State_Running {
            // A paused live source delivers nothing.
            first = true;
            continue;
        }

        let mut sample: Option<IMediaSample> = None;
        // SAFETY: GetBuffer blocks until a buffer is free; Decommit on Stop
        // makes it fail so the loop can exit.
        if unsafe { allocator.GetBuffer(&mut sample, None, None, 0) }.is_err() {
            continue;
        }
        let Some(sample) = sample else { continue };
        // SAFETY: the sample buffer is valid for GetSize bytes until released.
        let filled = unsafe {
            match (sample.GetPointer(), usize::try_from(sample.GetSize())) {
                (Ok(ptr), Ok(size)) if !ptr.is_null() && size >= bytes => {
                    source.fill(format, std::slice::from_raw_parts_mut(ptr, bytes));
                    true
                }
                _ => false,
            }
        };
        if !filled {
            continue;
        }

        let start = match clock.and_then(|c| unsafe { c.GetTime() }.ok()) {
            Some(now) => now - start_time,
            None => frame_number * duration,
        };
        let end = start + duration;
        // SAFETY: plain sample setup and delivery on valid interfaces.
        unsafe {
            let _ = sample.SetActualDataLength(bytes as i32);
            let _ = sample.SetSyncPoint(true);
            let _ = sample.SetTime(Some(&start), Some(&end));
            let _ = sample.SetDiscontinuity(first);
            let _ = input.Receive(&sample);
        }
        first = false;
        frame_number += 1;
    }
}
