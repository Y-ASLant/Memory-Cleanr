<p align="center">
  <img src="App.png" alt="Memory Cleaner" width="128" />
</p>

# Memory Cleaner

Windows 内存清理工具，基于 Rust + GPUI 构建。提供实时内存监控、可配置的清理区域、系统托盘常驻、全局快捷键，以及一键优化。

**[English](README_EN.md)**

## 功能

- **实时内存监控** — 物理内存与虚拟内存使用情况，环形进度图可视化；主窗口可见时每 **1 秒** 自动刷新，隐藏到托盘后暂停轮询以节省 CPU；鼠标悬停托盘图标时也会即时刷新 Tooltip
- **一键清理** — 按所选区域依次执行；进度与结果摘要显示在底部按钮内（完成后保留约 5 秒）
- **可配置清理区域** — 8 种内存区域通过复选框勾选（待机列表与普通/低优先级互斥）
- **全局快捷键** — 默认 `Ctrl+Alt+C` 触发清理；可在窗口行为对话框中开关、录制自定义组合键（`RegisterHotKey`）
- **系统通知** — 清理开始与完成时弹出 Windows Toast（可在窗口行为对话框中关闭）
- **系统托盘** — 右键菜单（优化内存、显示/隐藏窗口、退出）；左键单击显示/激活主窗口；**执行内存清理时**托盘图标旋转动画（每 120ms 转 90°，表示进行中）
- **窗口行为** — 标题栏齿轮菜单：置顶、关闭时隐藏到托盘、调试日志、优化通知、全局热键、语言
- **桌面图标缓存刷新** — 标题栏刷新按钮；结束并重启 Explorer 以清理 `IconCache.db` 等缓存
- **自定义标题栏** — 设置、图标缓存、展开/收起、最小化、关闭（无最大化按钮）
- **配置持久化** — `%APPDATA%\MemoryCleaner\settings.toml`，首次运行自动创建
- **调试日志** — 可选写入程序目录下的 `App.log`，按行内时间戳自动清理 7 天前的记录
- **自动提权** — 启动时检测管理员权限，不足时触发 UAC 提升
- **单实例** — 命名互斥量；重复启动时第二个实例直接退出
- **平台 UI 适配** — Windows 11 使用默认圆角；Windows 10 自动切换直角按钮、卡片、对话框等（build &lt; 22000）
- **界面国际化** — 简体中文 / English，窗口行为对话框中可切换；默认「跟随系统」（通过 `GetUserDefaultUILanguage` 检测）

## 系统要求

| 项目 | 要求 |
|------|------|
| 操作系统 | Windows 10 / 11 桌面版（64 位）；Windows 11 为 build ≥ 22000 |
| Rust | 1.96 或更高（Edition 2024） |
| 构建工具 | MSVC（Visual Studio Build Tools 或 Visual Studio） |
| 其他 | 可用 GPU 驱动；大部分清理操作需管理员权限 |

## 快速开始

### 构建

```bash
make build
# 或
cargo build --release
```

构建产物：`target/release/MemoryCleaner.exe`

### 运行

```bash
cargo run
# 或（推荐，需管理员权限）
cargo run --release
```

> 程序启动时会自动请求 UAC 提权。若用户拒绝，部分清理操作将失败。

## 界面说明

主窗口固定宽度 **520px**，高度随展开状态变化（折叠约 **294px**，展开约 **630px**），自上而下分为：

1. **标题栏** — 应用名称、窗口行为（齿轮）、图标缓存刷新、展开/收起、最小化、关闭
2. **内存卡片** — 物理内存与虚拟内存环形图（默认始终可见）
3. **展开面板** — 点击箭头后展开「清理区域」复选框面板
4. **一键清理** — 底部操作按钮；清理进度与结果直接显示在按钮内

默认折叠，仅显示内存卡片与清理按钮；展开后窗口自动增高以容纳清理区域设置。窗口行为通过标题栏齿轮图标打开的对话框配置；点击弹窗外空白区域不会关闭对话框。

关闭主窗口时，若开启「关闭时隐藏到托盘」，窗口会隐藏并销毁 GPUI 窗口句柄，进程继续在托盘运行；再次点击托盘图标可重新打开主窗口。

### 界面截图（Windows 10 / 11 对比）

实机运行对比（序号 1–4 两平台一致）：

| | Windows 10（直角） | Windows 11（圆角） |
|:--|:--|:--|
| **1 · 折叠** | ![Win10 折叠](assets/Win10_1.png) | ![Win11 折叠](assets/Win11_1.png) |
| **2 · 窗口行为** | ![Win10 窗口行为](assets/Win10_2.png) | ![Win11 窗口行为](assets/Win11_2.png) |
| **3 · 展开** | ![Win10 展开](assets/Win10_3.png) | ![Win11 展开](assets/Win11_3.png) |
| **4 · 窗口行为（展开背景）** | ![Win10 窗口行为-展开](assets/Win10_4.png) | ![Win11 窗口行为-展开](assets/Win11_4.png) |

### Windows 10 / 11 界面风格

启动时通过 `RtlGetVersion` 检测系统 build 号，自动调整 gpui-component 主题圆角，无需用户配置：

| 系统 | 条件 | 界面 |
|------|------|------|
| Windows 11 | build ≥ 22000 | 默认圆角（`radius` 6px / `radius_lg` 8px） |
| Windows 10 | build &lt; 22000 | 直角（`radius` / `radius_lg` 为 0，关闭组件阴影） |

按钮、GroupBox 卡片、开关、复选框、对话框、设置面板等均跟随主题 `radius`；内存环形图保持圆形。实现见 `src/ui/theme.rs` 与 `src/win32/os.rs`。

## 清理区域

| 区域 | 说明 | 需要管理员 |
|------|------|-----------|
| 工作集 | 清空所有进程工作集 | 是 |
| 系统文件缓存 | 释放系统文件缓存 | 是 |
| 已修改页面 | 刷写已修改页面列表 | 是 |
| 待机列表 | 清空备用列表 | 是 |
| 待机列表（低优先级） | 清空低优先级备用列表 | 是 |
| 合并页面 | 释放合并页面 | 是 |
| 已修改文件 | 通过 Mount Manager 枚举卷并刷写文件系统缓存 | 是 |
| 注册表缓存 | 刷写注册表缓存 | 否 |

> 「待机列表」与「待机列表（低优先级）」只能二选一，勾选其一会自动取消另一项。
>
> **磁盘影响：** 8 个清理区域中，仅「已修改文件」「已修改页面」「注册表缓存」涉及磁盘写入；其余 5 项（工作集、系统文件缓存、待机列表、待机列表低优先级、合并页面）均为纯 RAM 操作，不写磁盘。详见下方 FAQ。

默认启用的区域：工作集、系统文件缓存、待机列表、合并页面（位掩码 `42`）。已修改页面、已修改文件和注册表缓存因涉及磁盘写入，默认不启用。

## 配置项

配置文件：`%APPDATA%\MemoryCleaner\settings.toml`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `always_on_top` | bool | `false` | 窗口始终置顶 |
| `close_to_notification_area` | bool | `true` | 点击关闭时隐藏到托盘而非退出 |
| `memory_areas` | u32 | `42` | 清理区域位掩码（各 `MemoryAreas` 标志位之和） |
| `language` | string | `"auto"` | 界面语言：`auto`（跟随系统）、`zh-CN`、`en` |
| `debug_logging` | bool | `false` | 将详细运行信息写入程序目录下的 `App.log` |
| `show_optimization_notifications` | bool | `true` | 清理开始/完成时弹出 Windows Toast |
| `cleanup_hotkey_enabled` | bool | `true` | 启用全局清理热键 |
| `cleanup_hotkey` | string | `"Ctrl+Alt+C"` | 热键组合（`Ctrl`/`Alt`/`Shift`/`Win` + 字母或数字） |

## 技术栈

| 依赖 | 用途 |
|------|------|
| [Rust](https://www.rust-lang.org/) 1.96+ | 语言与运行时 |
| [GPUI](https://gpui.rs)（Zed 源码） | GPU 加速 UI 框架 |
| [gpui-component](https://longbridge.github.io/gpui-component/zh-CN/docs/components/) | UI 组件（Button、Checkbox、Switch、GroupBox、ProgressCircle 等） |
| [windows-rs](https://github.com/microsoft/windows-rs) 0.62 | Win32 API（内存管理、权限、窗口控制、Toast、RegisterHotKey） |
| [tray-icon](https://crates.io/crates/tray-icon) | 系统托盘图标与菜单 |
| [smol](https://crates.io/crates/smol) | 异步定时与阻塞任务卸载 |
| [rust-i18n](https://crates.io/crates/rust-i18n) | 界面国际化（`locales/zh-CN.yml`） |
| [image](https://crates.io/crates/image) | 托盘 PNG 解码与缩放 |

## 项目结构

```
assets/                  # 界面截图（Win10 / Win11 对比，1–4 序号一致）
locales/
└── zh-CN.yml            # 中英文 UI 文案（rust-i18n _version: 2 格式）

src/
├── main.rs              # 入口：UAC、单实例、通知初始化、托盘/热键、GPUI 启动
├── app.rs               # 应用状态、内存轮询、优化流程、窗口隐藏/恢复、热键录制
├── icon_cache.rs        # Explorer 图标缓存清理
├── locale.rs            # locale 应用、列表分隔符、系统语言映射
├── log.rs               # 调试日志写入 App.log，按行内时间戳清理过期记录
├── memory.rs            # 内存查询（GlobalMemoryStatusEx）
├── messages.rs          # 清理结果文案组装
├── optimize.rs          # 8 种清理区域与 NtSetSystemInformation 调用
├── privileges.rs        # Windows 特权提升
├── settings.rs          # TOML 配置读写
├── tray.rs              # 系统托盘图标、Tooltip、菜单、清理过程旋转动画
├── version.rs           # 版本常量
├── win32/               # Windows API 封装
│   ├── hotkey.rs        # RegisterHotKey 全局热键（独立消息循环线程）
│   ├── notification.rs  # Windows Toast 与开始菜单快捷方式
│   ├── nt.rs            # NtSetSystemInformation 等 NT 原语
│   ├── os.rs            # RtlGetVersion、GetUserDefaultUILanguage
│   ├── process.rs       # 进程枚举/结束（Explorer 重启）
│   ├── single_instance.rs
│   └── window.rs        # 窗口置顶、隐藏到托盘
└── ui/                  # GPUI UI 组件
    ├── layout.rs
    ├── memory_card.rs
    ├── settings_page.rs # 清理区域、窗口行为对话框、热键录制
    ├── theme.rs
    └── title_bar.rs
```

## 常见问题

**为什么需要管理员权限？**

大部分清理操作通过 `NtSetSystemInformation` 等内核接口完成，需要 `SeProfileSingleProcessPrivilege`、`SeIncreaseQuotaPrivilege` 等特权。程序启动时会自动检测并请求 UAC 提权。

**释放内存会导致系统变慢吗？**

Windows 会按需将常用页面重新加载到内存。清理后短期内可能因缓存重建而略有延迟，但不会造成长期影响；在内存紧张时，主动清理可释放更多可用内存。

**全局热键不生效怎么办？**

检查窗口行为对话框中是否启用了热键，以及组合键是否与其他软件冲突。热键通过 `RegisterHotKey` 注册，需要至少一个修饰键（Ctrl/Alt/Shift/Win）加一个字母或数字。

**如何查看日志？**

- **始终可用：** 诊断信息通过 `OutputDebugString` 输出，可用 [DebugView](https://learn.microsoft.com/en-us/sysinternals/downloads/debugview) 查看（Release 构建无控制台窗口）。
- **调试日志：** 在标题栏齿轮菜单中开启「调试日志」后，详细运行信息写入程序目录下的 `App.log`（与 `MemoryCleaner.exe` 同目录）。每行格式为 `[unix_secs.millis] 消息`；写入新日志时会自动删除时间戳早于 7 天的旧行。

**哪些清理操作会影响磁盘寿命？**

8 个清理区域按磁盘行为分为两类：

- **纯 RAM 操作（不影响磁盘）：** 工作集、系统文件缓存、待机列表、待机列表（低优先级）、合并页面。这些操作仅回收或丢弃内存中的缓存页面，不产生任何磁盘写入。清理后再次访问文件时会从磁盘重新读取，但读取不会消耗 SSD 写入寿命。

- **涉及磁盘写入：**
  - **已修改文件** — 通过 `NtFlushBuffersFile` 刷写所有卷的文件系统缓存到磁盘，写入量取决于积压的脏数据量，影响最大
  - **已修改页面** — 通过 `FlushModifiedList` 将内存中的脏页写回对应文件
  - **注册表缓存** — 通过 `RegFlushKey` 写入注册表配置单元，数据量小，影响最小

日常手动使用无须担心；若频繁清理，建议避开「已修改文件」和「已修改页面」以减少磁盘写入。

## 友链

- [Linux DO](https://linux.do/new)
