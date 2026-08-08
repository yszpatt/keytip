//! 终端内程序探测（通用、可递归穿透）。
//!
//! niri（及任何 Wayland 合成器）只能看到"终端模拟器"这一层窗口（app-id=kitty），
//! 看不到终端里正在运行的 TUI 程序（nvim / yazi / htop ...），更看不到"终端里的
//! 复用器（tmux/screen）里"再跑的程序。
//!
//! 本模块的通用思路：
//!   1. 从终端进程出发，顺着 `/proc` 子树找"当前前台交互的程序"（忽略 shell 等壳）。
//!   2. 如果找到的程序本身是一个**复用器**（tmux/screen），它只是中间层——我们
//!      "桥接"进它的前台 pane，继续往下找真正的 TUI。这样 tmux→yazi、screen→nvim
//!      都能自然穿透，无需为某个具体应用写特例。
//!   3. 最终得到一条"容器链"：`[tmux, yazi]`（容器在前、真正的应用 leaf 在后），
//!      再据此生成一串候选 app-id 供 shortcuts.json 匹配（越具体越优先）。
//!
//! 设计要点：
//!   - 复用器列表 [`MULTIPLEXERS`] 与桥接逻辑 [`bridge_multiplexer`] 解耦：新增一种
//!     复用器只需在这两个地方各加一行，探测主流程不变（真正"通用"）。
//!   - 不依赖"最深层 comm 全局唯一"之类的脆弱假设；每进入一层都从新的起点重新向下走。

use std::collections::HashMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_tmux_client_suffix() {
        // 内核里 tmux 客户端 comm 写作 "tmux: client"，必须规范化为 "tmux"
        // 才能拼出与 shortcuts.json 一致的复合 id。
        assert_eq!(sanitize_comm("tmux: client"), "tmux");
        assert_eq!(sanitize_comm("tmux: server"), "tmux");
    }

    #[test]
    fn sanitize_lowercases_and_trims() {
        assert_eq!(sanitize_comm("  Neovim  "), "neovim");
        assert_eq!(sanitize_comm("yazi"), "yazi");
        assert_eq!(sanitize_comm(""), "");
    }

    #[test]
    fn composite_id_normalizes_inner() {
        assert_eq!(composite_id("kitty", "tmux: client"), "kitty:tmux");
        assert_eq!(composite_id("kitty", "nvim"), "kitty:nvim");
    }

    #[test]
    fn multiplexers_are_not_shells() {
        // 复用器不再被当成"壳"忽略，而是作为可穿透的中间层。
        assert!(!SHELL_LIKE.contains(&"tmux"));
        assert!(!SHELL_LIKE.contains(&"screen"));
        // 真正的 shell 仍被忽略。
        assert!(SHELL_LIKE.contains(&"zsh"));
        assert!(SHELL_LIKE.contains(&"bash"));
        // 复用器识别正确。
        assert!(is_multiplexer("tmux"));
        assert!(is_multiplexer("screen"));
        assert!(!is_multiplexer("nvim"));
    }

    #[test]
    fn lookup_keys_prefer_leaf_then_container() {
        // kitty → tmux → yazi：应优先 yazi 类候选，tmux 仅作兜底。
        let keys = resolve_lookup_keys("kitty", &["tmux".into(), "yazi".into()]);
        assert_eq!(
            keys,
            vec![
                "kitty:tmux:yazi", // 最具体（全链 + leaf）
                "kitty:yazi",      // 终端 + leaf（跳过复用器）—— 现有配置命中此项
                "yazi",            // 裸 leaf
                "kitty:tmux",      // 容器链（无 leaf）
                "kitty",           // 终端兜底
            ]
        );
    }

    #[test]
    fn lookup_keys_bare_terminal_app() {
        // kitty 里直接跑 nvim（无复用器）。
        let keys = resolve_lookup_keys("kitty", &["nvim".into()]);
        assert_eq!(keys, vec!["kitty:nvim", "nvim", "kitty"]);
    }

    #[test]
    fn lookup_keys_empty_chain_falls_back_to_terminal() {
        // 终端里只有 shell，没有任何 TUI → 退回终端自身。
        let keys = resolve_lookup_keys("kitty", &[]);
        assert_eq!(keys, vec!["kitty"]);
    }
}

/// 被视为"壳/基础设施"、不应单独作为展示目标的进程名。
/// 仅包含真正的 shell 与终端辅助进程（如 kitten）。
/// 注意：**复用器（tmux/screen）不在此列**——它们由 [`MULTIPLEXERS`] 单独管理，
/// 作为可穿透的中间层，而不是被忽略的壳。
const SHELL_LIKE: &[&str] = &[
    "zsh", "bash", "sh", "fish", "dash", "tcsh", "ksh", "elvish", "nu", // shells
    "kitten", // 终端辅助（kitty 的 kitten 是壳，不是目标应用）
];

/// 已知"终端复用器"：它们是中间层，需要桥接进其前台 pane 后才能继续向下探测。
/// 新增一种复用器：在此加名字，并在 [`bridge_multiplexer`] 加对应桥接分支即可。
const MULTIPLEXERS: &[&str] = &["tmux", "screen"];

/// 进程树节点：记录 pid、comm 与深度，用于筛候选 + 取最深。
struct ProcNode {
    pid: i32,
    comm: String,
    depth: usize,
}

/// 读取 `/proc/<pid>/stat` 的 comm 字段（字段 2，括号包裹）。
fn read_comm(pid: i32) -> Option<String> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm 在第一个 '(' 和最后一个 ')' 之间
    let start = raw.find('(')?;
    let end = raw.rfind(')')?;
    if end <= start {
        return None;
    }
    Some(raw[start + 1..end].to_string())
}

/// 通过 `/proc/<pid>/task/<pid>/children` 读取直接子进程列表（内核直接给出，最可靠）。
fn direct_children(pid: i32) -> Vec<i32> {
    let path = format!("/proc/{pid}/task/{pid}/children");
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    raw.split_whitespace()
        .filter_map(|s| s.parse::<i32>().ok())
        .collect()
}

/// 遍历进程树，收集所有后代（含深度）。遇到无法读取的节点（权限/已退出）跳过。
fn walk(pid: i32, depth: usize, out: &mut Vec<ProcNode>) {
    for child in direct_children(pid) {
        let comm = match read_comm(child) {
            Some(c) => c,
            None => continue,
        };
        out.push(ProcNode {
            pid: child,
            comm,
            depth,
        });
        walk(child, depth + 1, out);
    }
}

/// 规范化 comm 名，使其能安全用作复合 app-id 的片段。
///
/// 内核 `/proc/<pid>/stat` 的 comm 字段有时带后缀，例如 tmux 客户端写作 `tmux: client`
/// （冒号 + 空格）。这里统一：去掉冒号/空格及其后的所有内容，仅保留首个标识符片段并转小写。
fn sanitize_comm(comm: &str) -> String {
    let trimmed = comm.trim();
    let ident: String = trimmed
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ':')
        .collect();
    ident.to_lowercase()
}

/// 判断进程名是否为"壳"（shell / 终端辅助），应被探测忽略。
fn is_shell(comm: &str) -> bool {
    SHELL_LIKE.contains(&comm)
}

/// 判断进程名是否为"复用器"（tmux/screen 等中间层）。
fn is_multiplexer(comm: &str) -> bool {
    MULTIPLEXERS.contains(&comm)
}

/// 从给定 pid 出发，向下找"前台交互的程序"：忽略 shell，取最深的那个非 shell 进程。
///
/// 返回 `(规范化后的 comm, 该进程 pid)`。若整棵子树只有 shell（无 TUI）则返回 `None`。
/// 注意：复用器（tmux/screen）**不算** shell，会被当作候选返回——由调用方决定是否桥接。
fn deepest_foreground_app(pid: u32) -> Option<(String, u32)> {
    let mut nodes = Vec::new();
    walk(pid as i32, 1, &mut nodes);

    // 按规范化 comm 记录"最深进程"：同 comm 取最深，并保留该最深节点的 pid（用于桥接）。
    let mut best: HashMap<String, (i32, usize)> = HashMap::new();
    for n in &nodes {
        if is_shell(&n.comm) {
            continue;
        }
        let key = sanitize_comm(&n.comm);
        if key.is_empty() {
            continue;
        }
        let e = best.entry(key).or_insert((n.pid, 0));
        if n.depth > e.1 {
            *e = (n.pid, n.depth);
        }
    }

    best.into_iter()
        .max_by(|a, b| a.1 .1.cmp(&b.1 .1).then_with(|| b.0.cmp(&a.0)))
        .map(|(comm, (pid, _))| (comm, pid as u32))
}

/// 读取进程的 controlling tty 路径（如 `/dev/pts/5`）。
///
/// 优先读 `/proc/<pid>/fd/0`（标准输入通常直连终端）；失败则依次尝试 fd/1、fd/2。
/// tmux 客户端与其所附着 pane 共享同一 tty，靠这点把"客户端"映射到"前台 pane"。
fn read_ctty(pid: u32) -> Option<String> {
    for fd in ["0", "1", "2"] {
        let link = std::fs::read_link(format!("/proc/{pid}/fd/{fd}")).ok()?;
        let s = link.to_string_lossy().to_string();
        if s.starts_with("/dev/pts/") || s.starts_with("/dev/tty") {
            return Some(s);
        }
    }
    None
}

/// 桥接进 tmux 的前台 pane：拿到与本客户端（tty）对应的 pane，返回其进程 pid。
///
/// tmux 的 pane 有独立的 pty，**不等于**客户端的 tty，所以不能用"pane_tty == client_tty"
/// 来匹配（那永远对不上）。正确做法：用客户端 tty 作为 `display-message -t` 的目标，
/// 直接问 tmux"这个客户端当前附着的是哪个 pane、其 pid 多少"。
fn tmux_foreground_pid(client_pid: u32) -> Option<u32> {
    let tty = read_ctty(client_pid)?;
    let out = std::process::Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "-t",
            &tty,
            "#{pane_pid}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let pid = s.trim().split_whitespace().next()?;
    pid.parse().ok()
}

/// 桥接进某个复用器的前台进程（通用分发）。
///
/// 给定复用器进程（comm + pid），返回它"当前前台 pane"里跑的进程 pid，以便继续向下探测。
/// 不同复用器的桥接方式不同：tmux 用 `tmux list-panes` 按 tty 匹配；screen 暂无实现
/// （返回 None，此时该复用器会被当作 leaf，按 `<terminal>:screen` 匹配，仍可用）。
fn bridge_multiplexer(comm: &str, pid: u32) -> Option<u32> {
    match comm {
        "tmux" => tmux_foreground_pid(pid),
        // "screen" => screen_foreground_pid(pid),  // 未来可加
        _ => None,
    }
}

/// 通用递归穿透：从终端进程出发，逐层穿越复用器，返回"容器链 + 真正的应用"。
///
/// 返回示例：
///   - kitty 里裸跑 nvim            -> `["nvim"]`
///   - kitty 里跑 tmux（仅 shell）  -> `["tmux"]`
///   - kitty 里 tmux 再跑 yazi      -> `["tmux", "yazi"]`
///   - kitty 里只有 zsh（无 TUI）   -> `[]`
///
/// 每层逻辑完全一致：找最深非 shell 程序；若是复用器就桥接进其 pane 继续，否则即为 leaf。
pub fn detect_chain(terminal_pid: u32) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut pid = terminal_pid;
    // 防御性上限，避免复用器桥接异常时死循环。
    for _ in 0..8 {
        let (comm, child_pid) = match deepest_foreground_app(pid) {
            Some(x) => x,
            None => break,
        };
        if is_multiplexer(&comm) {
            match bridge_multiplexer(&comm, child_pid) {
                Some(next) => {
                    chain.push(comm);
                    pid = next;
                    continue;
                }
                None => {
                    // 无法桥接（如 tmux IPC 失败）→ 该复用器本身作为 leaf。
                    chain.push(comm);
                    break;
                }
            }
        } else {
            chain.push(comm);
            break;
        }
    }
    chain
}

/// 已知会被穿透探测的终端模拟器 app-id。
/// 只有这些才需要走终端内程序探测；普通 GUI 程序（如 zen/firefox）直接用它自己的 app-id。
pub const TERMINAL_APP_IDS: &[&str] = &[
    "kitty",
    "Alacritty",
    "org.alacritty.Alacritty",
    "foot",
    "footclient",
    "org.wezterm.wezterm",
    "wezterm",
    "com.mitchellh.ghostty",
    "ghostty",
    "st",
    "urxvt",
    "rxvt-unicode",
    "gnome-terminal",
    "org.gnome.Terminal",
    "konsole",
    "org.kde.konsole",
    "tilix",
    "terminator",
    "lmate-terminal",
    "xfce4-terminal",
];

/// 判断 app-id 是否为受支持的终端模拟器。
pub fn is_terminal(app_id: &str) -> bool {
    TERMINAL_APP_IDS.contains(&app_id)
}

/// 由"容器链"生成候选 app-id，供 shortcuts.json 匹配（越具体优先级越高）。
///
/// 规则：容器链 = 复用器层（可能为空）+ 真正的应用 leaf（可能为空）。
///   1. 带 leaf 的候选（最具体 → 最泛）：`终端:复用器…:leaf`、…、`终端:leaf`、`leaf`。
///   2. 不带 leaf 的候选（容器兜底）：`终端:复用器…`、`终端`。
///
/// 这样 tmux→yazi 会优先匹配 `kitty:yazi`（现有配置），而非 `kitty:tmux`；只有找不到
/// yazi 配置时才退回显示 tmux 快捷键。纯 tmux（无 TUI）则只生成 `kitty:tmux` / `kitty`。
pub fn resolve_lookup_keys(terminal_app_id: &str, chain: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let n = chain.len();
    if n == 0 {
        // 终端内无 TUI：直接用终端自身 app-id。
        keys.push(terminal_app_id.to_string());
        return keys;
    }
    let leaf = &chain[n - 1];
    let containers = &chain[..n - 1];

    // 带 leaf：从"全链 + leaf"到"裸 leaf"，逐步砍掉前面的容器。
    for i in (0..=containers.len()).rev() {
        let mut parts = vec![terminal_app_id.to_string()];
        parts.extend_from_slice(&containers[..i]);
        parts.push(leaf.clone());
        keys.push(parts.join(":"));
    }
    // 裸 leaf（无容器前缀）：某些应用可能直接以裸名配置（如 `yazi`、`nvim`）。
    keys.push(leaf.clone());
    // 不带 leaf：从"全容器链"到"终端自身"，作为兜底。
    for i in (1..=containers.len()).rev() {
        let mut parts = vec![terminal_app_id.to_string()];
        parts.extend_from_slice(&containers[..i]);
        keys.push(parts.join(":"));
    }
    // 终端自身永远作为最后兜底。
    keys.push(terminal_app_id.to_string());
    keys
}

/// 构造复合 app-id：终端 + 内部程序，如 `kitty:nvim`。
///
/// 内部程序名会先经 `sanitize_comm` 规范化（去掉 `: client` 之类后缀、转小写），
/// 以保证与 shortcuts.json 中的键严格一致。
pub fn composite_id(terminal_app_id: &str, inner: &str) -> String {
    let inner = sanitize_comm(inner);
    format!("{terminal_app_id}:{inner}")
}
