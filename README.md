# Memory Cleanr

Windows 内存清理工具，基于 Rust + GPUI 构建。提供实时内存监控、可配置的清理区域、系统托盘常驻，以及一键优化。

## 功能

- **实时内存监控** — 物理内存与虚拟内存使用情况，饼图可视化，约每 3 秒自动刷新
- **一键清理** — 按所选区域依次执行，显示进度与结果摘要
- **可配置清理区域** — 8 种内存区域通过复选框勾选（待机列表与普通/低优先级互斥）
- **系统托盘** — 右键菜单（优化内存、显示/隐藏窗口、退出）；左键单击显示主窗口
- **窗口行为** — 置顶、关闭时隐藏到托盘、启动时最小化到托盘
- **自定义标题栏** — 最小化、关闭（无最大化按钮）
- **配置持久化** — `%APPDATA%\MemoryCleaner\settings.toml`，首次运行自动创建
- **自动提权** — 启动时检测管理员权限，不足时触发 UAC 提升

## 系统要求

| 项目 | 要求 |
|------|------|
| 操作系统 | Windows 10 / 11（仅支持 Windows） |
| Rust | 1.96 或更高（Edition 2024） |
| 构建工具 | MSVC（Visual Studio Build Tools 或 Visual Studio） |

## 快速开始

### 构建

```bash
make build
# 或
cargo build --release
```

构建产物：`target/release/memory-cleanr.exe`

### 运行

```bash
cargo run
# 或（推荐，需管理员权限）
cargo run --release
```

> 程序启动时会自动请求 UAC 提权。若用户拒绝，部分清理操作将失败。

## 界面说明

主窗口默认折叠尺寸 **560 × 300**（最小可缩至该尺寸），展开设置后为 **560 × 620**，自上而下分为：

1. **标题栏** — 应用名称与窗口控制
2. **内存卡片** — 物理内存与虚拟内存饼图（默认始终可见）
3. **展开设置** — 点击后展开窗口行为开关与清理区域复选框
4. **一键清理** — 底部操作按钮；清理进度与结果显示在按钮上方

默认折叠，仅显示内存卡片与清理按钮；展开后窗口自动增高以容纳全部设置。

## 清理区域

| 区域 | 说明 | 需要管理员 |
|------|------|-----------|
| 工作集 | 清空所有进程工作集 | 是 |
| 系统文件缓存 | 释放系统文件缓存 | 是 |
| 已修改页面 | 刷写已修改页面列表 | 是 |
| 待机列表 | 清空备用列表 | 是 |
| 待机列表（低优先级） | 清空低优先级备用列表 | 是 |
| 合并页面 | 释放合并页面 | 是 |
| 已修改文件 | 清理各固定磁盘的已修改文件缓存 | 是 |
| 注册表缓存 | 刷写注册表缓存 | 否 |

> 「待机列表」与「待机列表（低优先级）」只能二选一，勾选其一会自动取消另一项。

默认启用的区域：工作集、系统文件缓存、已修改页面、待机列表、合并页面、已修改文件（位掩码 `111`）。

## 配置项

配置文件：`%APPDATA%\MemoryCleaner\settings.toml`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `always_on_top` | bool | `false` | 窗口始终置顶 |
| `close_to_notification_area` | bool | `true` | 点击关闭时隐藏到托盘而非退出 |
| `show_virtual_memory` | bool | `true` | 显示虚拟内存卡片（仅配置文件，暂无 UI 开关） |
| `start_minimized` | bool | `false` | 启动时直接最小化到托盘 |
| `memory_areas` | u32 | `111` | 清理区域位掩码（各 `MemoryAreas` 标志位之和） |

以下为预留字段，当前版本尚未实现：

| 配置项 | 默认值 | 计划用途 |
|--------|--------|----------|
| `auto_optimization_interval` | `0` | 自动优化间隔 |
| `auto_optimization_memory_usage` | `0` | 自动优化内存阈值 |
| `show_optimization_notifications` | `true` | 优化完成通知 |
| `tray_icon_show_memory_usage` | `false` | 托盘图标显示内存占用 |
| `tray_icon_use_transparent_background` | `false` | 托盘图标透明背景 |
| `tray_icon_warning_level` | `80` | 托盘图标警告阈值（%） |
| `tray_icon_danger_level` | `90` | 托盘图标危险阈值（%） |

## 技术栈

| 依赖 | 用途 |
|------|------|
| [Rust](https://www.rust-lang.org/) 1.96+ | 语言与运行时 |
| [GPUI](https://gpui.rs)（Zed 源码） | GPU 加速 UI 框架 |
| [gpui-component](https://longbridge.github.io/gpui-component/zh-CN/docs/components/) | UI 组件（Button、Checkbox、Switch、GroupBox、PieChart 等） |
| [windows-rs](https://github.com/microsoft/windows-rs) 0.62 | Win32 API（内存管理、权限、窗口控制） |
| [tray-icon](https://crates.io/crates/tray-icon) | 系统托盘图标与菜单 |
| [smol](https://crates.io/crates/smol) | 异步定时与阻塞任务卸载 |

## 项目结构

```
src/
├── main.rs              # 入口：UAC 提权、托盘安装、GPUI 窗口初始化
├── app.rs               # 应用状态、内存轮询、优化流程、托盘事件
├── memory.rs            # 内存查询（GlobalMemoryStatusEx）
├── optimize.rs          # 8 种清理区域与 NtSetSystemInformation 调用
├── privileges.rs        # Windows 特权提升
├── settings.rs          # TOML 配置读写
├── tray.rs              # 系统托盘图标与右键菜单
├── win32/               # Windows API 封装
│   ├── mod.rs
│   ├── nt.rs            # NtSetSystemInformation 等 NT 原语
│   └── window.rs        # 窗口置顶、隐藏到托盘等
└── ui/                  # UI 组件
    ├── mod.rs
    ├── memory_card.rs   # 内存饼图卡片
    ├── settings_page.rs # 设置与清理区域面板
    └── title_bar.rs     # 自定义标题栏
```

## 常见问题

**为什么需要管理员权限？**

大部分清理操作通过 `NtSetSystemInformation` 等内核接口完成，需要 `SeProfileSingleProcessPrivilege`、`SeIncreaseQuotaPrivilege` 等特权。程序启动时会自动检测并请求 UAC 提权。

**释放内存会导致系统变慢吗？**

Windows 会按需将常用页面重新加载到内存。清理后短期内可能因缓存重建而略有延迟，但不会造成长期影响；在内存紧张时，主动清理可释放更多可用内存。

**可以设置开机自启吗？**

当前版本未内置。可将 `memory-cleanr.exe` 的快捷方式放入：

```
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup
```

**Release 构建如何查看日志？**

Release 使用 `windows_subsystem = "windows"`，标准错误不可见。诊断信息通过 `OutputDebugString` 输出，可用 [DebugView](https://learn.microsoft.com/en-us/sysinternals/downloads/debugview) 查看。

## 许可

[MIT](LICENSE)
