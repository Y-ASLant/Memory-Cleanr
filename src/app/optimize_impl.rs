use std::sync::Arc;
use std::time::Duration;

use rust_i18n::t;
use smol::Timer;

use crate::messages::{build_cleanup_result_message, format_freed_message};
use crate::optimize::{self, MemoryAreas};

use super::{MemoryCleanerApp, OPTIMIZE_RESULT_DISPLAY, show_toast};

impl MemoryCleanerApp {
    async fn run_optimize_step(
        this: gpui::WeakEntity<Self>,
        cx: &mut gpui::AsyncApp,
        name: String,
        run: optimize::OptimizeStepFn,
        step_index: usize,
        total_steps: usize,
    ) -> bool {
        let step_base = step_index as f32 / total_steps as f32;
        let step_span = 1.0 / total_steps as f32;

        let _ = this.update(cx, |app, cx| {
            app.optimize_step = t!("optimize.step", name = name.clone()).to_string();
            app.set_optimize_percent(step_base * 100.0);
            cx.notify();
        });

        Timer::after(Duration::from_millis(60)).await;

        let result = smol::unblock(run).await;

        if let Err(e) = &result {
            crate::log::write(&format!("[optimize] {name} failed: {e:#}"));
        }

        let _ = this.update(cx, |app, cx| {
            app.set_optimize_percent((step_base + step_span) * 100.0);
            cx.notify();
        });

        Timer::after(Duration::from_millis(100)).await;
        result.is_ok()
    }

    async fn run_modified_file_cache_step(
        this: gpui::WeakEntity<Self>,
        cx: &mut gpui::AsyncApp,
        step_index: usize,
        total_steps: usize,
    ) -> bool {
        use crate::win32::volume::{VolumeFlushSession, complete_volume_flush};

        let step_base = step_index as f32 / total_steps as f32;
        let step_span = 1.0 / total_steps as f32;
        let name = MemoryAreas::MODIFIED_FILE_CACHE.label();

        let session = match smol::unblock(VolumeFlushSession::open).await {
            Ok(session) if session.is_empty() => {
                let _ = this.update(cx, |app, cx| {
                    app.optimize_step = t!("optimize.step", name = name.clone()).to_string();
                    app.set_optimize_percent((step_base + step_span) * 100.0);
                    cx.notify();
                });
                return true;
            }
            Ok(session) => Arc::new(session),
            Err(error) => {
                crate::log::write(&format!(
                    "[optimize] modified file cache volume enumeration failed: {error:#}"
                ));
                let _ = this.update(cx, |app, cx| {
                    app.optimize_step = t!("optimize.step", name = name.clone()).to_string();
                    app.set_optimize_percent((step_base + step_span) * 100.0);
                    cx.notify();
                });
                return false;
            }
        };

        let volume_total = session.len();
        let mut report = optimize::VolumeFlushReport::default();

        for index in 0..volume_total {
            let volume_label = session.label(index).to_string();
            let sub_base = index as f32 / volume_total as f32;

            let _ = this.update(cx, |app, cx| {
                app.optimize_step = t!(
                    "optimize.step_with_progress",
                    name = name.clone(),
                    volume = volume_label.clone(),
                    current = (index + 1).to_string(),
                    total = volume_total.to_string()
                )
                .to_string();
                app.set_optimize_percent((step_base + sub_base * step_span) * 100.0);
                cx.notify();
            });

            let session_for_flush = Arc::clone(&session);
            let flush_index = index;
            let flush_result = smol::unblock(move || session_for_flush.flush(flush_index)).await;
            report.record(&volume_label, flush_result);

            let _ = this.update(cx, |app, cx| {
                app.set_optimize_percent(
                    (step_base + (index + 1) as f32 / volume_total as f32 * step_span) * 100.0,
                );
                cx.notify();
            });
        }

        complete_volume_flush(report).is_ok()
    }

    pub fn run_optimize(&mut self, cx: &mut gpui::Context<Self>) {
        if self.is_optimizing {
            return;
        }

        let areas = self.settings.memory_areas();
        let excluded = self.settings.excluded_processes.clone();
        let steps = match optimize::step_plan(areas, &excluded) {
            Ok(s) if !s.is_empty() => s,
            _ => {
                self.optimize_status = t!("tooltip.select_areas").to_string();
                cx.notify();
                return;
            }
        };

        let avail_before = self.physical.avail;
        let total = steps.len();
        let notify = self.settings.show_optimization_notifications;
        self.is_optimizing = true;
        self.optimize_step = t!("button.cleanup_preparing").to_string();
        self.set_optimize_percent(0.0);
        self.optimize_status.clear();
        self.optimize_has_errors = false;
        crate::tray::start_spin();
        cx.notify();

        cx.spawn(async move |this, cx| {
            if notify {
                show_toast(
                    t!("notification.optimize_start_title").to_string(),
                    t!("notification.optimize_start_body").to_string(),
                )
                .await;
            }

            let mut completed: Vec<String> = Vec::new();
            let mut errors: Vec<String> = Vec::new();

            for (index, (name, run)) in steps.into_iter().enumerate() {
                let ok = if name == MemoryAreas::MODIFIED_FILE_CACHE.label() {
                    Self::run_modified_file_cache_step(this.clone(), cx, index, total).await
                } else {
                    Self::run_optimize_step(this.clone(), cx, name.clone(), run, index, total).await
                };

                if ok {
                    completed.push(name.clone());
                    crate::log::write(&format!("[optimize] {name} succeeded"));
                } else {
                    errors.push(name);
                }
            }

            let notification = this
                .update(cx, |app, cx| {
                    let _ = app.refresh_memory();
                    let avail_after = app.physical.avail;
                    let freed_detail = format_freed_message(avail_before, avail_after);
                    app.optimize_step.clear();
                    app.is_optimizing = false;
                    app.set_optimize_percent(0.0);
                    app.anim_optimize.current = 0.0;
                    crate::tray::stop_spin();
                    let completed_refs: Vec<&str> = completed.iter().map(|s| s.as_str()).collect();
                    let errors_refs: Vec<&str> = errors.iter().map(|s| s.as_str()).collect();
                    app.optimize_has_errors = !errors.is_empty();
                    app.optimize_status =
                        build_cleanup_result_message(&completed_refs, &errors_refs, &freed_detail);
                    crate::log::write(&format!("[optimize] result: {}", app.optimize_status));
                    app.sync_tray();
                    cx.notify();
                    if app.settings.show_optimization_notifications {
                        Some((
                            t!("notification.optimize_title").to_string(),
                            app.optimize_status.clone(),
                        ))
                    } else {
                        None
                    }
                })
                .ok()
                .flatten();

            if let Some((title, body)) = notification {
                show_toast(title, body).await;
            }

            Timer::after(OPTIMIZE_RESULT_DISPLAY).await;

            let _ = this.update(cx, |app, cx| {
                app.optimize_status.clear();
                app.optimize_has_errors = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_icon_cache_confirm_dialog(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.is_busy() {
            return;
        }

        use gpui_component::WindowExt;
        use gpui_component::dialog::DialogButtonProps;

        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            alert
                .title(t!("icon_cache.confirm_title"))
                .description(t!("icon_cache.confirm_desc"))
                .overlay_closable(false)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("dialog.confirm"))
                        .cancel_text(t!("dialog.cancel"))
                        .show_cancel(true),
                )
                .on_ok({
                    let weak = weak.clone();
                    move |_, _window, cx| {
                        let _ = weak.update(cx, |app, cx| app.run_icon_cache_refresh(cx));
                        true
                    }
                })
        });
    }

    pub fn run_icon_cache_refresh(&mut self, cx: &mut gpui::Context<Self>) {
        if self.is_busy() {
            return;
        }

        self.is_refreshing_icon_cache = true;
        self.icon_cache_status = t!("icon_cache.refreshing").to_string();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = smol::unblock(crate::icon_cache::refresh).await;
            let message = outcome.user_message();
            crate::log_msg(&format!("[icon_cache] {message}"));
            for failure in &outcome.failures {
                crate::log::write(&format!("[icon_cache] {failure}"));
            }

            let _ = this.update(cx, |app, cx| {
                app.is_refreshing_icon_cache = false;
                app.icon_cache_status = message;
                cx.notify();
            });

            Timer::after(OPTIMIZE_RESULT_DISPLAY).await;

            let _ = this.update(cx, |app, cx| {
                app.icon_cache_status.clear();
                cx.notify();
            });
        })
        .detach();
    }
}
