//! The virtual camera's preferred format, stored per user in the registry.
//!
//! The app writes it when the output resolution changes; the DirectShow
//! filter reads it when an application creates the camera, which can happen
//! while Gimbly is not running.

use crate::PreferredFormat;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, RRF_RT_REG_DWORD,
    RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR};

const KEY: &str = "Software\\Gimbly\\VirtualCamera";

fn read_dword(name: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: valid key/value names and a DWORD-sized output buffer.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(KEY),
            &HSTRING::from(name),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    result.is_ok().then_some(value)
}

/// The stored format, if present and valid.
pub fn read_preferred_format() -> Option<PreferredFormat> {
    PreferredFormat::validated(
        read_dword("Width")?,
        read_dword("Height")?,
        read_dword("Fps")?,
    )
}

pub fn write_preferred_format(format: PreferredFormat) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    // SAFETY: creates/opens our own key and closes it before returning.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(KEY),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()?;
        let mut result = Ok(());
        for (name, value) in [
            ("Width", format.width),
            ("Height", format.height),
            ("Fps", format.fps),
        ] {
            result = RegSetValueExW(
                key,
                &HSTRING::from(name),
                None,
                REG_DWORD,
                Some(&value.to_le_bytes()),
            )
            .ok();
            if result.is_err() {
                break;
            }
        }
        let _ = RegCloseKey(key);
        result
    }
}
