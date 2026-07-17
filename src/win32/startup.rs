//! Current-user autostart via `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run`.

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS,
    REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW,
    RegSetValueExW,
};
use windows::core::{Error, PCWSTR};

use crate::settings::Settings;
use crate::version::PROCESS_BASE_NAME;

const RUN_KEY_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Registry / CLI flag for silent login autostart (tray only, no main window).
pub const STARTUP_ARG: &str = "--startup";

pub fn is_startup_launch() -> bool {
    std::env::args().any(|arg| arg == STARTUP_ARG)
}

/// Args passed to the elevated child so startup mode survives UAC relaunch.
pub fn elevation_relaunch_args() -> String {
    elevation_relaunch_args_for(is_startup_launch())
}

pub fn elevation_relaunch_args_for(is_startup_launch: bool) -> String {
    if is_startup_launch {
        format!("{ELEVATED_ARG} {STARTUP_ARG}")
    } else {
        ELEVATED_ARG.to_string()
    }
}

const ELEVATED_ARG: &str = "--elevated";

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn win32_ok(status: windows::Win32::Foundation::WIN32_ERROR) -> Result<()> {
    if status.is_ok() {
        Ok(())
    } else {
        Err(Error::from(status).into())
    }
}

/// Command line written to the Run key (quoted when the path contains spaces).
pub fn startup_command() -> Result<String> {
    let exe = std::env::current_exe().context("current_exe unavailable")?;
    Ok(format_exe_launch_command(
        &exe.display().to_string(),
        &[STARTUP_ARG],
    ))
}

/// Build `"C:\Path With Spaces\app.exe" --arg` style command lines.
pub fn format_exe_launch_command(exe_path: &str, args: &[&str]) -> String {
    let exe_part = if exe_path.contains(' ') {
        format!("\"{exe_path}\"")
    } else {
        exe_path.to_string()
    };
    if args.is_empty() {
        exe_part
    } else {
        format!("{exe_part} {}", args.join(" "))
    }
}

pub fn is_enabled() -> bool {
    read_value().is_ok()
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    if enabled {
        write_value(&startup_command()?)
    } else {
        delete_value()
    }
}

pub fn sync(settings: &Settings) -> Result<()> {
    set_enabled(settings.run_at_startup)
}

fn read_value() -> Result<String> {
    let subkey = wide_null(RUN_KEY_PATH);
    let value_name = wide_null(PROCESS_BASE_NAME);

    let mut byte_len = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut byte_len),
        )
    };
    if !status.is_ok() || byte_len < size_of::<u16>() as u32 {
        bail!("startup registry value missing");
    }

    let mut buffer = vec![0u8; byte_len as usize];
    win32_ok(unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut byte_len),
        )
    })?;

    let wide_len = (byte_len as usize / size_of::<u16>()).saturating_sub(1);
    let wide: &[u16] =
        unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u16>(), wide_len) };
    Ok(String::from_utf16_lossy(wide))
}

fn open_run_key(access: REG_SAM_FLAGS) -> Result<HKEY> {
    let subkey = wide_null(RUN_KEY_PATH);
    let mut key = HKEY::default();
    win32_ok(unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            access,
            None,
            &mut key,
            None,
        )
    })?;
    Ok(key)
}

fn write_value(command: &str) -> Result<()> {
    let key = open_run_key(KEY_SET_VALUE | KEY_WRITE)?;
    let value_name = wide_null(PROCESS_BASE_NAME);
    let data = wide_null(command);
    let bytes: Vec<u8> = data.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let result = win32_ok(unsafe {
        RegSetValueExW(key, PCWSTR(value_name.as_ptr()), None, REG_SZ, Some(&bytes))
    });
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

fn delete_value() -> Result<()> {
    let key = open_run_key(KEY_SET_VALUE | KEY_WRITE)?;
    let value_name = wide_null(PROCESS_BASE_NAME);
    let status = unsafe { RegDeleteValueW(key, PCWSTR(value_name.as_ptr())) };
    let result = if status.is_ok() || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(Error::from(status).into())
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_includes_startup_flag() {
        let command = startup_command().expect("current_exe");
        assert!(command.ends_with(STARTUP_ARG));
        let exe = std::env::current_exe().expect("current_exe");
        assert_eq!(
            command,
            format_exe_launch_command(&exe.display().to_string(), &[STARTUP_ARG])
        );
    }

    #[test]
    fn format_exe_launch_command_quotes_paths_with_spaces() {
        assert_eq!(
            format_exe_launch_command(r"C:\Program Files\App.exe", &["--startup"]),
            r#""C:\Program Files\App.exe" --startup"#
        );
        assert_eq!(
            format_exe_launch_command(r"C:\App.exe", &["--startup"]),
            r"C:\App.exe --startup"
        );
        assert_eq!(format_exe_launch_command(r"C:\App.exe", &[]), r"C:\App.exe");
    }

    #[test]
    fn elevation_relaunch_args_for_startup_mode() {
        assert_eq!(
            elevation_relaunch_args_for(true),
            format!("{ELEVATED_ARG} {STARTUP_ARG}")
        );
        assert_eq!(elevation_relaunch_args_for(false), ELEVATED_ARG);
    }
}
