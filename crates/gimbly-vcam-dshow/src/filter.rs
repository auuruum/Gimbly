//! The DirectShow source filter object (`IBaseFilter`).

use crate::media::VideoFormat;
use crate::pin::OutputPin;
use crate::stream;
use crate::{ObjectCount, clsid};
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use windows::Win32::Foundation::{E_NOTIMPL, S_OK};
use windows::Win32::Media::DirectShow::*;
use windows::Win32::Media::IReferenceClock;
use windows::Win32::System::Com::IPersist_Impl;
use windows_core::{Error, GUID, Interface, PCWSTR, PWSTR, Ref, Result, implement};

/// Name of the single output pin.
pub const PIN_NAME: &str = "Video";

/// An established connection of the output pin.
pub struct Connection {
    pub peer: IPin,
    pub input: IMemInputPin,
    pub allocator: IMemAllocator,
    pub format: VideoFormat,
}

pub struct Inner {
    pub state: FILTER_STATE,
    /// Reference time passed to `Run`; stream time is clock time minus this.
    pub start_time: i64,
    pub clock: Option<IReferenceClock>,
    /// The graph that owns the filter. Not reference counted: a filter must
    /// not keep its graph alive.
    pub graph: *mut c_void,
    pub name: Vec<u16>,
    /// Format chosen with `IAMStreamConfig::SetFormat`.
    pub requested: Option<VideoFormat>,
    pub connection: Option<Connection>,
    pub stream: Option<stream::Handle>,
}

// SAFETY: DirectShow filter-graph interfaces are free-threaded, and all access
// to `Inner` is serialised by the mutex in `Shared`.
unsafe impl Send for Inner {}

/// State shared by the filter, its pin and the delivery thread.
pub struct Shared {
    inner: Mutex<Inner>,
    /// The filter's own `IBaseFilter` pointer, not reference counted; cleared
    /// when the filter is destroyed.
    filter: AtomicPtr<c_void>,
    /// Formats this filter instance offers, preferred first. Read from
    /// Gimbly's settings when the filter is created.
    pub catalog: Vec<VideoFormat>,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A counted reference to the owning filter, if it still exists.
    pub fn filter(&self) -> Option<IBaseFilter> {
        let raw = self.filter.load(Ordering::Acquire);
        // SAFETY: the pointer is set while the filter is alive and cleared in
        // its Drop; cloning adds a reference.
        unsafe { IBaseFilter::from_raw_borrowed(&raw).cloned() }
    }

    pub fn graph(&self) -> Option<IFilterGraph> {
        let raw = self.lock().graph;
        // SAFETY: the graph pointer is valid while the filter is in the graph
        // (JoinFilterGraph(NULL) clears it before the graph goes away).
        unsafe { IFilterGraph::from_raw_borrowed(&raw).cloned() }
    }
}

#[implement(IBaseFilter, IAMFilterMiscFlags)]
pub struct VirtualCameraFilter {
    shared: Arc<Shared>,
    pin: IPin,
    _count: ObjectCount,
}

impl VirtualCameraFilter {
    pub fn create() -> IBaseFilter {
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                state: State_Stopped,
                start_time: 0,
                clock: None,
                graph: std::ptr::null_mut(),
                name: Vec::new(),
                requested: None,
                connection: None,
                stream: None,
            }),
            filter: AtomicPtr::new(std::ptr::null_mut()),
            catalog: VideoFormat::catalog(gimbly_vcam_shm::read_preferred_format()),
        });
        let pin: IPin = OutputPin::new(shared.clone()).into();
        let filter: IBaseFilter = VirtualCameraFilter {
            shared: shared.clone(),
            pin,
            _count: ObjectCount::new(),
        }
        .into();
        shared.filter.store(filter.as_raw(), Ordering::Release);
        live_source_vtable::install(&filter);
        filter
    }

    fn stop_streaming(&self) {
        let (stream, allocator) = {
            let mut inner = self.shared.lock();
            inner.state = State_Stopped;
            (
                inner.stream.take(),
                inner.connection.as_ref().map(|c| c.allocator.clone()),
            )
        };
        if let Some(allocator) = allocator {
            // Unblocks a delivery thread waiting in GetBuffer.
            // SAFETY: plain COM call on a valid allocator.
            let _ = unsafe { allocator.Decommit() };
        }
        if let Some(stream) = stream {
            stream.stop_and_join();
        }
    }
}

impl Drop for VirtualCameraFilter {
    fn drop(&mut self) {
        self.shared
            .filter
            .store(std::ptr::null_mut(), Ordering::Release);
        self.stop_streaming();
    }
}

impl IPersist_Impl for VirtualCameraFilter_Impl {
    fn GetClassID(&self) -> Result<GUID> {
        Ok(clsid())
    }
}

impl IMediaFilter_Impl for VirtualCameraFilter_Impl {
    fn Stop(&self) -> Result<()> {
        self.stop_streaming();
        Ok(())
    }

    fn Pause(&self) -> Result<()> {
        let mut inner = self.shared.lock();
        if inner.state == State_Stopped
            && let Some(c) = &inner.connection
        {
            // SAFETY: plain COM call on a valid allocator.
            unsafe { c.allocator.Commit()? };
            let handle = stream::start(
                self.shared.clone(),
                c.allocator.clone(),
                c.input.clone(),
                c.format,
            );
            inner.stream = Some(handle);
        }
        inner.state = State_Paused;
        Ok(())
    }

    fn Run(&self, tstart: i64) -> Result<()> {
        if self.shared.lock().state == State_Stopped {
            self.Pause()?;
        }
        let mut inner = self.shared.lock();
        inner.start_time = tstart;
        inner.state = State_Running;
        Ok(())
    }

    fn GetState(&self, _timeout_ms: u32) -> Result<FILTER_STATE> {
        Ok(self.shared.lock().state)
    }

    fn SetSyncSource(&self, pclock: Ref<IReferenceClock>) -> Result<()> {
        self.shared.lock().clock = pclock.cloned();
        Ok(())
    }

    fn GetSyncSource(&self) -> Result<IReferenceClock> {
        // "No clock" is S_OK with a NULL pointer; the vtable shim writes the
        // NULL (see live_source_vtable).
        self.shared
            .lock()
            .clock
            .clone()
            .ok_or_else(|| Error::from_hresult(S_OK))
    }
}

impl IBaseFilter_Impl for VirtualCameraFilter_Impl {
    fn EnumPins(&self) -> Result<IEnumPins> {
        Ok(crate::pin::PinEnum::new(self.pin.clone()).into())
    }

    fn FindPin(&self, id: &PCWSTR) -> Result<IPin> {
        // SAFETY: DirectShow passes a valid NUL-terminated string.
        let id = unsafe { id.to_string() }.unwrap_or_default();
        if id == PIN_NAME {
            Ok(self.pin.clone())
        } else {
            Err(VFW_E_NOT_FOUND.into())
        }
    }

    fn QueryFilterInfo(&self, pinfo: *mut FILTER_INFO) -> Result<()> {
        if pinfo.is_null() {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        }
        let mut name = [0u16; 128];
        {
            let inner = self.shared.lock();
            let len = inner.name.len().min(127);
            name[..len].copy_from_slice(&inner.name[..len]);
        }
        // SAFETY: the caller provides a FILTER_INFO to fill; the graph
        // reference we hand out is counted (the caller releases it).
        unsafe {
            pinfo.write(FILTER_INFO {
                achName: name,
                pGraph: std::mem::ManuallyDrop::new(self.shared.graph()),
            })
        };
        Ok(())
    }

    fn JoinFilterGraph(&self, pgraph: Ref<IFilterGraph>, pname: &PCWSTR) -> Result<()> {
        let mut inner = self.shared.lock();
        inner.graph = pgraph.as_ref().map_or(std::ptr::null_mut(), |g| g.as_raw());
        inner.name = if pname.is_null() {
            Vec::new()
        } else {
            // SAFETY: valid NUL-terminated string from the graph.
            unsafe { pname.as_wide() }.to_vec()
        };
        Ok(())
    }

    fn QueryVendorInfo(&self) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
}

impl IAMFilterMiscFlags_Impl for VirtualCameraFilter_Impl {
    fn GetMiscFlags(&self) -> u32 {
        AM_FILTER_MISC_FLAGS_IS_SOURCE.0 as u32
    }
}

/// Two `IMediaFilter` methods need return values the generated COM shims
/// cannot produce: a paused live source must answer `GetState` with
/// `VFW_S_CANT_CUE` (so the graph does not wait for a preroll sample), and
/// `GetSyncSource` must be able to return S_OK with a NULL clock. We copy the
/// generated vtable once, wrap those two entries, and point the filter at the
/// copy. Everything else still goes through the generated shims.
mod live_source_vtable {
    use super::*;

    type GetStateFn =
        unsafe extern "system" fn(*mut c_void, u32, *mut FILTER_STATE) -> windows_core::HRESULT;
    type GetSyncSourceFn =
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> windows_core::HRESULT;

    struct Patched {
        vtable: IBaseFilter_Vtbl,
        get_state: GetStateFn,
        get_sync_source: GetSyncSourceFn,
    }

    static PATCHED: OnceLock<Patched> = OnceLock::new();

    unsafe extern "system" fn get_state(
        this: *mut c_void,
        timeout: u32,
        state: *mut FILTER_STATE,
    ) -> windows_core::HRESULT {
        let Some(p) = PATCHED.get() else {
            return windows::Win32::Foundation::E_UNEXPECTED;
        };
        // SAFETY: forwards to the generated shim with the caller's arguments.
        let hr = unsafe { (p.get_state)(this, timeout, state) };
        // SAFETY: `state` was just written by the shim on success.
        if hr == S_OK && !state.is_null() && unsafe { *state } == State_Paused {
            VFW_S_CANT_CUE
        } else {
            hr
        }
    }

    unsafe extern "system" fn get_sync_source(
        this: *mut c_void,
        clock: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        let Some(p) = PATCHED.get() else {
            return windows::Win32::Foundation::E_UNEXPECTED;
        };
        if clock.is_null() {
            return windows::Win32::Foundation::E_POINTER;
        }
        // SAFETY: the shim overwrites this on success; on our "no clock"
        // result it leaves the NULL in place.
        unsafe {
            *clock = std::ptr::null_mut();
            (p.get_sync_source)(this, clock)
        }
    }

    pub fn install(filter: &IBaseFilter) {
        let slot = filter.as_raw() as *mut *const IBaseFilter_Vtbl;
        // SAFETY: a COM interface pointer points at its vtable pointer. The
        // generated vtable is a static, so copying it is sound, and the copy
        // lives in a static for the lifetime of the DLL.
        unsafe {
            let original = *slot;
            let patched = PATCHED.get_or_init(|| {
                let mut vtable = std::ptr::read(original);
                let get_state_orig = vtable.base__.GetState;
                let get_sync_source_orig = vtable.base__.GetSyncSource;
                vtable.base__.GetState = get_state;
                vtable.base__.GetSyncSource = get_sync_source;
                Patched {
                    vtable,
                    get_state: get_state_orig,
                    get_sync_source: get_sync_source_orig,
                }
            });
            *slot = &patched.vtable;
        }
    }
}
