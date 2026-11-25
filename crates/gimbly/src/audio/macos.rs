//! AVFoundation capture. The audio data output is asked for 48 kHz stereo
//! 16-bit interleaved PCM, so samples arrive ready to write.

use super::{Format, Sink, matches};
use dispatch2::DispatchQueue;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::*;
use objc2_core_media::CMSampleBuffer;
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::Duration;

const FORMAT: Format = Format {
    rate: 48_000,
    channels: 2,
};
/// `kAudioFormatLinearPCM` ('lpcm').
const LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");

struct DelegateData {
    samples: SyncSender<Vec<i16>>,
}

define_class!(
    // SAFETY: the delegate is retained until its serial callback queue is
    // drained; its only field is a Send channel.
    #[unsafe(super(NSObject))]
    #[ivars = DelegateData]
    struct AudioDelegate;

    unsafe impl NSObjectProtocol for AudioDelegate {}

    unsafe impl AVCaptureAudioDataOutputSampleBufferDelegate for AudioDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn receive(
            &self,
            _output: &AVCaptureOutput,
            sample: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            if let Some(samples) = copy_samples(sample) {
                let _ = self.ivars().samples.try_send(samples);
            }
        }
    }
);

fn copy_samples(sample: &CMSampleBuffer) -> Option<Vec<i16>> {
    // SAFETY: the callback holds a live sample; at most the reported number
    // of bytes is copied into owned memory of that size.
    unsafe {
        let block = sample.data_buffer()?;
        let len = block.data_length() / 2 * 2;
        if len == 0 || len > 1 << 20 {
            return None;
        }
        let mut samples = vec![0i16; len / 2];
        let dst = NonNull::new(samples.as_mut_ptr())?.cast();
        (block.copy_data_bytes(0, len, dst) == 0).then_some(samples)
    }
}

fn authorize(stop: &AtomicBool) -> Result<(), String> {
    // SAFETY: the completion block is retained by AVFoundation and captures
    // only a Send channel.
    unsafe {
        let audio = AVMediaTypeAudio.ok_or("AVFoundation audio media type is unavailable")?;
        match AVCaptureDevice::authorizationStatusForMediaType(audio) {
            AVAuthorizationStatus::Authorized => return Ok(()),
            AVAuthorizationStatus::NotDetermined => {}
            _ => return Err("Microphone access is disabled; the recording has no sound. Allow Gimbly in System Settings → Privacy & Security → Microphone.".into()),
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let completed = block2::RcBlock::new(move |allowed: objc2::runtime::Bool| {
            let _ = tx.try_send(allowed.as_bool());
        });
        AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &completed);
        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(true) => return Ok(()),
                Ok(false) => return Err("Microphone access was denied".into()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("The microphone permission request was interrupted".into()),
            }
        }
        Ok(())
    }
}

fn find(model: &str) -> Result<Retained<AVCaptureDevice>, String> {
    // SAFETY: enumerates audio devices and reads their names.
    #[allow(deprecated)]
    unsafe {
        let audio = AVMediaTypeAudio.ok_or("AVFoundation audio media type is unavailable")?;
        AVCaptureDevice::devicesWithMediaType(audio)
            .iter()
            .find(|d| matches(&d.localizedName().to_string(), model))
            .ok_or_else(|| "The camera microphone was not found; the recording has no sound".into())
    }
}

pub(super) fn capture(model: &str, stop: &AtomicBool, sink: &mut Sink<'_>) -> Result<(), String> {
    autoreleasepool(|_| {
        authorize(stop)?;
        let device = find(model)?;
        let (tx, rx) = mpsc::sync_channel(64);
        let delegate: Retained<AudioDelegate> = {
            let delegate = AudioDelegate::alloc().set_ivars(DelegateData { samples: tx });
            // SAFETY: initializes the allocated delegate's NSObject superclass.
            unsafe { msg_send![super(delegate), init] }
        };
        let queue = DispatchQueue::new("gimbly.audio", None);
        // SAFETY: session setup, start and stop run on this thread; the
        // delegate and queue outlive every callback.
        let (session, output) = unsafe {
            let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
                .map_err(|e| e.to_string())?;
            let output = AVCaptureAudioDataOutput::new();
            let session = AVCaptureSession::new();
            if !session.canAddInput(&input) || !session.canAddOutput(&output) {
                return Err("AVFoundation rejected the camera microphone".into());
            }
            session.addInput(&input);
            session.addOutput(&output);
            let key = |k: &str| NSString::from_str(k);
            let keys = [
                key("AVFormatIDKey"),
                key("AVSampleRateKey"),
                key("AVNumberOfChannelsKey"),
                key("AVLinearPCMBitDepthKey"),
                key("AVLinearPCMIsFloatKey"),
                key("AVLinearPCMIsBigEndianKey"),
                key("AVLinearPCMIsNonInterleaved"),
            ];
            let values: [Retained<AnyObject>; 7] = [
                NSNumber::new_u32(LINEAR_PCM).into(),
                NSNumber::new_u32(FORMAT.rate).into(),
                NSNumber::new_u32(u32::from(FORMAT.channels)).into(),
                NSNumber::new_u32(16).into(),
                NSNumber::new_bool(false).into(),
                NSNumber::new_bool(false).into(),
                NSNumber::new_bool(false).into(),
            ];
            let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
            let settings =
                NSDictionary::<NSString, AnyObject>::from_retained_objects(&key_refs, &values);
            output.setAudioSettings(Some(&settings));
            output.setSampleBufferDelegate_queue(
                Some(ProtocolObject::from_ref(&*delegate)),
                Some(&queue),
            );
            session.startRunning();
            (session, output)
        };
        let mut result = Ok(());
        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(samples) => {
                    if let Err(e) = sink(FORMAT, &samples) {
                        result = Err(format!("Could not write the sound: {e}"));
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
        }
        // SAFETY: detach before releasing the delegate, then drain its
        // serial queue so no callback is still running.
        unsafe {
            output.setSampleBufferDelegate_queue(None, None);
            session.stopRunning();
        }
        queue.exec_sync(|| {});
        result
    })
}
