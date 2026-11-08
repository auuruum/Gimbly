//! Windows named file mapping that backs the frame [`Region`].

use crate::{MAPPING_NAME, REGION_BYTES, Region};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
    MapViewOfFile, OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::core::HSTRING;

/// Milliseconds since boot; the same clock in every process of the session.
pub fn now_ms() -> u64 {
    // SAFETY: no preconditions.
    unsafe { GetTickCount64() }
}

/// An owned mapping of the shared frame region.
pub struct SharedMemory {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    region: Region,
}

// SAFETY: the handle and view are process-wide kernel objects; access to the
// mapped memory goes through `Region`, which is Send + Sync.
unsafe impl Send for SharedMemory {}
unsafe impl Sync for SharedMemory {}

impl SharedMemory {
    /// Creates (or reuses) the mapping as the producer and initialises it.
    pub fn create() -> windows::core::Result<Self> {
        let name = HSTRING::from(MAPPING_NAME);
        // SAFETY: plain Win32 calls with valid arguments; failures are mapped
        // to errors and the handle is closed on every early return.
        unsafe {
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                REGION_BYTES as u32,
                &name,
            )?;
            Self::map(handle, FILE_MAP_ALL_ACCESS.0).inspect(|shm| shm.region.init())
        }
    }

    /// Opens an existing mapping read-only as a consumer.
    pub fn open() -> windows::core::Result<Self> {
        let name = HSTRING::from(MAPPING_NAME);
        // SAFETY: as above.
        unsafe {
            let handle = OpenFileMappingW(FILE_MAP_READ.0, false, &name)?;
            Self::map(handle, FILE_MAP_READ.0)
        }
    }

    unsafe fn map(handle: HANDLE, access: u32) -> windows::core::Result<Self> {
        // SAFETY: `handle` is a valid file mapping of at least REGION_BYTES.
        let view = unsafe {
            MapViewOfFile(
                handle,
                windows::Win32::System::Memory::FILE_MAP(access),
                0,
                0,
                REGION_BYTES,
            )
        };
        if view.Value.is_null() {
            let err = windows::core::Error::from_thread();
            // SAFETY: we own the handle.
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(err);
        }
        // SAFETY: the view is page aligned, REGION_BYTES long and lives until
        // Drop unmaps it; Region does not outlive SharedMemory.
        let region = unsafe { Region::from_raw(view.Value.cast()) };
        Ok(Self {
            handle,
            view,
            region,
        })
    }

    pub fn region(&self) -> &Region {
        &self.region
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        // SAFETY: view and handle were created in `map` and are released once.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}
