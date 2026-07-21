<p align="center">
  <img src="App.png" alt="Memory Cleaner" width="128" />
</p>

# Memory Cleaner

A Windows memory optimization tool built with Rust + GPUI. Real-time memory monitoring, configurable cleanup regions, system tray resident, global hotkey, and one-click optimization.

**[中文文档](README.md)**

## Features

- **Real-time Memory Monitoring** — Physical and virtual memory usage with ring progress charts; auto-refreshes every **1 second** when the main window is visible, pauses polling when minimized to tray to save CPU; tray icon tooltip updates instantly on hover
- **One-Click Cleanup** — Executes cleanup steps sequentially based on selected regions; progress and result summary shown in the bottom button (retained for ~5 seconds after completion)
- **Configurable Cleanup Regions** — 8 memory regions selectable via checkboxes (Standby List and Standby List Low Priority are mutually exclusive)
- **Global Hotkey** — Default `Ctrl+Alt+C` triggers cleanup; can be toggled and custom combo recorded in the Window Behavior dialog (`RegisterHotKey`)
- **System Notifications** — Windows Toast popups on cleanup start and completion (can be disabled in the Window Behavior dialog)
- **System Tray** — Right-click menu (Optimize Memory, Show/Hide Window, Exit); left-click shows/activates the main window; **spin animation** during cleanup (rotates 90° every 120ms to indicate activity)
- **Window Behavior** — Title bar gear menu: Always on Top, Close to Tray, Debug Logging, Optimization Notifications, Global Hotkey, Language
- **Desktop Icon Cache Refresh** — Title bar refresh button; terminates and restarts Explorer to clear `IconCache.db` and related caches
- **Custom Title Bar** — Settings, icon cache refresh, expand/collapse, minimize, close (no maximize button)
- **Configuration Persistence** — `%APPDATA%\MemoryCleaner\settings.toml`, auto-created on first run
- **Debug Logging** — Optional `App.log` output in the application directory, with automatic cleanup of entries older than 7 days based on timestamp
- **Auto Elevation** — Detects admin privileges at startup and triggers UAC elevation if needed
- **Single Instance** — Named mutex; second instance exits immediately
- **Platform UI Adaptation** — Windows 11 uses default rounded corners; Windows 10 automatically switches to square buttons, cards, dialogs, etc. (build < 22000)
- **Interface Internationalization** — Simplified Chinese / English, switchable in the Window Behavior dialog; default is "Follow System" (detected via `GetUserDefaultUILanguage`)

## System Requirements

| Item | Requirement |
|------|-------------|
| OS | Windows 10 / 11 Desktop (64-bit); Windows 11 requires build ≥ 22000 |
| Rust | 1.96 or later (Edition 2024) |
| Build Tools | MSVC (Visual Studio Build Tools or Visual Studio) |
| Other | Available GPU driver; most cleanup operations require administrator privileges |

## Quick Start

### Build

```bash
make build
# or
cargo build --release
```

Build artifact: `target/release/MemoryCleaner.exe`

### Run

```bash
cargo run
# or (recommended, requires admin privileges)
cargo run --release
```

> The application will automatically request UAC elevation at startup. If the user declines, some cleanup operations will fail.

## Interface Overview

Main window has a fixed width of **520px**, height varies with expand state (collapsed ~**294px**, expanded ~**630px**), top to bottom:

1. **Title Bar** — App name, Window Behavior (gear), icon cache refresh, expand/collapse, minimize, close
2. **Memory Cards** — Physical and virtual memory ring charts (always visible by default)
3. **Expand Panel** — Click the arrow to reveal the "Cleanup Regions" checkbox panel
4. **One-Click Cleanup** — Bottom action button; cleanup progress and results displayed directly in the button

Collapsed by default, showing only memory cards and the cleanup button; expanding the window automatically increases height to accommodate cleanup region settings. Window behavior is configured via the dialog opened from the title bar gear icon; clicking the blank area outside the dialog will not close it.

When closing the main window, if "Close to Tray" is enabled, the window hides and destroys the GPUI window handle, and the process continues running in the tray; clicking the tray icon again reopens the main window.

### Interface Screenshots (Windows 10 / 11 Comparison)

Side-by-side comparison (numbered 1–4 consistent across platforms):

| | Windows 10 (Square Corners) | Windows 11 (Rounded Corners) |
|:--|:--|:--|
| **1 · Collapsed** | ![Win10 Collapsed](assets/Win10_1.png) | ![Win11 Collapsed](assets/Win11_1.png) |
| **2 · Window Behavior** | ![Win10 Window Behavior](assets/Win10_2.png) | ![Win11 Window Behavior](assets/Win11_2.png) |
| **3 · Expanded** | ![Win10 Expanded](assets/Win10_3.png) | ![Win11 Expanded](assets/Win11_3.png) |
| **4 · Window Behavior (Expanded BG)** | ![Win10 Window Behavior-Expanded](assets/Win10_4.png) | ![Win11 Window Behavior-Expanded](assets/Win11_4.png) |

### Windows 10 / 11 UI Style

At startup, the system build number is detected via `RtlGetVersion`, and gpui-component theme corner radius is adjusted automatically — no user configuration needed:

| System | Condition | UI |
|--------|-----------|----|
| Windows 11 | build ≥ 22000 | Default rounded corners (`radius` 6px / `radius_lg` 8px) |
| Windows 10 | build < 22000 | Square corners (`radius` / `radius_lg` set to 0, component shadows disabled) |

Buttons, GroupBox cards, switches, checkboxes, dialogs, settings panels, etc. all follow the theme `radius`; memory ring charts remain circular. Implementation in `src/ui/theme.rs` and `src/win32/os.rs`.

## Cleanup Regions

| Region | Description | Requires Admin |
|--------|-------------|:-:|
| Working Set | Clear all process working sets | Yes |
| System File Cache | Release system file cache | Yes |
| Modified Pages | Flush modified page list | Yes |
| Standby List | Clear standby list | Yes |
| Standby List (Low Priority) | Clear low-priority standby list | Yes |
| Merged Pages | Release merged pages | Yes |
| Modified Files | Flush file system cache for all volumes via Mount Manager | Yes |
| Registry Cache | Flush registry cache | No |

> "Standby List" and "Standby List (Low Priority)" are mutually exclusive — selecting one automatically deselects the other.
>
> **Disk impact:** Of the 8 cleanup regions, only "Modified Files", "Modified Pages", and "Registry Cache" involve disk writes; the remaining 5 (Working Set, System File Cache, Standby List, Standby List Low Priority, Merged Pages) are pure RAM operations. See FAQ below.

Default enabled regions: Working Set, System File Cache, Standby List, Merged Pages (bitmask `42`). Modified Pages, Modified Files, and Registry Cache are disabled by default as they involve disk writes.

## Configuration

Config file: `%APPDATA%\MemoryCleaner\settings.toml`

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `always_on_top` | bool | `false` | Window always on top |
| `close_to_notification_area` | bool | `true` | Hide to tray on close instead of exiting |
| `memory_areas` | u32 | `42` | Cleanup region bitmask (sum of `MemoryAreas` flag bits) |
| `language` | string | `"auto"` | Interface language: `auto` (follow system), `zh-CN`, `en` |
| `debug_logging` | bool | `false` | Write detailed runtime info to `App.log` in the application directory |
| `show_optimization_notifications` | bool | `true` | Show Windows Toast on cleanup start/completion |
| `cleanup_hotkey_enabled` | bool | `true` | Enable global cleanup hotkey |
| `cleanup_hotkey` | string | `"Ctrl+Alt+C"` | Hotkey combo (`Ctrl`/`Alt`/`Shift`/`Win` + letter or digit) |

## Tech Stack

| Dependency | Purpose |
|------------|---------|
| [Rust](https://www.rust-lang.org/) 1.96+ | Language and runtime |
| [GPUI](https://gpui.rs) (Zed source) | GPU-accelerated UI framework |
| [gpui-component](https://longbridge.github.io/gpui-component/zh-CN/docs/components/) | UI components (Button, Checkbox, Switch, GroupBox, ProgressCircle, etc.) |
| [windows-rs](https://github.com/microsoft/windows-rs) 0.62 | Win32 API (memory management, privileges, window control, Toast, RegisterHotKey) |
| [tray-icon](https://crates.io/crates/tray-icon) | System tray icon and menu |
| [smol](https://crates.io/crates/smol) | Async scheduling and blocking task offload |
| [rust-i18n](https://crates.io/crates/rust-i18n) | Interface internationalization (`locales/zh-CN.yml`) |
| [image](https://crates.io/crates/image) | Tray PNG decoding and scaling |

## Project Structure

```
assets/                  # UI screenshots (Win10 / Win11 comparison, numbered 1–4)
locales/
└── zh-CN.yml            # Chinese & English UI strings (rust-i18n _version: 2 format)

src/
├── main.rs              # Entry: UAC, single-instance, notification init, tray/hotkey, GPUI launch
├── app.rs               # Application state, memory polling, optimization, window hide/restore, hotkey recording
├── icon_cache.rs        # Explorer icon cache cleanup
├── locale.rs            # Locale apply, list separator, system language mapping
├── log.rs               # Debug logging to App.log, timestamp-based entry retention
├── memory.rs            # Memory query (GlobalMemoryStatusEx)
├── messages.rs          # Cleanup result message assembly
├── optimize.rs          # 8 cleanup regions and NtSetSystemInformation calls
├── privileges.rs        # Windows privilege elevation
├── settings.rs          # TOML config read/write
├── tray.rs              # System tray icon, tooltip, menu, cleanup spin animation
├── version.rs           # Version constant
├── win32/               # Windows API wrappers
│   ├── hotkey.rs        # RegisterHotKey global hotkey (dedicated message loop thread)
│   ├── notification.rs  # Windows Toast and Start Menu shortcut
│   ├── nt.rs            # NtSetSystemInformation and NT primitives
│   ├── os.rs            # RtlGetVersion, GetUserDefaultUILanguage
│   ├── process.rs       # Process enumeration/termination (Explorer restart)
│   ├── single_instance.rs
│   └── window.rs        # Window always-on-top, hide to tray
└── ui/                  # GPUI UI components
    ├── layout.rs
    ├── memory_card.rs
    ├── settings_page.rs # Cleanup regions, window behavior dialog, hotkey recording
    ├── theme.rs
    └── title_bar.rs
```

## FAQ

**Why are admin privileges required?**

Most cleanup operations are performed through kernel interfaces like `NtSetSystemInformation`, which require privileges such as `SeProfileSingleProcessPrivilege` and `SeIncreaseQuotaPrivilege`. The application automatically detects and requests UAC elevation at startup.

**Will freeing memory slow down the system?**

Windows will reload frequently used pages into memory on demand. There may be a brief delay after cleanup due to cache rebuilding, but no long-term impact; when memory is tight, active cleanup can free up more available memory.

**What if the global hotkey doesn't work?**

Check whether the hotkey is enabled in the Window Behavior dialog, and whether the key combination conflicts with other software. Hotkeys are registered via `RegisterHotKey` and require at least one modifier key (Ctrl/Alt/Shift/Win) plus a letter or digit.

**How do I view logs?**

- **Always available:** Diagnostic output goes to `OutputDebugString`, viewable with [DebugView](https://learn.microsoft.com/en-us/sysinternals/downloads/debugview) (Release builds have no console window).
- **Debug logging:** Enable "Debug Logging" in the title bar gear menu; detailed runtime info is written to `App.log` in the application directory (same directory as `MemoryCleaner.exe`). Each line is formatted as `[unix_secs.millis] message`; entries with timestamps older than 7 days are automatically purged on write.

**Which cleanup operations affect disk lifespan?**

The 8 cleanup regions fall into two categories by disk behavior:

- **Pure RAM operations (no disk impact):** Working Set, System File Cache, Standby List, Standby List (Low Priority), Merged Pages. These only reclaim or discard cached pages in memory without any disk writes. Re-accessing files after cleanup triggers re-reads from disk, but reads do not consume SSD write endurance.

- **Involve disk writes:**
  - **Modified Files** — Flushes file system cache for all volumes via `NtFlushBuffersFile`; write volume depends on accumulated dirty data — highest impact
  - **Modified Pages** — Writes dirty pages from memory back to their backing files via `FlushModifiedList`
  - **Registry Cache** — Writes registry hives to disk via `RegFlushKey`; small data volume — lowest impact

Occasional manual use is not a concern. If you clean frequently, consider excluding "Modified Files" and "Modified Pages" to reduce disk writes.

## Links

- [Linux DO](https://linux.do/new)
