/// Ensure only one instance of the application is running.
#[cfg(target_os = "windows")]
pub fn ensure_single_instance() -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let mutex_name: Vec<u16> = "MemoryCleanr_{B8F3A7E2-4C1D-4F5A-9B6E-2D8C3F7A1E9B}"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let _handle = CreateMutexW(None, true, windows::core::PCWSTR(mutex_name.as_ptr()));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Err("Application is already running".into());
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_single_instance() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
