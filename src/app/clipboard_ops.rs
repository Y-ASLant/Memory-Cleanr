use gpui_component::WindowExt;
use std::time::Duration;

use rust_i18n::t;
use smol::Timer;

use crate::clipboard::{self, ContentType};
use crate::win32;

use super::MemoryCleanerApp;

impl MemoryCleanerApp {
    /// Show or toggle the clipboard history panel (tray / no direct window handle).
    pub fn show_clipboard_window(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.settings.clipboard_enabled {
            return;
        }

        if self.window_visible() {
            self.clipboard_visible = !self.clipboard_visible;
        } else {
            self.clipboard_visible = true;
            self.activate_window(cx);
        }

        if self.clipboard_visible {
            self.refresh_clipboard_items();
        }

        self.apply_clipboard_window_size(cx);
        cx.notify();
    }

    /// Enter or leave clipboard mode from the title bar (resize via the live window).
    pub fn set_clipboard_visible(
        &mut self,
        visible: bool,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if visible && !self.settings.clipboard_enabled {
            return;
        }
        if self.clipboard_visible == visible {
            return;
        }
        if visible {
            // Keep whatever app the user was editing so paste can return focus there.
            win32::focus::save_current_focus();
            if let Ok(hwnd) = win32::window::hwnd_from_window(window) {
                win32::focus::set_our_hwnd(hwnd);
            }
        }
        self.clipboard_visible = visible;
        if visible {
            self.refresh_clipboard_items();
        }
        // Must resize on the click's window — handle.update can leave the clipboard height
        // stuck after returning, which looks like a collapsed layout with empty space.
        window.resize(super::window_size(self.settings_expanded, self.clipboard_visible));
        cx.notify();
    }

    pub(crate) fn apply_clipboard_window_size(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(handle) = self.window {
            let size = super::window_size(self.settings_expanded, self.clipboard_visible);
            if let Err(e) = handle.update(cx, |_, window, _| {
                window.resize(size);
            }) {
                crate::log_msg(&format!("[window] clipboard resize failed: {e:#}"));
            }
        }
    }

    pub fn refresh_clipboard_items(&mut self) {
        if let Some(storage) = &self.clipboard_storage {
            // Virtual list can scroll many rows; keep a generous in-memory window.
            let limit = self.settings.clipboard_max_history.clamp(200, 5_000) as usize;
            match storage.query(self.clipboard_filter, None, limit, 0) {
                Ok(items) => self.clipboard_items = items,
                Err(e) => {
                    crate::log_msg(&format!("[clipboard] query failed: {e:#}"));
                }
            }
        }
    }

    pub fn set_clipboard_filter(
        &mut self,
        filter: Option<ContentType>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.clipboard_filter == filter {
            return;
        }
        crate::ui::clipboard_panel::begin_filter_slide(self, filter, cx);
        self.clipboard_filter = filter;
        self.refresh_clipboard_items();
        cx.notify();
    }

    pub fn open_clipboard_clear_confirm(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        use gpui_component::dialog::DialogButtonProps;

        let count = self
            .clipboard_items
            .iter()
            .filter(|item| !item.is_pinned)
            .count();
        if count == 0 {
            return;
        }
        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            alert
                .title(t!("clipboard.clear_confirm_title"))
                .description(t!("clipboard.clear_confirm_desc", count = count))
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
                        let _ = weak.update(cx, |app, cx| app.clear_clipboard_history(cx));
                        true
                    }
                })
        });
    }

    pub fn clear_clipboard_history(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(storage) = &self.clipboard_storage {
            match storage.clear_unpinned() {
                Ok(_count) => self.refresh_clipboard_items(),
                Err(e) => crate::log_msg(&format!("[clipboard] clear failed: {e:#}")),
            }
        }
        if let Some(hovered) = self.clipboard_hovered_id.take() {
            crate::ui::clipboard_panel::begin_clipboard_hover_fade(self, hovered, cx);
        }
        self.clipboard_selected = None;
        cx.notify();
    }

    pub fn open_clipboard_delete_confirm(
        &mut self,
        id: i64,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        use gpui_component::dialog::DialogButtonProps;

        let weak = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            alert
                .title(t!("clipboard.delete_confirm_title"))
                .description(t!("clipboard.delete_confirm_desc"))
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
                        let _ = weak.update(cx, |app, cx| {
                            app.begin_clipboard_item_delete(id, cx);
                        });
                        true
                    }
                })
        });
    }

    /// Fade the card out, collapse siblings into the gap, then remove from storage.
    pub fn begin_clipboard_item_delete(&mut self, id: i64, cx: &mut gpui::Context<Self>) {
        if self.clipboard_deleting_id.is_some() || self.clipboard_dragging_id.is_some() {
            return;
        }
        let Some(index) = self.clipboard_items.iter().position(|item| item.id == id) else {
            return;
        };

        self.clipboard_deleting_id = Some(id);
        self.clipboard_hovered_id = None;
        crate::ui::clipboard_panel::begin_clipboard_hover_fade(self, id, cx);
        crate::ui::clipboard_panel::begin_delete_collapse(self, index, cx);
        cx.notify();

        let anim_ms = crate::ui::clipboard_panel::DELETE_ANIM_MS;
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(anim_ms)).await;
            let _ = this.update(cx, |app, cx| {
                // FLIP handoff: siblings are already visually at -ROW_HEIGHT; drop the
                // empty slot and clear transforms in the same frame so layout catches up
                // without a flash jump.
                app.clipboard_deleting_id = None;
                app.clipboard_shift_anims.clear();
                app.clipboard_shift_tick_gen = app.clipboard_shift_tick_gen.wrapping_add(1);
                app.delete_clipboard_item(id, cx);
            });
        })
        .detach();
    }

    pub fn paste_clipboard_item(&mut self, id: i64, cx: &mut gpui::Context<Self>) {
        let Some(storage) = &self.clipboard_storage else {
            return;
        };
        let Ok(Some(item)) = storage.get(id) else {
            return;
        };

        // Hide on UI thread → paste on worker → show again (window not destroyed).
        cx.spawn(async move |this, cx| {
            let write = smol::unblock({
                let item = item.clone();
                move || {
                    crate::clipboard::monitor::pause_monitor(Duration::from_millis(800));
                    match item.content_type {
                        ContentType::Text => item
                            .text_content
                            .as_deref()
                            .map(crate::win32::clipboard::set_text)
                            .unwrap_or_else(|| Err(anyhow::anyhow!("missing text content"))),
                        ContentType::File => item
                            .file_paths
                            .as_deref()
                            .map(crate::win32::clipboard::set_files)
                            .unwrap_or_else(|| Err(anyhow::anyhow!("missing file paths"))),
                    }
                }
            })
            .await;
            if let Err(e) = write {
                crate::log_msg(&format!("[clipboard] set clipboard failed: {e:#}"));
                return;
            }

            let _ = this.update(cx, |app, cx| {
                if let Some(handle) = app.window {
                    let _ = handle.update(cx, |_, window, _| {
                        if let Ok(hwnd) = win32::window::hwnd_from_window(window) {
                            win32::focus::set_our_hwnd(hwnd);
                            win32::window::hide_hwnd(hwnd);
                        }
                    });
                }
            });

            Timer::after(Duration::from_millis(100)).await;

            let paste = smol::unblock(crate::win32::clipboard::paste_into_target).await;
            if let Err(e) = paste {
                crate::log_msg(&format!("[clipboard] paste failed: {e:#}"));
            }

            let _ = this.update(cx, |app, cx| {
                if let Some(handle) = app.window {
                    let _ = handle.update(cx, |_, window, _| {
                        if let Ok(hwnd) = win32::window::hwnd_from_window(window) {
                            // Reappear first without stealing focus, then take focus back.
                            win32::window::show_hwnd_noactivate(hwnd);
                            let _ = win32::focus::restore_our_foreground();
                        }
                    });
                }
            });
        })
        .detach();
    }

    pub fn delete_clipboard_item(&mut self, id: i64, cx: &mut gpui::Context<Self>) {
        if let Some(storage) = &self.clipboard_storage {
            match storage.delete(id) {
                Ok(()) => self.refresh_clipboard_items(),
                Err(e) => crate::log_msg(&format!("[clipboard] delete failed: {e:#}")),
            }
        }
        if self.clipboard_hovered_id == Some(id) {
            self.clipboard_hovered_id = None;
            crate::ui::clipboard_panel::begin_clipboard_hover_fade(self, id, cx);
        }
        cx.notify();
    }

    pub fn toggle_clipboard_pin(&mut self, id: i64, cx: &mut gpui::Context<Self>) {
        if let Some(storage) = &self.clipboard_storage {
            match storage.toggle_pin(id) {
                Ok(_pinned) => self.refresh_clipboard_items(),
                Err(e) => crate::log_msg(&format!("[clipboard] toggle pin failed: {e:#}")),
            }
        }
        cx.notify();
    }

    pub fn move_clipboard_item(&mut self, from_id: i64, to_id: i64, cx: &mut gpui::Context<Self>) {
        if from_id == to_id {
            return;
        }
        self.clear_clipboard_drag_preview();
        if let Some(storage) = &self.clipboard_storage {
            match storage.move_item_by_id(from_id, to_id) {
                Ok(()) => self.refresh_clipboard_items(),
                Err(e) => crate::log_msg(&format!("[clipboard] move failed: {e:#}")),
            }
        }
        cx.notify();
    }

    pub fn clear_clipboard_drag_preview(&mut self) {
        self.clipboard_dragging_id = None;
        self.clipboard_drop_target_id = None;
        self.clipboard_shift_anims.clear();
        self.clipboard_shift_tick_gen = self.clipboard_shift_tick_gen.wrapping_add(1);
    }

    /// Process a raw clipboard content (called from monitor thread via channel).
    pub fn handle_clipboard_content(
        &mut self,
        content: clipboard::RawClipboardContent,
        cx: &mut gpui::Context<Self>,
    ) {
        use crate::clipboard::handler;
        let processed = match handler::process(content, None) {
            Ok(p) => p,
            Err(e) => {
                crate::log_msg(&format!("[clipboard] process failed: {e:#}"));
                return;
            }
        };

        if let Some(storage) = &self.clipboard_storage {
            match storage.insert(
                processed.content_type,
                processed.text_content.as_deref(),
                &processed.preview,
                processed.file_paths.as_deref(),
                &processed.content_hash,
                processed.byte_size,
                None,
            ) {
                Ok(_id) => {
                    if self.clipboard_visible {
                        self.refresh_clipboard_items();
                    }
                }
                Err(e) => {
                    crate::log_msg(&format!("[clipboard] insert failed: {e:#}"));
                }
            }
        }
        cx.notify();
    }
}
