# Repository Guidelines

## Project Overview

Memory Cleanr is a **Windows-only** GUI memory-optimization tool written in Rust with the **GPUI** framework (from the Zed editor). It frees physical and virtual memory by calling Windows NT memory-management APIs (`NtSetSystemInformation`, `SetSystemFileCacheSize`, etc.), runs as a system-tray resident app, and requires administrator privileges for most operations. Licensed MIT.

## Architecture & Data Flow

```
main.rs  →  ensure_elevated() → single-instance check → tray install → GPUI app launch
                │
                ├─ app.rs          (core state, polling loop, optimization dispatch)
                ├─ memory.rs       (GlobalMemoryStatusEx → MemoryStatus)
                ├─ optimize.rs     (MemoryAreas bitflags → NT cache-purge steps)
                ├─ settings.rs     (TOML persistence at %APPDATA%\MemoryCleaner\settings.toml)
                ├─ privileges.rs   (SeProfileSingleProcessPrivilege, SeIncreaseQuotaPrivilege)
                ├─ tray.rs         (tray-icon crate, App.png embedded via include_bytes!)
                ├─ version.rs      (version constant)
                ├─ ui/             (GPUI components: layout, memory_card, settings_page, title_bar)
                └─ win32/          (nt.rs: raw NT bindings; single_instance.rs: mutex; window.rs: HWND ops)
```

- **Entry flow:** `main.rs` → elevation → single-instance mutex → install tray → run GPUI app → open window with saved settings.
- **Async runtime:** `smol` for async task execution (optimization progress updates).
- **UI stack:** GPUI + `gpui-component` (Button, Checkbox, Switch, GroupBox, PieChart).
- **Native layer:** `src/win32/` wraps low-level Windows APIs; `src/optimize.rs` orchestrates the cleanup steps.
- **Console suppression:** Release builds use `#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]`; diagnostics go to `OutputDebugStringA` (viewable via DebugView).

## Key Directories

| Path | Purpose |
|---|---|
| `src/` | Application source (binary crate, main.rs entry point) |
| `src/ui/` | GPUI UI components (layout, memory_card, settings_page, title_bar) |
| `src/win32/` | Win32/NT API bindings (nt, single_instance, window) |
| `vendor/proc-macro-error2/` | Vendored patch for Rust 1.97+ compatibility (see below) |
| `.codegraph/` | Codegraph index (gitignored) |

## Development Commands

```bash
# Format
make fmt                  # cargo fmt

# Lint (clippy with -D warnings — warnings are errors)
make check                # cargo clippy -- -D warnings

# Build (release, runs clippy first)
make build                # cargo build --release

# Run (debug)
cargo run

# Run (release behavior — console suppressed)
cargo run --release

# Clean
make clean                # cargo clean
```

**No test suite exists.** There are no `#[test]` functions, no `tests/` directory, no `[dev-dependencies]`, and no CI/CD configuration.

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

## Important Files

| File | Role |
|---|---|
| `src/main.rs` | Entry point — elevation, single-instance, tray, GPUI launch |
| `src/app.rs` | Core application state and render logic (~25 KB, largest file) |
| `src/optimize.rs` | Memory cleanup orchestration (8 cleaning regions) |
| `src/settings.rs` | TOML settings schema and persistence |
| `src/win32/nt.rs` | Raw NT API bindings (`NtSetSystemInformation`, structs, enums) |
| `Cargo.toml` | Dependencies, features, release profile (LTO, strip, abort-on-panic) |
| `build.rs` | Icon embedding via `winres` |
| `Makefile` | fmt / check / build / clean targets |

## Runtime / Tooling Preferences

- **Toolchain:** Rust 1.96+ with MSVC (Windows Build Tools or Visual Studio required).
- **No rust-toolchain.toml, .cargo/config.toml, clippy.toml, or rustfmt.toml** — defaults only.
- **Async:** `smol` (not tokio).
- **Vendored patch:** `proc-macro-error2` 2.0.1 is vendored under `vendor/` to fix `E0365` on Rust 1.97+ (changes `extern crate proc_macro` to `pub extern crate proc_macro`). Remove when upstream releases a fix.
- **Release profile:** Aggressive optimization — LTO enabled, symbols stripped, `opt-level = "z"` (size), single codegen unit, `panic = "abort"`.
- **Package manager:** Cargo only. No npm, no other package managers.

## Testing & QA

- **No tests exist.** Zero unit tests, integration tests, or benchmarks.
- **No CI/CD** pipelines configured.
- **Manual testing** is the current workflow — run `cargo run` or `cargo run --release` on a Windows machine with admin privileges.
- **Diagnostics:** Use DebugView (Sysinternals) to read `OutputDebugStringA` output in release builds.
