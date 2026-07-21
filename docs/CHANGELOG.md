# 更新日志

本文件记录 Memory Cleaner 的版本更新，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

**编写约定**（详见 `AGENTS.md` → Documentation & Changelog）：每个版本只记录相对上一 tag 的**最终差异**，不记录开发过程中的中间修改或逐步修复。


## [1.0.3] - 2026-07-20

相对 [1.0.2] 的最终变更如下。

### 新增

- **平滑动画**：内存使用率环形图、清理进度条、内存数值文字（已用/可用字节）在数据刷新时平滑过渡，而非直接跳变。动画采用指数衰减插值，~300ms 到达目标值的 95%。
- **动画模块**：新增 `src/anim.rs`，`AnimatedValue` 插值器可供全 crate 复用。
- **动画智能暂停**：窗口隐藏到托盘时动画循环完全停止（零 CPU），窗口恢复后自动重启。

### 修复

- **托盘菜单状态**：关闭窗口后右键托盘菜单正确显示「显示窗口」而非「隐藏窗口」；从托盘恢复窗口后正确显示「隐藏窗口」。
- **窗口错误路径**：`activate_window` 和 `open_window` 失败时正确重置 `window_shown` 状态。
- **优化完成后托盘同步**：内存清理完成后立即同步托盘提示文本，不再等待下次鼠标悬停。
## [1.0.2] - 2026-07-19

相对 [1.0.1] 的最终变更如下。

### 新增

- **开机自启**：设置中可开启「登录 Windows 后静默启动到系统托盘」，不显示主窗口（`src/win32/startup.rs`）。
- **进程排除选择器**：列表项显示实例数与内存占用；无法读取内存时显示占位符。
- **文档**：`README_EN.md`（英文说明）、`docs/API_COMPARISON_MEMREDUCT.md`（与 Mem Reduct 清理 API 对比）。

### 变更

- **已修改文件缓存**：由遍历 `A:`–`Z:` 固定磁盘盘符，改为通过 Mount Manager 枚举 `\??\Volume{GUID}` 并刷写；新增 `src/win32/volume.rs` 统一管理枚举、刷写与结果汇总；至少一个卷刷写成功即视为该步骤成功。
- **清理进度文案**：已修改文件步骤由显示盘符（如 `C:`）改为显示 `Volume{GUID}`。
- **进程排除交互**：从进程列表选择后直接加入排除列表，移除中间「待确认」状态。
- **Release 构建**：`opt-level` 由 `z` 调整为 `s`。
- **应用图标**：更新 `App.ico` / `App.png`。
- **名称修正**：项目名称由 `Memory Cleanr` 修正为 `Memory Cleaner`，同步更新二进制名、包名、窗口标题、托盘快捷方式、互斥量等所有引用。

## [1.0.1] - 2026-07-16

相对 [1.0.0] 的主要变更：进程排除、全局清理热键（默认 Ctrl+Alt+C）与热键录制、优化完成 Toast、界面国际化、托盘清理动画、图标缓存刷新、调试日志、Windows 10 方形圆角主题等。详见 git history。

## [1.0.0] - 2026-07-11

首个公开发布：8 种内存清理区域、GPUI 界面、系统托盘、管理员提升、设置持久化。

---

# Changelog

Records Memory Cleaner releases. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

**Writing rules** (see `AGENTS.md` → Documentation & Changelog): each release entry covers **only the final diff** vs the previous tag — not intermediate commits or step-by-step fixes during development.


## [1.0.3] - 2026-07-20

Final changes since [1.0.2].

### Added

- **Smooth animations** — Memory usage rings, cleanup progress bar, and memory text values (used/avail bytes) now transition smoothly between data refreshes instead of jumping. Uses exponential-decay interpolation, reaching 95% of target in ~300 ms.
- **Animation module** — New `src/anim.rs` with reusable `AnimatedValue` interpolator for the entire crate.
- **Smart animation pause** — Animation loop fully stops (zero CPU) when the window is hidden to tray; automatically restarts on restore.

### Fixed

- **Tray menu state** — After closing the window, the tray context menu correctly shows "Show Window" instead of "Hide Window"; after restoring from tray, it correctly shows "Hide Window".
- **Window error paths** — `activate_window` and `open_window` failure paths now correctly reset `window_shown` state.
- **Post-optimization tray sync** — Tray tooltip text updates immediately after cleanup completes, instead of waiting for the next mouse hover.
## [1.0.2] - 2026-07-19

Final changes since [1.0.1].

### Added

- **Run at startup** — New setting to launch silently into the system tray after Windows sign-in, without showing the main window (`src/win32/startup.rs`).
- **Process exclusion picker** — List entries show instance count and memory usage; a placeholder when memory cannot be read.
- **Documentation** — `README_EN.md` (English readme) and `docs/API_COMPARISON_MEMREDUCT.md` (cleanup API comparison with Mem Reduct).

### Changed

- **Modified file cache** — Volume discovery switched from iterating fixed drive letters `A:`–`Z:` to Mount Manager enumeration of `\??\Volume{GUID}` with flush via `NtCreateFile` / `NtFlushBuffersFile`; new `src/win32/volume.rs` centralizes enumeration, flush, and reporting; the step succeeds when at least one volume flushes successfully.
- **Cleanup progress text** — Modified-file step now shows `Volume{GUID}` instead of drive letters (e.g. `C:`).
- **Process exclusion UX** — Selecting a process from the list adds it to the exclusion list immediately; removed the intermediate pending-confirmation state.
- **Release build** — `opt-level` changed from `z` to `s`.
- **App icons** — Updated `App.ico` and `App.png`.
- **Name correction** — Project name corrected from `Memory Cleanr` to `Memory Cleaner`; binary name, package name, window title, tray shortcut, mutex, and all other references updated accordingly.

## [1.0.1] - 2026-07-16

Since [1.0.0]: process exclusion, global cleanup hotkey (default Ctrl+Alt+C) with recording, post-optimization toast, UI i18n, tray spin animation during cleanup, icon cache refresh, debug logging, Windows 10 square-corner theme, and more. See git history.

## [1.0.0] - 2026-07-11

Initial public release: 8 memory cleanup regions, GPUI UI, system tray, administrator elevation, settings persistence.

[1.0.3]: https://github.com/Y-ASLant/MemoryCleaner/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/Y-ASLant/MemoryCleaner/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/Y-ASLant/MemoryCleaner/releases/tag/v1.0.1
[1.0.0]: https://github.com/Y-ASLant/MemoryCleaner/releases/tag/v1.0.0
