use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    TOKEN_ACCESS_MASK, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_PRIVILEGES_ATTRIBUTES,
    TOKEN_QUERY,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

fn with_process_token<T>(
    access: TOKEN_ACCESS_MASK,
    f: impl FnOnce(HANDLE) -> Result<T>,
) -> Result<T> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), access, &mut token)
            .context("OpenProcessToken failed")?;
        let result = f(token);
        let _ = CloseHandle(token);
        result
    }
}

pub fn enable_privilege(name: &str) -> Result<()> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

    with_process_token(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, |token| unsafe {
        let mut luid = LUID::default();
        LookupPrivilegeValueW(PCWSTR::null(), PCWSTR(wide.as_ptr()), &mut luid)
            .context(format!("LookupPrivilegeValue failed for {name}"))?;

        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: TOKEN_PRIVILEGES_ATTRIBUTES(SE_PRIVILEGE_ENABLED.0),
            }],
        };

        AdjustTokenPrivileges(token, false, Some(&tp as *const _), 0, None, None)
            .context("AdjustTokenPrivileges failed")?;

        if GetLastError() != ERROR_SUCCESS {
            bail!("AdjustTokenPrivileges: privilege not held by token ({name})");
        }

        Ok(())
    })
}
