//! 通过 Mount Manager 枚举 `\\??\\Volume{GUID}` 并刷写卷缓存（对齐 Mem Reduct 路径）。

use std::collections::HashSet;
use std::mem::size_of;

use anyhow::{Context, Result, bail};
use rust_i18n::t;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::PCWSTR;

use crate::log;
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

/// RAII：保证 Mount Manager 句柄在任何路径下都会关闭。
struct MountManagerHandle(HANDLE);

impl MountManagerHandle {
    fn open() -> Result<Self> {
        open_mount_manager().map(Self)
    }

    fn handle(&self) -> HANDLE {
        self.0
    }
}

impl Drop for MountManagerHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// 待刷写的卷目标。
#[derive(Clone, Debug, PartialEq, Eq)]
struct VolumeFlushTarget {
    label: String,
    symbolic_link: Vec<u16>,
}

/// 一次 Mount Manager 枚举结果，可在 UI 线程间共享并逐卷刷写。
#[derive(Debug)]
pub struct VolumeFlushSession {
    targets: Vec<VolumeFlushTarget>,
}

impl VolumeFlushSession {
    /// 枚举当前系统中所有应刷写的卷。
    pub fn open() -> Result<Self> {
        Ok(Self {
            targets: collect_volume_targets()?,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }

    pub fn label(&self, index: usize) -> &str {
        &self.targets[index].label
    }

    pub fn flush(&self, index: usize) -> Result<()> {
        flush_volume_cache(&self.targets[index])
    }
}

/// 批量刷写卷缓存的结果摘要。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeFlushReport {
    pub succeeded: usize,
    pub failed: Vec<String>,
}

impl VolumeFlushReport {
    pub fn is_success(&self) -> bool {
        self.succeeded > 0 || self.failed.is_empty()
    }

    pub fn record(&mut self, label: &str, result: Result<()>) {
        match result {
            Ok(()) => self.succeeded += 1,
            Err(error) => {
                log_volume_failure(label, &error);
                self.failed.push(label.to_string());
            }
        }
    }
}

/// 枚举卷标签列表（供 UI 展示进度时使用）。
pub fn list_volume_flush_targets() -> Result<Vec<String>> {
    let session = VolumeFlushSession::open()?;
    Ok((0..session.len())
        .map(|index| session.label(index).to_string())
        .collect())
}

/// 刷写 `session` 中的全部卷，并在每卷开始前调用 `on_progress(current, total, label)`。
pub fn flush_volume_session(
    session: &VolumeFlushSession,
    mut on_progress: impl FnMut(usize, usize, &str),
) -> VolumeFlushReport {
    let mut report = VolumeFlushReport::default();
    let total = session.len();

    for index in 0..total {
        let label = session.label(index);
        on_progress(index + 1, total, label);
        report.record(label, session.flush(index));
    }

    report
}

/// 打开 session 并刷写全部卷（无 UI 进度回调）。
pub fn flush_all_volume_caches() -> Result<VolumeFlushReport> {
    let session = VolumeFlushSession::open()?;
    Ok(flush_volume_session(&session, |_, _, _| {}))
}

/// 将刷写报告转为步骤结果：至少 1 个卷成功，或无卷可刷时视为成功。
pub fn complete_volume_flush(report: VolumeFlushReport) -> Result<()> {
    log_volume_flush_summary(&report);

    if report.is_success() {
        Ok(())
    } else {
        let volumes = report.failed.join(", ");
        bail!(t!("optimize.volume_flush_failed", volumes = volumes))
    }
}

fn collect_volume_targets() -> Result<Vec<VolumeFlushTarget>> {
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

fn flush_volume_cache(target: &VolumeFlushTarget) -> Result<()> {
    let handle = open_volume_symbolic_link(&target.symbolic_link)
        .with_context(|| format!("open volume {}", target.label))?;

    let flush_result = flush_volume_handle(handle);
    close_volume_handle(handle);
    flush_result.with_context(|| format!("flush volume {}", target.label))
}

fn log_volume_failure(label: &str, error: &anyhow::Error) {
    log::write(&format!(
        "[optimize] modified file cache volume {label}: failed: {error:#}"
    ));
}

fn log_volume_flush_summary(report: &VolumeFlushReport) {
    if report.failed.is_empty() {
        return;
    }

    let failed = report.failed.join(", ");
    if report.succeeded > 0 {
        log::write(&format!(
            "[optimize] modified file cache partial success: {} succeeded, {} failed ({failed})",
            report.succeeded,
            report.failed.len()
        ));
    } else {
        log::write(&format!(
            "[optimize] modified file cache all volumes failed ({failed})"
        ));
    }
}

fn query_mount_points() -> Result<Vec<Vec<u16>>> {
    let mount_mgr = MountManagerHandle::open()?;
    let buffer = query_mount_points_buffer(mount_mgr.handle())?;
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
    let declared_size = header.size as usize;
    if declared_size < size_of::<MountMgrMountPointsHeader>() || declared_size > buffer.len() {
        bail!("mount points buffer size mismatch");
    }

    let mount_point_size = size_of::<MountMgrMountPoint>();
    let entries_end = size_of::<MountMgrMountPointsHeader>()
        .saturating_add(header.number_of_mount_points as usize * mount_point_size);

    if entries_end > declared_size {
        bail!("mount points buffer truncated");
    }

    let mut links = Vec::new();
    for index in 0..header.number_of_mount_points as usize {
        let offset = size_of::<MountMgrMountPointsHeader>() + index * mount_point_size;
        let mount_point = read_mount_point(buffer, offset, declared_size)?;
        if mount_point.symbolic_link_name_length == 0 {
            continue;
        }

        let name_offset = mount_point.symbolic_link_name_offset as usize;
        let name_bytes = mount_point.symbolic_link_name_length as usize;
        let name_end = name_offset.saturating_add(name_bytes);
        if name_end > declared_size || !name_bytes.is_multiple_of(2) {
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

fn read_mount_point(buffer: &[u8], offset: usize, limit: usize) -> Result<MountMgrMountPoint> {
    if offset.saturating_add(size_of::<MountMgrMountPoint>()) > limit {
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

    fn sample_mount_points_buffer() -> Vec<u8> {
        let link = volume_guid_wide();
        let link_bytes = link.len() * 2;
        let header_size = size_of::<MountMgrMountPointsHeader>();
        let entry_size = size_of::<MountMgrMountPoint>();
        let link_offset = header_size + entry_size;
        let total_size = link_offset + link_bytes;

        let mut buffer = vec![0u8; total_size];
        buffer[..4].copy_from_slice(&(total_size as u32).to_le_bytes());
        buffer[4..8].copy_from_slice(&1u32.to_le_bytes());

        let entry_offset = header_size;
        buffer[entry_offset..entry_offset + 4].copy_from_slice(&(link_offset as u32).to_le_bytes());
        buffer[entry_offset + 4..entry_offset + 6]
            .copy_from_slice(&(link_bytes as u16).to_le_bytes());

        for (index, chunk) in link.iter().enumerate() {
            let start = link_offset + index * 2;
            buffer[start..start + 2].copy_from_slice(&chunk.to_le_bytes());
        }

        buffer
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
    fn parse_mount_points_honors_declared_size() {
        let links = parse_volume_symbolic_links(&sample_mount_points_buffer()).expect("parse");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0], volume_guid_wide());
    }

    #[test]
    fn parse_mount_points_rejects_inconsistent_size() {
        let mut buffer = sample_mount_points_buffer();
        let inflated_size = (buffer.len() as u32).saturating_add(8);
        buffer[..4].copy_from_slice(&inflated_size.to_le_bytes());
        assert!(parse_volume_symbolic_links(&buffer).is_err());
    }

    #[test]
    fn volume_flush_report_success_rules() {
        assert!(
            VolumeFlushReport {
                succeeded: 1,
                failed: vec!["Volume{x}".into()],
            }
            .is_success()
        );
        assert!(VolumeFlushReport::default().is_success());
        assert!(
            !VolumeFlushReport {
                succeeded: 0,
                failed: vec!["Volume{x}".into()],
            }
            .is_success()
        );
    }

    #[test]
    fn volume_flush_report_record_tracks_failures() {
        let mut report = VolumeFlushReport::default();
        report.record("Volume{a}", Ok(()));
        report.record("Volume{b}", Err(anyhow::anyhow!("boom")));
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, vec!["Volume{b}".to_string()]);
    }
}
