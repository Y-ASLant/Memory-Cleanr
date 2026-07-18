use rust_i18n::t;

use crate::messages::{build_cleanup_result_message, format_freed_message};
use crate::optimize::{self, MemoryAreas, OptimizeStepFn};
use crate::settings::Settings;
use crate::win32::volume::{VolumeFlushSession, complete_volume_flush};

#[derive(Debug, Clone)]
pub struct OptimizeStepUpdate {
    pub step_label: String,
    pub percent: f32,
}

#[derive(Debug, Clone)]
pub struct OptimizeRunResult {
    pub completed: Vec<String>,
    pub errors: Vec<String>,
    pub freed_detail: String,
    pub status_message: String,
    pub has_errors: bool,
}

pub fn notify_optimize_start(settings: &Settings) {
    if !settings.show_optimization_notifications {
        return;
    }
    if let Err(error) = crate::win32::notification::show(
        &t!("notification.optimize_start_title"),
        &t!("notification.optimize_start_body"),
    ) {
        crate::log_msg(&format!("[notification] failed: {error:#}"));
    }
}

pub fn notify_optimize_complete(settings: &Settings, result: &OptimizeRunResult) {
    if !settings.show_optimization_notifications {
        return;
    }
    if result.status_message.is_empty() || result.status_message == t!("tooltip.select_areas") {
        return;
    }
    if let Err(error) = crate::win32::notification::show(
        &t!("notification.optimize_title"),
        &result.status_message,
    ) {
        crate::log_msg(&format!("[notification] failed: {error:#}"));
    }
}

struct StepProgress {
    step_index: usize,
    total_steps: usize,
}

impl StepProgress {
    fn new(step_index: usize, total_steps: usize) -> Self {
        Self {
            step_index,
            total_steps,
        }
    }

    fn base(&self) -> f32 {
        self.step_index as f32 / self.total_steps as f32
    }

    fn span(&self) -> f32 {
        1.0 / self.total_steps as f32
    }

    fn start_percent(&self) -> f32 {
        self.base() * 100.0
    }

    fn end_percent(&self) -> f32 {
        (self.base() + self.span()) * 100.0
    }

    fn sub_percent(&self, sub_index: usize, sub_total: usize) -> f32 {
        (self.base() + sub_index as f32 / sub_total as f32 * self.span()) * 100.0
    }
}

pub fn run<F>(settings: &Settings, avail_before: u64, mut on_progress: F) -> OptimizeRunResult
where
    F: FnMut(OptimizeStepUpdate),
{
    let Some(_lock) = crate::win32::optimize_lock::OptimizeLock::try_acquire() else {
        return OptimizeRunResult {
            completed: Vec::new(),
            errors: Vec::new(),
            freed_detail: String::new(),
            status_message: t!("optimize.already_running").to_string(),
            has_errors: false,
        };
    };

    let areas = settings.memory_areas();
    let excluded = &settings.excluded_processes;
    let steps = match optimize::step_plan(areas, excluded) {
        Ok(steps) if !steps.is_empty() => steps,
        _ => {
            return OptimizeRunResult {
                completed: Vec::new(),
                errors: Vec::new(),
                freed_detail: String::new(),
                status_message: t!("tooltip.select_areas").to_string(),
                has_errors: false,
            };
        }
    };

    let total = steps.len();
    on_progress(OptimizeStepUpdate {
        step_label: t!("button.cleanup_preparing").to_string(),
        percent: 0.0,
    });

    let mut completed = Vec::new();
    let mut errors = Vec::new();

    for (index, (name, run)) in steps.into_iter().enumerate() {
        let ok = if name == MemoryAreas::MODIFIED_FILE_CACHE.label() {
            run_modified_file_cache_step(&name, index, total, &mut on_progress)
        } else {
            run_step(&name, run, index, total, &mut on_progress)
        };

        if ok {
            completed.push(name.clone());
            crate::log::write(&format!("[optimize] {name} succeeded"));
        } else {
            errors.push(name);
        }
    }

    let avail_after = crate::memory::MemoryStatus::query()
        .map(|s| s.avail_phys)
        .unwrap_or(avail_before);
    let freed_detail = format_freed_message(avail_before, avail_after);
    let completed_refs: Vec<&str> = completed.iter().map(|s| s.as_str()).collect();
    let errors_refs: Vec<&str> = errors.iter().map(|s| s.as_str()).collect();
    let status_message = build_cleanup_result_message(&completed_refs, &errors_refs, &freed_detail);
    crate::log::write(&format!("[optimize] result: {status_message}"));

    OptimizeRunResult {
        has_errors: !errors.is_empty(),
        completed,
        errors,
        freed_detail,
        status_message,
    }
}

fn run_step<F>(
    name: &str,
    run: OptimizeStepFn,
    step_index: usize,
    total_steps: usize,
    on_progress: &mut F,
) -> bool
where
    F: FnMut(OptimizeStepUpdate),
{
    let progress = StepProgress::new(step_index, total_steps);

    on_progress(OptimizeStepUpdate {
        step_label: t!("optimize.step", name = name.to_string()).to_string(),
        percent: progress.start_percent(),
    });

    let result = run();
    if let Err(e) = &result {
        crate::log::write(&format!("[optimize] {name} failed: {e:#}"));
    }

    on_progress(OptimizeStepUpdate {
        step_label: t!("optimize.step", name = name.to_string()).to_string(),
        percent: progress.end_percent(),
    });

    result.is_ok()
}

fn run_modified_file_cache_step<F>(
    name: &str,
    step_index: usize,
    total_steps: usize,
    on_progress: &mut F,
) -> bool
where
    F: FnMut(OptimizeStepUpdate),
{
    let progress = StepProgress::new(step_index, total_steps);

    let session = match VolumeFlushSession::open() {
        Ok(session) if session.is_empty() => {
            on_progress(OptimizeStepUpdate {
                step_label: t!("optimize.step", name = name.to_string()).to_string(),
                percent: progress.end_percent(),
            });
            return true;
        }
        Ok(session) => session,
        Err(error) => {
            crate::log::write(&format!(
                "[optimize] modified file cache volume enumeration failed: {error:#}"
            ));
            on_progress(OptimizeStepUpdate {
                step_label: t!("optimize.step", name = name.to_string()).to_string(),
                percent: progress.end_percent(),
            });
            return false;
        }
    };

    let volume_total = session.len();
    let mut report = optimize::VolumeFlushReport::default();

    for index in 0..volume_total {
        let volume_label = session.label(index).to_string();

        on_progress(OptimizeStepUpdate {
            step_label: t!(
                "optimize.step_with_progress",
                name = name.to_string(),
                volume = volume_label.clone(),
                current = (index + 1).to_string(),
                total = volume_total.to_string()
            )
            .to_string(),
            percent: progress.sub_percent(index, volume_total),
        });

        report.record(&volume_label, session.flush(index));

        on_progress(OptimizeStepUpdate {
            step_label: t!(
                "optimize.step_with_progress",
                name = name.to_string(),
                volume = volume_label,
                current = (index + 1).to_string(),
                total = volume_total.to_string()
            )
            .to_string(),
            percent: progress.sub_percent(index + 1, volume_total),
        });
    }

    complete_volume_flush(report).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn run_reports_select_areas_when_nothing_enabled() {
        let settings = Settings {
            memory_areas: 0,
            ..Settings::default()
        };
        let result = run(&settings, 0, |_| {});
        assert_eq!(
            result.status_message,
            t!("tooltip.select_areas").to_string()
        );
        assert!(result.completed.is_empty());
        assert!(!result.has_errors);
    }

    #[test]
    fn run_step_emits_progress_without_running_system_cleanup() {
        let mut progress = Vec::new();
        let ok = run_step(
            "Mock Step",
            Box::new(|| Ok(())),
            0,
            2,
            &mut |update| progress.push(update),
        );
        assert!(ok);
        assert_eq!(progress.len(), 2);
        assert!((progress[0].percent - 0.0).abs() < f32::EPSILON);
        assert!((progress[1].percent - 50.0).abs() < f32::EPSILON);
    }
}
