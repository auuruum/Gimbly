//! The capture output pin and the enumerators it hands out.

use crate::ObjectCount;
use crate::filter::{Connection, PIN_NAME, Shared};
use crate::media::{self, VideoFormat};
use std::mem::{ManuallyDrop, size_of};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{
    E_INVALIDARG, E_NOTIMPL, E_POINTER, E_UNEXPECTED, S_FALSE, S_OK, SIZE,
};
use windows::Win32::Media::DirectShow::*;
use windows::Win32::Media::KernelStreaming::{IKsPropertySet, IKsPropertySet_Impl};
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, AMPROPSETID_Pin, CLSID_MemoryAllocator, FORMAT_VideoInfo, PIN_CATEGORY_CAPTURE,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemAlloc};
use windows_core::{
    Error, GUID, HRESULT, IUnknownImpl, Interface, OutRef, PWSTR, Ref, Result, implement,
};

const KSPROPERTY_SUPPORT_GET: u32 = 1;

#[implement(IPin, IAMStreamConfig, IKsPropertySet)]
pub struct OutputPin {
    shared: Arc<Shared>,
    _count: ObjectCount,
}

impl OutputPin {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            _count: ObjectCount::new(),
        }
    }

    /// Formats to try or enumerate: only the requested one after
    /// `SetFormat`, otherwise everything we offer.
    fn offered(&self) -> Vec<VideoFormat> {
        match self.shared.lock().requested {
            Some(f) => vec![f],
            None => self.shared.catalog.clone(),
        }
    }

    fn current_format(&self) -> VideoFormat {
        let inner = self.shared.lock();
        inner
            .connection
            .as_ref()
            .map(|c| c.format)
            .or(inner.requested)
            .unwrap_or(self.shared.catalog[0])
    }

    /// A parsed format is usable when its picture is one we offer; the frame
    /// rate may be any value the parser accepted.
    fn allowed(&self, format: &VideoFormat) -> bool {
        self.shared.catalog.iter().any(|f| f.same_picture(format))
    }
}

/// Agrees on sample buffers with the downstream input pin: its own allocator
/// if it offers one that fits, otherwise the system memory allocator.
fn decide_allocator(receiver: &IPin, format: VideoFormat) -> Result<(IMemInputPin, IMemAllocator)> {
    let input: IMemInputPin = receiver.cast()?;
    let bytes = format.frame_bytes() as i32;
    let mut request = ALLOCATOR_PROPERTIES {
        cBuffers: 3,
        cbBuffer: bytes,
        cbAlign: 1,
        cbPrefix: 0,
    };
    // SAFETY: plain COM calls on valid interfaces.
    unsafe {
        if let Ok(wanted) = input.GetAllocatorRequirements() {
            request.cBuffers = request.cBuffers.max(wanted.cBuffers);
            request.cbAlign = wanted.cbAlign.max(1);
            request.cbPrefix = wanted.cbPrefix;
        }
        let configure = |allocator: IMemAllocator| -> Result<IMemAllocator> {
            let actual = allocator.SetProperties(&request)?;
            if actual.cbBuffer < bytes {
                return Err(VFW_E_NO_ALLOCATOR.into());
            }
            input.NotifyAllocator(&allocator, false)?;
            Ok(allocator)
        };
        if let Ok(allocator) = input.GetAllocator()
            && let Ok(allocator) = configure(allocator)
        {
            return Ok((input, allocator));
        }
        let allocator: IMemAllocator =
            CoCreateInstance(&CLSID_MemoryAllocator, None, CLSCTX_INPROC_SERVER)?;
        let allocator = configure(allocator)?;
        Ok((input, allocator))
    }
}

fn wide_name() -> [u16; 128] {
    let mut name = [0u16; 128];
    for (dst, src) in name.iter_mut().zip(PIN_NAME.encode_utf16()) {
        *dst = src;
    }
    name
}

impl IPin_Impl for OutputPin_Impl {
    fn Connect(&self, preceivepin: Ref<IPin>, pmt: *const AM_MEDIA_TYPE) -> Result<()> {
        let receiver = preceivepin.ok()?;
        let requested = {
            let inner = self.shared.lock();
            if inner.connection.is_some() {
                return Err(VFW_E_ALREADY_CONNECTED.into());
            }
            if inner.state != State_Stopped {
                return Err(VFW_E_NOT_STOPPED.into());
            }
            inner.requested
        };
        // SAFETY: `pmt` is null or a valid media type from the caller.
        let candidates: Vec<VideoFormat> = match unsafe { media::parse(pmt) } {
            Some(exact) => [exact].into_iter().filter(|f| self.allowed(f)).collect(),
            None => match unsafe { pmt.as_ref() } {
                Some(partial) => self
                    .offered()
                    .into_iter()
                    .filter(|&f| unsafe { media::admits(partial, f) })
                    .collect(),
                None => requested.map_or_else(|| self.shared.catalog.clone(), |f| vec![f]),
            },
        };
        let me: IPin = self.to_interface();
        for format in candidates {
            let mut mt = media::new_media_type(format);
            // The peer may call back into this pin, so no lock is held here.
            // SAFETY: valid interfaces and media type.
            let accepted = unsafe { receiver.ReceiveConnection(&me, &mt) }.is_ok();
            // SAFETY: `mt` was built by new_media_type.
            unsafe { media::free_contents(&mut mt) };
            if !accepted {
                continue;
            }
            match decide_allocator(receiver, format) {
                Ok((input, allocator)) => {
                    self.shared.lock().connection = Some(Connection {
                        peer: receiver.clone(),
                        input,
                        allocator,
                        format,
                    });
                    return Ok(());
                }
                Err(_) => {
                    // SAFETY: undo the peer's half of the connection.
                    let _ = unsafe { receiver.Disconnect() };
                }
            }
        }
        Err(VFW_E_NO_ACCEPTABLE_TYPES.into())
    }

    fn ReceiveConnection(&self, _pconnector: Ref<IPin>, _pmt: *const AM_MEDIA_TYPE) -> Result<()> {
        // Output pins never receive connections.
        Err(E_UNEXPECTED.into())
    }

    fn Disconnect(&self) -> Result<()> {
        let connection = {
            let mut inner = self.shared.lock();
            if inner.state != State_Stopped {
                return Err(VFW_E_NOT_STOPPED.into());
            }
            inner.connection.take()
        };
        match connection {
            Some(c) => {
                // SAFETY: plain COM call.
                let _ = unsafe { c.allocator.Decommit() };
                Ok(())
            }
            None => Err(Error::from_hresult(S_FALSE)),
        }
    }

    fn ConnectedTo(&self) -> Result<IPin> {
        self.shared
            .lock()
            .connection
            .as_ref()
            .map(|c| c.peer.clone())
            .ok_or_else(|| VFW_E_NOT_CONNECTED.into())
    }

    fn ConnectionMediaType(&self, pmt: *mut AM_MEDIA_TYPE) -> Result<()> {
        if pmt.is_null() {
            return Err(E_POINTER.into());
        }
        let format = self.shared.lock().connection.as_ref().map(|c| c.format);
        match format {
            // SAFETY: the caller supplies storage for one AM_MEDIA_TYPE and
            // frees the format block it receives.
            Some(f) => unsafe {
                pmt.write(media::new_media_type(f));
                Ok(())
            },
            None => unsafe {
                pmt.write(std::mem::zeroed());
                Err(VFW_E_NOT_CONNECTED.into())
            },
        }
    }

    fn QueryPinInfo(&self, pinfo: *mut PIN_INFO) -> Result<()> {
        if pinfo.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the caller supplies a PIN_INFO and releases pFilter.
        unsafe {
            pinfo.write(PIN_INFO {
                pFilter: ManuallyDrop::new(self.shared.filter()),
                dir: PINDIR_OUTPUT,
                achName: wide_name(),
            })
        };
        Ok(())
    }

    fn QueryDirection(&self) -> Result<PIN_DIRECTION> {
        Ok(PINDIR_OUTPUT)
    }

    fn QueryId(&self) -> Result<PWSTR> {
        let wide: Vec<u16> = PIN_NAME.encode_utf16().chain(Some(0)).collect();
        // SAFETY: the caller frees the string with CoTaskMemFree.
        unsafe {
            let buffer = CoTaskMemAlloc(wide.len() * 2) as *mut u16;
            if buffer.is_null() {
                return Err(windows::Win32::Foundation::E_OUTOFMEMORY.into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), buffer, wide.len());
            Ok(PWSTR(buffer))
        }
    }

    fn QueryAccept(&self, pmt: *const AM_MEDIA_TYPE) -> HRESULT {
        // SAFETY: `pmt` is null or valid.
        let Some(format) = (unsafe { media::parse(pmt) }).filter(|f| self.allowed(f)) else {
            return S_FALSE;
        };
        match self.shared.lock().connection.as_ref() {
            Some(c) if !c.format.same_picture(&format) => S_FALSE,
            _ => S_OK,
        }
    }

    fn EnumMediaTypes(&self) -> Result<IEnumMediaTypes> {
        Ok(MediaTypeEnum::new(self.offered(), 0).into())
    }

    fn QueryInternalConnections(&self, _appin: OutRef<IPin>, _npin: *mut u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EndOfStream(&self) -> Result<()> {
        Err(E_UNEXPECTED.into())
    }

    fn BeginFlush(&self) -> Result<()> {
        Err(E_UNEXPECTED.into())
    }

    fn EndFlush(&self) -> Result<()> {
        Err(E_UNEXPECTED.into())
    }

    fn NewSegment(&self, _tstart: i64, _tstop: i64, _drate: f64) -> Result<()> {
        Ok(())
    }
}

impl IAMStreamConfig_Impl for OutputPin_Impl {
    fn SetFormat(&self, pmt: *const AM_MEDIA_TYPE) -> Result<()> {
        if pmt.is_null() {
            self.shared.lock().requested = None;
            return Ok(());
        }
        // SAFETY: valid media type from the caller.
        let format = unsafe { media::parse(pmt) }
            .filter(|f| self.allowed(f))
            .ok_or_else(|| Error::from(VFW_E_INVALIDMEDIATYPE))?;
        let reconnect = {
            let mut inner = self.shared.lock();
            let connected = inner.connection.as_ref().map(|c| c.format);
            match connected {
                Some(current) if current == format => return Ok(()),
                Some(_) if inner.state != State_Stopped => return Err(VFW_E_NOT_STOPPED.into()),
                _ => {}
            }
            inner.requested = Some(format);
            connected.is_some()
        };
        if reconnect {
            let graph = self
                .shared
                .graph()
                .ok_or_else(|| Error::from(VFW_E_ALREADY_CONNECTED))?;
            let me: IPin = self.to_interface();
            // SAFETY: asks the graph to reconnect this pin with the new format.
            unsafe { graph.Reconnect(&me)? };
        }
        Ok(())
    }

    fn GetFormat(&self) -> Result<*mut AM_MEDIA_TYPE> {
        let mt = media::alloc_media_type(self.current_format());
        if mt.is_null() {
            return Err(windows::Win32::Foundation::E_OUTOFMEMORY.into());
        }
        Ok(mt)
    }

    fn GetNumberOfCapabilities(&self, picount: *mut i32, pisize: *mut i32) -> Result<()> {
        if picount.is_null() || pisize.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: checked non-null out-pointers.
        unsafe {
            *picount = self.shared.catalog.len() as i32;
            *pisize = size_of::<VIDEO_STREAM_CONFIG_CAPS>() as i32;
        }
        Ok(())
    }

    fn GetStreamCaps(
        &self,
        iindex: i32,
        ppmt: *mut *mut AM_MEDIA_TYPE,
        pscc: *mut u8,
    ) -> Result<()> {
        if ppmt.is_null() || pscc.is_null() {
            return Err(E_POINTER.into());
        }
        let formats = &self.shared.catalog;
        let format = usize::try_from(iindex)
            .ok()
            .and_then(|i| formats.get(i).copied())
            .ok_or_else(|| Error::from(if iindex < 0 { E_INVALIDARG } else { S_FALSE }))?;
        let size = SIZE {
            cx: format.width as i32,
            cy: format.height as i32,
        };
        let bits = format.frame_bytes() as i32 * 8;
        let caps = VIDEO_STREAM_CONFIG_CAPS {
            guid: FORMAT_VideoInfo,
            VideoStandard: 0,
            InputSize: size,
            MinCroppingSize: size,
            MaxCroppingSize: size,
            CropGranularityX: 1,
            CropGranularityY: 1,
            CropAlignX: 1,
            CropAlignY: 1,
            MinOutputSize: size,
            MaxOutputSize: size,
            OutputGranularityX: 1,
            OutputGranularityY: 1,
            StretchTapsX: 0,
            StretchTapsY: 0,
            ShrinkTapsX: 0,
            ShrinkTapsY: 0,
            MinFrameInterval: media::UNITS / media::MAX_FPS as i64,
            MaxFrameInterval: media::UNITS,
            MinBitsPerSecond: bits,
            MaxBitsPerSecond: bits.saturating_mul(media::MAX_FPS as i32),
        };
        // SAFETY: the caller provides a VIDEO_STREAM_CONFIG_CAPS-sized buffer
        // (it asked GetNumberOfCapabilities for the size) and frees *ppmt.
        unsafe {
            std::ptr::write_unaligned(pscc as *mut VIDEO_STREAM_CONFIG_CAPS, caps);
            *ppmt = media::alloc_media_type(format);
        }
        Ok(())
    }
}

impl IKsPropertySet_Impl for OutputPin_Impl {
    fn Set(
        &self,
        _set: *const GUID,
        _id: u32,
        _inst: *const core::ffi::c_void,
        _inst_len: u32,
        _data: *const core::ffi::c_void,
        _data_len: u32,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Get(
        &self,
        guidpropset: *const GUID,
        dwpropid: u32,
        _instance: *const core::ffi::c_void,
        _instance_len: u32,
        ppropdata: *mut core::ffi::c_void,
        cbpropdata: u32,
        pcbreturned: *mut u32,
    ) -> Result<()> {
        // SAFETY: pointers come from the caller and are checked before use.
        unsafe {
            if guidpropset.is_null() || *guidpropset != AMPROPSETID_Pin {
                return Err(E_PROP_SET_UNSUPPORTED.into());
            }
            if dwpropid != AMPROPERTY_PIN_CATEGORY.0 as u32 {
                return Err(E_PROP_ID_UNSUPPORTED.into());
            }
            if !pcbreturned.is_null() {
                *pcbreturned = size_of::<GUID>() as u32;
            }
            if ppropdata.is_null() {
                return Ok(());
            }
            if (cbpropdata as usize) < size_of::<GUID>() {
                return Err(E_UNEXPECTED.into());
            }
            std::ptr::write_unaligned(ppropdata as *mut GUID, PIN_CATEGORY_CAPTURE);
        }
        Ok(())
    }

    fn QuerySupported(&self, guidpropset: *const GUID, dwpropid: u32) -> Result<u32> {
        // SAFETY: checked pointer from the caller.
        if guidpropset.is_null() || unsafe { *guidpropset } != AMPROPSETID_Pin {
            return Err(E_PROP_SET_UNSUPPORTED.into());
        }
        if dwpropid != AMPROPERTY_PIN_CATEGORY.0 as u32 {
            return Err(E_PROP_ID_UNSUPPORTED.into());
        }
        Ok(KSPROPERTY_SUPPORT_GET)
    }
}

#[implement(IEnumMediaTypes)]
pub struct MediaTypeEnum {
    formats: Vec<VideoFormat>,
    position: Mutex<usize>,
    _count: ObjectCount,
}

impl MediaTypeEnum {
    fn new(formats: Vec<VideoFormat>, position: usize) -> Self {
        Self {
            formats,
            position: Mutex::new(position),
            _count: ObjectCount::new(),
        }
    }

    fn position(&self) -> std::sync::MutexGuard<'_, usize> {
        self.position.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl IEnumMediaTypes_Impl for MediaTypeEnum_Impl {
    fn Next(&self, count: u32, out: *mut *mut AM_MEDIA_TYPE, fetched: *mut u32) -> HRESULT {
        if out.is_null() || (count > 1 && fetched.is_null()) {
            return E_POINTER;
        }
        let mut position = self.position();
        let mut n = 0u32;
        while n < count && *position < self.formats.len() {
            // SAFETY: the caller provides room for `count` pointers.
            unsafe { *out.add(n as usize) = media::alloc_media_type(self.formats[*position]) };
            *position += 1;
            n += 1;
        }
        if !fetched.is_null() {
            // SAFETY: checked non-null.
            unsafe { *fetched = n };
        }
        if n == count { S_OK } else { S_FALSE }
    }

    fn Skip(&self, count: u32) -> Result<()> {
        let mut position = self.position();
        *position = (*position + count as usize).min(self.formats.len());
        if *position == self.formats.len() && count > 0 {
            Err(Error::from_hresult(S_FALSE))
        } else {
            Ok(())
        }
    }

    fn Reset(&self) -> Result<()> {
        *self.position() = 0;
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumMediaTypes> {
        Ok(MediaTypeEnum::new(self.formats.clone(), *self.position()).into())
    }
}

#[implement(IEnumPins)]
pub struct PinEnum {
    pin: IPin,
    position: Mutex<usize>,
    _count: ObjectCount,
}

impl PinEnum {
    pub fn new(pin: IPin) -> Self {
        Self {
            pin,
            position: Mutex::new(0),
            _count: ObjectCount::new(),
        }
    }
}

impl IEnumPins_Impl for PinEnum_Impl {
    fn Next(&self, count: u32, out: *mut Option<IPin>, fetched: *mut u32) -> HRESULT {
        if out.is_null() || (count > 1 && fetched.is_null()) {
            return E_POINTER;
        }
        let mut position = self.position.lock().unwrap_or_else(|e| e.into_inner());
        let n = if count > 0 && *position == 0 {
            // SAFETY: the caller provides room for at least one pointer.
            unsafe { out.write(Some(self.pin.clone())) };
            *position = 1;
            1
        } else {
            0
        };
        if !fetched.is_null() {
            // SAFETY: checked non-null.
            unsafe { *fetched = n };
        }
        if n == count { S_OK } else { S_FALSE }
    }

    fn Skip(&self, count: u32) -> Result<()> {
        let mut position = self.position.lock().unwrap_or_else(|e| e.into_inner());
        let next = *position + count as usize;
        *position = next.min(1);
        if next > 1 {
            Err(Error::from_hresult(S_FALSE))
        } else {
            Ok(())
        }
    }

    fn Reset(&self) -> Result<()> {
        *self.position.lock().unwrap_or_else(|e| e.into_inner()) = 0;
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumPins> {
        let position = *self.position.lock().unwrap_or_else(|e| e.into_inner());
        let clone = PinEnum::new(self.pin.clone());
        *clone.position.lock().unwrap_or_else(|e| e.into_inner()) = position;
        Ok(clone.into())
    }
}
