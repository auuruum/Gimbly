//! The camera's CDC serial port. The camera streams the box of the tracked
//! target on it (see `docs/clean-room/2026-11-22-preview-tracking.md`);
//! the extension unit does not report it.
//!
//! Opening a serial port needs OS APIs, so each OS has a small section
//! below: Windows finds the port in the device registry, Linux in sysfs,
//! and macOS by the `cu.usbmodem` name. The camera only streams while it
//! gets the official app's keep-alive (it stopped about 10 s after the last
//! one), so the port found for this camera gets it from the start. A port
//! that is only a guess is written to once it sent a valid camera message.

use super::{Shared, TargetBox, vendor};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long one read waits; also how quickly the stream notices `stop`.
const READ_TIMEOUT: Duration = Duration::from_millis(200);
const KEEPALIVE_EVERY: Duration = Duration::from_millis(500);
/// A guessed port that sends no camera message in this time is not the
/// camera's.
const VERIFY_FOR: Duration = Duration::from_secs(3);
/// The camera sends about 30 messages a second while awake; after this
/// long without one, the box is stale.
const SILENT_FOR: Duration = Duration::from_secs(2);
const RETRY_EVERY: Duration = Duration::from_secs(2);

/// Reads the target stream into [`super::SessionStatus::target`] until
/// `stop` is set. `device_id` is the video device's OS identifier.
pub(super) fn target_stream(
    device_id: &str,
    shared: &Shared,
    repaint: &(dyn Fn() + Send + Sync),
    stop: &AtomicBool,
) {
    let mut reported = false;
    while !stop.load(Ordering::Relaxed) {
        for (path, trusted) in platform::candidates(device_id) {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match platform::Port::open(&path) {
                Ok(mut port) => {
                    if follow(&mut port, trusted, shared, repaint, stop) {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied && !reported => {
                    reported = true;
                    shared.status().error = Some(platform::permission_hint(&path));
                    repaint();
                }
                Err(_) => {}
            }
        }
        let until = Instant::now() + RETRY_EVERY;
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(READ_TIMEOUT);
        }
    }
}

/// Follows one open port. `trusted`: the OS says it is this camera's.
/// Returns true when it was the camera's port and `stop` ended it.
fn follow(
    port: &mut platform::Port,
    trusted: bool,
    shared: &Shared,
    repaint: &(dyn Fn() + Send + Sync),
    stop: &AtomicBool,
) -> bool {
    let opened = Instant::now();
    let mut verified = false;
    let mut last_message = opened;
    let mut keepalive: Option<Instant> = None;
    let mut buffer = Vec::new();
    let mut bytes = [0u8; 512];
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break verified;
        }
        let read = match port.read(&mut bytes) {
            Ok(n) => n,
            Err(_) => break false,
        };
        for message in vendor::split_serial(&mut buffer, bytes.get(..read).unwrap_or_default()) {
            if message.source != 0x02 || message.command != vendor::TARGET_STREAM {
                continue;
            }
            let Some(targets) = vendor::read_targets(&message.data) else {
                continue;
            };
            verified = true;
            last_message = Instant::now();
            set_target(shared, repaint, targets.first().copied());
        }
        if !trusted && !verified && opened.elapsed() >= VERIFY_FOR {
            break false;
        }
        if last_message.elapsed() >= SILENT_FOR {
            // Asleep or unplugged; a trusted port waits for it to come back.
            set_target(shared, repaint, None);
            if !trusted && verified {
                break false;
            }
        }
        // A failed write shows up as a failed read soon after.
        if (trusted || verified) && keepalive.is_none_or(|at| at.elapsed() >= KEEPALIVE_EVERY) {
            let _ = port.write(&vendor::target_keepalive());
            keepalive = Some(Instant::now());
        }
    };
    set_target(shared, repaint, None);
    result
}

fn set_target(shared: &Shared, repaint: &(dyn Fn() + Send + Sync), target: Option<TargetBox>) {
    let mut status = shared.status();
    if status.target != target {
        status.target = target;
        drop(status);
        repaint();
    }
}

/// The name of group `gid` in `/etc/group` text (`name:x:gid:members`).
/// Only plain names are returned, as they end up in a message.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn group_name(groups: &str, gid: u32) -> Option<String> {
    groups.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let id: u32 = fields.nth(1)?.trim().parse().ok()?;
        (id == gid
            && !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .then(|| name.to_owned())
    })
}

#[cfg(windows)]
mod platform {
    use super::READ_TIMEOUT;
    use std::io;
    use windows::Win32::Devices::Communication::{
        COMMTIMEOUTS, DCB, GetCommState, SetCommState, SetCommTimeouts,
    };
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, OPEN_EXISTING, ReadFile, WriteFile,
    };
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW,
        RegGetValueW, RegOpenKeyExW,
    };
    use windows::core::{HSTRING, PWSTR};

    const USB_ENUM: &str = r"SYSTEM\CurrentControlSet\Enum\USB";

    /// The video interface's symbolic link names the USB function, e.g.
    /// `\\?\usb#vid_3564&pid_ff04&mi_00#7&245ef964&1&0000#{...}`. The
    /// serial function of the same camera has the same instance prefix
    /// (`7&245ef964&1&`) under another `MI_` key.
    pub(super) fn candidates(device_id: &str) -> Vec<(String, bool)> {
        let mut parts = device_id.split('#');
        let (Some(_), Some(function), Some(instance)) = (parts.next(), parts.next(), parts.next())
        else {
            return Vec::new();
        };
        let Some(ids) = function
            .to_ascii_uppercase()
            .split("&MI_")
            .next()
            .map(|ids| format!("{ids}&MI_"))
        else {
            return Vec::new();
        };
        let Some(prefix) = instance
            .rfind('&')
            .and_then(|at| instance.get(..=at))
            .map(str::to_ascii_uppercase)
        else {
            return Vec::new();
        };
        let mut ports = Vec::new();
        for function in subkeys(USB_ENUM) {
            if !function.to_ascii_uppercase().starts_with(&ids) {
                continue;
            }
            let path = format!(r"{USB_ENUM}\{function}");
            for instance in subkeys(&path) {
                if instance.to_ascii_uppercase().starts_with(&prefix)
                    && let Some(port) = port_name(&format!(r"{path}\{instance}\Device Parameters"))
                {
                    ports.push((format!(r"\\.\{port}"), true));
                }
            }
        }
        ports
    }

    fn subkeys(path: &str) -> Vec<String> {
        let mut key = HKEY::default();
        let mut names = Vec::new();
        // SAFETY: opens a key read-only, enumerates its subkey names into a
        // local buffer whose length is passed along, and closes the key.
        unsafe {
            if RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                &HSTRING::from(path),
                None,
                KEY_READ,
                &mut key,
            )
            .is_err()
            {
                return names;
            }
            for index in 0.. {
                let mut name = [0u16; 256];
                let mut len = name.len() as u32;
                if RegEnumKeyExW(
                    key,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
                .is_err()
                {
                    break;
                }
                let len = (len as usize).min(name.len());
                names.push(String::from_utf16_lossy(&name[..len]));
            }
            let _ = RegCloseKey(key);
        }
        names
    }

    fn port_name(path: &str) -> Option<String> {
        let mut name = [0u16; 64];
        let mut size = std::mem::size_of_val(&name) as u32;
        // SAFETY: reads a string value into a local buffer of `size` bytes.
        unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                &HSTRING::from(path),
                &HSTRING::from("PortName"),
                RRF_RT_REG_SZ,
                None,
                Some(name.as_mut_ptr().cast()),
                Some(&mut size),
            )
            .ok()
            .ok()?;
        }
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let name = String::from_utf16_lossy(&name[..len]);
        // Only plain COM port names; the path is built from it.
        name.strip_prefix("COM")
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            .then_some(name)
    }

    pub(super) fn permission_hint(path: &str) -> String {
        format!(
            "Gimbly could not open the camera's serial port {path}, so the tracking box is hidden"
        )
    }

    pub(super) struct Port(HANDLE);

    // SAFETY: the handle is a plain file handle used from one thread at a time.
    unsafe impl Send for Port {}

    impl Port {
        pub(super) fn open(path: &str) -> io::Result<Self> {
            // SAFETY: opens an existing device for exclusive read and write and
            // sets its read timeout; the handle is closed in `Drop`.
            unsafe {
                let handle = CreateFileW(
                    &HSTRING::from(path),
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                )
                .map_err(io::Error::other)?;
                let port = Self(handle);
                let timeouts = COMMTIMEOUTS {
                    ReadIntervalTimeout: 10,
                    ReadTotalTimeoutMultiplier: 0,
                    ReadTotalTimeoutConstant: READ_TIMEOUT.as_millis() as u32,
                    WriteTotalTimeoutMultiplier: 0,
                    WriteTotalTimeoutConstant: 100,
                };
                SetCommTimeouts(port.0, &timeouts).map_err(io::Error::other)?;
                // Set the line up as the official app does (9600 baud, 8 bits,
                // DTR off): the camera ignored the port until it was.
                let mut state = DCB {
                    DCBlength: std::mem::size_of::<DCB>() as u32,
                    ..Default::default()
                };
                GetCommState(port.0, &mut state).map_err(io::Error::other)?;
                state.BaudRate = 9600;
                state.ByteSize = 8;
                SetCommState(port.0, &state).map_err(io::Error::other)?;
                Ok(port)
            }
        }

        /// Waits up to [`READ_TIMEOUT`]; 0 bytes means nothing arrived.
        pub(super) fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let mut read = 0u32;
            // SAFETY: reads into `buffer` with its length; synchronous handle.
            unsafe { ReadFile(self.0, Some(buffer), Some(&mut read), None) }
                .map_err(io::Error::other)?;
            Ok((read as usize).min(buffer.len()))
        }

        pub(super) fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
            let mut written = 0u32;
            // SAFETY: writes `bytes` with its length; synchronous handle.
            unsafe { WriteFile(self.0, Some(bytes), Some(&mut written), None) }
                .map_err(io::Error::other)
        }
    }

    impl Drop for Port {
        fn drop(&mut self) {
            // SAFETY: closes the handle this value owns.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use super::READ_TIMEOUT;
    use std::ffi::CString;
    use std::io;

    #[cfg(target_os = "linux")]
    pub(super) fn candidates(device_id: &str) -> Vec<(String, bool)> {
        // The serial function is another interface of the same USB device:
        // .../1-2/1-2:1.4/tty/ttyACM0 next to .../1-2/1-2:1.0 (video).
        let Some(usb) = super::super::linux::usb_parent(std::path::Path::new(device_id)) else {
            return Vec::new();
        };
        let mut ports = Vec::new();
        for interface in std::fs::read_dir(&usb).into_iter().flatten().flatten() {
            for tty in std::fs::read_dir(interface.path().join("tty"))
                .into_iter()
                .flatten()
                .flatten()
            {
                let name = tty.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("ttyACM") && name.chars().all(|c| c.is_ascii_alphanumeric()) {
                    ports.push((format!("/dev/{name}"), true));
                }
            }
        }
        ports
    }

    #[cfg(target_os = "linux")]
    pub(super) fn permission_hint(path: &str) -> String {
        use std::os::unix::fs::MetadataExt;
        let group = std::fs::metadata(path)
            .ok()
            .and_then(|meta| {
                let groups = std::fs::read_to_string("/etc/group").ok()?;
                super::group_name(&groups, meta.gid())
            })
            .unwrap_or_else(|| "that owns it (usually uucp or dialout)".into());
        format!(
            "Gimbly cannot open the camera's serial port {path}, so the tracking box is hidden. \
             Install Gimbly's udev rule (packaging/linux/70-gimbly.rules, see the Linux setup), \
             or add your user to the group {group} and sign in again."
        )
    }

    /// macOS names a CDC port after the USB location, e.g. location
    /// `0x14200000` gives `cu.usbmodem14201`. Ports with the camera's
    /// prefix come first; the others are tried read-only after them.
    #[cfg(target_os = "macos")]
    pub(super) fn candidates(device_id: &str) -> Vec<(String, bool)> {
        let prefix = super::super::macos::usb_location(device_id)
            .map(|location| format!("cu.usbmodem{:x}", location >> 16));
        let mut ports: Vec<String> = std::fs::read_dir("/dev")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.starts_with("cu.usbmodem"))
            .collect();
        let ours = |name: &String| {
            prefix
                .as_ref()
                .is_some_and(|p| name.starts_with(p.as_str()))
        };
        ports.sort_by_key(|name| !ours(name));
        ports
            .into_iter()
            .map(|name| (format!("/dev/{name}"), ours(&name)))
            .collect()
    }

    #[cfg(target_os = "macos")]
    pub(super) fn permission_hint(path: &str) -> String {
        format!("Gimbly cannot read the camera's serial port {path}, so the tracking box is hidden")
    }

    pub(super) struct Port(libc::c_int);

    impl Port {
        pub(super) fn open(path: &str) -> io::Result<Self> {
            let path = CString::new(path).map_err(io::Error::other)?;
            // SAFETY: opens a NUL-terminated path; the descriptor is closed in
            // `Drop`. Raw mode only changes this descriptor's terminal settings.
            unsafe {
                let fd = libc::open(
                    path.as_ptr(),
                    libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC,
                );
                if fd < 0 {
                    return Err(io::Error::last_os_error());
                }
                let port = Self(fd);
                let mut settings: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(fd, &mut settings) == 0 {
                    libc::cfmakeraw(&mut settings);
                    let _ = libc::tcsetattr(fd, libc::TCSANOW, &settings);
                }
                Ok(port)
            }
        }

        /// Waits up to [`READ_TIMEOUT`]; 0 bytes means nothing arrived.
        pub(super) fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let mut poll = libc::pollfd {
                fd: self.0,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: polls one valid descriptor, then reads into `buffer` with
            // its length.
            unsafe {
                let ready = libc::poll(&mut poll, 1, READ_TIMEOUT.as_millis() as libc::c_int);
                if ready < 0 {
                    let error = io::Error::last_os_error();
                    return if error.kind() == io::ErrorKind::Interrupted {
                        Ok(0)
                    } else {
                        Err(error)
                    };
                }
                if ready == 0 {
                    return Ok(0);
                }
                if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    return Err(io::Error::other("The serial port closed"));
                }
                let read = libc::read(self.0, buffer.as_mut_ptr().cast(), buffer.len());
                if read < 0 {
                    let error = io::Error::last_os_error();
                    return if error.kind() == io::ErrorKind::WouldBlock {
                        Ok(0)
                    } else {
                        Err(error)
                    };
                }
                Ok((read as usize).min(buffer.len()))
            }
        }

        pub(super) fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
            // SAFETY: writes `bytes` with its length to a valid descriptor.
            let written = unsafe { libc::write(self.0, bytes.as_ptr().cast(), bytes.len()) };
            if written < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    impl Drop for Port {
        fn drop(&mut self) {
            // SAFETY: closes the descriptor this value owns.
            unsafe {
                libc::close(self.0);
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use std::io;

    pub(super) fn candidates(_: &str) -> Vec<(String, bool)> {
        Vec::new()
    }

    pub(super) fn permission_hint(path: &str) -> String {
        format!("Cannot open {path}")
    }

    pub(super) struct Port;

    impl Port {
        pub(super) fn open(_: &str) -> io::Result<Self> {
            Err(io::Error::other("Serial ports are not supported here"))
        }
        pub(super) fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
        pub(super) fn write(&mut self, _: &[u8]) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod group_tests {
    use super::group_name;

    #[test]
    fn finds_the_port_group() {
        let groups = "root:x:0:root\nuucp:x:986:\ndialout:x:20:aurum\n";
        assert_eq!(group_name(groups, 986).as_deref(), Some("uucp"));
        assert_eq!(group_name(groups, 20).as_deref(), Some("dialout"));
        assert_eq!(group_name(groups, 7), None);
    }

    #[test]
    fn ignores_damaged_and_odd_lines() {
        assert_eq!(group_name("", 0), None);
        assert_eq!(group_name("garbage\n::\nname:x\n", 0), None);
        assert_eq!(group_name("bad name:x:5:\n", 5), None);
        assert_eq!(group_name(":x:5:\n", 5), None);
        assert_eq!(group_name("a:x:99999999999:\n", 0), None);
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn rejects_ids_without_an_instance() {
        assert!(super::platform::candidates(r"\\?\root#camera").is_empty());
        assert!(super::platform::candidates("").is_empty());
    }
}
