use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, GetLastError};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, FlushFileBuffers,
    OPEN_EXISTING,
};
use windows::Win32::System::Memory::SetSystemFileCacheSize;

use crate::privileges::enable_privilege;
use crate::win32::nt::{
    InfoClass, MemoryCombineInformationEx, SystemFileCacheInformation64, SystemMemoryListCommand,
    nt_set_system_information,
};

type OptimizeFn = fn() -> Result<()>;
pub type OptimizeStepFn = OptimizeFn;
type StepPlan = Vec<(&'static str, OptimizeFn)>;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct MemoryAreas: u32 {
        const WORKING_SET               = 1 << 0;
        const SYSTEM_FILE_CACHE         = 1 << 1;
        const MODIFIED_PAGE_LIST        = 1 << 2;
        const STANDBY_LIST              = 1 << 3;
        const STANDBY_LIST_LOW_PRIORITY = 1 << 4;
        const COMBINED_PAGE_LIST        = 1 << 5;
        const MODIFIED_FILE_CACHE       = 1 << 6;
        const REGISTRY_CACHE            = 1 << 7;
    }
}

impl MemoryAreas {
    pub const DEFAULT: Self = Self::WORKING_SET
        .union(Self::SYSTEM_FILE_CACHE)
        .union(Self::MODIFIED_PAGE_LIST)
        .union(Self::STANDBY_LIST)
        .union(Self::COMBINED_PAGE_LIST)
        .union(Self::MODIFIED_FILE_CACHE);

    pub const fn label(self) -> &'static str {
        match self {
            Self::WORKING_SET => "工作集",
            Self::SYSTEM_FILE_CACHE => "系统文件缓存",
            Self::MODIFIED_PAGE_LIST => "已修改页面",
            Self::STANDBY_LIST => "待机列表",
            Self::STANDBY_LIST_LOW_PRIORITY => "待机列表(低优先级)",
            Self::COMBINED_PAGE_LIST => "合并页面",
            Self::MODIFIED_FILE_CACHE => "已修改文件",
            Self::REGISTRY_CACHE => "注册表缓存",
            _ => "未知区域",
        }
    }
}

struct OptimizeStep {
    area: MemoryAreas,
    run: OptimizeFn,
}

const OPTIMIZE_STEPS: &[OptimizeStep] = &[
    OptimizeStep {
        area: MemoryAreas::WORKING_SET,
        run: optimize_working_set,
    },
    OptimizeStep {
        area: MemoryAreas::SYSTEM_FILE_CACHE,
        run: optimize_system_file_cache,
    },
    OptimizeStep {
        area: MemoryAreas::MODIFIED_PAGE_LIST,
        run: optimize_modified_page_list,
    },
    OptimizeStep {
        area: MemoryAreas::STANDBY_LIST,
        run: || optimize_standby_list(false),
    },
    OptimizeStep {
        area: MemoryAreas::STANDBY_LIST_LOW_PRIORITY,
        run: || optimize_standby_list(true),
    },
    OptimizeStep {
        area: MemoryAreas::COMBINED_PAGE_LIST,
        run: optimize_combined_page_list,
    },
    OptimizeStep {
        area: MemoryAreas::MODIFIED_FILE_CACHE,
        run: optimize_modified_file_cache,
    },
    OptimizeStep {
        area: MemoryAreas::REGISTRY_CACHE,
        run: optimize_registry_cache,
    },
];

pub fn step_plan(areas: MemoryAreas) -> Result<StepPlan> {
    if areas.is_empty() {
        bail!("no memory areas selected");
    }

    Ok(OPTIMIZE_STEPS
        .iter()
        .filter(|step| areas.contains(step.area))
        .map(|step| (step.area.label(), step.run))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_plan_rejects_empty_selection() {
        assert!(step_plan(MemoryAreas::empty()).is_err());
    }

    #[test]
    fn step_plan_preserves_optimize_order() {
        let areas = MemoryAreas::MODIFIED_FILE_CACHE | MemoryAreas::WORKING_SET;
        let plan = step_plan(areas).expect("plan");
        let labels: Vec<_> = plan.into_iter().map(|(label, _)| label).collect();
        assert_eq!(labels, vec!["工作集", "已修改文件"]);
    }

    #[test]
    fn memory_area_labels_are_stable() {
        assert_eq!(MemoryAreas::WORKING_SET.label(), "工作集");
        assert_eq!(MemoryAreas::REGISTRY_CACHE.label(), "注册表缓存");
    }
}

fn purge_memory_list(command: SystemMemoryListCommand, privilege: &str, what: &str) -> Result<()> {
    enable_privilege(privilege).with_context(|| format!("{what} requires {privilege}"))?;
    unsafe {
        nt_set_system_information(
            InfoClass::MemoryList,
            (&command as *const SystemMemoryListCommand)
                .cast_mut()
                .cast::<core::ffi::c_void>(),
            std::mem::size_of::<SystemMemoryListCommand>() as u32,
        )
    }
    .with_context(|| format!("NtSetSystemInformation ({what}) failed"))?;
    Ok(())
}

fn optimize_working_set() -> Result<()> {
    purge_memory_list(
        SystemMemoryListCommand::EmptyWorkingSets,
        "SeProfileSingleProcessPrivilege",
        "Working Set",
    )
}

fn optimize_system_file_cache() -> Result<()> {
    enable_privilege("SeIncreaseQuotaPrivilege")
        .context("System File Cache requires SeIncreaseQuotaPrivilege")?;

    let cache_info = SystemFileCacheInformation64 {
        minimum_working_set: -1i64,
        maximum_working_set: -1i64,
        ..Default::default()
    };

    unsafe {
        nt_set_system_information(
            InfoClass::FileCache,
            &cache_info as *const _ as *mut _,
            std::mem::size_of::<SystemFileCacheInformation64>() as u32,
        )
    }
    .context("NtSetSystemInformation (SystemFileCacheInformation) failed")?;

    unsafe {
        let flush_size: usize = usize::MAX;
        SetSystemFileCacheSize(flush_size, flush_size, 0)
            .context("SetSystemFileCacheSize failed")?;
    }

    Ok(())
}

fn optimize_modified_page_list() -> Result<()> {
    purge_memory_list(
        SystemMemoryListCommand::FlushModifiedList,
        "SeProfileSingleProcessPrivilege",
        "Modified Page List",
    )
}

fn optimize_standby_list(low_priority: bool) -> Result<()> {
    let command = if low_priority {
        SystemMemoryListCommand::PurgeLowPriorityStandbyList
    } else {
        SystemMemoryListCommand::PurgeStandbyList
    };
    purge_memory_list(command, "SeProfileSingleProcessPrivilege", "Standby List")
}

fn optimize_combined_page_list() -> Result<()> {
    enable_privilege("SeProfileSingleProcessPrivilege")
        .context("Combined Page List requires SeProfileSingleProcessPrivilege")?;

    let combine_info = MemoryCombineInformationEx::default();

    unsafe {
        nt_set_system_information(
            InfoClass::CombinePhysicalMemory,
            &combine_info as *const _ as *mut _,
            std::mem::size_of::<MemoryCombineInformationEx>() as u32,
        )
    }
    .context("NtSetSystemInformation (Combined Page List) failed")?;

    Ok(())
}

pub fn optimize_drive_cache(drive_letter: char) -> Result<()> {
    let path = format!("\\\\.\\{}:", drive_letter);
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let handle = unsafe {
        CreateFileW(
            windows::core::PCWSTR(wide.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };

    let h = handle.context(format!("open volume {drive_letter}:"))?;
    if h.is_invalid() {
        bail!("invalid handle for volume {drive_letter}:");
    }

    let flush_result = unsafe { FlushFileBuffers(h) };
    if flush_result.is_err() {
        let last_error = unsafe { GetLastError() };
        unsafe {
            let _ = CloseHandle(h);
        }
        bail!("FlushFileBuffers on volume {drive_letter}: failed ({last_error:?})");
    }
    unsafe {
        let _ = CloseHandle(h);
    }

    Ok(())
}

fn optimize_modified_file_cache() -> Result<()> {
    // Fallback when not using app per-drive progress UI.
    let mut failed = Vec::new();
    for drive_letter in fixed_drives() {
        if optimize_drive_cache(drive_letter).is_err() {
            failed.push(drive_letter);
        }
    }

    if failed.is_empty() {
        Ok(())
    } else {
        bail!("驱动 {:?} 刷新失败", failed)
    }
}

fn optimize_registry_cache() -> Result<()> {
    use windows::Win32::System::Registry::{
        HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, RegFlushKey,
    };

    unsafe {
        let keys = [
            HKEY_CURRENT_USER,
            HKEY_LOCAL_MACHINE,
            HKEY_CLASSES_ROOT,
            HKEY_USERS,
        ];
        for key in keys {
            let _ = RegFlushKey(key);
        }
    }

    Ok(())
}

pub fn fixed_drives() -> Vec<char> {
    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let path = format!("{}:\\", letter as char);
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let drive_type = unsafe {
            windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(wide.as_ptr()))
        };
        if drive_type == 3u32 {
            drives.push(letter as char);
        }
    }
    drives
}
