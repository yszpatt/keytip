# KeyTip

Wayland（niri）下的快捷键提示浮层工具。通过全局快捷键唤起，自动抓取当前活动窗口所属程序，浮层展示该程序的快捷键。

> 仅支持 **niri** 合成器（基于 niri IPC 与 wlr 协议实现，无需 X11）。

---

## 特性

- **全局唤起**：在 niri 配置 binds 里用 `Mod+Slash`（Super+/）唤起，由合成器负责全局捕获。
- **自动识别程序**：通过 `niri msg focused-window` 取当前焦点窗口的 `app-id`，按它查对应的快捷键档案。
- **终端穿透（通用、可递归）**：焦点是终端模拟器（kitty / 终端等）时，通用地递归探测其内部运行的 TUI 程序（如 nvim），并优先展示 `kitty:nvim` 复合档案；若终端内还套着复用器（tmux / screen），会继续穿透到其前台 pane 里的程序（如 `kitty → tmux → yazi` 优先匹配 `kitty:yazi`）。无需为某个应用写特例。
- **浮层展示**：EGui 渲染，按上下文分组 + 搜索过滤，Esc / 失焦自动关闭；窗口外观为半透明深蓝工具窗。
- **收藏页 / 全集页**：可把常用快捷键加星收藏，用标签页在「收藏」与「全集」间切换。
- **单实例 + toggle**：已显示时再按 `Super+/` 会关闭窗口（若被其它窗口盖住则改为提到最前）。
- **中文支持**：内置注入系统中文字体（CJK），中文显示无方块；搜索框支持 fcitx5 中文输入法（见下方「中文输入（IME）」）。
- **无窗口兜底**：空桌面（无焦点窗口）时自动改为展示 niri 合成器自身的快捷键，并实时解析 niri 配置文件（`~/.config/niri/config.kdl` 及其 `include` 引入的分文件）导入按键，无需手动维护。每条绑定都给出**有明确含义**的中文描述：优先采用 niri 自带的 `hotkey-overlay-title`（如 "WeChat"、"Power Menu: Toggle"）；无标题时则智能推导——`spawn` 解析程序名/脚本意图（`kitty`→启动终端、`dms ipc call powermenu toggle`→电源菜单；切换、`.sh` 脚本显示干净命令名），动作名统一汉化（聚焦左列、切换全屏窗口、禁止快捷键穿透等）。
- **手动补充快捷键**：内置默认库 + 用户手动补充通道（`keytip add`）。
- **Rust 实现**：单一二进制，无运行时依赖，常驻低开销。

---

## 架构

```
src/
  main.rs     入口：唤起→取焦点窗口→终端穿透解析→查库→弹浮层；CLI 子命令 add/help
  niri.rs     niri IPC 封装（focused-window / focus-window / active_monitor_logical_size ...）
  store.rs    数据模型 + 默认库/用户配置合并 + 手动补充 + 收藏读写
  term.rs     终端穿透：识别终端内运行的 TUI / 复用器，递归生成候选 app-id
  overlay.rs  EGui 浮层 UI（分组、搜索、收藏标签页、自动换行、占满窗口）
  ipc.rs      单实例 socket（toggle：close / focus）
  fonts.rs    注入系统中文字体（CJK）
data/defaults/  内置默认库（<app-id>.json）
examples/        验证示例（verify_font / verify_egui / verify_toplevel / verify_layershell / verify_portal_shortcut）
```

数据流：`niri Mod+Slash spawn` → `niri msg -j focused-window` 取 app-id → （若为终端则 `term::detect_chain` 递归穿透内部程序）→ 按候选 app-id 查 `ShortcutStore`（内置库 ∪ 用户配置）→ EGui 浮层分组展示。

---

## 安装

### 方式一：从源码（cargo）

```bash
git clone <repo> keytip && cd keytip
cargo build --release

# 1) 二进制
mkdir -p ~/.local/bin
cp target/release/keytip ~/.local/bin/keytip
chmod +x ~/.local/bin/keytip

# 2) 内置默认库
mkdir -p ~/.local/share/keytip/defaults
cp data/defaults/*.json ~/.local/share/keytip/defaults/

# 3) 用户配置目录（收藏 + 手动补充写这里）
mkdir -p ~/.config/keytip
```

### 方式二：安装脚本

```bash
./install.sh
```

脚本会自动 `cargo build --release`、安装二进制与默认库，并在结尾提示需要补的 niri 配置片段。

### 方式三：PKGBUILD（Arch / 衍生发行版）

```bash
makepkg -si        # 或 makepkg 后用 pacman -U 安装生成的 .pkg.tar.zst
```

打包进 `/usr/bin/keytip`、默认库到 `/usr/share/keytip/defaults/`、文档到 `/usr/share/doc/keytip/`。

---

### niri 配置

**1) 唤起键** —— 在 niri 配置里（如 `~/.config/niri/dms/binds.kdl`，或你实际放 binds 的位置）加入：

```kdl
Mod+Slash { spawn "/home/yszpat/.local/bin/keytip"; }
```

> 建议把上面路径换成你机器上 `keytip` 的实际绝对路径（`which keytip` 查看）。

⚠️ **两个必踩的坑**：

1. **键名必须用 keysym 名**：niri 的 KDL 解析器不接受字面符号 `/`，要写 `Slash`。写成 `Mod+/` 会导致整个 binds.kdl 解析失败（`niri validate` 报 `unexpected token`），**该文件里所有绑定一起失效**。其他符号键同理：`Comma` `Period` `Minus` `Equal` `BracketLeft`。
2. **必须用绝对路径**：niri 的 PATH 通常**不含 `~/.local/bin`**（实测只有 `/usr/local/sbin:/usr/local/bin:/usr/bin:...`），写 `spawn "keytip"` 会静默失败（127 未找到命令）。
   注意 `niri msg action spawn` 返回 0 只是 IPC 确认，不代表程序真的跑起来了；排查时要把被 spawn 进程的输出重定向到文件才看得到真相。

**2) 窗口规则** —— 在 `~/.config/niri/config.kdl` 加（让 keytip 以无边框浮动窗出现，并固定在屏幕左侧）：

```kdl
window-rule {
    match app-id="keytip"
    open-floating true
    open-focused true          // 必需：否则窗口拿不到焦点会被失焦逻辑一闪即关；同时让 open-floating 生效
    default-floating-position x=50 y=0 relative-to="left"  // 初始位置：屏幕左侧，左缘距屏左 50 逻辑像素
    focus-ring { off; }
    border { off; }
    shadow { off; }
}
```

改完用 `niri validate` 校验（应显示 `config is valid`），niri 保存后自动重载。按 **Super+/**（= `Mod+Slash`）即可唤起。

> 注：`Super+K`（Mod+K）已被 niri 占用为 `focus-window-up`，请勿复用。

---

## 使用

- **唤起**：`Super+/`，显示当前窗口程序的快捷键。
- **搜索**：按 `f` 进入搜索框，过滤匹配按键 / 动作 / 描述；再按 `f` 切换查找态。
- **滚动**：`j` / `k` 上下滚动列表（搜索框聚焦时让位给输入）。
- **切换标签**：`t` 在「★ 收藏 / 全集」两页间切换。
- **收藏**：点条目左侧的 ☆ / ★ 切换收藏（写入 `~/.config/keytip/favorites.json`，跨重启保留）。
- **关闭**：`Esc`（搜索框聚焦时先退出查找，再按一次关闭）或点击其它窗口（失焦自动关闭）。
- **toggle**：已显示时再按 `Super+/` 会关闭；若被其它窗口盖住则把它提到最前。

### 窗口尺寸行为

浮层窗口大小按**当前焦点窗口所在的显示器**自动计算（多屏各自合适）：

- 宽 ≈ 该显示器**物理宽 / 8**（约等于逻辑宽 1/4）；
- 高 ≈ 该显示器**物理高 × 0.4**（约等于逻辑高 80%），并有 niri 浮动窗口的物理高度上限钳制。

> ⚠️ **niri ×2 渲染怪癖**：实测 niri 对 keytip 浮动窗口会把请求的 logical 尺寸统一 **×2** 渲染成物理像素（与显示器 scale 无关），因此代码中请求值是按「物理尺寸」反推的。这是为适配该怪癖而做的处理，若 niri 行为变化需相应调整 `main.rs` 中的 `win_w / win_h` 公式。

### 手动补充快捷键

```bash
keytip add <app_id> <keys> <action> [description] [context]
# 例：为 firefox 补充
keytip add org.mozilla.firefox "Ctrl+T" "new_tab" "新建标签页" "标签"
# 终端内程序用复合 app-id，如给 kitty 里的 nvim 补充：
keytip add kitty:nvim "Space+w" "save" "保存" "文件"
```

写入 `~/.config/keytip/shortcuts.json`，优先级高于内置默认库。

### 终端穿透（通用、可递归）

焦点为终端（kitty 等）时，自动识别其中运行的 TUI 程序（如 nvim），并优先展示 `kitty:nvim` 复合档案；没配则退回终端自身快捷键。终端里若还套着复用器（tmux / screen），会继续穿透到其前台 pane 里的程序，例如 `tmux` 里跑 `yazi` 会优先匹配 `kitty:yazi`，而非 `kitty:tmux`。

> 调试：对某个终端 pid 跑 `keytip --detect-chain <pid>` 可打印穿透链与候选（不弹 GUI），用于排查"某终端内程序没识别到"的问题。

---

## 中文输入（IME）

搜索框默认用 egui 的 `TextEdit`。在 Wayland + niri 下，**egui-winit 0.29.1 在 Linux 平台会直接丢弃所有 IME 事件**（[emilk/egui#5008](https://github.com/emilk/egui/issues/5008)），导致 fcitx5 提交的中文 `Event::Ime(Commit)` 永远到不了 `TextEdit`，表现为「英文能打、中文打不进」。

### 修复方式（已 vendored）

本仓库在 `vendor/egui-winit-0.29.1-patched/` 中保留了一份打了补丁的 egui-winit：去掉了 `WindowEvent::Ime` 处理器里 `if cfg!(target_os = "linux") { ignore }` 的守卫，让 Linux 与非 Linux 平台一样正常处理 Preedit / Commit。`Cargo.toml` 通过 `[patch.crates-io]` 指向它：

```toml
[patch.crates-io]
egui-winit = { path = "vendor/egui-winit-0.29.1-patched" }
```

现代 winit 0.30 + Wayland text-input-v3 已可正确处理 CJK 输入，这样 `cargo build` 无需额外步骤即可获得中文输入能力。

> ⚠️ **维护提示**：该补丁是对上游 egui-winit 的本地 fork。**若将来升级 `egui-winit` 版本，必须**对对应新版本重新 vendored 并重新应用同样的 Linux IME 守卫移除（否则中文输入会再次失效）。升级后可 `cargo build --release` 实测搜索框中文输入来验证。

### 焦点初始化循环

光打补丁还不够：winit 仅在收到 text-input 的 `enter` 事件且当时 `ime_allowed()==true` 时调用 `text_input.enable()`（把输入路由到本窗口）。keytip 被 niri `spawn` 时立即聚焦，首帧 `enter` 早于 egui 把 `ime_allowed` 置真，导致 `enable()` 被永久跳过。

`overlay.rs` 每帧常驻 `IMEAllowed(true)`，并在启动首帧主动制造一次「焦点离开→回到」循环（先聚焦背后的其它窗口触发 `leave`，再聚焦回 keytip 触发新 `enter`，此时 `ime_allowed` 已为真，`enable()` 执行，fcitx5 开始路由中文）。相关辅助函数在 `niri.rs`：`focus_window_by_id`、`first_other_window_id`（实时查询非自身窗口，避免复用启动时自身 id 导致循环卡死）。

---

## 无窗口时展示 niri 快捷键

在空桌面（没有焦点窗口）按 `Super+/` 唤起时，由于"当前活动窗口"不存在，keytip 会自动改为展示 **niri 合成器自身的快捷键**：

- **自动读取配置**：定位 niri 配置文件（默认 `~/.config/niri/config.kdl`，也兼容常见的 `dms/binds.kdl`、`binds.kdl` 拆分位置；可用环境变量 `KEYTIP_NIRI_CONFIG` 显式指定路径）。
- **解析 `binds` 块**（零依赖，针对 niri 的 KDL 格式定制）：
  - 正确跳过 `//` 行注释与 `/* */` 块注释（尊重字符串字面量内的 `//`）；
  - 跟随 `include "...";`（niri 的引入指令，等价于 KDL 的 `import`）递归合并分文件；
  - 处理绑定上的属性（`hotkey-overlay-title="..."`、`allow-when-locked=true`、`repeat=false` 等）与多参数动作（`spawn "a" "b" "c"`）；
  - 支持嵌套和弦（`Mod+H { Mod+L { close; } }`）。
- **分类与汉化**：按键按首个动作归类（启动程序 / 窗口 / 工作区 / 列与布局 / 截图 …），动作描述翻译成中文友好文本（如 `spawn "kitty"` → "启动 kitty"）。
- **收藏可用**：niri 快捷键同样支持加星收藏（app-id 为 `niri`，写入 `~/.config/keytip/favorites.json`），跨重启保留。

> 因为是"实时解析"，改完 niri 配置后无需任何额外步骤，下次在空桌面唤起 keytip 看到的就是最新按键。

---

## 已知限制 / 后续

- **浮层置顶**：✅ 已解决 —— 在 window-rule 里补上 `open-focused true` 后，`open-floating` 随之生效，实测窗口 `floating=True`。原先设想的 layer-shell 重构不再必要（`examples/verify_layershell.rs` 保留作为备选方案验证）。
- **半透明背景**：✅ 已实现（`with_transparent(true)` + App 全透明 + 半透明深蓝底板）。最终观感依赖合成器透明支持，niri 下正常。
- **CJK 字体**：✅ 已实现（`fonts.rs` 注入系统中文字体，否则中文显示为方块）。
- **中文输入（IME）**：✅ 已解决 —— 见上方「中文输入（IME）」：vendored egui-winit 补丁（去掉 Linux IME 丢弃守卫）+ 焦点初始化循环，搜索框 fcitx5 中文输入正常上屏。
- **niri ×2 渲染怪癖**：见上方「窗口尺寸行为」。HDMI 等次屏若高度被 niri 物理上限钳到 65% 左右，属于该上限所致，非代码 bug。
- **导入器**：首版预留 `Importer` 接口，仅框架 + 默认库；具体程序解析器（VS Code / 终端 / Firefox 配置）待后续增量。当前默认库仅含少量示例（`data/defaults/`）。

---

## 开发

```bash
cargo build
cargo run --example verify_font           # 验证中文字体注入
cargo run --example verify_egui           # 验证 EGui 窗口
cargo run --example verify_toplevel       # 验证活动窗口枚举（wlr-foreign-toplevel）
cargo run --example verify_layershell     # 验证 layer-shell 置顶（备选方案）
cargo run --example verify_portal_shortcut # 验证 portal 全局快捷键
```

### 调试日志

keytip 通常由 niri 后台 spawn，`stderr` 用户看不到。每次唤起会把诊断信息（解析到的 app-id、候选链、命中条目数）追加写到 `~/.cache/keytip/last.log`，便于定位"某程序不显示快捷键"类问题。

编译 release 启用 LTO，内存占用较高，若遇到 OOM 可重试或清理 swap。
