//! The app feeds the signed Core Media I/O extension through its sink stream.
//! The operating system carries IOSurface buffers between the two processes.

use super::macos_media::{self as media, Pool, SIZES, owned};
use crate::camera::{CaptureConfig, YuvRange};
use dispatch2::DispatchQueue;
use gimbly_vcam_shm::{Nv12View, PreferredFormat};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, define_class, msg_send};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_core_media::{
    CMFormatDescription, CMSampleBuffer, CMSimpleQueue, CMVideoFormatDescriptionGetDimensions,
};
use objc2_core_media_io::*;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_system_extensions::*;
use std::cell::RefCell;
use std::mem::size_of;
use std::ptr::{NonNull, null, null_mut};
use std::time::{Duration, Instant};

const EXTENSION_ID: &str = "io.github.auuruum.gimbly.camera";

thread_local! {
    static ACTIVATION: RefCell<Activation> = const { RefCell::new(Activation::Idle) };
    static DELEGATE: RefCell<Option<Retained<ActivationDelegate>>> = const { RefCell::new(None) };
}

enum Activation {
    Idle,
    Pending,
    Finished(String),
}

define_class!(
    // SAFETY: all activation callbacks use the main dispatch queue.
    #[unsafe(super(NSObject))]
    struct ActivationDelegate;

    unsafe impl NSObjectProtocol for ActivationDelegate {}

    unsafe impl OSSystemExtensionRequestDelegate for ActivationDelegate {
        #[unsafe(method(request:actionForReplacingExtension:withExtension:))]
        unsafe fn replace_extension(
            &self,
            _: &OSSystemExtensionRequest,
            _: &OSSystemExtensionProperties,
            _: &OSSystemExtensionProperties,
        ) -> OSSystemExtensionReplacementAction {
            OSSystemExtensionReplacementAction::Replace
        }

        #[unsafe(method(requestNeedsUserApproval:))]
        unsafe fn needs_approval(&self, _: &OSSystemExtensionRequest) {
            ACTIVATION.with_borrow_mut(|state| *state = Activation::Pending);
        }

        #[unsafe(method(request:didFinishWithResult:))]
        unsafe fn activation_finished(
            &self,
            _: &OSSystemExtensionRequest,
            result: OSSystemExtensionRequestResult,
        ) {
            let text = if result == OSSystemExtensionRequestResult::WillCompleteAfterReboot {
                "Restart macOS to finish activating Gimbly Virtual Camera."
            } else {
                "The camera extension is activating. Try again in a moment."
            };
            ACTIVATION.with_borrow_mut(|state| *state = Activation::Finished(text.into()));
        }

        #[unsafe(method(request:didFailWithError:))]
        unsafe fn activation_failed(&self, _: &OSSystemExtensionRequest, error: &NSError) {
            ACTIVATION.with_borrow_mut(|state| {
                *state =
                    Activation::Finished(format!("Camera extension activation failed: {error}"))
            });
        }
    }
);

pub fn register() -> Result<Option<String>, String> {
    if find_sink().is_ok() {
        return Ok(None);
    }
    if let Some(error) = ACTIVATION.with_borrow_mut(|state| match std::mem::replace(state, Activation::Idle) {
        Activation::Idle => None,
        Activation::Pending => {
            *state = Activation::Pending;
            Some("Allow the Gimbly camera extension in System Settings, then turn on Virtual Camera again.".into())
        }
        Activation::Finished(message) => Some(message),
    }) {
        return Err(error);
    }

    // SAFETY: reads the installed bundle path and submits a request on the
    // main queue. The weak delegate is kept alive for the app's lifetime.
    unsafe {
        let bundle = NSBundle::mainBundle().bundlePath().to_string();
        let extension = std::path::Path::new(&bundle).join(format!(
            "Contents/Library/SystemExtensions/{}.systemextension",
            EXTENSION_ID
        ));
        if !bundle.starts_with("/Applications/") || !extension.is_dir() {
            return Err("Install the signed Gimbly.app with its camera extension in /Applications. See the macOS setup in CONTRIBUTING.md.".into());
        }
        let delegate: Retained<ActivationDelegate> = msg_send![ActivationDelegate::alloc(), init];
        let request = OSSystemExtensionRequest::activationRequestForExtension_queue(
            &NSString::from_str(EXTENSION_ID),
            DispatchQueue::main(),
        );
        request.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        DELEGATE.with_borrow_mut(|stored| *stored = Some(delegate));
        ACTIVATION.with_borrow_mut(|state| *state = Activation::Pending);
        OSSystemExtensionManager::sharedManager().submitRequest(&request);
    }
    Err("Camera extension activation requested. Allow it in System Settings, then turn on Virtual Camera again.".into())
}

fn address(selector: u32, scope: u32) -> CMIOObjectPropertyAddress {
    CMIOObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kCMIOObjectPropertyElementMain,
    }
}

fn ids(object: u32, property: CMIOObjectPropertyAddress) -> Result<Vec<u32>, String> {
    let mut size = 0;
    // SAFETY: valid out-pointer, no property qualifier.
    let status = unsafe { CMIOObjectGetPropertyDataSize(object, &property, 0, null(), &mut size) };
    if status != 0 || size > 4096 || !size.is_multiple_of(4) {
        return Err(format!(
            "Could not read the CMIO device list (OSStatus {status})"
        ));
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    let mut ids = vec![0u32; size as usize / 4];
    let mut used = 0;
    // SAFETY: ids holds size bytes, and used is a valid out-pointer.
    let status = unsafe {
        CMIOObjectGetPropertyData(
            object,
            &property,
            0,
            null(),
            size,
            &mut used,
            ids.as_mut_ptr().cast(),
        )
    };
    if status != 0 || used > size || !used.is_multiple_of(4) {
        return Err(format!(
            "Could not read the CMIO device list (OSStatus {status})"
        ));
    }
    ids.truncate(used as usize / 4);
    Ok(ids)
}

fn find_sink() -> Result<(u32, u32), String> {
    let devices = ids(
        kCMIOObjectSystemObject,
        address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal),
    )?;
    for device in devices {
        let uid = property_ref::<CFString>(device, kCMIODevicePropertyDeviceUID);
        if uid.is_ok_and(|uid| uid.to_string().eq_ignore_ascii_case(media::DEVICE_ID)) {
            let streams = ids(
                device,
                address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeOutput),
            )?;
            if let Some(stream) = streams.first() {
                return Ok((device, *stream));
            }
        }
    }
    Err("The Gimbly camera extension is not active".into())
}

/// CMIO returns an owned CF reference for these properties.
fn property_ref<T: objc2_core_foundation::Type>(
    object: u32,
    selector: u32,
) -> Result<CFRetained<T>, String> {
    let mut raw = null_mut::<T>();
    let mut used = 0;
    let address = address(selector, kCMIOObjectPropertyScopeGlobal);
    let size = size_of::<*mut T>() as u32;
    // SAFETY: callers use a CF pointer property matching T.
    let status = unsafe {
        CMIOObjectGetPropertyData(
            object,
            &address,
            0,
            null(),
            size,
            &mut used,
            (&mut raw as *mut *mut T).cast(),
        )
    };
    if used != size {
        return Err(format!("Unexpected CMIO property size (OSStatus {status})"));
    }
    owned(status, raw, "camera property")
}

fn set_property<T>(object: u32, selector: u32, value: &T) -> Result<(), String> {
    let address = address(selector, kCMIOObjectPropertyScopeGlobal);
    // SAFETY: caller supplies the property ABI type and its exact size.
    let status = unsafe {
        CMIOObjectSetPropertyData(
            object,
            &address,
            0,
            null(),
            size_of::<T>() as u32,
            std::ptr::from_ref(value).cast(),
        )
    };
    if status != 0 {
        return Err(format!(
            "Could not configure the camera extension stream (OSStatus {status})"
        ));
    }
    Ok(())
}

pub struct Output {
    device: u32,
    stream: u32,
    queue: CFRetained<CMSimpleQueue>,
    pool: Pool,
    period: Duration,
    next: Instant,
}

impl Output {
    pub fn open(config: CaptureConfig, size: Option<(u32, u32)>) -> Result<Self, String> {
        let (device, stream) = find_sink()?;
        let (w, h) = config.virtual_size(size.ok_or("Camera output size is not available")?);
        let index = SIZES
            .iter()
            .enumerate()
            .filter(|(_, (sw, sh))| (w > h) == (sw > sh))
            .min_by_key(|(_, (sw, sh))| {
                (u64::from(w) * u64::from(h)).abs_diff(u64::from(*sw) * u64::from(*sh))
            })
            .map(|(i, _)| i)
            .ok_or("The virtual camera has no matching size")?;
        let &(w, h) = SIZES.get(index).ok_or("Invalid camera format index")?;
        let fps = config.fps.clamp(1, media::max_fps(index));
        let format = media::format_description(w, h)?;
        let current =
            property_ref::<CMFormatDescription>(stream, kCMIOStreamPropertyFormatDescription)?;
        // SAFETY: current is a video format description returned by CMIO.
        let dimensions = unsafe { CMVideoFormatDescriptionGetDimensions(&current) };
        if dimensions.width != w as i32 || dimensions.height != h as i32 {
            set_property(
                stream,
                kCMIOStreamPropertyFormatDescription,
                &CFRetained::as_ptr(&format),
            )?;
        }
        set_property(stream, kCMIOStreamPropertyFrameRate, &f64::from(fps))?;
        let pool = Pool::new(
            PreferredFormat::validated(w, h, fps).ok_or("Invalid virtual camera format")?,
        )?;
        let mut queue = null_mut();
        // SAFETY: Copy transfers a reference to this queue. The app is its
        // single producer and never blocks waiting for the sink.
        let status = unsafe { CMIOStreamCopyBufferQueue(stream, None, null_mut(), &mut queue) };
        let queue = owned(status, queue, "camera extension queue")?;
        // SAFETY: stream was found on this device, and queue is ready.
        let status = unsafe { CMIODeviceStartStream(device, stream) };
        if status != 0 {
            return Err(format!(
                "Could not start the camera extension (OSStatus {status})"
            ));
        }
        Ok(Self {
            device,
            stream,
            queue,
            pool,
            period: Duration::from_secs_f64(1.0 / f64::from(fps)),
            next: Instant::now(),
        })
    }

    pub fn publish(&mut self, view: &Nv12View<'_>, range: YuvRange) -> Result<(), String> {
        let now = Instant::now();
        if now + Duration::from_millis(1) < self.next {
            return Ok(());
        }
        self.next = (self.next + self.period).max(now);
        // SAFETY: the live queue supports one producer and one consumer.
        if unsafe { self.queue.count() } >= 2 {
            return Ok(());
        }
        if let Some((sample, _)) = self.pool.sample(Some(view), range == YuvRange::Full) {
            let ptr = CFRetained::into_raw(sample);
            // SAFETY: transfer the sample's reference to Core Media I/O.
            let status = unsafe { self.queue.enqueue(ptr.cast()) };
            if status != 0 {
                // SAFETY: failed enqueue leaves ownership with this producer.
                drop(unsafe { CFRetained::from_raw(ptr) });
            }
        }
        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // SAFETY: stop the matching stream, then release any remaining
        // samples whose owning references have not transferred to the sink.
        unsafe {
            CMIODeviceStopStream(self.device, self.stream);
            while let Some(sample) =
                NonNull::new(self.queue.dequeue().cast::<CMSampleBuffer>().cast_mut())
            {
                drop(CFRetained::from_raw(sample));
            }
        }
    }
}
