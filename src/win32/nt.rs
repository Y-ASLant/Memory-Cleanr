use std::mem::size_of;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{CloseHandle, HANDLE};

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
    pub current_size: usize,
    pub peak_size: usize,
    pub page_fault_count: u32, // ULONG — must be 4 bytes, not 8
    pub _pad: u32,             // explicit padding to align the following SIZE_T fields
    pub minimum_working_set: usize,
    pub maximum_working_set: usize,
    pub current_size_in_pages: usize,
    pub peak_size_in_pages: usize,
    pub minimum_working_set_size: usize,
    pub maximum_working_set_size: usize,
    pub unused1: u32,
    pub unused2: u32,
    pub unused3: u32,
    pub unused4: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MemoryCombineInformationEx {
    pub handle: usize,
    pub pages_combined: u32,
    pub flags: u32,
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut core::ffi::c_void,
    security_qos: *mut core::ffi::c_void,
}

#[repr(C)]
union IoStatusBlockStatus {
    status: i32,
    pointer: *mut core::ffi::c_void,
}

#[repr(C)]
struct IoStatusBlock {
    status: IoStatusBlockStatus,
    information: usize,
}

const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
const FILE_OPEN: u32 = 1;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
const FILE_WRITE_DATA: u32 = 0x0000_0002;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetSystemInformation(
        system_information_class: u32,
        system_information: *mut core::ffi::c_void,
        system_information_length: u32,
    ) -> i32;

    fn NtCreateFile(
        file_handle: *mut HANDLE,
        desired_access: u32,
        object_attributes: *const ObjectAttributes,
        io_status_block: *mut IoStatusBlock,
        allocation_size: *mut i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *mut core::ffi::c_void,
        ea_length: u32,
    ) -> i32;

    fn NtFlushBuffersFile(file_handle: HANDLE, io_status_block: *mut IoStatusBlock) -> i32;
}

fn nt_status_to_result(status: i32, context: &str) -> Result<()> {
    if status >= 0 {
        Ok(())
    } else {
        bail!("{context}: NTSTATUS 0x{status:08X}");
    }
}

/// 使用 Mount Manager 返回的 `\??\Volume{GUID}` 符号链接打开卷（Mem Reduct 同款路径）。
pub fn open_volume_symbolic_link(symbolic_link: &[u16]) -> Result<HANDLE> {
    if symbolic_link.is_empty() {
        bail!("open volume: empty symbolic link");
    }

    let byte_len = symbolic_link
        .len()
        .checked_mul(2)
        .context("open volume: symbolic link length overflow")?;

    let mut name = UnicodeString {
        length: byte_len as u16,
        maximum_length: byte_len as u16,
        buffer: symbolic_link.as_ptr().cast_mut(),
    };

    let object_attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: HANDLE::default(),
        object_name: &mut name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: std::ptr::null_mut(),
        security_qos: std::ptr::null_mut(),
    };

    let mut io_status = IoStatusBlock {
        status: IoStatusBlockStatus { status: 0 },
        information: 0,
    };
    let mut handle = HANDLE::default();

    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_WRITE_DATA | SYNCHRONIZE,
            &object_attributes,
            &mut io_status,
            std::ptr::null_mut(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null_mut(),
            0,
        )
    };

    nt_status_to_result(status, "NtCreateFile volume")?;
    Ok(handle)
}

/// 刷写卷缓存（`NtFlushBuffersFile`）。
pub fn flush_volume_handle(handle: HANDLE) -> Result<()> {
    let mut io_status = IoStatusBlock {
        status: IoStatusBlockStatus { status: 0 },
        information: 0,
    };

    let status = unsafe { NtFlushBuffersFile(handle, &mut io_status) };
    nt_status_to_result(status, "NtFlushBuffersFile")?;
    Ok(())
}

/// 关闭 `NtCreateFile` 返回的句柄。
pub fn close_volume_handle(handle: HANDLE) {
    unsafe {
        let _ = CloseHandle(handle);
    }
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
        bail!("NTSTATUS 0x{status:08X}");
    }
}
