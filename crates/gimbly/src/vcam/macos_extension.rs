//! The sandboxed CMIO service has a source for video apps and one sink for
//! Gimbly. It keeps only the latest sample and runs workers only while a
//! client has started a stream.

use crate::media::{self, DEVICE_ID, NV12, Pool, SIZES};
use gimbly_vcam_shm::Nv12View;
use gimbly_vcam_shm::{FRIENDLY_NAME, PreferredFormat};
use objc2::rc::{Retained, Weak, autoreleasepool};
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_core_foundation::{CFDictionary, CFRetained};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeRoundingMethod};
use objc2_core_media_io::*;
use objc2_core_video::*;
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSRunLoop, NSSet, NSString, NSUUID,
};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const APP_ID: &str = "io.github.auuruum.gimbly";
const SOURCE_ID: &str = "AE4DC59A-E39E-44CA-B930-6C6131DD04C0";
const SINK_ID: &str = "CB8E8E4D-E69F-4C66-BF6E-41BAB49E90F4";

define_class!(
    // SAFETY: Foundation NSObject subclasses implementing the CMIO
    // protocols. The provider and device return only immutable metadata.
    #[unsafe(super(NSObject))]
    struct Provider;

    unsafe impl NSObjectProtocol for Provider {}

    unsafe impl CMIOExtensionProviderSource for Provider {
        #[unsafe(method(connectClient:error:))]
        unsafe fn connect(&self, _client: &CMIOExtensionClient, _error: *mut *mut NSError) -> Bool {
            Bool::YES
        }

        #[unsafe(method(disconnectClient:))]
        unsafe fn disconnect(&self, _client: &CMIOExtensionClient) {}

        #[unsafe(method_id(availableProperties))]
        unsafe fn properties(&self) -> Retained<NSSet<CMIOExtensionProperty>> {
            NSSet::from_slice(&[unsafe { CMIOExtensionPropertyProviderManufacturer }])
        }

        #[unsafe(method_id(providerPropertiesForProperties:error:))]
        unsafe fn get(
            &self,
            requested: &NSSet<CMIOExtensionProperty>,
            _error: *mut *mut NSError,
        ) -> Retained<CMIOExtensionProviderProperties> {
            // SAFETY: CMIO property keys and metadata belong to this provider.
            unsafe {
                let props = CMIOExtensionProviderProperties::new();
                if requested.containsObject(CMIOExtensionPropertyProviderManufacturer) {
                    props.setManufacturer(Some(&NSString::from_str("Gimbly")));
                }
                props
            }
        }

        #[unsafe(method(setProviderProperties:error:))]
        unsafe fn set(
            &self,
            _props: &CMIOExtensionProviderProperties,
            _error: *mut *mut NSError,
        ) -> Bool {
            Bool::YES
        }
    }
);

define_class!(
    // SAFETY: the device source provides immutable model metadata.
    #[unsafe(super(NSObject))]
    struct Device;

    unsafe impl NSObjectProtocol for Device {}

    unsafe impl CMIOExtensionDeviceSource for Device {
        #[unsafe(method_id(availableProperties))]
        unsafe fn properties(&self) -> Retained<NSSet<CMIOExtensionProperty>> {
            NSSet::from_slice(&[unsafe { CMIOExtensionPropertyDeviceModel }])
        }

        #[unsafe(method_id(devicePropertiesForProperties:error:))]
        unsafe fn get(
            &self,
            requested: &NSSet<CMIOExtensionProperty>,
            _error: *mut *mut NSError,
        ) -> Retained<CMIOExtensionDeviceProperties> {
            // SAFETY: static CMIO key and ordinary NSString property.
            unsafe {
                let props = CMIOExtensionDeviceProperties::new();
                if requested.containsObject(CMIOExtensionPropertyDeviceModel) {
                    props.setModel(Some(&NSString::from_str(FRIENDLY_NAME)));
                }
                props
            }
        }

        #[unsafe(method(setDeviceProperties:error:))]
        unsafe fn set(
            &self,
            _props: &CMIOExtensionDeviceProperties,
            _error: *mut *mut NSError,
        ) -> Bool {
            Bool::YES
        }
    }
);

/// CMIO's immutable sample stays retained while the source reads its pixels.
struct Sample(CFRetained<CMSampleBuffer>);
// SAFETY: after the sink hands over a sample, nobody mutates it; Core
// Foundation retains/releases are thread-safe and pixel reads use a lock.
unsafe impl Send for Sample {}

type Latest = Arc<Mutex<Option<(Instant, Sample)>>>;

/// CMIO calls these methods from media delivery queues in addition to the
/// control queue. Retain the stream and sink client for the worker.
struct Delivery {
    stream: Retained<CMIOExtensionStream>,
    client: Option<Retained<CMIOExtensionClient>>,
}
// SAFETY: the worker uses only CMIO's thread-safe send/consume methods;
// source/client lifetime is protected by the retained references.
unsafe impl Send for Delivery {}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Format {
    index: usize,
    fps: u32,
}

struct State {
    format: Arc<Mutex<Format>>,
    stream: Weak<CMIOExtensionStream>,
    sink_client: Option<Retained<CMIOExtensionClient>>,
    active: Option<Active>,
}

struct Active {
    count: usize,
    running: Arc<AtomicBool>,
    stop: Sender<()>,
}

struct StreamData {
    sink: bool,
    formats: Retained<NSArray<CMIOExtensionStreamFormat>>,
    latest: Latest,
    state: Mutex<State>,
}

define_class!(
    // SAFETY: state shared across the CMIO callbacks is synchronized.
    #[unsafe(super(NSObject))]
    #[ivars = StreamData]
    struct Stream;

    unsafe impl NSObjectProtocol for Stream {}

    unsafe impl CMIOExtensionStreamSource for Stream {
        #[unsafe(method_id(formats))]
        unsafe fn formats(&self) -> Retained<NSArray<CMIOExtensionStreamFormat>> {
            self.ivars().formats.clone()
        }

        #[unsafe(method_id(availableProperties))]
        unsafe fn properties(&self) -> Retained<NSSet<CMIOExtensionProperty>> {
            // SAFETY: these are constant framework property keys.
            let mut props = unsafe {
                vec![
                    CMIOExtensionPropertyStreamActiveFormatIndex,
                    CMIOExtensionPropertyStreamFrameDuration,
                ]
            };
            if self.ivars().sink {
                // SAFETY: queue properties apply only to the sink.
                props.extend(unsafe {
                    [
                        CMIOExtensionPropertyStreamSinkBufferQueueSize,
                        CMIOExtensionPropertyStreamSinkBuffersRequiredForStartup,
                    ]
                });
            }
            NSSet::from_slice(&props)
        }

        #[unsafe(method_id(streamPropertiesForProperties:error:))]
        unsafe fn get(
            &self,
            requested: &NSSet<CMIOExtensionProperty>,
            _error: *mut *mut NSError,
        ) -> Retained<CMIOExtensionStreamProperties> {
            let state = self.ivars().state.lock().unwrap_or_else(|e| e.into_inner());
            let format = *state.format.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: all properties match their declared Foundation types.
            unsafe {
                let props = CMIOExtensionStreamProperties::new();
                if requested.containsObject(CMIOExtensionPropertyStreamActiveFormatIndex) {
                    props.setActiveFormatIndex(Some(&NSNumber::new_usize(format.index)));
                }
                if requested.containsObject(CMIOExtensionPropertyStreamFrameDuration)
                    && let Some(time) = media::duration(format.fps).as_dictionary(None)
                {
                    // SAFETY: CFDictionary and NSDictionary are toll-free bridged.
                    let dictionary = &*CFRetained::as_ptr(&time).as_ptr().cast::<NSDictionary>();
                    props.setFrameDuration(Some(dictionary));
                }
                if self.ivars().sink {
                    if requested.containsObject(CMIOExtensionPropertyStreamSinkBufferQueueSize) {
                        props.setSinkBufferQueueSize(Some(&NSNumber::new_u32(3)));
                    }
                    if requested
                        .containsObject(CMIOExtensionPropertyStreamSinkBuffersRequiredForStartup)
                    {
                        props.setSinkBuffersRequiredForStartup(Some(&NSNumber::new_u32(1)));
                    }
                }
                props
            }
        }

        #[unsafe(method(setStreamProperties:error:))]
        unsafe fn set(
            &self,
            props: &CMIOExtensionStreamProperties,
            _error: *mut *mut NSError,
        ) -> Bool {
            let state = self.ivars().state.lock().unwrap_or_else(|e| e.into_inner());
            let mut format = state.format.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: typed properties are read from a live CMIO object.
            let index = unsafe { props.activeFormatIndex() }.map_or(format.index, |i| i.as_usize());
            if index >= SIZES.len() {
                return Bool::NO;
            }
            let max = media::max_fps(index);
            let fps = if let Some(dictionary) = unsafe { props.frameDuration() } {
                // SAFETY: CoreMedia serializes the frame duration to a
                // toll-free-bridged dictionary.
                let duration = unsafe {
                    CMTime::from_dictionary(Some(
                        &*Retained::as_ptr(&dictionary).cast::<CFDictionary>(),
                    ))
                };
                (1..=max).find(|fps| {
                    // SAFETY: CoreMedia compares the typed times by value.
                    unsafe { duration.compare(media::duration(*fps)) == 0 }
                })
            } else {
                Some(format.fps.min(max))
            };
            let Some(fps) = fps else { return Bool::NO };
            *format = Format { index, fps };
            Bool::YES
        }

        #[unsafe(method(authorizedToStartStreamForClient:))]
        unsafe fn authorize(&self, client: &CMIOExtensionClient) -> Bool {
            if !self.ivars().sink {
                return Bool::YES;
            }
            let mut state = self.ivars().state.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: CMIO supplies this live client, including its signing ID.
            if state.active.is_some()
                || !unsafe { client.signingID() }.is_some_and(|id| id.to_string() == APP_ID)
            {
                return Bool::NO;
            }
            state.sink_client = Some(client.retain());
            Bool::YES
        }

        #[unsafe(method(startStreamAndReturnError:))]
        unsafe fn start(&self, _error: *mut *mut NSError) -> Bool {
            Bool::new(self.start_worker())
        }

        #[unsafe(method(stopStreamAndReturnError:))]
        unsafe fn stop(&self, _error: *mut *mut NSError) -> Bool {
            self.stop_worker();
            Bool::YES
        }
    }
);

impl Stream {
    fn start_worker(&self) -> bool {
        let data = self.ivars();
        let mut state = data.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = &mut state.active {
            active.count = active.count.saturating_add(1);
            return true;
        }
        let Some(stream) = state.stream.load() else {
            return false;
        };
        let delivery = Delivery {
            stream,
            client: state.sink_client.take(),
        };
        if data.sink && delivery.client.is_none() {
            return false;
        }
        let format = state.format.clone();
        let latest = data.latest.clone();
        let running = Arc::new(AtomicBool::new(true));
        let worker_running = running.clone();
        let (stop, stopped) = mpsc::channel();
        let result = std::thread::Builder::new()
            .name("gimbly-cmio-stream".into())
            .spawn(move || {
                let mut pool: Option<(Format, Pool)> = None;
                let consuming = Arc::new(AtomicBool::new(false));
                let mut next = Instant::now();
                while worker_running.load(Ordering::Acquire) {
                    let config = *format.lock().unwrap_or_else(|e| e.into_inner());
                    autoreleasepool(|_| {
                        if delivery.client.is_some() {
                            delivery.consume(&latest, &worker_running, &consuming);
                        } else {
                            delivery.publish(&latest, &mut pool, config);
                        }
                    });
                    next += Duration::from_secs_f64(1.0 / f64::from(config.fps));
                    if Instant::now() > next {
                        next =
                            Instant::now() + Duration::from_secs_f64(1.0 / f64::from(config.fps));
                    }
                    if !matches!(
                        stopped.recv_timeout(next.saturating_duration_since(Instant::now())),
                        Err(RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
            });
        if result.is_err() {
            return false;
        }
        state.active = Some(Active {
            count: 1,
            running,
            stop,
        });
        true
    }

    fn stop_worker(&self) {
        let data = self.ivars();
        let mut state = data.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = &mut state.active {
            if active.count > 1 {
                active.count -= 1;
            } else if let Some(active) = state.active.take() {
                active.running.store(false, Ordering::Release);
                let _ = active.stop.send(());
                if data.sink {
                    *data.latest.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
            }
        }
    }
}

impl Delivery {
    fn consume(&self, latest: &Latest, running: &Arc<AtomicBool>, consuming: &Arc<AtomicBool>) {
        let Some(client) = &self.client else { return };
        if consuming.swap(true, Ordering::AcqRel) {
            return;
        }
        let latest = latest.clone();
        let running = running.clone();
        let consuming = consuming.clone();
        let stream = self.stream.clone();
        let done = block2::RcBlock::new(
            move |sample: *mut CMSampleBuffer,
                  sequence: u64,
                  _flags: CMIOExtensionStreamDiscontinuityFlags,
                  _more: Bool,
                  error: *mut NSError| {
                if error.is_null()
                    && let Some(sample) = NonNull::new(sample)
                {
                    // SAFETY: CoreMedia passes a live sample to the completion
                    // block. Keep our own reference for the source stream.
                    let sample = unsafe { CFRetained::retain(sample) };
                    // SAFETY: the app timestamps these samples with the host clock.
                    let pts = unsafe {
                        sample
                            .presentation_time_stamp()
                            .convert_scale(1_000_000_000, CMTimeRoundingMethod::Default)
                    };
                    {
                        let mut slot = latest.lock().unwrap_or_else(|e| e.into_inner());
                        if running.load(Ordering::Acquire) {
                            *slot = Some((Instant::now(), Sample(sample)));
                        }
                    }
                    if let Ok(ns) = u64::try_from(pts.value) {
                        // SAFETY: notify the same sink about the sequence it supplied.
                        unsafe {
                            let output = CMIOExtensionScheduledOutput::scheduledOutputWithSequenceNumber_hostTimeInNanoseconds(sequence, ns);
                            stream.notifyScheduledOutputChanged(&output);
                        }
                    }
                }
                consuming.store(false, Ordering::Release);
            },
        );
        // SAFETY: client owns this sink stream. CMIO copies the completion
        // block. Its captured stream is retained and its state is synchronized.
        unsafe {
            self.stream
                .consumeSampleBufferFromClient_completionHandler(client, &done)
        };
    }

    fn publish(&self, latest: &Latest, pool: &mut Option<(Format, Pool)>, config: Format) {
        let Some(&(width, height)) = SIZES.get(config.index) else {
            return;
        };
        if pool.as_ref().is_none_or(|(current, _)| *current != config) {
            let Some(format) = PreferredFormat::validated(width, height, config.fps) else {
                return;
            };
            *pool = Pool::new(format).ok().map(|pool| (config, pool));
        }
        let Some((_, pool)) = pool else { return };
        let sample = {
            let slot = latest.lock().unwrap_or_else(|e| e.into_inner());
            slot.as_ref()
                .filter(|(received, _)| received.elapsed() < Duration::from_secs(1))
                .map(|(_, sample)| sample.0.clone())
        };
        let output = sample
            .as_ref()
            .and_then(|sample| with_sample(sample, |view| pool.sample(Some(view), false)).flatten())
            .or_else(|| pool.sample(None, false));
        if let Some((sample, ns)) = output {
            // SAFETY: this is the source stream; timestamps are host time.
            unsafe {
                self.stream
                    .sendSampleBuffer_discontinuity_hostTimeInNanoseconds(
                        &sample,
                        CMIOExtensionStreamDiscontinuityFlags::empty(),
                        ns,
                    )
            };
        }
    }
}

fn uuid(text: &str) -> Result<Retained<NSUUID>, String> {
    NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(text))
        .ok_or("Invalid Gimbly stream UUID".into())
}

fn add_stream(
    device: &CMIOExtensionDevice,
    sink: bool,
    latest: Latest,
) -> Result<Retained<Stream>, String> {
    let mut formats = Vec::new();
    for (index, &(w, h)) in SIZES.iter().enumerate() {
        let description = media::format_description(w, h)?;
        // SAFETY: the valid durations form a continuous bounded interval.
        formats.push(unsafe {
            CMIOExtensionStreamFormat::streamFormatWithFormatDescription_maxFrameDuration_minFrameDuration_validFrameDurations(
                &description, media::duration(1), media::duration(media::max_fps(index)), None,
            )
        });
    }
    let source = Stream::alloc().set_ivars(StreamData {
        sink,
        latest,
        formats: NSArray::from_retained_slice(&formats),
        state: Mutex::new(State {
            format: Arc::new(Mutex::new(Format { index: 0, fps: 30 })),
            stream: Weak::default(),
            sink_client: None,
            active: None,
        }),
    });
    // SAFETY: initialize the superclass and register the correct direction
    // and UUID with this CMIO extension device.
    unsafe {
        let source: Retained<Stream> = msg_send![super(source), init];
        let id = uuid(if sink { SINK_ID } else { SOURCE_ID })?;
        let stream =
            CMIOExtensionStream::streamWithLocalizedName_streamID_direction_clockType_source(
                &NSString::from_str(if sink { "Gimbly Input" } else { "Gimbly Video" }),
                &id,
                if sink {
                    CMIOExtensionStreamDirection::Sink
                } else {
                    CMIOExtensionStreamDirection::Source
                },
                CMIOExtensionStreamClockType::HostTime,
                ProtocolObject::from_ref(&*source),
            );
        source
            .ivars()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stream = Weak::new(&stream);
        device.addStream_error(&stream).map_err(|e| e.to_string())?;
        Ok(source)
    }
}

pub fn run() -> Result<(), String> {
    autoreleasepool(|_| {
        // SAFETY: starts Core Media I/O using retained, fully initialized
        // provider and device sources and keeps them alive on the run loop.
        unsafe {
            let provider_source: Retained<Provider> = msg_send![Provider::alloc(), init];
            let device_source: Retained<Device> = msg_send![Device::alloc(), init];
            let provider = CMIOExtensionProvider::providerWithSource_clientQueue(
                ProtocolObject::from_ref(&*provider_source),
                None,
            );
            let id = uuid(DEVICE_ID)?;
            let device =
                CMIOExtensionDevice::deviceWithLocalizedName_deviceID_legacyDeviceID_source(
                    &NSString::from_str(FRIENDLY_NAME),
                    &id,
                    Some(&NSString::from_str(DEVICE_ID)),
                    ProtocolObject::from_ref(&*device_source),
                );
            let latest = Latest::default();
            let _source = add_stream(&device, false, latest.clone())?;
            let _sink = add_stream(&device, true, latest)?;
            provider
                .addDevice_error(&device)
                .map_err(|e| e.to_string())?;
            CMIOExtensionProvider::startServiceWithProvider(&provider);
            NSRunLoop::currentRunLoop().run();
            Ok(())
        }
    })
}

pub fn with_sample<R>(sample: &CMSampleBuffer, f: impl FnOnce(&Nv12View<'_>) -> R) -> Option<R> {
    // SAFETY: the live sample retains its image buffer.
    let buffer = unsafe { sample.image_buffer() }?;
    let (w, h) = (
        CVPixelBufferGetWidth(&buffer),
        CVPixelBufferGetHeight(&buffer),
    );
    if !SIZES.contains(&(u32::try_from(w).ok()?, u32::try_from(h).ok()?))
        || CVPixelBufferGetPixelFormatType(&buffer) != NV12
        || CVPixelBufferGetPlaneCount(&buffer) != 2
    {
        return None;
    }
    let lock = CVPixelBufferLockFlags::ReadOnly;
    // SAFETY: matching read-only lock/unlock of the retained image buffer.
    if unsafe { CVPixelBufferLockBaseAddress(&buffer, lock) } != 0 {
        return None;
    }
    let result = (|| {
        let plane = |index, height| {
            let stride = CVPixelBufferGetBytesPerRowOfPlane(&buffer, index);
            let ptr = CVPixelBufferGetBaseAddressOfPlane(&buffer, index).cast::<u8>();
            if stride < w
                || stride > 16_384
                || ptr.is_null()
                || CVPixelBufferGetHeightOfPlane(&buffer, index) < height
            {
                return None;
            }
            // SAFETY: plane stays locked throughout the callback, and its
            // geometry is checked against the slice length.
            Some((
                unsafe { std::slice::from_raw_parts(ptr, stride * height) },
                stride,
            ))
        };
        let (y, y_stride) = plane(0, h)?;
        let (uv, uv_stride) = plane(1, h / 2)?;
        Some(f(&Nv12View {
            width: w,
            height: h,
            y,
            y_stride,
            uv,
            uv_stride,
        }))
    })();
    // SAFETY: balances this function's successful lock.
    unsafe { CVPixelBufferUnlockBaseAddress(&buffer, lock) };
    result
}
