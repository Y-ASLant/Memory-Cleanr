use rust_i18n::t;

use gpui::*;
use gpui_component::{Root, WindowExt};

use crate::win32;

use super::{MemoryCleanerApp, window_options, window_size};

impl MemoryCleanerApp {
    pub(crate) fn open_window(&mut self, cx: &mut Context<Self>) {
        if self.window.is_some() || self.window_opening {
            return;
        }

        self.window_opening = true;
        let expanded = self.settings_expanded;
        let clipboard_visible = self.clipboard_visible;
        cx.spawn(async move |this, cx| {
            let entity = match this.upgrade() {
                Some(entity) => entity,
                None => return,
            };

            let options = cx.update(|app| window_options(expanded, clipboard_visible, app));
            let opened = cx.open_window(options, |window, cx| {
                entity.update(cx, |app, cx| {
                    app.attach_window(window, cx, false);
                    app.window_opening = false;
                });
                window.set_window_title(crate::version::APP_NAME);
                let _ = win32::window::remove_maximize_button(window);
                crate::ui::theme::init_light_theme(window, cx);
                window.activate_window();
                cx.new(|cx| Root::new(entity.clone(), window, cx))
            });

            if opened.is_err() {
                entity.update(cx, |app, _| {
                    app.window_opening = false;
                    app.window_shown = false;
                });
            } else {
                entity.update(cx, |app, _| app.sync_tray());
            }
        })
        .detach();
    }

    pub fn window_visible(&self) -> bool {
        self.window.is_some() && self.window_shown
    }

    pub fn activate_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.window {
            match handle.update(cx, |_, window, _| -> anyhow::Result<()> {
                crate::log_msg("[window] activate_window");
                win32::window::show_from_tray(window)?;
                window.activate_window();
                Ok(())
            }) {
                Ok(Ok(())) => {
                    self.window_shown = true;
                    self.pause_memory_refresh();
                    self.pause_anim();
                    self.start_memory_refresh(cx);
                    self.start_anim(cx);
                    self.sync_tray();
                    return;
                }
                Ok(Err(e)) => crate::log_msg(&format!("[window] show_from_tray failed: {e:#}")),
                Err(_) => crate::log_msg("[window] activate_window handle update failed"),
            }
            self.release_window_handle(cx, "activate_failed");
        }
        self.open_window(cx);
    }

    /// Destroy the GPUI window referenced by `self.window`, then clear tracking state.
    /// Safe to call when no handle is held (still resets `window_shown` and pauses loops).
    pub(crate) fn release_window_handle(&mut self, cx: &mut Context<Self>, source: &str) {
        if let Some(handle) = self.window.take() {
            match handle.update(cx, |_, window, _| window.remove_window()) {
                Ok(()) => crate::log_msg(&format!("[window] release_window ok source={source}")),
                Err(_) => {
                    crate::log_msg(&format!("[window] release_window failed source={source}"))
                }
            }
        } else {
            crate::log_msg(&format!(
                "[window] release_window no handle source={source}"
            ));
        }
        self.window_shown = false;
        win32::focus::clear_our_hwnd();
        self.pause_memory_refresh();
        self.pause_anim();
    }

    /// Remove the GPUI window and drop our handle. `activate_window` recreates it via
    /// `open_window()`.
    pub(crate) fn destroy_window_to_tray(&mut self, window: &mut Window, source: &str) {
        window.remove_window();
        self.window = None;
        self.window_shown = false;
        win32::focus::clear_our_hwnd();
        self.pause_memory_refresh();
        self.pause_anim();
        crate::log_msg(&format!("[close] hide_to_tray destroy ok source={source}"));
    }

    /// Handle a close request. Returns `true` when the app should quit entirely.
    pub fn request_close(&mut self, source: &str, window: &mut Window) -> bool {
        crate::log_msg(&format!(
            "[close] request_close source={source} close_to_tray={}",
            self.settings.close_to_notification_area
        ));
        self.settings.save();
        if self.settings.close_to_notification_area {
            self.destroy_window_to_tray(window, source);
            self.sync_tray();
            false
        } else {
            true
        }
    }

    pub fn hide_to_tray(&mut self, cx: &mut Context<Self>) {
        self.release_window_handle(cx, "tray_menu");
        self.sync_tray();
    }

    pub fn open_window_behavior_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::layout::{
            DIALOG_PADDING_HORIZONTAL, DIALOG_PADDING_TOP, WINDOW_BEHAVIOR_DIALOG_WIDTH,
        };
        use crate::ui::settings_page::render_window_behavior_dialog;

        self.cancel_cleanup_hotkey_recording(cx);

        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let weak = weak.clone();
            dialog
                .title(t!("dialog.window_behavior"))
                .w(px(WINDOW_BEHAVIOR_DIALOG_WIDTH))
                .pt(px(DIALOG_PADDING_TOP))
                .pb(px(super::CONTENT_PADDING))
                .pl(px(DIALOG_PADDING_HORIZONTAL))
                .pr(px(DIALOG_PADDING_HORIZONTAL))
                .overlay_closable(false)
                .content({
                    let weak = weak.clone();
                    move |content, _window, cx| {
                        content.child(render_window_behavior_dialog(weak.clone(), cx))
                    }
                })
        });
    }

    pub fn toggle_settings_expanded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_expanded = !self.settings_expanded;
        window.resize(window_size(self.settings_expanded, self.clipboard_visible));
        cx.notify();
    }

    pub fn set_always_on_top(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = win32::window::set_always_on_top(window, enabled) {
            crate::log_msg(&format!(
                "[window] set_always_on_top({enabled}) failed: {error:#}"
            ));
            cx.notify();
            return;
        }
        self.settings.always_on_top = enabled;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_close_to_tray(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.close_to_notification_area = enabled;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_run_at_startup(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if let Err(error) = win32::startup::set_enabled(enabled) {
            crate::log_msg(&format!(
                "[startup] set_enabled({enabled}) failed: {error:#}"
            ));
            cx.notify();
            return;
        }
        self.settings.run_at_startup = enabled;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_show_optimization_notifications(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.show_optimization_notifications = enabled;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_cleanup_hotkey_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.cleanup_hotkey_enabled = enabled;
        if !enabled {
            self.cleanup_hotkey_recording = false;
        }
        win32::hotkey::sync(&self.settings);
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn start_cleanup_hotkey_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.cleanup_hotkey_enabled {
            return;
        }
        self.cleanup_hotkey_recording = true;
        window.focus(&self.hotkey_capture_focus, cx);
        cx.notify();
    }

    pub fn handle_cleanup_hotkey_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !self.cleanup_hotkey_recording {
            return;
        }

        if event.keystroke.key.eq_ignore_ascii_case("escape") {
            self.cleanup_hotkey_recording = false;
            cx.notify();
            return;
        }

        let keystroke = &event.keystroke;
        let Some(chord) = win32::hotkey::HotkeyBinding::format_chord(
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.modifiers.shift,
            keystroke.modifiers.platform,
            &keystroke.key,
        ) else {
            return;
        };

        self.settings.cleanup_hotkey = chord;
        self.cleanup_hotkey_recording = false;
        win32::hotkey::sync(&self.settings);
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn cancel_cleanup_hotkey_recording(&mut self, cx: &mut Context<Self>) {
        if self.cleanup_hotkey_recording {
            self.cleanup_hotkey_recording = false;
            cx.notify();
        }
    }

    pub fn set_debug_logging(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.debug_logging = enabled;
        crate::log::set_debug_enabled(enabled);
        if enabled {
            crate::log::write(&t!(
                "log.debug_enabled",
                path = crate::log::log_file_path().display().to_string()
            ));
        }
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_clipboard_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.clipboard_enabled = enabled;
        if !enabled {
            self.clipboard_visible = false;
            if let Some(handle) = self.clipboard_monitor.take() {
                drop(handle);
                crate::log_msg("[clipboard] monitor stopped");
            }
        }
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_clipboard_win_v_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.clipboard_win_v_enabled = enabled;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_clipboard_max_history(&mut self, value: u32, cx: &mut Context<Self>) {
        self.settings.clipboard_max_history = value;
        self.queue_settings_save(cx);
        cx.notify();
    }

    pub fn set_clipboard_auto_cleanup_days(&mut self, value: u32, cx: &mut Context<Self>) {
        self.settings.clipboard_auto_cleanup_days = value;
        self.queue_settings_save(cx);
        cx.notify();
    }
}
