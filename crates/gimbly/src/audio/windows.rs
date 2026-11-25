//! WASAPI shared-mode capture. Windows converts the microphone's mix format
//! to 16-bit PCM at the same rate and channel count.

use super::{Format, Sink, matches};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

/// Buffer length asked from WASAPI, in 100 ns units (200 ms).
const BUFFER: i64 = 2_000_000;

struct Com;

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: pairs with the successful CoInitializeEx in `capture`.
        unsafe { CoUninitialize() };
    }
}

struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateEventW and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn find(model: &str) -> windows::core::Result<Option<IMMDevice>> {
    // SAFETY: plain COM enumeration on a thread with COM initialised.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        for i in 0..devices.GetCount()? {
            let device = devices.Item(i)?;
            let name = device
                .OpenPropertyStore(STGM_READ)?
                .GetValue(&PKEY_Device_FriendlyName)?
                .to_string();
            if matches(&name, model) {
                return Ok(Some(device));
            }
        }
        Ok(None)
    }
}

pub(super) fn capture(model: &str, stop: &AtomicBool, sink: &mut Sink<'_>) -> Result<(), String> {
    let error = |e: windows::core::Error| format!("Camera microphone: {e}");
    // SAFETY: initialises COM for this thread; `_com` undoes it.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(error)?;
    let _com = Com;
    let device = find(model)
        .map_err(error)?
        .ok_or("The camera microphone was not found; the recording has no sound")?;
    // SAFETY: WASAPI calls with valid arguments. The mix format pointer is
    // read once and freed; captured buffers are read only between
    // GetBuffer and ReleaseBuffer, for the frame count WASAPI reports.
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(error)?;
        let mix = client.GetMixFormat().map_err(error)?;
        let (rate, channels) = ((*mix).nSamplesPerSec, (*mix).nChannels);
        CoTaskMemFree(Some(mix.cast_const().cast()));
        if rate == 0 || channels == 0 || channels > 8 {
            return Err("The camera microphone reported an unusable format".into());
        }
        let format = Format { rate, channels };
        let block = channels * 2;
        let wanted = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * u32::from(block),
            nBlockAlign: block,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                BUFFER,
                0,
                &wanted,
                None,
            )
            .map_err(error)?;
        let event = Event(CreateEventW(None, false, false, None).map_err(error)?);
        client.SetEventHandle(event.0).map_err(error)?;
        let reader: IAudioCaptureClient = client.GetService().map_err(error)?;
        client.Start().map_err(error)?;
        let mut silence = Vec::new();
        let result = (|| {
            while !stop.load(Ordering::Relaxed) {
                WaitForSingleObject(event.0, 100);
                while reader.GetNextPacketSize().map_err(error)? > 0 {
                    let mut data = std::ptr::null_mut();
                    let (mut frames, mut flags) = (0u32, 0u32);
                    reader
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                        .map_err(error)?;
                    let count = frames as usize * usize::from(channels);
                    let written =
                        if data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                            silence.resize(count, 0);
                            sink(format, &silence)
                        } else {
                            sink(format, std::slice::from_raw_parts(data.cast(), count))
                        };
                    reader.ReleaseBuffer(frames).map_err(error)?;
                    written.map_err(|e| format!("Could not write the sound: {e}"))?;
                }
            }
            Ok(())
        })();
        let _ = client.Stop();
        result
    }
}
