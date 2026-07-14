# Repository Guidelines

## Project Overview

Memory Cleanr is a **Windows-only** GUI memory-optimization tool written in Rust with the **GPUI** framework (from the Zed editor). It frees physical and virtual memory by calling Windows NT memory-management APIs (`NtSetSystemInformation`, `SetSystemFileCacheSize`, etc.), runs as a system-tray resident app, and requires administrator privileges for most operations. Licensed MIT.

## Architecture & Data Flow

```
main.rs → ensure_elevated() → single-instance check → notification::init
       → tray install + hotkey::sync → GPUI app launch
 │
 ├─ app.rs (core state, memory refresh, optimization, window hide/restore)
 ├─ log.rs (optional App.log file output, timestamp-based retention)
 ├─ locale.rs (rust-i18n locale apply, list separator, lang-id mapping)
 ├─ memory.rs (GlobalMemoryStatusEx → MemoryStatus)
 ├─ optimize.rs (MemoryAreas bitflags → NT cache-purge steps)
 ├─ settings.rs (TOML persistence at %APPDATA%\MemoryCleaner\settings.toml)
 ├─ privileges.rs (SeProfileSingleProcessPrivilege, SeIncreaseQuotaPrivilege)
 ├─ tray.rs (tray-icon crate, App.png embedded via include_bytes!)
 ├─ icon_cache.rs (Explorer icon cache purge)
 ├─ version.rs (version constant)
 ├─ ui/ (GPUI components: layout, memory_card, settings_page, theme, title_bar)
 └─ win32/ (hotkey, notification, nt, os, process, single_instance, window)
```

- **Entry flow:** `main.rs` → elevation → single-instance mutex → `locale::apply` → `notification::init` → install tray + bind hotkey sender → `hotkey::sync` → GPUI app with `QuitMode::Explicit` → `open_main_window`.
- **i18n:** `rust-i18n` with `locales/zh-CN.yml` (single file, `_version: 2`, zh-CN + en). `rust_i18n::i18n!` is invoked once in `lib.rs`. `settings.language` is `auto` | `zh-CN` | `en`; `auto` uses `GetUserDefaultUILanguage` via `win32::os::system_ui_locale()`. Language changes call `MemoryCleanerApp::apply_locale()` to refresh memory labels and tray menu text immediately.
- **Async runtime:** `smol` for async task execution (optimization progress updates, memory polling, toast display).
- **UI stack:** GPUI + `gpui-component` (Button, Checkbox, Switch, GroupBox, ProgressCircle, Kbd).
- **Native layer:** `src/win32/` wraps low-level Windows APIs; `src/optimize.rs` orchestrates the cleanup steps.
- **Console suppression:** `main.rs` uses `#![windows_subsystem = "windows"]`; diagnostics go to `OutputDebugStringA` (viewable via DebugView). Optional file logging via `src/log.rs` when `debug_logging` is enabled.
- **Tray command channel:** A single `mpsc` channel carries `TrayCommand` from tray events, global hotkeys, and (future) background tasks into `app.rs` via blocking `recv()` — no idle polling loop.
- **Window lifecycle:** Closing with `close_to_notification_area` hides the GPUI window to tray and may destroy the window handle; `activate_window` reopens it. Memory polling pauses while hidden.

## Key Directories

| Path | Purpose |
|---|---|
| `src/` | Application source (binary crate, main.rs entry point) |
| `src/ui/` | GPUI UI components (layout, memory_card, settings_page, theme, title_bar) |
| `locales/` | rust-i18n translation YAML (`zh-CN.yml`, zh-CN + en strings) |
| `src/win32/` | Win32/NT API bindings (hotkey, notification, nt, os, process, single_instance, window) |
| `vendor/proc-macro-error2/` | Vendored patch for Rust 1.97+ compatibility (see below) |
| `.codegraph/` | Codegraph index (gitignored) |

## Development Commands

```bash
# Format
make fmt # cargo fmt

# Lint (clippy with -D warnings — warnings are errors)
make check # cargo clippy -- -D warnings

# Test
make test # cargo test

# Build (release, runs clippy first)
make build # cargo build --release

# Run (debug)
cargo run

# Run (release behavior — console suppressed)
cargo run --release

# Clean
make clean # cargo clean
```

**Tests:** `make test` / `cargo test` — 36 unit tests in `src/` plus 2 integration tests in `tests/settings_persistence.rs`. Pure logic (memory formatting, cleanup messages, settings TOML/locale, tray tooltip, hotkey parsing, optimize step plan, layout metrics, icon-cache outcomes, notification XML escape) is covered; Win32/GPUI paths remain manual QA.

## Code Conventions & Common Patterns

- **Language:** Rust, Edition 2024 (requires Rust 1.96+).
- **Platform:** Windows-only. All modules assume `target_os = "windows"`.
- **Error handling:** Functions return `Result<T, E>` or use `Option` for fallible lookups. `anyhow` is used in optimize/icon_cache paths; settings and most UI code use concrete errors.
- **Unsafe / FFI:** `unsafe` is concentrated in `src/win32/` (NT API calls, privilege token manipulation, hotkey message loop) and `src/optimize.rs` (NtSetSystemInformation). Each unsafe block is narrowly scoped.
- **Naming:** Standard Rust conventions — `snake_case` functions/variables, `PascalCase` types, `SCREAMING_SNAKE_CASE` constants. Win32 wrappers match the original API names.
- **State management:** `MemoryCleanerApp` in `app.rs` owns all application state (settings, memory stats, optimization progress, hotkey recording). UI reads from this state via GPUI's `Render` trait.
- **Settings persistence:** TOML file at `%APPDATA%\MemoryCleaner\settings.toml`, written atomically (temp file + rename), debounced 300 ms.
- **Bitflags:** `MemoryAreas` in `optimize.rs` uses the `bitflags` crate to represent configurable cleaning regions.
- **Embedded assets:** `App.ico` compiled into the binary via `winres` (`build.rs`); `App.png` embedded via `include_bytes!` in `tray.rs`.
- **Debug logging:** `log_msg()` always writes to `OutputDebugString` (and stderr in debug builds). `log::write()` additionally appends to `App.log` beside the executable when `settings.debug_logging` is true. Before each write, `log.rs` purges lines whose `[unix_secs.millis]` prefix is older than 7 days (`LOG_RETENTION_SECS`).
- **Platform UI chrome:** `win32::os::is_windows_11_or_later()` uses `RtlGetVersion` (build ≥ 22000 = Win11). `ui::theme::init_light_theme` sets gpui-component `radius` / `radius_lg` to 0 and disables `shadow` on Win10 so buttons, cards, and dialogs render with square corners. Custom UI must use `cx.theme().radius`, not hardcoded `rounded(px(...))`.

## Important Files

| File | Role |
|---|---|
| `src/main.rs` | Entry point — elevation, single-instance, notification init, tray/hotkey setup, GPUI launch |
| `src/app.rs` | Core application state, memory refresh loop, optimization, window hide/restore, hotkey recording |
| `src/tray.rs` | Tray icon install, tooltip/menu sync, command dispatch |
| `src/win32/hotkey.rs` | `RegisterHotKey` in dedicated thread; sends `TrayCommand::Optimize` |
| `src/win32/notification.rs` | Windows Toast + Start Menu shortcut for AppUserModelID |
| `src/log.rs` | Optional `App.log` file output with timestamp-based line retention |
| `src/ui/theme.rs` | Light theme init + Win10 square-corner chrome |
| `src/locale.rs` | rust-i18n locale apply, list separator, lang-id mapping |
| `src/win32/os.rs` | Windows build detection (Win10 vs Win11), system UI locale |
| `src/optimize.rs` | Memory cleanup orchestration (8 cleaning regions) |
| `src/settings.rs` | TOML settings schema and persistence |
| `src/win32/nt.rs` | Raw NT API bindings (`NtSetSystemInformation`, structs, enums) |
| `Cargo.toml` | Dependencies, features, release profile (LTO, strip, abort-on-panic) |
| `build.rs` | Icon embedding via `winres` |
| `Makefile` | fmt / check / build / clean targets |

## UI Layout Notes

- **Window size:** fixed width 520px; collapsed height ~294px, expanded ~456px (`src/app.rs` + `src/ui/layout.rs`).
- **Collapsed view:** memory cards + cleanup button.
- **Expanded view:** adds cleanup-area checkboxes panel (`settings_page::render_settings_content`).
- **Window behavior dialog** (always on top, close-to-tray, debug logging, optimization notifications, cleanup hotkey + recording, language): opened from title-bar gear icon; `overlay_closable(false)` — clicking the backdrop does not close it.
- **Optimization feedback:** progress and result text render inside the cleanup button; result clears after 5 seconds (`OPTIMIZE_RESULT_DISPLAY`).
- **Memory refresh:** `MEMORY_REFRESH_INTERVAL` = 1 s while main window is visible; paused when hidden to tray (`pause_memory_refresh` / `start_memory_refresh`).
- **Platform chrome:** Win10 (build &lt; 22000) uses square corners via theme tokens; Win11 keeps gpui-component defaults.

## Unimplemented Settings (Reserved)

These fields exist in `settings.toml` for forward compatibility but have no runtime logic yet:

- `auto_optimization_interval` / `auto_optimization_memory_usage` — scheduled or threshold-triggered auto cleanup
- `tray_icon_*` — dynamic tray icon based on memory usage (see **Dynamic Tray Icon** below)

Implemented since earlier docs (do **not** list as unimplemented):

- `show_optimization_notifications` — Windows Toast on optimize start/complete
- `cleanup_hotkey_enabled` / `cleanup_hotkey` — global hotkey via `RegisterHotKey`

## Dynamic Tray Icon (Not Yet Implemented)

Reserved settings in `settings.rs`:

| Field | Default | Intended behavior |
|---|---|---|
| `tray_icon_show_memory_usage` | `false` | Master switch; when off, keep static `App.png` icon |
| `tray_icon_use_transparent_background` | `false` | Draw icon on transparent vs solid dark background |
| `tray_icon_warning_level` | `80` | Physical memory % ≥ this → warning (yellow) tier |
| `tray_icon_danger_level` | `90` | Physical memory % ≥ this → danger (red) tier |

### Recommended implementation

1. **New module** `src/tray_icon.rs` (or extend `tray.rs`):
   - Pure functions: `memory_tier(percent, warning, danger) -> Normal | Warning | Danger`
   - `render_tray_icon(base_rgba, percent, tier, show_percent, transparent_bg) -> Vec<u8>` using the existing `image` crate
   - Build 32×32 RGBA buffer → `tray_icon::Icon::from_rgba` → `TrayIcon::set_icon`

2. **Icon rendering strategy** (pick one, test on Win10/11 tray):
   - **Tier tint:** Decode embedded `App.png` once, overlay a colored ring or multiply tint by tier color (green / `#F59E0B` / `#EF4444`, matching `memory_card.rs` thresholds).
   - **Percent badge:** When `tray_icon_show_memory_usage`, draw 1–2 digit `used_percent` centered (reuse ring colors for text).
   - **Cache by bucket:** Key `(tier, percent_decile, show_percent, transparent_bg)` in a `OnceLock<HashMap<_, Icon>>` to avoid reallocating every tick; at most ~30 entries.

3. **Hook into `sync_display`** in `tray.rs`:
   ```rust
   pub fn sync_display(physical, virtual_mem, window_visible, settings: &Settings) {
       // existing tooltip + menu text ...
       if settings.tray_icon_show_memory_usage {
           if let Some(icon) = tray_icon::icon_for_memory(physical, settings) {
               let _ = tray.icon.set_icon(Some(icon));
           }
       }
   }
   ```
   Track `last_tray_tier` / `last_tray_percent` on `MemoryCleanerApp` and skip `set_icon` when unchanged (reduces flicker).

4. **Background polling when hidden:** Current `start_memory_refresh` only runs while `window_shown`. For a live tray icon when the window is hidden, add a separate loop in `start_background_tasks`:
   - If `tray_icon_show_memory_usage`: poll memory every 5–10 s even when hidden, call `refresh_memory` + `sync_tray`.
   - If disabled: keep current behavior (refresh on tray hover only when hidden).

5. **Settings UI:** Add rows to `render_window_behavior_dialog` in `settings_page.rs` — switch for `tray_icon_show_memory_usage`, optional transparent background, numeric inputs or sliders for warning/danger levels (clamp 1–99, enforce `warning < danger` in `normalize_*`).

6. **Tests:** Unit-test `memory_tier`, icon cache key stability, and that `sync_display` skips `set_icon` when tier unchanged (mock or extract pure logic).

7. **Manual QA:** Verify 32×32 clarity in light/dark taskbar, HiDPI scaling, and that frequent `set_icon` does not flicker the notification area.

## Runtime / Tooling Preferences

- **Toolchain:** Rust 1.96+ with MSVC (Windows Build Tools or Visual Studio required).
- **No rust-toolchain.toml, .cargo/config.toml, clippy.toml, or rustfmt.toml** — defaults only.
- **Async:** `smol` (not tokio).
- **Vendored patch:** `proc-macro-error2` 2.0.1 is vendored under `vendor/` to fix `E0365` on Rust 1.97+ (changes `extern crate proc_macro` to `pub extern crate proc_macro`). Remove when upstream releases a fix.
- **Release profile:** Aggressive optimization — LTO enabled, symbols stripped, `opt-level = "z"` (size), single codegen unit, `panic = "abort"`.
- **Package manager:** Cargo only. No npm, no other package managers.
- **Binary name:** `MemoryCleanr.exe` (see `[[bin]]` name in `Cargo.toml`).

## Testing & QA

- **Unit tests:** `cargo test` — memory formatting, cleanup messages, settings TOML, tray tooltip, hotkey chord parse/format, optimize step plan, layout metrics, icon-cache outcomes, notification XML escape.
- **Integration tests:** `tests/settings_persistence.rs` — settings save/load and atomic write in isolated `%APPDATA%`.
- **Manual QA:** Win32 memory cleanup, tray, GPUI dialogs, Explorer restart, global hotkey, Windows Toast (admin required for most cleanup).
- **Diagnostics:** DebugView for `OutputDebugString`; optional `App.log` when debug logging is enabled.
