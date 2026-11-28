//! Desktop services for Linux and macOS.

use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

pub fn attach_parent_console() {}

pub fn pick_folder(start: &str) -> Option<String> {
    let mut dialog = rfd::FileDialog::new().set_title("Choose Storage Path");
    if Path::new(start).is_dir() {
        dialog = dialog.set_directory(start);
    }
    dialog.pick_folder()?.into_os_string().into_string().ok()
}

/// The file picker for a PNG or JPEG picture (the portal on Linux).
pub fn pick_image(start: &str) -> Option<String> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Add a Sleep Background picture")
        .add_filter("Pictures (PNG, JPEG)", &["png", "jpg", "jpeg"]);
    if Path::new(start).is_dir() {
        dialog = dialog.set_directory(start);
    }
    dialog.pick_file()?.into_os_string().into_string().ok()
}

fn open(path: &str) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(opener)
        .arg(path)
        .spawn()
        .map_err(|e| format!("Could not start {opener}: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn open_url(url: &str) {
    let _ = open(url);
}

pub fn open_folder(path: &str) -> Result<(), String> {
    let absolute = Path::new(path)
        .canonicalize()
        .map_err(|e| format!("Could not open {path}: {e}"))?;
    let absolute = absolute
        .to_str()
        .ok_or("The folder path is not valid UTF-8")?;
    open(absolute)
}

pub fn local_time() -> [u16; 6] {
    let t = jiff::Zoned::now();
    [
        t.year() as u16,
        t.month() as u16,
        t.day() as u16,
        t.hour() as u16,
        t.minute() as u16,
        t.second() as u16,
    ]
}

#[cfg(target_os = "linux")]
fn login_item() -> Option<PathBuf> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))?;
    Some(home.join("autostart/gimbly.desktop"))
}

#[cfg(target_os = "linux")]
fn desktop_entry(exe: &Path) -> Result<String, String> {
    let path = exe
        .to_str()
        .filter(|p| !p.chars().any(char::is_control))
        .ok_or("The executable path cannot be written to a desktop entry")?;
    // Escape the Exec argument, then the desktop entry string containing it.
    let mut command = String::from("\"");
    for c in path.chars() {
        match c {
            '\\' | '"' | '`' | '$' => command.push('\\'),
            '%' => command.push('%'),
            _ => {}
        }
        command.push(c);
    }
    command.push('"');
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=Gimbly\nExec={}\n",
        command.replace('\\', "\\\\")
    ))
}

#[cfg(target_os = "linux")]
pub fn starts_automatically() -> bool {
    login_item().is_some_and(|p| p.is_file())
}

#[cfg(target_os = "linux")]
pub fn set_starts_automatically(on: bool) -> Result<(), String> {
    let entry = login_item().ok_or("No user configuration directory is available")?;
    if !on {
        return match std::fs::remove_file(entry) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let parent = entry.parent().ok_or("Invalid autostart directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let content = desktop_entry(&exe)?;
    let temporary = entry.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, content).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, &entry).map_err(|e| {
        let _ = std::fs::remove_file(temporary);
        e.to_string()
    })
}

#[cfg(target_os = "macos")]
fn login_item() -> Result<objc2::rc::Retained<objc2_service_management::SMAppService>, String> {
    use objc2::{msg_send, runtime::AnyClass};
    use objc2_foundation::NSBundle;
    let class = AnyClass::get(c"SMAppService").ok_or("Login items require macOS 13 or later")?;
    let is_app = NSBundle::mainBundle()
        .bundlePath()
        .to_string()
        .ends_with(".app");
    if !is_app {
        return Err("Launch Gimbly.app to manage its login item".into());
    }
    // SAFETY: SMAppService is available, as checked above.
    Ok(unsafe { msg_send![class, mainAppService] })
}

#[cfg(target_os = "macos")]
pub fn starts_automatically() -> bool {
    use objc2_service_management::SMAppServiceStatus;
    // SAFETY: status is a read-only query on the main app service.
    login_item().is_ok_and(|item| unsafe { item.status() == SMAppServiceStatus::Enabled })
}

#[cfg(target_os = "macos")]
pub fn set_starts_automatically(on: bool) -> Result<(), String> {
    use objc2_service_management::SMAppServiceStatus;
    let item = login_item()?;
    // SAFETY: this changes only the login item for the current app bundle.
    unsafe {
        if on {
            if item.status() != SMAppServiceStatus::RequiresApproval {
                item.registerAndReturnError().map_err(|e| e.to_string())?;
            }
            if item.status() == SMAppServiceStatus::RequiresApproval {
                return Err("Allow Gimbly under System Settings → General → Login Items.".into());
            }
        } else {
            item.unregisterAndReturnError().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_quotes_executable_and_literal_percent() {
        let entry = desktop_entry(Path::new("/a b/gimbly%f")).unwrap();
        assert!(entry.contains("Exec=\"/a b/gimbly%%f\"\n"));
        assert!(desktop_entry(Path::new("/a\nExec=other")).is_err());
    }
}
