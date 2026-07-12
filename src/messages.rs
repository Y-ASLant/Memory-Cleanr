use crate::memory::MemoryStatus;

pub fn format_freed_message(avail_before: u64, avail_after: u64) -> String {
    if avail_after > avail_before {
        format!(
            "+{}",
            MemoryStatus::format_bytes(avail_after - avail_before)
        )
    } else {
        String::new()
    }
}

pub fn build_cleanup_result_message(
    completed: &[&str],
    errors: &[&str],
    freed_detail: &str,
) -> String {
    match (completed.is_empty(), errors.is_empty()) {
        (true, true) => "未执行清理".into(),
        (true, false) => format!("清理失败：{}", errors.join("、")),
        (false, true) => {
            if freed_detail.is_empty() {
                format!("清理完成（{} 项）", completed.len())
            } else {
                format!("清理完成 · {freed_detail}")
            }
        }
        (false, false) => format!("完成 {} 项，失败：{}", completed.len(), errors.join("、")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_freed_message_only_when_memory_increased() {
        assert_eq!(format_freed_message(1_000, 2_000_000_000), "+1.86 GB");
        assert_eq!(format_freed_message(2_000, 1_000), "");
    }

    #[test]
    fn build_cleanup_result_message_variants() {
        assert_eq!(build_cleanup_result_message(&[], &[], ""), "未执行清理");
        assert_eq!(
            build_cleanup_result_message(&[], &["工作集"], ""),
            "清理失败：工作集"
        );
        assert_eq!(
            build_cleanup_result_message(&["工作集"], &[], ""),
            "清理完成（1 项）"
        );
        assert_eq!(
            build_cleanup_result_message(&["工作集"], &[], "+512.00 MB"),
            "清理完成 · +512.00 MB"
        );
        assert_eq!(
            build_cleanup_result_message(&["工作集", "待机列表"], &["注册表缓存"], ""),
            "完成 2 项，失败：注册表缓存"
        );
    }
}
