//! Disable/enable the system Win+V hotkey via registry.
//!
//! The system clipboard history (Win+V) is controlled by the `DisabledHotkeys`
//! registry value under `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced`.
//! Adding 'V' disables it; removing 'V' re-enables it. Explorer must be restarted
//! for the change to take effect.

use anyhow::Result;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ};

const EXPLORER_ADVANCED: windows::core::PCWSTR =
    windows::core::w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced");
const DISABLED_HOTKEYS: windows::core::PCWSTR = windows::core::w!("DisabledHotkeys");

/// Disable system Win+V by adding 'V' to the DisabledHotkeys registry value.
/// Returns Ok(true) if the value was changed, Ok(false) if already disabled.
pub fn disable_win_v() -> Result<bool> {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW};

    unsafe {
        let mut key = Default::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            EXPLORER_ADVANCED,
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut key,
        );
        if status != ERROR_SUCCESS {
            anyhow::bail!("RegOpenKeyExW failed: {status:?}");
        }

        // Read current value
        let current = read_reg_string(key, DISABLED_HOTKEYS);

        if current.contains('V') {
            let _ = RegCloseKey(key);
            return Ok(false);
        }

        // Append 'V'
        let new_val = if current.is_empty() {
            "V".to_string()
        } else {
            format!("{current}V")
        };

        let wide: Vec<u16> = new_val.encode_utf16().chain(std::iter::once(0)).collect();
        let status = RegSetValueExW(
            key,
            DISABLED_HOTKEYS,
            Some(0),
            REG_SZ,
            Some(std::slice::from_raw_parts(
                wide.as_ptr() as *const u8,
                wide.len() * 2,
            )),
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            anyhow::bail!("RegSetValueExW failed: {status:?}");
        }
        Ok(true)
    }
}

/// Enable system Win+V by removing 'V' from the DisabledHotkeys registry value.
/// Returns Ok(true) if the value was changed, Ok(false) if already enabled.
pub fn enable_win_v() -> Result<bool> {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW};

    unsafe {
        let mut key = Default::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            EXPLORER_ADVANCED,
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut key,
        );
        if status != ERROR_SUCCESS {
            anyhow::bail!("RegOpenKeyExW failed: {status:?}");
        }

        let current = read_reg_string(key, DISABLED_HOTKEYS);

        if !current.contains('V') {
            let _ = RegCloseKey(key);
            return Ok(false);
        }

        let new_val: String = current.chars().filter(|&c| c != 'V').collect();

        if new_val.is_empty() {
            // Delete the value entirely
            use windows::Win32::System::Registry::RegDeleteValueW;
            let _ = RegDeleteValueW(key, DISABLED_HOTKEYS);
        } else {
            let wide: Vec<u16> = new_val.encode_utf16().chain(std::iter::once(0)).collect();
            let status = RegSetValueExW(
                key,
                DISABLED_HOTKEYS,
                Some(0),
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    wide.as_ptr() as *const u8,
                    wide.len() * 2,
                )),
            );
            if status != ERROR_SUCCESS {
                let _ = RegCloseKey(key);
                anyhow::bail!("RegSetValueExW failed: {status:?}");
            }
        }

        let _ = RegCloseKey(key);
        Ok(true)
    }
}

/// Check if system Win+V is currently disabled.
pub fn is_win_v_disabled() -> bool {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW};

    unsafe {
        let mut key = Default::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            EXPLORER_ADVANCED,
            Some(0),
            KEY_READ,
            &mut key,
        );
        if status != ERROR_SUCCESS {
            return false;
        }
        let val = read_reg_string(key, DISABLED_HOTKEYS);
        let _ = RegCloseKey(key);
        val.contains('V')
    }
}

unsafe fn read_reg_string(
    key: windows::Win32::System::Registry::HKEY,
    value_name: windows::core::PCWSTR,
) -> String {
    use windows::Win32::System::Registry::{REG_VALUE_TYPE, RegQueryValueExW};

    let mut buf = [0u16; 256];
    let mut buf_len = (buf.len() * 2) as u32;
    let mut value_type = REG_VALUE_TYPE(0);

    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name,
            None,
            Some(&mut value_type),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_len),
        )
    };

    if status != ERROR_SUCCESS || value_type != REG_SZ {
        return String::new();
    }

    let chars = buf_len as usize / 2;
    let len = buf[..chars].iter().position(|&c| c == 0).unwrap_or(chars);
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_current_state() {
        // Just verify it doesn't panic
        let _disabled = is_win_v_disabled();
    }
}
