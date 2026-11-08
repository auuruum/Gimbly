//! Gimbly virtual camera for Windows: an in-process COM server that
//! exposes a DirectShow video capture source ("Gimbly Virtual Camera").
//! Applications that list DirectShow cameras load this DLL and receive the
//! frames Gimbly publishes through `gimbly-vcam-shm`.
//!
//! The crate is empty on other platforms.

#[cfg(windows)]
mod filter;
#[cfg(windows)]
mod media;
#[cfg(windows)]
mod pin;
#[cfg(windows)]
mod registry;
#[cfg(windows)]
mod stream;

#[cfg(windows)]
pub use exports::*;

/// Live COM objects created by this DLL; `DllCanUnloadNow` refuses while any
/// exist.
#[cfg(windows)]
static OBJECTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(windows)]
struct ObjectCount;

#[cfg(windows)]
impl ObjectCount {
    fn new() -> Self {
        OBJECTS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self
    }
}

#[cfg(windows)]
impl Drop for ObjectCount {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

#[cfg(windows)]
fn clsid() -> windows_core::GUID {
    windows_core::GUID::from_u128(gimbly_vcam_shm::FILTER_CLSID)
}

#[cfg(windows)]
mod exports {
    use super::{OBJECTS, ObjectCount, clsid, filter::VirtualCameraFilter, registry};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
    use windows::Win32::Foundation::{
        CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, HINSTANCE, HMODULE,
        S_FALSE, S_OK,
    };
    use windows::Win32::Media::DirectShow::IBaseFilter;
    use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
    use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
    use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows_core::{BOOL, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};

    static MODULE: AtomicIsize = AtomicIsize::new(0);
    static SERVER_LOCKS: AtomicUsize = AtomicUsize::new(0);

    #[implement(IClassFactory)]
    struct Factory {
        _count: ObjectCount,
    }

    impl IClassFactory_Impl for Factory_Impl {
        fn CreateInstance(
            &self,
            outer: Ref<IUnknown>,
            riid: *const GUID,
            object: *mut *mut c_void,
        ) -> Result<()> {
            if object.is_null() || riid.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: checked non-null out-pointer.
            unsafe { *object = std::ptr::null_mut() };
            if outer.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let filter = VirtualCameraFilter::create();
            // SAFETY: riid/object are valid per the checks above.
            unsafe { filter.query(riid, object).ok() }
        }

        fn LockServer(&self, lock: BOOL) -> Result<()> {
            if lock.as_bool() {
                SERVER_LOCKS.fetch_add(1, Ordering::AcqRel);
            } else {
                let _ = SERVER_LOCKS
                    .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
            }
            Ok(())
        }
    }

    #[unsafe(no_mangle)]
    extern "system" fn DllMain(module: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
        if reason == DLL_PROCESS_ATTACH {
            MODULE.store(module.0 as isize, Ordering::Release);
        }
        true.into()
    }

    /// # Safety
    /// Called by COM with valid pointers.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn DllGetClassObject(
        rclsid: *const GUID,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> HRESULT {
        if rclsid.is_null() || riid.is_null() || object.is_null() {
            return E_POINTER;
        }
        // SAFETY: checked non-null pointers from COM.
        unsafe {
            *object = std::ptr::null_mut();
            if *rclsid != clsid() {
                return CLASS_E_CLASSNOTAVAILABLE;
            }
            let factory: IClassFactory = Factory {
                _count: ObjectCount::new(),
            }
            .into();
            factory.query(riid, object)
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn DllCanUnloadNow() -> HRESULT {
        if OBJECTS.load(Ordering::Acquire) == 0 && SERVER_LOCKS.load(Ordering::Acquire) == 0 {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn module_path() -> Option<String> {
        let module = HMODULE(MODULE.load(Ordering::Acquire) as *mut c_void);
        let mut buffer = vec![0u16; 32_768];
        // SAFETY: valid module handle (or null for the host exe) and buffer.
        let len = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
        (len > 0 && len < buffer.len()).then(|| String::from_utf16_lossy(&buffer[..len]))
    }

    /// Registers the virtual camera for the current user.
    #[unsafe(no_mangle)]
    pub extern "system" fn DllRegisterServer() -> HRESULT {
        let Some(path) = module_path() else {
            return E_FAIL;
        };
        match registry::register(&clsid(), &path) {
            Ok(()) => S_OK,
            Err(e) => e.code(),
        }
    }

    /// Removes the current user's registration.
    #[unsafe(no_mangle)]
    pub extern "system" fn DllUnregisterServer() -> HRESULT {
        match registry::unregister(&clsid()) {
            Ok(()) => S_OK,
            Err(e) => e.code(),
        }
    }

    #[allow(dead_code)]
    fn _assert_filter_type(_: IBaseFilter) {}
}
