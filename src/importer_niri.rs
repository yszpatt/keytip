//! 导入 niri 自身的快捷键配置（无窗口时的兜底展示源）。
//!
//! 不引入 KDL 解析库（保持精简）：niri 的 `binds` 格式高度规整——始终是
//! `COMBO { 动作... }` 的块形式，且节点名即按键组合。这里写一个针对性的轻量解析器：
//! - 正确处理 `//` 行注释与 `/* */` 块注释（尊重字符串字面量，避免误删路径中的 `//`）；
//! - 处理 `"..."` 字符串（含转义），避免把字符串里的 `{` `}` `;` 当成结构符；
//! - 递归跟随 `import "...";` 引入的其它文件（相对当前文件目录）；
//! - 支持嵌套和弦（`Mod+H { Mod+L { close; } }` 之类）。
//!
//! 生成的 `ShortcutEntry` 按"首个动作名"分类（窗口/工作区/列与布局/截图…），
//! 动作描述翻译成中文友好文本（如 `spawn "keytip"` → "启动 keytip"）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::store::ShortcutEntry;

/// 无窗口兜底时使用的 app-id（与收藏分组、窗口标题共用）。
pub const NIRI_APP_ID: &str = "niri";

/// 定位 niri 配置文件。
///
/// 优先级：
///   1. 环境变量 `KEYTIP_NIRI_CONFIG`（显式指定，便于测试/自定义）。
///   2. `$XDG_CONFIG_HOME/niri/config.kdl`（niri 标准位置）。
///   3. `~/.config/niri/config.kdl`。
///   4. 常见的 binds 拆分位置：`dms/binds.kdl`、`binds.kdl`（部分用户把按键单独拆文件再 `import`）。
fn niri_config_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("KEYTIP_NIRI_CONFIG") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    let home = std::env::var("HOME").ok()?;
    let xdg = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{home}/.config"));
    let candidates = [
        format!("{xdg}/niri/config.kdl"),
        format!("{home}/.config/niri/config.kdl"),
        format!("{xdg}/niri/dms/binds.kdl"),
        format!("{home}/.config/niri/dms/binds.kdl"),
        format!("{xdg}/niri/binds.kdl"),
        format!("{home}/.config/niri/binds.kdl"),
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// 加载 niri 快捷键；无窗口兜底时调用。
///
/// 找不到配置或其中没有任何 `binds` 时，返回一个"提示"占位条目，
/// 让浮层不至于空白，便于用户排查配置文件位置。
pub fn load_niri_shortcuts() -> Vec<ShortcutEntry> {
    let mut entries: Vec<ShortcutEntry> = Vec::new();
    if let Some(path) = niri_config_path() {
        let base = path.parent().map(|x| x.to_path_buf()).unwrap_or_default();
        let mut seen = HashSet::new();
        collect_from_file(&path, &base, &mut seen, &mut entries);
    }
    if entries.is_empty() {
        entries.push(ShortcutEntry {
            context: "提示".to_string(),
            keys: String::new(),
            action: "notice".to_string(),
            description: "未找到 niri 配置文件或其中没有 binds（检查 ~/.config/niri/config.kdl）".to_string(),
        });
    } else {
        // 顶部插一条说明，告知这是实时从 niri 配置解析而来。
        entries.insert(
            0,
            ShortcutEntry {
                context: "提示".to_string(),
                keys: String::new(),
                action: "notice".to_string(),
                description: "以下为实时解析 niri 配置（binds）得到的快捷键".to_string(),
            },
        );
    }
    entries
}

/// 递归解析一个 niri 配置文件，把其中的 binds 提取为 ShortcutEntry。
fn collect_from_file(
    path: &Path,
    base: &Path,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<ShortcutEntry>,
) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    // 防止 import 环导致无限递归。
    if !seen.insert(canonical) {
        return;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[importer_niri] 读取 {} 失败：{e}", path.display());
            return;
        }
    };
    let stripped = strip_comments(&text);

    // 1) 先处理 import 引入的其它文件（相对当前文件目录）。
    for imp in find_imports(&stripped) {
        let p = base.join(&imp);
        if p.exists() {
            let ibase = p.parent().map(|x| x.to_path_buf()).unwrap_or_else(|| base.to_path_buf());
            collect_from_file(&p, &ibase, seen, out);
        }
    }

    // 2) 提取所有 `binds { ... }` 块，逐条解析其中的按键。
    let mut blocks = Vec::new();
    extract_binds(&mut blocks, &stripped);
    for block in blocks {
        for node in split_child_nodes(&block) {
            let node = node.trim();
            if node.is_empty() {
                continue;
            }
            let combo = combo_of(node);
            if combo.is_empty() {
                continue;
            }
            // niri 用 `"/-"` 注释掉某条绑定（KDL 的"注释节点"语法），跳过它。
            if combo == "/-" {
                continue;
            }
            let desc = describe_bind(node);
            if desc.is_empty() {
                continue;
            }
            let first = first_action_name(node);
            out.push(ShortcutEntry {
                context: categorize(&first).to_string(),
                keys: combo,
                action: if first.is_empty() { "action".to_string() } else { first },
                description: desc,
            });
        }
    }
}

/// 去掉 KDL 注释（`//` 行注释与 `/* */` 块注释），字符串内的 `//` `/* */` 不视为注释。
fn strip_comments(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    let mut in_str = false;
    while i < n {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < n {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 找出所有 `import "路径";` / `include "路径";` 引入的文件路径。
///
/// 注意：niri 用的是 `include`（不是 KDL 的 `import`），两种都要支持。
fn find_imports(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let is_import = matches_ident(&chars, i, "import");
        let is_include = matches_ident(&chars, i, "include");
        if is_import || is_include {
            let kw_len = if is_import { 6 } else { 7 };
            let mut j = i + kw_len;
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            if j < n && chars[j] == '"' {
                let mut p = String::new();
                j += 1;
                while j < n && chars[j] != '"' {
                    if chars[j] == '\\' && j + 1 < n {
                        p.push(chars[j + 1]);
                        j += 2;
                    } else {
                        p.push(chars[j]);
                        j += 1;
                    }
                }
                out.push(p);
            }
        }
        i += 1;
    }
    out
}

/// 提取所有顶层 `binds { ... }` 块的块内容（含花括号）。
fn extract_binds(out: &mut Vec<String>, text: &str) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if matches_ident(&chars, i, "binds") {
            let mut j = i + 5;
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            if j < n && chars[j] == '{' {
                let (block, end) = match_brace(&chars, j);
                // match_brace 返回的是含外层花括号的整段，这里剥掉外层 `{ }`，
                // 只保留 binds 块"内部"内容，便于后续 split_child_nodes 直接切出各绑定节点。
                let inner = if block.starts_with('{') && block.ends_with('}') {
                    block[1..block.len() - 1].to_string()
                } else {
                    block
                };
                out.push(inner.trim().to_string());
                i = end;
                continue;
            }
        }
        i += 1;
    }
}

/// 从 `{` 处匹配到配对的 `}`，返回整段（含花括号）与结束位置之后。
fn match_brace(chars: &[char], open: usize) -> (String, usize) {
    let n = chars.len();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut i = open;
    let mut s = String::new();
    while i < n {
        let c = chars[i];
        if in_str {
            s.push(c);
            if c == '\\' && i + 1 < n {
                s.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            s.push(c);
            i += 1;
            continue;
        }
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                s.push(c);
                return (s, i + 1);
            }
        }
        s.push(c);
        i += 1;
    }
    (s, i)
}

/// 把一个块的"内部内容"按顶层子节点切分（尊重字符串与嵌套 `{}`）。
/// 子节点以 `;` 或配对的 `}` 作为结束标志。
fn split_child_nodes(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut in_str = false;
    let mut depth = 0i32;
    let mut buf = String::new();
    let mut started = false;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if in_str {
            buf.push(c);
            if c == '\\' && i + 1 < n {
                buf.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                started = true;
                buf.push(c);
                i += 1;
            }
            '{' => {
                started = true;
                depth += 1;
                buf.push(c);
                i += 1;
            }
            '}' => {
                depth -= 1;
                buf.push(c);
                if depth == 0 {
                    out.push(buf.trim().to_string());
                    buf.clear();
                    started = false;
                }
                i += 1;
            }
            ';' => {
                if depth == 0 {
                    if started {
                        buf.push(c);
                        out.push(buf.trim().to_string());
                        buf.clear();
                        started = false;
                    }
                    i += 1;
                } else {
                    buf.push(c);
                    i += 1;
                }
            }
            _ => {
                if !started && !c.is_whitespace() {
                    started = true;
                }
                buf.push(c);
                i += 1;
            }
        }
    }
    if started && !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

/// 取一个绑定节点的按键组合（块前的第一个 token）。
fn combo_of(node: &str) -> String {
    let t = node.trim();
    let idx = t.find('{').unwrap_or(t.len());
    t[..idx].trim().split_whitespace().next().unwrap_or("").to_string()
}

/// 取一个绑定节点的"首个动作名"（用于分类与 action 字段）。
fn first_action_name(node: &str) -> String {
    let t = node.trim();
    if let Some(idx) = t.find('{') {
        let inner = t[idx + 1..].trim_end_matches('}').trim();
        for child in split_child_nodes(inner) {
            let c = child.trim();
            if c.is_empty() {
                continue;
            }
            let (name, _) = parse_action_node(c);
            if !name.is_empty() {
                return name;
            }
        }
        return String::new();
    }
    let (name, _) = parse_action_node(t);
    name
}

/// 生成"顶层绑定节点"的友好描述：只描述块内的动作（按键组合本身已作为 `keys` 单独展示）。
///
/// 优先级：
/// 1. niri 自带的 `hotkey-overlay-title="..."`（人类可读标题，最贴近用户意图）；
/// 2. 否则由动作推导（spawn 解析程序名/脚本意图、dms ipc 翻译成中文等）。
fn describe_bind(node: &str) -> String {
    if let Some(title) = overlay_title_of(node) {
        return title;
    }
    let t = node.trim();
    if let Some(idx) = t.find('{') {
        let inner = t[idx + 1..].trim_end_matches('}').trim();
        let mut parts = Vec::new();
        for child in split_child_nodes(inner) {
            let c = child.trim();
            if c.is_empty() {
                continue;
            }
            parts.push(describe_node(c));
        }
        parts.join("；")
    } else {
        describe_node(t)
    }
}

/// 生成绑定节点的友好描述（递归处理嵌套和弦 `A { B { ... } }`）。
fn describe_node(node: &str) -> String {
    let t = node.trim();
    let has_block = t.contains('{');
    if !has_block {
        let (name, args) = parse_action_node(t);
        return friendly_action(&name, &args);
    }
    let idx = t.find('{').unwrap();
    let name_region = t[..idx].trim();
    let combo = name_region.split_whitespace().next().unwrap_or("").to_string();
    let inner = t[idx + 1..].trim_end_matches('}').trim();
    let mut parts = Vec::new();
    for child in split_child_nodes(inner) {
        let c = child.trim();
        if c.is_empty() {
            continue;
        }
        parts.push(describe_node(c));
    }
    let inner_desc = if parts.is_empty() {
        String::new()
    } else {
        parts.join("；")
    };
    if combo.is_empty() {
        inner_desc
    } else if inner_desc.is_empty() {
        combo
    } else {
        format!("{combo} → {inner_desc}")
    }
}

/// 取一个绑定节点上的 `hotkey-overlay-title="..."` 属性值（niri 自带的"人类可读标题"）。
///
/// niri 几乎每条 bind 都带这个属性（如 `hotkey-overlay-title="Open Terminal"`、
/// `"WeChat"`、`"Power Menu: Toggle"`），它正是用户想要的"明确含义"，优先作为描述。
fn overlay_title_of(node: &str) -> Option<String> {
    let chars: Vec<char> = node.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if matches_ident(&chars, i, "hotkey-overlay-title") {
            let mut j = i + "hotkey-overlay-title".len();
            while j < n && (chars[j].is_whitespace() || chars[j] == '=') {
                j += 1;
            }
            if j < n && chars[j] == '"' {
                let mut v = String::new();
                j += 1;
                while j < n && chars[j] != '"' {
                    if chars[j] == '\\' && j + 1 < n {
                        v.push(chars[j + 1]);
                        j += 2;
                    } else {
                        v.push(chars[j]);
                        j += 1;
                    }
                }
                let t = v.trim().to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
        i += 1;
    }
    None
}

/// 把单个动作节点文本拆成 (动作名, 参数列表)。`spawn "keytip";` → ("spawn", ["keytip"])。
///
/// 动作节点上的 KDL 属性（`scope="output"`、`allow-when-locked=true` 等）形如
/// `key=value`，不属于位置参数，过滤掉以免污染描述。
fn parse_action_node(text: &str) -> (String, Vec<String>) {
    let toks = tokenize(text);
    let name = toks
        .first()
        .map(|s| s.trim_end_matches(';').to_string())
        .unwrap_or_default();
    let args: Vec<String> = toks
        .iter()
        .skip(1)
        .map(|s| s.trim_end_matches(';').trim_matches('"').to_string())
        .filter(|s| !s.contains('='))
        .collect();
    (name, args)
}

/// 把一行文本按空白切成 token，保留字符串整体（含引号）。
fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    for c in text.chars() {
        if in_str {
            cur.push(c);
            if c == '\\' {
                // 转义：保留反斜杠与下一字符原样（如 \" 或 \\）。
                // tokenize 不负责展开转义，由调用方按需 trim_matches。
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            cur.push(c);
            continue;
        }
        if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 已知 app-id / 命令名 → 中文显示名（用于 spawn 描述，避免露出裸命令或脚本路径）。
fn app_alias(cmd: &str) -> Option<&'static str> {
    let c = cmd.trim_matches('"');
    let stem = std::path::Path::new(c)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(c);
    let bare = c.trim_start_matches('~').trim_start_matches('/');
    let _ = bare;
    match stem {
        "kitty" | "alacritty" | "wezterm" | "foot" => Some("终端"),
        "zen-browser" | "zen" | "firefox" | "chrome" | "chromium" | "brave" => Some("浏览器"),
        "feishu" => Some("飞书"),
        "telegram" | "telegram-desktop" => Some("Telegram"),
        "wechat" | "wechat-uos" => Some("微信"),
        "code" | "code-oss" | "codium" => Some("VS Code"),
        "nvim" | "vim" | "neovide" => Some("Neovim"),
        "nautilus" | "thunar" | "dolphin" | "pcmanfm" => Some("文件管理器"),
        "kando" => Some("Kando 径向菜单"),
        "mark-shot" | "grim" | "satty" | "slurp" => Some("截图工具"),
        "keytip" => Some("KeyTip 快捷键提示"),
        _ => None,
    }
}

/// 把 `spawn` 的参数翻译成有明确含义的中文描述。
///
/// - 形如 `dms ipc call <module> <action>` → 翻译模块/动作（如 powermenu toggle → 电源菜单；切换）；
/// - 直接命令 → 用 app_alias 显示名（"启动 终端"），无别名时显示干净的程序名（去路径/.sh/引号）；
/// - `sh -c "..."` → 尽量从命令里猜意图，猜不到则显示"运行脚本"。
fn spawn_description(args: &[String]) -> String {
    if let Some(a) = args.first() {
        // dms ipc call <module> <action...>
        if a == "dms" && args.len() >= 4 && args[1] == "ipc" && args[2] == "call" {
            let module = &args[3];
            let act = args.get(4).map(|s| s.as_str()).unwrap_or("");
            let m = dms_module(module);
            let verb = dms_action(act);
            let mv = if verb.is_empty() {
                m.to_string()
            } else {
                format!("{m}；{verb}")
            };
            return if mv.is_empty() { "DMS 控制".to_string() } else { mv };
        }
        if a == "sh" && args.get(1).map(|s| s.as_str()) == Some("-c") {
            // 尽量从 shell 命令里提取可读意图（取第一个像样的令牌）。
            if let Some(cmd) = args.get(2) {
                let head = cmd.split_whitespace().next().unwrap_or("");
                if let Some(alias) = app_alias(head) {
                    return format!("运行：{alias}");
                }
                if !head.is_empty() {
                    return format!("运行：{head}");
                }
            }
            return "运行脚本".to_string();
        }
        if let Some(alias) = app_alias(a) {
            return format!("启动 {alias}");
        }
        // 裸程序：去掉路径、~、.sh 后缀、引号，呈现干净的命令名。
        let bare = a
            .trim_matches('"')
            .rsplit('/')
            .next()
            .unwrap_or(a)
            .trim_start_matches('~')
            .trim_start_matches('/');
        let stem = bare.strip_suffix(".sh").unwrap_or(bare);
        if stem.is_empty() {
            "启动程序".to_string()
        } else {
            format!("启动 {stem}")
        }
    } else {
        "启动程序".to_string()
    }
}

/// dms IPC 模块名 → 中文。
fn dms_module(m: &str) -> &'static str {
    match m {
        "spotlight" => "启动器",
        "clipboard" => "剪贴板",
        "processlist" => "进程列表",
        "powermenu" => "电源菜单",
        "settings" => "设置",
        "dankdash" => "壁纸",
        "notifications" => "通知中心",
        "notepad" => "便签",
        "lock" => "锁屏",
        "audio" => "音量",
        "mic" | "microphone" => "麦克风",
        "mpris" => "媒体播放",
        "brightness" => "亮度",
        _ => "",
    }
}

/// dms IPC 动作名 → 中文（toggle / focusOrToggle / increment / lock 等）。
fn dms_action(a: &str) -> &'static str {
    match a {
        "toggle" => "切换",
        "focusOrToggle" => "聚焦/切换",
        "increment" => "增加",
        "decrement" => "减少",
        "mute" => "静音",
        "micmute" => "麦克风静音",
        "playPause" => "播放/暂停",
        "previous" => "上一个",
        "next" => "下一个",
        "lock" => "锁定",
        "wallpaper" => "切换壁纸",
        _ => "",
    }
}

/// 把 niri 动作翻译成中文友好描述。
fn friendly_action(name: &str, args: &[String]) -> String {
    let arg = args.join(" ");
    match name {
        "spawn" => spawn_description(args),
        "close" | "close-window" => "关闭窗口".to_string(),
        "quit" => "退出 niri".to_string(),
        "toggle-overview" => "切换总览".to_string(),
        "show-hotkey-overlay" => "显示快捷键浮层".to_string(),
        "set-column-width" => "设置列宽".to_string(),
        "toggle-column-tabbed-display" => "切换列标签显示".to_string(),
        "switch-focus-between-floating-and-tiling" => "浮动/平铺间切换焦点".to_string(),
        "focus-window-left" => "聚焦左侧窗口".to_string(),
        "focus-window-right" => "聚焦右侧窗口".to_string(),
        "focus-window-up" => "聚焦上方窗口".to_string(),
        "focus-window-down" => "聚焦下方窗口".to_string(),
        "move-window-left" => "窗口移向左侧".to_string(),
        "move-window-right" => "窗口移向右侧".to_string(),
        "move-window-up" => "窗口移向上方".to_string(),
        "move-window-down" => "窗口移向下方".to_string(),
        "swap-windows" => "交换窗口".to_string(),
        "column-left" => "列左移".to_string(),
        "column-right" => "列右移".to_string(),
        "focus-column-left" => "聚焦左列".to_string(),
        "focus-column-right" => "聚焦右列".to_string(),
        "move-column-left" => "列移向左".to_string(),
        "move-column-right" => "列移向右".to_string(),
        "move-column-to-first" => "列移到最左".to_string(),
        "move-column-to-last" => "列移到最右".to_string(),
        "center-column" => "列居中".to_string(),
        "consume-or-expel-window-left" => "吞入/吐出窗口（左）".to_string(),
        "consume-or-expel-window-right" => "吞入/吐出窗口（右）".to_string(),
        "toggle-column-tabbed" => "切换列标签排布".to_string(),
        "toggle-column-tiled" => "切换列平铺排布".to_string(),
        "focus-monitor-left" => "聚焦左显示器".to_string(),
        "focus-monitor-right" => "聚焦右显示器".to_string(),
        "focus-monitor-up" => "聚焦上显示器".to_string(),
        "focus-monitor-down" => "聚焦下显示器".to_string(),
        "move-window-to-monitor-left" => "窗口移到左显示器".to_string(),
        "move-window-to-monitor-right" => "窗口移到右显示器".to_string(),
        "move-window-to-monitor-up" => "窗口移到上显示器".to_string(),
        "move-window-to-monitor-down" => "窗口移到下显示器".to_string(),
        "switch-workspace" | "focus-workspace" => {
            if let Some(a) = args.first() {
                format!("切换工作区 {a}")
            } else {
                "切换工作区".to_string()
            }
        }
        "move-to-workspace" => {
            if let Some(a) = args.first() {
                format!("移动至工作区 {a}")
            } else {
                "移动至工作区".to_string()
            }
        }
        "move-to-workspace-down" | "move-to-workspace-up" => "移动至相邻工作区".to_string(),
        "workspace" => "新建工作区".to_string(),
        "workspace-up" => "上一工作区".to_string(),
        "workspace-down" => "下一工作区".to_string(),
        "screenshot" => "截图".to_string(),
        "screenshot-screen" => "截取屏幕".to_string(),
        "screenshot-window" => "截取窗口".to_string(),
        "toggle-fullscreen" => "切换全屏".to_string(),
        "fullscreen" | "fullscreen-window" => "全屏窗口".to_string(),
        "reset-window-height" => "重置窗口高度".to_string(),
        "set-window-height" => {
            if let Some(a) = args.first() {
                format!("设置窗口高度 {a}")
            } else {
                "设置窗口高度".to_string()
            }
        }
        "toggle-keyboard-shortcuts-inhibit" => "禁止/允许键盘快捷键穿透".to_string(),
        "next-window" => "切换到下一个窗口".to_string(),
        "previous-window" => "切换到上一个窗口".to_string(),
        "toggle-window-floating" => "切换窗口浮动".to_string(),
        "set-window-floating" => "设为浮动窗口".to_string(),
        "maximize-column" => "最大化列".to_string(),
        "switch-layout" | "set-layout" => {
            if let Some(a) = args.first() {
                format!("切换布局 {a}")
            } else {
                "切换布局".to_string()
            }
        }
        "toggle-window-always-on-top" => "切换窗口置顶".to_string(),
        "set-window-always-on-top" => "窗口置顶".to_string(),
        _ => {
            if arg.is_empty() {
                name.to_string()
            } else {
                format!("{name} {arg}")
            }
        }
    }
}

/// 按首个动作名归类，决定浮层里的分组标题。
fn categorize(first_action: &str) -> &'static str {
    if first_action == "spawn" {
        "启动程序"
    } else if first_action == "close" || first_action == "quit" || first_action == "close-window" {
        "窗口"
    } else if first_action.starts_with("focus-window")
        || first_action.starts_with("move-window")
        || first_action == "swap-windows"
        || first_action.starts_with("move-window-to-monitor")
        || first_action.starts_with("focus-monitor")
        || first_action.starts_with("consume-or-expel-window")
        || first_action == "next-window"
        || first_action == "previous-window"
    {
        "窗口与显示器"
    } else if first_action.contains("column")
        || first_action.contains("layout")
        || first_action.starts_with("switch-layout")
        || first_action.starts_with("set-layout")
        || first_action.contains("window-height")
        || first_action == "reset-window-height"
        || first_action == "maximize-column"
        || first_action == "set-column-width"
        || first_action == "toggle-column-tabbed-display"
    {
        "列与布局"
    } else if first_action == "toggle-overview" || first_action == "show-hotkey-overlay" {
        "总览"
    } else if first_action.contains("workspace") {
        "工作区"
    } else if first_action.starts_with("screenshot") {
        "截图"
    } else if first_action.starts_with("switch") {
        "切换"
    } else if first_action.contains("fullscreen")
        || first_action.contains("floating")
        || first_action.contains("always-on-top")
        || first_action == "toggle-keyboard-shortcuts-inhibit"
    {
        "窗口状态"
    } else {
        "其它"
    }
}

/// 判断 `chars[i..]` 是否以完整单词 `word` 开头，且前后都不是标识符字符。
fn matches_ident(chars: &[char], i: usize, word: &str) -> bool {
    let b: Vec<char> = word.chars().collect();
    if i + b.len() > chars.len() {
        return false;
    }
    for (k, ch) in b.iter().enumerate() {
        if chars[i + k] != *ch {
            return false;
        }
    }
    if i > 0 && is_ident_char(chars[i - 1]) {
        return false;
    }
    let after_ok = if i + b.len() < chars.len() {
        !is_ident_char(chars[i + b.len()])
    } else {
        true
    };
    after_ok
}

/// niri 标识符允许出现的字符（用于边界判断，避免把 `mybinds` 误判为 `binds`）。
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '+' | '@' | '$' | ':' | '.' | '/')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 复刻 collect_from_file 的核心解析链路（去文件/import），便于纯内存测试。
    fn parse_sample(cfg: &str) -> Vec<ShortcutEntry> {
        let cfg = strip_comments(cfg);
        let mut entries = Vec::new();
        let mut blocks = Vec::new();
        extract_binds(&mut blocks, &cfg);
        for block in &blocks {
            for node in split_child_nodes(block) {
                let node = node.trim();
                if node.is_empty() {
                    continue;
                }
                let combo = combo_of(node);
                if combo.is_empty() {
                    continue;
                }
                let desc = describe_bind(node);
                let first = first_action_name(node);
                entries.push(ShortcutEntry {
                    context: categorize(&first).to_string(),
                    keys: combo,
                    action: if first.is_empty() {
                        "action".to_string()
                    } else {
                        first
                    },
                    description: desc,
                });
            }
        }
        entries
    }

    #[test]
    fn parses_basic_binds() {
        let cfg = r#"
        binds {
            Mod+Slash { spawn "keytip"; }
            Mod+Q { close; }
            Mod+Enter { spawn "kitty"; }
            Mod+Left { focus-window-left; }
            Mod+Shift+1 { move-to-workspace 1; }
            Mod+H { Mod+L { close; } }  // 和弦
        }
        "#;
        let entries = parse_sample(cfg);
        assert_eq!(entries.len(), 6, "entries={:?}", entries);

        let kt = entries.iter().find(|e| e.keys == "Mod+Slash").unwrap();
        assert_eq!(kt.description, "启动 KeyTip 快捷键提示");
        assert_eq!(kt.context, "启动程序");

        let q = entries.iter().find(|e| e.keys == "Mod+Q").unwrap();
        assert_eq!(q.description, "关闭窗口");

        let left = entries.iter().find(|e| e.keys == "Mod+Left").unwrap();
        assert_eq!(left.description, "聚焦左侧窗口");

        let mw = entries.iter().find(|e| e.keys == "Mod+Shift+1").unwrap();
        assert_eq!(mw.description, "移动至工作区 1");

        let chord = entries.iter().find(|e| e.keys == "Mod+H").unwrap();
        assert!(chord.description.contains("→"), "chord desc={}", chord.description);
    }

    #[test]
    fn strips_comments_and_keeps_urls_in_strings() {
        let cfg = r#"
        // 这是注释
        binds {
            Mod+/ { spawn "echo http://example.com"; }  // 字符串里含 //
            Mod+T { spawn "firefox"; }
        }
        "#;
        let entries = parse_sample(cfg);
        assert_eq!(entries.len(), 2, "entries={:?}", entries);

        let slash = entries.iter().find(|e| e.keys == "Mod+/").unwrap();
        // spawn 第一个参数是 URL 字符串：去掉引号、路径前缀后作为程序名展示（http:// 被 rsplit('/') 去掉）。
        assert!(
            slash.description.contains("example.com"),
            "desc={}",
            slash.description
        );
    }

    #[test]
    fn follows_imports() {
        // import 与 include 都要能抽出路径（niri 用 include）。
        let cfg = r#"import "a.kdl"; include "dms/binds.kdl"; binds { Mod+Slash { spawn "keytip"; } }"#;
        let imports = find_imports(&strip_comments(cfg));
        assert_eq!(imports, vec!["a.kdl".to_string(), "dms/binds.kdl".to_string()]);
    }

    #[test]
    fn prefers_hotkey_overlay_title_over_action() {
        // niri 自带的 hotkey-overlay-title 应优先作为描述，而非裸露的 spawn 命令。
        let cfg = r#"
        binds {
            Mod+B hotkey-overlay-title="Open a Zen" { spawn "zen-browser"; }
            Mod+D hotkey-overlay-title="WeChat" { spawn "~/.config/niri/scripts/toggle-wechat.sh"; }
            Super+X hotkey-overlay-title="Power Menu: Toggle" { spawn "dms" "ipc" "call" "powermenu" "toggle"; }
        }
        "#;
        let entries = parse_sample(cfg);
        assert_eq!(entries.len(), 3, "entries={:?}", entries);
        let b = entries.iter().find(|e| e.keys == "Mod+B").unwrap();
        assert_eq!(b.description, "Open a Zen");
        assert_eq!(b.context, "启动程序");
        let d = entries.iter().find(|e| e.keys == "Mod+D").unwrap();
        // 即便命令是 .sh 脚本路径，overlay-title 仍应原样作为描述。
        assert_eq!(d.description, "WeChat");
        let x = entries.iter().find(|e| e.keys == "Super+X").unwrap();
        assert_eq!(x.description, "Power Menu: Toggle");
    }

    #[test]
    fn spawn_without_overlay_title_is_meaningful() {
        // 没有 overlay-title 时，spawn 也要尽量给出明确含义（别名 / dms 翻译 / 干净命令名）。
        let cfg = r#"
        binds {
            Mod+T { spawn "kitty"; }
            Mod+1 { spawn "dms" "ipc" "call" "audio" "increment" "3"; }
            Mod+2 { spawn "sh" "-c" "feishu"; }
            Mod+3 { spawn "~/.config/niri/scripts/toggle-siyuan.sh"; }
        }
        "#;
        let entries = parse_sample(cfg);
        let t = entries.iter().find(|e| e.keys == "Mod+T").unwrap();
        assert_eq!(t.description, "启动 终端");
        let a = entries.iter().find(|e| e.keys == "Mod+1").unwrap();
        assert_eq!(a.description, "音量；增加");
        let s = entries.iter().find(|e| e.keys == "Mod+2").unwrap();
        assert_eq!(s.description, "运行：飞书");
        let y = entries.iter().find(|e| e.keys == "Mod+3").unwrap();
        assert_eq!(y.description, "启动 toggle-siyuan");
    }

    #[test]
    fn handles_block_and_line_comments_in_binds() {
        let cfg = r#"
        binds /* 块注释 */ {
            /* 内部块注释 */ Mod+A { focus-workspace 1; }
            Mod+B { move-window-right; } /* 尾随注释 */
        }
        "#;
        let entries = parse_sample(cfg);
        assert_eq!(entries.len(), 2, "entries={:?}", entries);
        let a = entries.iter().find(|e| e.keys == "Mod+A").unwrap();
        assert_eq!(a.description, "切换工作区 1");
        assert_eq!(a.context, "工作区");
        let b = entries.iter().find(|e| e.keys == "Mod+B").unwrap();
        assert_eq!(b.description, "窗口移向右侧");
    }

    #[test]
    fn loads_from_file_with_import() {
        let dir = std::env::temp_dir().join(format!("keytip_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let main = dir.join("config.kdl");
        let sub = dir.join("inc.kdl");
        let _ = std::fs::write(&sub, "binds { Mod+Z { close; } }");
        let _ = std::fs::write(
            &main,
            format!("import \"{}\";\nbinds {{ Mod+X {{ spawn \"app\"; }} }}\n", sub.display()),
        );
        std::env::set_var("KEYTIP_NIRI_CONFIG", &main);
        let entries = load_niri_shortcuts();
        std::env::remove_var("KEYTIP_NIRI_CONFIG");
        let _ = std::fs::remove_dir_all(&dir);

        // 非空时额外插一条"提示"占位：1 + Mod+X + Mod+Z = 至少 3 条。
        assert!(entries.len() >= 3, "entries={:?}", entries);
        assert!(entries.iter().any(|e| e.keys == "Mod+X"));
        assert!(entries.iter().any(|e| e.keys == "Mod+Z"));
    }

    #[test]
    fn real_config_smoke() {
        let home = std::env::var("HOME").unwrap_or_default();
        let candidates = [
            format!("{home}/.config/niri/config.kdl"),
            format!("{home}/.config/niri/dms/binds.kdl"),
        ];
        let path = candidates.into_iter().find(|p| std::path::Path::new(p).exists());
        let Some(path) = path else {
            eprintln!("[real_config_smoke] skip: 本机无 niri 配置");
            return;
        };
        std::env::set_var("KEYTIP_NIRI_CONFIG", &path);
        let entries = load_niri_shortcuts();
        std::env::remove_var("KEYTIP_NIRI_CONFIG");
        eprintln!("[real_config_smoke] {} => {} entries", path, entries.len());
        assert!(entries.len() >= 2, "真实 niri 配置应解析出 binds");
        for e in entries.iter().take(40) {
            eprintln!("  [{}] {} | {} | {}", e.context, e.keys, e.action, e.description);
        }
        // 分类分布
        let mut hist: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for e in &entries {
            *hist.entry(e.context.clone()).or_default() += 1;
        }
        let mut hs: Vec<_> = hist.into_iter().collect();
        hs.sort_by(|a, b| b.1.cmp(&a.1));
        eprintln!("  分类分布: {:?}", hs);
    }
}

