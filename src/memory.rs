use anyhow::{Context, Result};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

#[derive(Debug, Clone, PartialEq)]
pub struct MemorySection {
    pub title: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
    pub used_percent: f32,
}

impl MemorySection {
    pub fn header(&self) -> String {
        format!(
            "{} ({})",
            self.title,
            MemoryStatus::format_bytes(self.total)
        )
    }

    pub fn usage_summary(&self) -> String {
        if self.total == 0 {
            return "—".into();
        }
        format!(
            "已用 {} · 可用 {}",
            MemoryStatus::format_bytes(self.used),
            MemoryStatus::format_bytes(self.avail),
        )
    }

    pub fn unavailable(title: &str) -> Self {
        Self {
            title: title.into(),
            total: 0,
            used: 0,
            avail: 0,
            used_percent: 0.0,
        }
    }

    pub fn is_unavailable(&self) -> bool {
        self.total == 0
    }

    pub fn percent_label(&self) -> String {
        if self.is_unavailable() {
            "—".into()
        } else {
            format!("{}%", self.used_percent.round() as u32)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryStatus {
    pub memory_load: u32,
    pub total_phys: u64,
    pub avail_phys: u64,
    pub total_page_file: u64,
    pub avail_page_file: u64,
}

impl MemoryStatus {
    pub fn query() -> Result<Self> {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };

        unsafe {
            GlobalMemoryStatusEx(&mut status).context("GlobalMemoryStatusEx failed")?;
        }

        Ok(Self {
            memory_load: status.dwMemoryLoad,
            total_phys: status.ullTotalPhys,
            avail_phys: status.ullAvailPhys,
            total_page_file: status.ullTotalPageFile,
            avail_page_file: status.ullAvailPageFile,
        })
    }

    pub fn used_phys(&self) -> u64 {
        self.total_phys.saturating_sub(self.avail_phys)
    }

    pub fn format_bytes(bytes: u64) -> String {
        const GB: u64 = 1024 * 1024 * 1024;
        const MB: u64 = 1024 * 1024;

        if bytes >= GB {
            format!("{:.2} GB", bytes as f64 / GB as f64)
        } else {
            format!("{:.2} MB", bytes as f64 / MB as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_section(used_percent: f32) -> MemorySection {
        MemorySection {
            title: "物理内存".into(),
            total: 8 * 1024 * 1024 * 1024,
            used: 4 * 1024 * 1024 * 1024,
            avail: 4 * 1024 * 1024 * 1024,
            used_percent,
        }
    }

    #[test]
    fn format_bytes_uses_gb_and_mb() {
        assert_eq!(
            MemoryStatus::format_bytes(2 * 1024 * 1024 * 1024),
            "2.00 GB"
        );
        assert_eq!(MemoryStatus::format_bytes(512 * 1024 * 1024), "512.00 MB");
    }

    #[test]
    fn percent_label_rounds_and_handles_unavailable() {
        assert_eq!(sample_section(45.4).percent_label(), "45%");
        assert_eq!(sample_section(45.6).percent_label(), "46%");
        assert_eq!(MemorySection::unavailable("物理内存").percent_label(), "—");
    }

    #[test]
    fn usage_summary_formats_used_and_available() {
        let summary = sample_section(50.0).usage_summary();
        assert!(summary.contains("已用 4.00 GB"));
        assert!(summary.contains("可用 4.00 GB"));
        assert_eq!(MemorySection::unavailable("物理内存").usage_summary(), "—");
    }
}
