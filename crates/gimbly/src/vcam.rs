//! Installing the virtual camera device.
//!
//! On Windows the virtual camera is a DirectShow source filter DLL built from
//! `crates/gimbly-vcam-dshow`. We copy it to `%LOCALAPPDATA%\Gimbly\`
//! under a name that contains a hash of its contents and register that copy
//! for the current user (no administrator rights). Video apps keep the DLL
//! loaded for as long as they run, so a fixed file name could never be
//! replaced while, say, Discord is open; with one file per build the new
//! version is registered immediately and old copies are removed once nothing
//! uses them. Linux uses v4l2loopback and macOS uses a Core Media I/O extension.

#[cfg(windows)]
pub use gimbly_vcam_shm::FRIENDLY_NAME;
#[cfg(not(windows))]
pub const FRIENDLY_NAME: &str = "Gimbly Virtual Camera";

/// FNV-1a, enough to tell builds of the DLL apart.
#[cfg_attr(not(windows), allow(dead_code))]
fn content_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Installs and registers the virtual camera. Returns a hint when apps that
/// already loaded an older build must be restarted to use the new one.
#[cfg(windows)]
pub fn register() -> Result<Option<String>, String> {
    use gimbly_vcam_shm::FILTER_DLL_NAME;
    use windows::Win32::Foundation::FreeLibrary;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{HRESULT, HSTRING, s};

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let built = exe.with_file_name(FILTER_DLL_NAME);
    let bytes = std::fs::read(&built).map_err(|_| {
        format!("{FILTER_DLL_NAME} was not found next to the app (build the whole workspace)")
    })?;
    let dir = std::env::var_os("LOCALAPPDATA")
        .map(|d| std::path::PathBuf::from(d).join("Gimbly"))
        .ok_or("LOCALAPPDATA is not set")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let stem = FILTER_DLL_NAME.trim_end_matches(".dll");
    let file_name = format!("{stem}-{:016x}.dll", content_hash(&bytes));
    let installed = dir.join(&file_name);
    if std::fs::read(&installed).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&installed, &bytes)
            .map_err(|e| format!("cannot install {}: {e}", installed.display()))?;
    }

    // SAFETY: we load our own DLL, call its documented COM self-registration
    // export with the expected signature and unload it again.
    unsafe {
        let module =
            LoadLibraryW(&HSTRING::from(installed.as_os_str())).map_err(|e| e.to_string())?;
        let result = match GetProcAddress(module, s!("DllRegisterServer")) {
            Some(proc) => {
                let register: unsafe extern "system" fn() -> HRESULT = std::mem::transmute(proc);
                register()
                    .ok()
                    .map_err(|e| format!("registration failed: {e}"))
            }
            None => Err("the DLL has no DllRegisterServer export".into()),
        };
        let _ = FreeLibrary(module);
        result?;
    }

    // Remove older builds; files still loaded by running apps cannot be
    // deleted and stay until a later start.
    let mut in_use = false;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != file_name
                && name.starts_with(stem)
                && name.ends_with(".dll")
                && std::fs::remove_file(entry.path()).is_err()
            {
                in_use = true;
            }
        }
    }
    Ok(in_use.then(|| {
        "Video apps that were already open keep the previous version until you restart them."
            .to_string()
    }))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn register() -> Result<Option<String>, String> {
    Err("the virtual camera is only implemented on Windows so far".into())
}

#[cfg(windows)]
pub struct Output(gimbly_vcam_shm::SharedMemory);

#[cfg(windows)]
impl Output {
    pub fn open(
        _config: crate::camera::CaptureConfig,
        _format: Option<(u32, u32)>,
    ) -> Result<Self, String> {
        gimbly_vcam_shm::SharedMemory::create()
            .map(Self)
            .map_err(|e| e.to_string())
    }

    pub fn publish(
        &mut self,
        view: &gimbly_vcam_shm::Nv12View<'_>,
        _range: crate::camera::YuvRange,
    ) -> Result<bool, String> {
        self.0.region().publish(view, gimbly_vcam_shm::now_ms());
        Ok(true)
    }
}

#[cfg(windows)]
impl Drop for Output {
    fn drop(&mut self) {
        self.0.region().mark_stopped();
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::is_loopback_driver;
#[cfg(target_os = "linux")]
pub use linux::{Output, register};

#[cfg(target_os = "linux")]
pub mod setup;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub struct Output;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
impl Output {
    pub fn open(
        _config: crate::camera::CaptureConfig,
        _format: Option<(u32, u32)>,
    ) -> Result<Self, String> {
        Err("Virtual camera output is unsupported on this system".into())
    }

    pub fn publish(
        &mut self,
        _view: &gimbly_vcam_shm::Nv12View<'_>,
        _range: crate::camera::YuvRange,
    ) -> Result<bool, String> {
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_media;
#[cfg(target_os = "macos")]
pub use macos::{Output, register};
#[cfg(test)]
mod tests {
    use super::content_hash;

    #[test]
    fn hash_tells_builds_apart() {
        assert_eq!(content_hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_ne!(content_hash(b"build a"), content_hash(b"build b"));
        assert_eq!(content_hash(b"same"), content_hash(b"same"));
    }
}
