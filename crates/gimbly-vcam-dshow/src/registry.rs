//! Per-user COM and DirectShow registration of the virtual camera.
//!
//! Everything goes under `HKEY_CURRENT_USER\Software\Classes`, which Windows
//! merges into `HKEY_CLASSES_ROOT` for the user's processes, so no
//! administrator rights are needed.

use gimbly_vcam_shm::FRIENDLY_NAME;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows_core::{GUID, HSTRING, PCWSTR, Result};

/// DirectShow category of video capture devices.
const VIDEO_INPUT_CATEGORY: &str = "{860BB310-5D01-11D0-BD3B-00A0C911CE86}";

pub fn guid_string(g: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

fn check(err: WIN32_ERROR) -> Result<()> {
    err.ok()
}

fn set_string(path: &str, name: Option<&str>, value: &str) -> Result<()> {
    let mut key = HKEY::default();
    // SAFETY: valid key path and out-pointer; the key is closed below.
    unsafe {
        check(RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        ))?;
        let data: Vec<u8> = value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        let name = name.map(HSTRING::from);
        let result = match &name {
            Some(n) => RegSetValueExW(key, n, None, REG_SZ, Some(&data)),
            None => RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(&data)),
        };
        let _ = RegCloseKey(key);
        check(result)
    }
}

fn delete_tree(path: &str) -> Result<()> {
    // SAFETY: deletes a key we created; a missing key is not an error.
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(path)) };
    if result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        check(result)
    }
}

fn paths(clsid: &str) -> (String, String) {
    (
        format!("Software\\Classes\\CLSID\\{clsid}"),
        format!("Software\\Classes\\CLSID\\{VIDEO_INPUT_CATEGORY}\\Instance\\{clsid}"),
    )
}

pub fn register(clsid: &GUID, module_path: &str) -> Result<()> {
    let clsid = guid_string(clsid);
    let (class_key, instance_key) = paths(&clsid);
    set_string(&class_key, None, FRIENDLY_NAME)?;
    let server_key = format!("{class_key}\\InprocServer32");
    set_string(&server_key, None, module_path)?;
    set_string(&server_key, Some("ThreadingModel"), "Both")?;
    set_string(&instance_key, Some("CLSID"), &clsid)?;
    set_string(&instance_key, Some("FriendlyName"), FRIENDLY_NAME)?;
    Ok(())
}

pub fn unregister(clsid: &GUID) -> Result<()> {
    let (class_key, instance_key) = paths(&guid_string(clsid));
    delete_tree(&instance_key)?;
    delete_tree(&class_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_guids_like_the_registry() {
        let g = GUID::from_u128(0x860BB310_5D01_11D0_BD3B_00A0C911CE86);
        assert_eq!(guid_string(&g), VIDEO_INPUT_CATEGORY);
    }
}
