use gpui::{point, px, AppContext, Point, Pixels};
use gpui_component::{Root, WindowExt};
use std::time::Duration;

use rust_i18n::t;
use smol::Timer;

use crate::clipboard::{self, ContentType};
use crate::ui::clipboard_item_card::{DRAG_CARD_WIDTH, ITEM_HEIGHT};
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
        self.clear_clipboard_drag_preview(cx);
        if let Some(storage) = &self.clipboard_storage {
            match storage.move_item_by_id(from_id, to_id) {
                Ok(()) => self.refresh_clipboard_items(),
                Err(e) => crate::log_msg(&format!("[clipboard] move failed: {e:#}")),
            }
        }
        cx.notify();
    }

    pub fn clear_clipboard_drag_preview(&mut self, cx: &mut gpui::Context<Self>) {
        self.clipboard_dragging_id = None;
        self.clipboard_drag_cursor_offset = None;
        self.clipboard_drop_target_id = None;
        self.clipboard_shift_anims.clear();
        self.clipboard_shift_tick_gen = self.clipboard_shift_tick_gen.wrapping_add(1);
        self.clipboard_drag_track_tick_gen = self.clipboard_drag_track_tick_gen.wrapping_add(1);
        self.close_clipboard_tearoff_preview(cx);
    }

    pub fn close_clipboard_tearoff_preview(&mut self, cx: &mut gpui::Context<Self>) {
        self.clipboard_tearoff_preview_opening = false;
        if let Some(handle) = self.clipboard_tearoff_preview_handle.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    /// Pointer re-entered the main window — restore in-window drag ghost and reorder.
    pub fn cancel_clipboard_tearoff(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.clipboard_drag_tearoff {
            return;
        }
        self.clipboard_drag_tearoff = false;
        self.close_clipboard_tearoff_preview(cx);
        if let Some(dragging_id) = self.clipboard_dragging_id {
            self.clipboard_drop_target_id = Some(dragging_id);
            crate::ui::clipboard_panel::sync_clipboard_shift_anims(self, cx);
        }
        cx.notify();
    }

    fn tearoff_preview_origin_for(&self, screen: Point<Pixels>, _cx: &gpui::App) -> Point<Pixels> {
        let offset = self.clipboard_drag_cursor_offset.unwrap_or_else(|| {
            point(px(DRAG_CARD_WIDTH / 2.), px(ITEM_HEIGHT / 2.))
        });
        crate::ui::tearoff_drag_preview::tearoff_preview_origin(screen, offset)
    }

    pub fn update_clipboard_tearoff_preview_position(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(handle) = self.clipboard_tearoff_preview_handle else {
            return;
        };
        let Ok(screen) = crate::win32::cursor::screen_point() else {
            return;
        };
        let origin = self.tearoff_preview_origin_for(screen, cx);
        let _ = handle.update(cx, |_, window, _| {
            let _ = crate::win32::window::set_window_screen_origin(window, origin);
        });
    }

    pub fn begin_clipboard_tearoff_preview(&mut self, item_id: i64, cx: &mut gpui::Context<Self>) {
        if self.clipboard_tearoff_preview_handle.is_some() || self.clipboard_tearoff_preview_opening {
            self.update_clipboard_tearoff_preview_position(cx);
            return;
        }

        let item = self
            .clipboard_items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
            .or_else(|| {
                self.clipboard_storage.as_ref().and_then(|storage| {
                    storage.get(item_id).ok().flatten()
                })
            });

        let Some(item) = item else {
            crate::log_msg(&format!("[clipboard] tearoff preview missing item {item_id}"));
            return;
        };

        self.clipboard_tearoff_preview_opening = true;
        cx.notify();

        let screen = crate::win32::cursor::screen_point().unwrap_or(point(px(200.), px(200.)));
        let origin = self.tearoff_preview_origin_for(screen, cx);
        let options = crate::ui::tearoff_drag_preview::tearoff_preview_window_options(origin);

        cx.spawn(async move |this, cx| {
            let opened = cx.open_window(options, |window, cx| {
                crate::ui::theme::init_light_theme(window, cx);
                let _ = crate::win32::window::set_always_on_top(window, true);
                let _ = crate::win32::window::set_tool_window(window);
                let preview =
                    cx.new(|_| crate::ui::tearoff_drag_preview::TearoffDragPreview::new(item));
                cx.new(|cx| Root::new(preview, window, cx))
            });

            let _ = this.update(cx, |app, cx| {
                app.clipboard_tearoff_preview_opening = false;
                match opened {
                    Ok(handle) => {
                        app.clipboard_tearoff_preview_handle = Some(handle.into());
                        app.update_clipboard_tearoff_preview_position(cx);
                    }
                    Err(e) => {
                        crate::log_msg(&format!("[clipboard] tearoff preview open failed: {e:#}"));
                    }
                }
            });
        })
        .detach();
    }

    /// Spawn a frameless desktop card when the user drags a row out of the main window.
    pub fn open_pinned_card_from_tearoff(&mut self, item_id: i64, cx: &mut gpui::Context<Self>) {
        use crate::app::pinned_card::{
            PinnedCardWindow, pinned_window_options, pinned_window_origin, window_title_for_item,
        };

        if let Some(handle) = self.pinned_card_handles.get(&item_id) {
            if handle
                .update(cx, |_, window, _| {
                    window.activate_window();
                })
                .is_ok()
            {
                return;
            }
            self.pinned_card_handles.remove(&item_id);
        }

        let item = self
            .clipboard_items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
            .or_else(|| {
                self.clipboard_storage.as_ref().and_then(|storage| {
                    storage.get(item_id).ok().flatten()
                })
            });

        let Some(item) = item else {
            crate::log_msg(&format!("[clipboard] tearoff missing item {item_id}"));
            return;
        };

        let screen = crate::win32::cursor::screen_point().unwrap_or(point(px(200.), px(200.)));
        let origin = pinned_window_origin(screen);
        let options = pinned_window_options(origin);
        let title = window_title_for_item(&item);
        let item_for_window = item.clone();

        cx.spawn(async move |this, cx| {
            let opened = cx.open_window(options, |window, cx| {
                window.set_window_title(&title);
                crate::ui::theme::init_light_theme(window, cx);
                let _ = crate::win32::window::set_tool_window(window);
                let pinned = cx.new(|_| PinnedCardWindow::new(item_for_window));
                cx.new(|cx| Root::new(pinned, window, cx))
            });

            match opened {
                Ok(handle) => {
                    let _ = this.update(cx, |app, _| {
                        app.pinned_card_handles.insert(item_id, handle.into());
                    });
                }
                Err(e) => {
                    crate::log_msg(&format!("[clipboard] pinned window open failed: {e:#}"));
                }
            }
        })
        .detach();
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
