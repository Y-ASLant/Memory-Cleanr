use anyhow::{Result, bail};
use windows::core::HRESULT;

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum InfoClass {
    FileCache = 21,
    MemoryList = 80,
    CombinePhysicalMemory = 130,
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum SystemMemoryListCommand {
    EmptyWorkingSets = 2,
    FlushModifiedList = 3,
    PurgeStandbyList = 4,
    PurgeLowPriorityStandbyList = 5,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SystemFileCacheInformation64 {
    pub current_size: u64,
    pub peak_size: u64,
    pub page_fault_count: u64,
    pub minimum_working_set: i64,
    pub maximum_working_set: i64,
    pub current_size_in_pages: u64,
    pub peak_size_in_pages: u64,
    pub minimum_working_set_size: u64,
    pub maximum_working_set_size: u64,
    pub unused1: u64,
    pub unused2: u64,
    pub unused3: u64,
    pub unused4: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MemoryCombineInformationEx {
    pub handle: usize,
    pub pages_combined: u32,
    pub flags: u32,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetSystemInformation(
        system_information_class: u32,
        system_information: *mut core::ffi::c_void,
        system_information_length: u32,
    ) -> i32;
}

/// # Safety
///
/// `info` must point to valid memory of at least `len` bytes for `class`.
pub unsafe fn nt_set_system_information(
    class: InfoClass,
    info: *mut core::ffi::c_void,
    len: u32,
) -> Result<()> {
    let status = unsafe { NtSetSystemInformation(class as u32, info, len) };

    if status == 0 {
        Ok(())
    } else {
        let hr = HRESULT::from_win32(status as u32);
        bail!("NTSTATUS 0x{status:08X} ({hr})");
    }
}
