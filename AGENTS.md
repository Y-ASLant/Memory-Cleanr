# Repository Guidelines

## Project Overview

Memory Cleanr is a **Windows-only** GUI memory-optimization tool written in Rust with the **GPUI** framework (from the Zed editor). It frees physical and virtual memory by calling Windows NT memory-management APIs (`NtSetSystemInformation`, `SetSystemFileCacheSize`, etc.), runs as a system-tray resident app, and requires administrator privileges for most operations. Licensed MIT.

## Architecture & Data Flow

```
main.rs  →  ensure_elevated() → single-instance check → tray install → GPUI app launch
                │
                ├─ app.rs          (core state, polling loop, optimization dispatch)
                ├─ log.rs          (optional App.log file output, timestamp-based retention)
                ├─ memory.rs       (GlobalMemoryStatusEx → MemoryStatus)
                ├─ optimize.rs     (MemoryAreas bitflags → NT cache-purge steps)
                ├─ settings.rs     (TOML persistence at %APPDATA%\MemoryCleaner\settings.toml)
                ├─ privileges.rs   (SeProfileSingleProcessPrivilege, SeIncreaseQuotaPrivilege)
                ├─ tray.rs         (tray-icon crate, App.png embedded via include_bytes!)
                ├─ version.rs      (version constant)
                ├─ ui/             (GPUI components: layout, memory_card, settings_page, theme, title_bar)
                └─ win32/          (nt.rs, os.rs, single_instance.rs, window.rs)
```

- **Entry flow:** `main.rs` → elevation → single-instance mutex → install tray → run GPUI app → `ui::theme::init_light_theme` → open window with saved settings.
- **Async runtime:** `smol` for async task execution (optimization progress updates).
- **UI stack:** GPUI + `gpui-component` (Button, Checkbox, Switch, GroupBox, ProgressCircle).
- **Native layer:** `src/win32/` wraps low-level Windows APIs; `src/optimize.rs` orchestrates the cleanup steps.
- **Console suppression:** `main.rs` uses `#![windows_subsystem = "windows"]`; diagnostics go to `OutputDebugStringA` (viewable via DebugView). Optional file logging via `src/log.rs` when `debug_logging` is enabled.

## Key Directories

| Path | Purpose |
|---|---|
| `src/` | Application source (binary crate, main.rs entry point) |
| `src/ui/` | GPUI UI components (layout, memory_card, settings_page, theme, title_bar) |
| `src/win32/` | Win32/NT API bindings (nt, os, single_instance, window) |
| `vendor/proc-macro-error2/` | Vendored patch for Rust 1.97+ compatibility (see below) |
| `.codegraph/` | Codegraph index (gitignored) |

## Development Commands

```bash
# Format
make fmt                  # cargo fmt

# Lint (clippy with -D warnings — warnings are errors)
make check                # cargo clippy -- -D warnings

# Test
make test                 # cargo test

# Build (release, runs clippy first)
make build                # cargo build --release

# Run (debug)
cargo run

# Run (release behavior — console suppressed)
cargo run --release

# Clean
make clean                # cargo clean
```

**Tests:** `make test` / `cargo test` — 18 unit tests in `src/` plus 2 integration tests in `tests/settings_persistence.rs`. Pure logic (memory formatting, cleanup messages, settings TOML, tray tooltip text, optimize step plan, layout metrics) is covered; Win32/GPUI paths remain manual QA.

## Code Conventions & Common Patterns

- **Language:** Rust, Edition 2024 (requires Rust 1.96+).
- **Platform:** Windows-only. All modules assume `target_os = "windows"`.
- **Error handling:** Functions return `Result<T, E>` or use `Option` for fallible lookups. No `anyhow` or `eyre` — errors are propagated via `?` with concrete error types.
- **Unsafe / FFI:** `unsafe` is concentrated in `src/win32/` (NT API calls, privilege token manipulation) and `src/optimize.rs` (NtSetSystemInformation). Each unsafe block is narrowly scoped.
- **Naming:** Standard Rust conventions — `snake_case` functions/variables, `PascalCase` types, `SCREAMING_SNAKE_CASE` constants. Win32 wrappers match the original API names.
- **State management:** `MemoryCleanerApp` in `app.rs` owns all application state (settings, memory stats, animation state, optimization progress). UI reads from this state via GPUI's `Render` trait.
- **Settings persistence:** TOML file at `%APPDATA%\MemoryCleaner\settings.toml`, written atomically (temp file + rename).
- **Bitflags:** `MemoryAreas` in `optimize.rs` uses the `bitflags` crate to represent configurable cleaning regions.
- **Embedded assets:** `App.ico` compiled into the binary via `winres` (build.rs); `App.png` embedded via `include_bytes!` in `tray.rs`.
- **Debug logging:** `log_msg()` always writes to `OutputDebugString` (and stderr in debug builds). `log::write()` additionally appends to `App.log` beside the executable when `settings.debug_logging` is true. Before each write, `log.rs` purges lines whose `[unix_secs.millis]` prefix is older than 7 days (`LOG_RETENTION_SECS`).
- **Platform UI chrome:** `win32::os::is_windows_11_or_later()` uses `RtlGetVersion` (build ≥ 22000 = Win11). `ui::theme::init_light_theme` sets gpui-component `radius` / `radius_lg` to 0 and disables `shadow` on Win10 so buttons, cards, and dialogs render with square corners. Custom UI must use `cx.theme().radius`, not hardcoded `rounded(px(...))`.

## Important Files

| File | Role |
|---|---|
| `src/main.rs` | Entry point — elevation, single-instance, tray, GPUI launch |
| `src/app.rs` | Core application state and render logic (~25 KB, largest file) |
| `src/log.rs` | Optional `App.log` file output with timestamp-based line retention |
| `src/ui/theme.rs` | Light theme init + Win10 square-corner chrome |
| `src/win32/os.rs` | Windows build detection (Win10 vs Win11) |
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
- **Window behavior dialog** (always on top, close-to-tray, debug logging): opened from title-bar gear icon; `overlay_closable(false)` — clicking the backdrop does not close it.
- **Optimization feedback:** progress and result text render inside the cleanup button; result clears after 5 seconds (`OPTIMIZE_RESULT_DISPLAY`).
- **Platform chrome:** Win10 (build &lt; 22000) uses square corners via theme tokens; Win11 keeps gpui-component defaults. Dialog centering uses `layout::centered_dialog_margin_top` with `WINDOW_BEHAVIOR_DIALOG_WIDTH` (480px).

## Unimplemented Settings (Reserved)

These fields exist in `settings.toml` for forward compatibility but have no runtime logic yet:

- `auto_optimization_interval` / `auto_optimization_memory_usage` — scheduled or threshold-triggered auto cleanup
- `show_optimization_notifications` — completion notifications (in-window toast / system toast when hidden)
- `tray_icon_*` — dynamic tray icon based on memory usage

## Runtime / Tooling Preferences

- **Toolchain:** Rust 1.96+ with MSVC (Windows Build Tools or Visual Studio required).
- **No rust-toolchain.toml, .cargo/config.toml, clippy.toml, or rustfmt.toml** — defaults only.
- **Async:** `smol` (not tokio).
- **Vendored patch:** `proc-macro-error2` 2.0.1 is vendored under `vendor/` to fix `E0365` on Rust 1.97+ (changes `extern crate proc_macro` to `pub extern crate proc_macro`). Remove when upstream releases a fix.
- **Release profile:** Aggressive optimization — LTO enabled, symbols stripped, `opt-level = "z"` (size), single codegen unit, `panic = "abort"`.
- **Package manager:** Cargo only. No npm, no other package managers.
- **Binary name:** `MemoryCleanr.exe` (see `[[bin]]` name in `Cargo.toml`).

## Testing & QA

- **Unit tests:** `cargo test` — memory formatting, cleanup messages, settings TOML, tray tooltip, optimize step plan, layout metrics, icon-cache outcomes.
- **Integration tests:** `tests/settings_persistence.rs` — settings save/load and atomic write in isolated `%APPDATA%`.
- **Manual QA:** Win32 memory cleanup, tray, GPUI dialogs, Explorer restart (admin required).
- **Diagnostics:** DebugView for `OutputDebugString`; optional `App.log` when debug logging is enabled.
