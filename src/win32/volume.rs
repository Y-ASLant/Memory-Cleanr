//! 通过 Mount Manager 枚举 `\\??\\Volume{GUID}` 并刷写卷缓存（对齐 Mem Reduct 路径）。

use std::collections::HashSet;
use std::mem::size_of;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{GetLastError, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::PCWSTR;

use crate::win32::nt::{close_volume_handle, flush_volume_handle, open_volume_symbolic_link};

const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;

const MOUNTMGR_DOS_DEVICE_NAME: &str = "\\\\.\\MountPointManager";
const MOUNTMGRCONTROLTYPE: u32 = b'm' as u32;
const METHOD_BUFFERED: u32 = 0;
const FILE_ANY_ACCESS: u32 = 0;
const IOCTL_MOUNTMGR_QUERY_POINTS: u32 =
    ctl_code(MOUNTMGRCONTROLTYPE, 2, METHOD_BUFFERED, FILE_ANY_ACCESS);
const INITIAL_MOUNT_POINTS_BUFFER: usize = 16 * 1024;

const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

/// Mount Manager `MOUNTMGR_MOUNT_POINT`（与 `mountmgr.h` 布局一致）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MountMgrMountPoint {
    symbolic_link_name_offset: u32,
    symbolic_link_name_length: u16,
    unique_id_offset: u32,
    unique_id_length: u16,
    device_name_offset: u32,
    device_name_length: u16,
}

/// Mount Manager `MOUNTMGR_MOUNT_POINTS` 头部。
#[repr(C)]
struct MountMgrMountPointsHeader {
    size: u32,
    number_of_mount_points: u32,
}

/// 待刷写的卷目标（用于 UI 进度与日志）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeFlushTarget {
    /// 短标签，例如 `Volume{abc...}`。
    pub label: String,
    /// Mount Manager 返回的 `\??\Volume{GUID}` 符号链接（供 `NtCreateFile` 使用）。
    symbolic_link: Vec<u16>,
}

/// 枚举所有应刷写的卷（仅 `Volume{GUID}` 挂载点，跳过盘符以避免重复刷写）。
pub fn list_volume_flush_targets() -> Result<Vec<VolumeFlushTarget>> {
    let mount_points = query_mount_points()?;
    let mut seen = HashSet::new();
    let mut targets = Vec::new();

    for symbolic_link in mount_points {
        if !is_volume_name_wide(&symbolic_link) {
            continue;
        }

        if !seen.insert(symbolic_link.clone()) {
            continue;
        }

        targets.push(VolumeFlushTarget {
            label: volume_display_label(&symbolic_link),
            symbolic_link,
        });
    }

    targets.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(targets)
}

/// 刷写单个卷的已修改文件缓存。
pub fn flush_volume_cache(target: &VolumeFlushTarget) -> Result<()> {
    let handle = open_volume_symbolic_link(&target.symbolic_link)
        .with_context(|| format!("open volume {}", target.label))?;

    let flush_result = flush_volume_handle(handle);
    close_volume_handle(handle);
    flush_result.with_context(|| format!("flush volume {}", target.label))
}

fn query_mount_points() -> Result<Vec<Vec<u16>>> {
    let mount_mgr = open_mount_manager()?;
    let buffer = query_mount_points_buffer(mount_mgr)?;
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(mount_mgr);
    }
    parse_volume_symbolic_links(&buffer)
}

fn open_mount_manager() -> Result<HANDLE> {
    let wide: Vec<u16> = MOUNTMGR_DOS_DEVICE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_READ_ATTRIBUTES.0 | SYNCHRONIZE_ACCESS,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .context("open MountPointManager")
}

fn query_mount_points_buffer(mount_mgr: HANDLE) -> Result<Vec<u8>> {
    let input = MountMgrMountPoint::default();
    let mut buffer = vec![0u8; INITIAL_MOUNT_POINTS_BUFFER];
    let mut bytes_returned = 0u32;

    let first = unsafe {
        DeviceIoControl(
            mount_mgr,
            IOCTL_MOUNTMGR_QUERY_POINTS,
            Some((&input as *const MountMgrMountPoint).cast()),
            size_of::<MountMgrMountPoint>() as u32,
            Some(buffer.as_mut_ptr().cast()),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if first.is_ok() {
        buffer.truncate(bytes_returned as usize);
        return Ok(buffer);
    }

    let err = unsafe { GetLastError() };
    if err != windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER {
        return Err(first.unwrap_err()).context("IOCTL_MOUNTMGR_QUERY_POINTS failed");
    }

    if bytes_returned == 0 {
        bail!("IOCTL_MOUNTMGR_QUERY_POINTS returned insufficient buffer with zero size");
    }

    buffer.resize(bytes_returned as usize, 0);
    bytes_returned = 0;

    unsafe {
        DeviceIoControl(
            mount_mgr,
            IOCTL_MOUNTMGR_QUERY_POINTS,
            Some((&input as *const MountMgrMountPoint).cast()),
            size_of::<MountMgrMountPoint>() as u32,
            Some(buffer.as_mut_ptr().cast()),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    }
    .context("IOCTL_MOUNTMGR_QUERY_POINTS retry failed")?;

    buffer.truncate(bytes_returned as usize);
    Ok(buffer)
}

fn parse_volume_symbolic_links(buffer: &[u8]) -> Result<Vec<Vec<u16>>> {
    if buffer.len() < size_of::<MountMgrMountPointsHeader>() {
        return Ok(Vec::new());
    }

    let header = read_mount_points_header(buffer)?;
    let mount_point_size = size_of::<MountMgrMountPoint>();
    let entries_end = size_of::<MountMgrMountPointsHeader>()
        .saturating_add(header.number_of_mount_points as usize * mount_point_size);

    if entries_end > buffer.len() {
        bail!("mount points buffer truncated");
    }

    let mut links = Vec::new();
    for index in 0..header.number_of_mount_points as usize {
        let offset = size_of::<MountMgrMountPointsHeader>() + index * mount_point_size;
        let mount_point = read_mount_point(buffer, offset)?;
        if mount_point.symbolic_link_name_length == 0 {
            continue;
        }

        let name_offset = mount_point.symbolic_link_name_offset as usize;
        let name_bytes = mount_point.symbolic_link_name_length as usize;
        let name_end = name_offset.saturating_add(name_bytes);
        if name_end > buffer.len() || !name_bytes.is_multiple_of(2) {
            continue;
        }

        let name_ptr = buffer[name_offset..name_end].as_ptr() as *const u16;
        let char_count = name_bytes / 2;
        let chars = unsafe { std::slice::from_raw_parts(name_ptr, char_count) };
        links.push(chars.to_vec());
    }

    Ok(links)
}

fn read_mount_points_header(buffer: &[u8]) -> Result<MountMgrMountPointsHeader> {
    let header_ptr = buffer.as_ptr() as *const MountMgrMountPointsHeader;
    Ok(unsafe { header_ptr.read_unaligned() })
}

fn read_mount_point(buffer: &[u8], offset: usize) -> Result<MountMgrMountPoint> {
    if offset.saturating_add(size_of::<MountMgrMountPoint>()) > buffer.len() {
        bail!("mount point entry out of range");
    }
    let point_ptr = buffer[offset..].as_ptr() as *const MountMgrMountPoint;
    Ok(unsafe { point_ptr.read_unaligned() })
}

/// `MOUNTMGR_IS_VOLUME_NAME` 的 Rust 实现（`mountmgr.h`）。
fn is_volume_name_wide(chars: &[u16]) -> bool {
    let len_bytes = chars.len() * 2;
    let len_ok = len_bytes == 96 || (len_bytes == 98 && chars.get(48) == Some(&(b'\\' as u16)));
    if !len_ok || chars.len() < 48 {
        return false;
    }

    chars[0] == b'\\' as u16
        && (chars[1] == b'?' as u16 || chars[1] == b'\\' as u16)
        && chars[2] == b'?' as u16
        && chars[3] == b'\\' as u16
        && chars[4] == b'V' as u16
        && chars[5] == b'o' as u16
        && chars[6] == b'l' as u16
        && chars[7] == b'u' as u16
        && chars[8] == b'm' as u16
        && chars[9] == b'e' as u16
        && chars[10] == b'{' as u16
        && chars[19] == b'-' as u16
        && chars[24] == b'-' as u16
        && chars[29] == b'-' as u16
        && chars[34] == b'-' as u16
        && chars[47] == b'}' as u16
}

fn wide_to_string(chars: &[u16]) -> String {
    String::from_utf16_lossy(chars)
}

fn volume_display_label(symbolic_link: &[u16]) -> String {
    let text = wide_to_string(symbolic_link);
    text.strip_prefix("\\??\\")
        .or_else(|| text.strip_prefix("\\\\?\\"))
        .unwrap_or(text.as_str())
        .trim_end_matches('\\')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume_guid_wide() -> Vec<u16> {
        "\\??\\Volume{11111111-2222-3333-4444-555555555555}"
            .encode_utf16()
            .collect()
    }

    #[test]
    fn mount_mgr_mount_point_has_expected_size() {
        assert_eq!(size_of::<MountMgrMountPoint>(), 24);
    }

    #[test]
    fn is_volume_name_matches_mountmgr_macro() {
        assert!(is_volume_name_wide(&volume_guid_wide()));
        assert!(!is_volume_name_wide(
            &"C:\\".encode_utf16().collect::<Vec<_>>()
        ));
    }

    #[test]
    fn volume_display_label_strips_prefix() {
        let label = volume_display_label(&volume_guid_wide());
        assert_eq!(label, "Volume{11111111-2222-3333-4444-555555555555}");
    }

    #[test]
    fn volume_target_keeps_mountmgr_symbolic_link() {
        let link = volume_guid_wide();
        let target = VolumeFlushTarget {
            label: volume_display_label(&link),
            symbolic_link: link.clone(),
        };
        assert_eq!(target.symbolic_link, link);
    }
}
