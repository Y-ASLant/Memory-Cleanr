use std::sync::Arc;
use std::time::Duration;

use smol::Timer;

use crate::memory::MemorySection;
use rust_i18n::t;

use super::{MEMORY_REFRESH_INTERVAL, MemoryCleanerApp, query_sections};

impl MemoryCleanerApp {
    pub(crate) fn pause_memory_refresh(&self) {
        self.memory_refresh_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn pause_anim(&self) {
        self.anim_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn start_anim(&self, cx: &mut gpui::Context<Self>) {
        if self.window.is_none() {
            return;
        }
        let generation = self
            .anim_generation
            .load(std::sync::atomic::Ordering::Relaxed);
        let gen_arc = Arc::clone(&self.anim_generation);
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(crate::anim::ANIM_INTERVAL).await;
                if gen_arc.load(std::sync::atomic::Ordering::Relaxed) != generation {
                    break;
                }
                let Ok(animating) = this.update(cx, |app, cx| {
                    if !app.anim_dirty {
                        return false;
                    }
                    let a = app.anim_physical.tick();
                    let b = app.anim_virtual.tick();
                    let c = app.anim_optimize.tick();
                    let d = app.anim_used_phys.tick();
                    let e = app.anim_avail_phys.tick();
                    let f = app.anim_used_virt.tick();
                    let g = app.anim_avail_virt.tick();
                    let still = a | b | c | d | e | f | g;
                    app.anim_dirty = still;
                    if still {
                        cx.notify();
                    }
                    still
                }) else {
                    break;
                };
                if !animating {
                    Timer::after(Duration::from_millis(50)).await;
                }
            }
        })
        .detach();
    }

    pub(crate) fn start_memory_refresh(&self, cx: &mut gpui::Context<Self>) {
        if self.window.is_none() {
            return;
        }

        let generation = self
            .memory_refresh_generation
            .load(std::sync::atomic::Ordering::Relaxed);
        let gen_arc = Arc::clone(&self.memory_refresh_generation);
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(MEMORY_REFRESH_INTERVAL).await;
                if gen_arc.load(std::sync::atomic::Ordering::Relaxed) != generation {
                    break;
                }
                let Ok(()) = this.update(cx, |app, cx| {
                    if app.window.is_none() || !app.window_shown {
                        return;
                    }
                    if app.refresh_memory() {
                        cx.notify();
                        app.sync_tray();
                    }
                }) else {
                    break;
                };
            }
        })
        .detach();
    }

    pub(crate) fn sync_anim_targets_from_sections(&mut self) {
        self.anim_physical.target = self.physical.used_percent;
        self.anim_virtual.target = self.virtual_mem.used_percent;
        self.anim_used_phys.target = self.physical.used as f32;
        self.anim_avail_phys.target = self.physical.avail as f32;
        self.anim_used_virt.target = self.virtual_mem.used as f32;
        self.anim_avail_virt.target = self.virtual_mem.avail as f32;
        self.anim_dirty = true;
    }

    pub fn refresh_memory(&mut self) -> bool {
        let Ok((physical, virtual_mem)) = query_sections() else {
            if self.physical.is_unavailable() && self.virtual_mem.is_unavailable() {
                return false;
            }
            self.physical = MemorySection::unavailable(&t!("memory.physical"));
            self.virtual_mem = MemorySection::unavailable(&t!("memory.virtual"));
            self.sync_anim_targets_from_sections();
            return true;
        };

        let changed = self.physical != physical || self.virtual_mem != virtual_mem;
        if changed {
            self.physical = physical;
            self.virtual_mem = virtual_mem;
            self.sync_anim_targets_from_sections();
        }
        changed
    }

    pub fn animated_used_phys(&self) -> u64 {
        self.anim_used_phys.current as u64
    }
    pub fn animated_avail_phys(&self) -> u64 {
        self.anim_avail_phys.current as u64
    }
    pub fn animated_used_virt(&self) -> u64 {
        self.anim_used_virt.current as u64
    }
    pub fn animated_avail_virt(&self) -> u64 {
        self.anim_avail_virt.current as u64
    }
    pub fn animated_optimize_percent(&self) -> f32 {
        self.anim_optimize.current
    }

    /// Set optimize progress and kick the animation loop.
    pub(crate) fn set_optimize_percent(&mut self, value: f32) {
        self.optimize_percent = value;
        self.anim_optimize.target = value;
        self.anim_dirty = true;
    }
}
