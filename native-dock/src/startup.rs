//! Per-user login startup for the portable native edition, without Python.
use std::{
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*};
pub fn set_enabled(root: &Path, enabled: bool) -> Result<(), String> {
    let command = format!("\"{}\"", root.join("Lumen.exe").display());
    let path = crate::model::wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = crate::model::wide("Lumen");
    let mut key = null_mut();
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        );
        if status != 0 {
            return Err(format!("Cannot open login startup settings: {status}"));
        }
        let status = if enabled {
            let value = crate::model::wide(command);
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as _,
                (value.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, name.as_ptr())
        };
        RegCloseKey(key);
        if status != 0 && !(status == ERROR_FILE_NOT_FOUND && !enabled) {
            return Err(format!("Cannot change login startup: {status}"));
        }
    }
    Ok(())
}
