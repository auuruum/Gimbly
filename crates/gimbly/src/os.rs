//! Small desktop-integration helpers: choosing and opening a folder, starting
//! with the session, opening a link and reading the local time.

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
        RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        SHCreateItemFromParsingName, SIGDN_FILESYSPATH, ShellExecuteW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const RUN_VALUE: &str = "Gimbly";

    /// Lets the GUI-subsystem release build print diagnostics in its caller's
    /// console without creating a new console window.
    pub fn attach_parent_console() {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

        // SAFETY: attaches only this process; no pointers or owned resources.
        // Failure is harmless when already attached or no parent console exists.
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }

    /// Shows the system folder picker on a helper thread (it needs its own
    /// single-threaded COM apartment) and returns the chosen folder.
    pub fn pick_folder(start: &str) -> Option<String> {
        pick(start, false)
    }

    /// The system file picker for a PNG or JPEG picture.
    pub fn pick_image(start: &str) -> Option<String> {
        pick(start, true)
    }

    fn pick(start: &str, image: bool) -> Option<String> {
        let start = start.to_string();
        std::thread::spawn(move || {
            // SAFETY: COM is initialised and released on this thread; all
            // interfaces are dropped before CoUninitialize.
            unsafe {
                let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
                let result = (|| -> windows::core::Result<String> {
                    let dialog: IFileOpenDialog =
                        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
                    if image {
                        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM)?;
                        dialog.SetTitle(w!("Add a Sleep Background picture"))?;
                        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
                            pszName: w!("Pictures (PNG, JPEG)"),
                            pszSpec: w!("*.png;*.jpg;*.jpeg"),
                        }])?;
                    } else {
                        dialog.SetOptions(
                            dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM,
                        )?;
                        dialog.SetTitle(w!("Choose Storage Path"))?;
                    }
                    if let Ok(folder) =
                        SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start), None)
                    {
                        let _ = dialog.SetFolder(&folder);
                    }
                    dialog.Show(None)?;
                    let item = dialog.GetResult()?;
                    let path = item.GetDisplayName(SIGDN_FILESYSPATH)?;
                    let text = path.to_string();
                    CoTaskMemFree(Some(path.0 as _));
                    text.map_err(|_| windows::core::Error::empty())
                })();
                if com {
                    CoUninitialize();
                }
                result.ok()
            }
        })
        .join()
        .ok()
        .flatten()
    }

    /// Whether Gimbly starts when the user signs in.
    pub fn starts_automatically() -> bool {
        let mut size = 0u32;
        // SAFETY: size query of a value in the user's Run key.
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from(RUN_VALUE),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
            .is_ok()
        }
    }

    pub fn set_starts_automatically(on: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut key = HKEY::default();
        // SAFETY: opens the user's Run key, writes or deletes our value and
        // closes the key.
        unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                None,
                KEY_READ | KEY_WRITE,
                &mut key,
            )
            .ok()
            .map_err(|e| e.to_string())?;
            let result = if on {
                let command = format!("\"{}\"", exe.display());
                let data: Vec<u8> = command
                    .encode_utf16()
                    .chain(Some(0))
                    .flat_map(u16::to_le_bytes)
                    .collect();
                RegSetValueExW(key, &HSTRING::from(RUN_VALUE), None, REG_SZ, Some(&data)).ok()
            } else {
                RegDeleteValueW(key, &HSTRING::from(RUN_VALUE)).ok()
            };
            let _ = RegCloseKey(key);
            result.map_err(|e| e.to_string())
        }
    }

    pub fn open_url(url: &str) {
        // SAFETY: asks the shell to open a URL with the default handler.
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(url),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    /// Shows a folder in Explorer.
    pub fn open_folder(path: &str) -> Result<(), String> {
        // SAFETY: asks the shell to open a folder; values above 32 mean
        // success.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("explore"),
                &HSTRING::from(path),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(format!("Could not open {path}"))
        }
    }

    /// Local wall-clock time: year, month, day, hour, minute, second.
    pub fn local_time() -> [u16; 6] {
        // SAFETY: GetLocalTime only fills the returned struct.
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        [t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond]
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "os/unix.rs"]
mod imp;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    pub fn attach_parent_console() {}

    pub fn pick_folder(_start: &str) -> Option<String> {
        None
    }

    pub fn pick_image(_start: &str) -> Option<String> {
        None
    }

    pub fn starts_automatically() -> bool {
        false
    }

    pub fn set_starts_automatically(_on: bool) -> Result<(), String> {
        Err("Starting automatically is only implemented on Windows so far".into())
    }

    pub fn open_url(_url: &str) {}

    pub fn open_folder(_path: &str) -> Result<(), String> {
        Err("Opening folders is only implemented on Windows so far".into())
    }

    /// UTC time until a local-time source is wired up on this platform.
    pub fn local_time() -> [u16; 6] {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        super::civil_time(secs)
    }
}

/// Converts seconds since 1970 (UTC) to year, month, day, hour, minute,
/// second (proleptic Gregorian calendar).
#[cfg(any(test, not(any(windows, target_os = "linux", target_os = "macos"))))]
fn civil_time(secs: u64) -> [u16; 6] {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Days-to-civil conversion after Howard Hinnant's public-domain algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    [
        year as u16,
        month as u16,
        day as u16,
        (rem / 3_600) as u16,
        (rem / 60 % 60) as u16,
        (rem % 60) as u16,
    ]
}

pub use imp::*;

/// Inspect the actual window backend. A Wayland desktop may also provide
/// DISPLAY for X11 clients.
pub fn is_wayland_window(_window: &eframe::CreationContext<'_>) -> bool {
    #[cfg(target_os = "linux")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        _window
            .window_handle()
            .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)))
    }
    #[cfg(not(target_os = "linux"))]
    false
}

/// Whether the helpers above do anything on this platform.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "linux", target_os = "macos"));

#[cfg(test)]
mod tests {
    use super::civil_time;

    #[test]
    fn converts_unix_time_to_calendar_time() {
        assert_eq!(civil_time(0), [1970, 1, 1, 0, 0, 0]);
        // 2024-02-29 23:59:59 UTC, a leap day.
        assert_eq!(civil_time(1_709_251_199), [2024, 2, 29, 23, 59, 59]);
        assert_eq!(civil_time(1_709_251_200), [2024, 3, 1, 0, 0, 0]);
    }
}
