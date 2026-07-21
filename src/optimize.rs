use rust_i18n::t;

use anyhow::{Context, Result, bail};
use windows::Win32::System::Memory::SetSystemFileCacheSize;

use crate::privileges::enable_privilege;
use crate::win32::nt::{
    InfoClass, MemoryCombineInformationEx, SystemFileCacheInformation64, SystemMemoryListCommand,
    nt_set_system_information,
};

type StepPlan = Vec<(String, OptimizeStepFn)>;

pub type OptimizeStepFn = Box<dyn Fn() -> Result<()> + Send>;

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
        .union(Self::STANDBY_LIST)
        .union(Self::COMBINED_PAGE_LIST);

    /// Returns the i18n key for this memory area's display name.
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::WORKING_SET => "area.working_set",
            Self::SYSTEM_FILE_CACHE => "area.system_file_cache",
            Self::MODIFIED_PAGE_LIST => "area.modified_page_list",
            Self::STANDBY_LIST => "area.standby_list",
            Self::STANDBY_LIST_LOW_PRIORITY => "area.standby_list_low_priority",
            Self::COMBINED_PAGE_LIST => "area.combined_page_list",
            Self::MODIFIED_FILE_CACHE => "area.modified_file_cache",
            Self::REGISTRY_CACHE => "area.registry_cache",
            _ => "area.unknown",
        }
    }

    /// Returns the localized display name for this memory area.
    pub fn label(self) -> String {
        t!(self.label_key()).to_string()
    }
}

struct OptimizeStep {
    area: MemoryAreas,
}

const OPTIMIZE_STEPS: &[OptimizeStep] = &[
    OptimizeStep {
        area: MemoryAreas::WORKING_SET,
    },
    OptimizeStep {
        area: MemoryAreas::SYSTEM_FILE_CACHE,
    },
    OptimizeStep {
        area: MemoryAreas::MODIFIED_PAGE_LIST,
    },
    OptimizeStep {
        area: MemoryAreas::STANDBY_LIST,
    },
    OptimizeStep {
        area: MemoryAreas::STANDBY_LIST_LOW_PRIORITY,
    },
    OptimizeStep {
        area: MemoryAreas::COMBINED_PAGE_LIST,
    },
    OptimizeStep {
        area: MemoryAreas::MODIFIED_FILE_CACHE,
    },
    OptimizeStep {
        area: MemoryAreas::REGISTRY_CACHE,
    },
];

pub fn step_plan(areas: MemoryAreas, excluded_processes: &[String]) -> Result<StepPlan> {
    if areas.is_empty() {
        bail!("no memory areas selected");
    }

    let excluded = excluded_processes.to_vec();
    Ok(OPTIMIZE_STEPS
        .iter()
        .filter(|step| areas.contains(step.area))
        .map(|step| {
            let label = step.area.label();
            let run: OptimizeStepFn = match step.area {
                MemoryAreas::WORKING_SET => {
                    let excluded = excluded.clone();
                    Box::new(move || optimize_working_set(&excluded))
                }
                MemoryAreas::SYSTEM_FILE_CACHE => Box::new(optimize_system_file_cache),
                MemoryAreas::MODIFIED_PAGE_LIST => Box::new(optimize_modified_page_list),
                MemoryAreas::STANDBY_LIST => Box::new(|| optimize_standby_list(false)),
                MemoryAreas::STANDBY_LIST_LOW_PRIORITY => Box::new(|| optimize_standby_list(true)),
                MemoryAreas::COMBINED_PAGE_LIST => Box::new(optimize_combined_page_list),
                MemoryAreas::MODIFIED_FILE_CACHE => Box::new(optimize_modified_file_cache),
                MemoryAreas::REGISTRY_CACHE => Box::new(optimize_registry_cache),
                _ => unreachable!("all defined MemoryAreas variants in OPTIMIZE_STEPS are covered"),
            };
            (label, run)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::with_locale;

    #[test]
    fn step_plan_rejects_empty_selection() {
        assert!(step_plan(MemoryAreas::empty(), &[]).is_err());
    }

    #[test]
    fn step_plan_preserves_optimize_order_zh() {
        with_locale("zh-CN", || {
            let areas = MemoryAreas::MODIFIED_FILE_CACHE | MemoryAreas::WORKING_SET;
            let plan = step_plan(areas, &[]).expect("plan");
            let labels: Vec<_> = plan.into_iter().map(|(label, _)| label).collect();
            assert_eq!(labels, vec!["工作集", "已修改文件"]);
        });
    }

    #[test]
    fn step_plan_preserves_optimize_order_en() {
        with_locale("en", || {
            let areas = MemoryAreas::MODIFIED_FILE_CACHE | MemoryAreas::WORKING_SET;
            let plan = step_plan(areas, &[]).expect("plan");
            let labels: Vec<_> = plan.into_iter().map(|(label, _)| label).collect();
            assert_eq!(labels, vec!["Working Set", "Modified File Cache"]);
        });
    }

    #[test]
    fn memory_area_labels_are_stable_zh() {
        with_locale("zh-CN", || {
            assert_eq!(MemoryAreas::WORKING_SET.label(), "工作集");
            assert_eq!(MemoryAreas::REGISTRY_CACHE.label(), "注册表缓存");
        });
    }

    #[test]
    fn memory_area_labels_are_stable_en() {
        with_locale("en", || {
            assert_eq!(MemoryAreas::WORKING_SET.label(), "Working Set");
            assert_eq!(MemoryAreas::REGISTRY_CACHE.label(), "Registry Cache");
        });
    }
}

fn purge_memory_list(command: SystemMemoryListCommand, privilege: &str, what: &str) -> Result<()> {
    enable_privilege(privilege).with_context(|| format!("{what} requires {privilege}"))?;
    unsafe {
        let mut cmd = command;
        nt_set_system_information(
            InfoClass::MemoryList,
            std::slice::from_mut(&mut cmd).as_mut_ptr().cast(),
            std::mem::size_of::<SystemMemoryListCommand>() as u32,
        )
    }
    .with_context(|| format!("NtSetSystemInformation ({what}) failed"))?;
    Ok(())
}

fn optimize_working_set(excluded: &[String]) -> Result<()> {
    if excluded.is_empty() {
        purge_memory_list(
            SystemMemoryListCommand::EmptyWorkingSets,
            "SeProfileSingleProcessPrivilege",
            "Working Set",
        )
    } else {
        enable_privilege("SeDebugPrivilege")
            .context("Working Set (per-process) requires SeDebugPrivilege")?;
        crate::win32::process::empty_working_sets_except(excluded)
            .context("Working Set per-process cleanup failed")
    }
}

fn optimize_system_file_cache() -> Result<()> {
    enable_privilege("SeIncreaseQuotaPrivilege")
        .context("System File Cache requires SeIncreaseQuotaPrivilege")?;

    let cache_info = SystemFileCacheInformation64 {
        minimum_working_set: usize::MAX,
        maximum_working_set: usize::MAX,
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

pub use crate::win32::volume::{VolumeFlushReport, complete_volume_flush, flush_all_volume_caches};

fn optimize_modified_file_cache() -> Result<()> {
    complete_volume_flush(flush_all_volume_caches()?)
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
