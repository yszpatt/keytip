//! niri 合成器 IPC 封装。
//!
//! 主路径：调用 `niri msg -j <command>` 查询 compositor 状态。
//! 这是结合 niri 配置的最优方案——比裸 foreign-toplevel 协议更简单、官方支持、零额外依赖。
//! 详见 docs/plan.md §6 决策 7。

use std::process::Command;

use serde::Deserialize;

/// `niri msg -j focused-window` 返回的窗口信息（仅取我们需要的字段）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    /// 应用的 app-id（与 niri window-rule 的 `match app-id` 一致），
    /// 是 keytip 映射到"程序快捷键档案"的主键。
    pub app_id: String,
    pub pid: u32,
    #[serde(default)]
    pub workspace_id: Option<u32>,
    #[serde(default)]
    pub is_focused: bool,
}

/// 调用 `niri msg -j focused-window` 获取当前焦点窗口信息。
///
/// 失败时返回 Err（例如 niri 未运行、或不在 niri 会话中）。
pub fn focused_window() -> Result<WindowInfo, String> {
    let out = Command::new("niri")
        .args(["msg", "-j", "focused-window"])
        .output()
        .map_err(|e| format!("无法执行 `niri msg`（niri 是否在运行？）：{e}"))?;

    if !out.status.success() {
        return Err(format!(
            "`niri msg focused-window` 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let info: WindowInfo = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("解析 niri 输出失败：{e}\n原始：{}", String::from_utf8_lossy(&out.stdout)))?;

    Ok(info)
}

/// 返回"真正要展示快捷键的目标窗口"：排除 keytip 自身，且只在**当前焦点
/// workspace** 内寻找。
///
/// 关键修复：keytip 由快捷键 `spawn` 唤起后通常会立即抢走焦点
/// （niri `open-focused true`），于是 `focused-window` 返回的是 keytip 自身。
/// 旧实现此时回退到 `list_windows().find(app_id != keytip)`——而 `list_windows()`
/// 返回**所有 workspace 的全部窗口**，会命中别的 workspace 里还开着的程序（如 kitty），
/// 导致"空桌面却显示 kitty 快捷键"。
///
/// 正确判断：
/// - 焦点窗口存在且不是 keytip → 用它（正常情况）；
/// - 焦点是 keytip，或焦点在背景（无窗口聚焦）→ 看 keytip 所在的**焦点 workspace**
///   内是否有其它窗口：有则用该窗口；没有则视为空桌面，返回 `None`
///   ——调用方据此退化为展示 niri 自身快捷键。
pub fn target_window() -> Option<WindowInfo> {
    let windows = list_windows().ok()?;
    // focused-window 可能返回 keytip 自身，或（焦点在背景时）失败。
    let focused = focused_window().ok();
    let focused_ws = focused_workspace_id();
    pick_target(focused.as_ref(), &windows, focused_ws)
}

/// 纯逻辑判定（便于单测）：给定焦点窗口、全部窗口、焦点 workspace id，选出目标窗口。
///
/// - `focused` 非 keytip → 直接返回它；
/// - 否则在 `focused_ws` 这个 workspace 内找第一个非 keytip 窗口；
/// - 找不到（含 `focused_ws` 无法确定且无焦点窗口）→ 返回 `None`（空桌面）。
fn pick_target(
    focused: Option<&WindowInfo>,
    windows: &[WindowInfo],
    focused_ws: Option<u32>,
) -> Option<WindowInfo> {
    if let Some(f) = focused {
        if f.app_id != "keytip" {
            return Some(f.clone());
        }
    }
    // 焦点是 keytip 或焦点在背景：仅限当前 workspace。
    windows
        .iter()
        .find(|w| w.app_id != "keytip" && w.workspace_id == focused_ws)
        .cloned()
}

/// 返回当前"焦点 workspace"的 id（`niri msg -j workspaces` 中 `is_focused` 为
/// 真的那个）。keytip 由快捷键 spawn 时，会被放到**唤起前所在的焦点 workspace**，
/// 因此用此 id 即可判断"keytip 出现在哪个 workspace、该 workspace 是否为空"。
fn focused_workspace_id() -> Option<u32> {
    let out = Command::new("niri")
        .args(["msg", "-j", "workspaces"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    if let serde_json::Value::Array(arr) = &v {
        for ws in arr {
            if ws.get("is_focused")?.as_bool()? {
                return ws.get("id")?.as_u64().map(|x| x as u32);
            }
        }
    }
    None
}

/// 列出所有窗口（`niri msg -j windows`），用于按 app-id 反查窗口 id。
fn list_windows() -> Result<Vec<WindowInfo>, String> {
    let out = Command::new("niri")
        .args(["msg", "-j", "windows"])
        .output()
        .map_err(|e| format!("无法执行 `niri msg -j windows`：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`niri msg -j windows` 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let v: Vec<WindowInfo> = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("解析 niri windows 失败：{e}"))?;
    Ok(v)
}

/// 把指定 app-id 的窗口提到最前（toggle 的 "focus" 分支：再次唤醒 keytip 时若被
/// 其他窗口盖住，把它提到最前）。
///
/// 不依赖启动时记录的 id——这里动态反查 keytip 当前真实的窗口 id，
/// 再 `niri msg action focus-window --id <id>` 把它提到最前。
/// 注意：此函数只用于"再次唤醒已有实例"，不会在首次唤起时抢走用户焦点。
pub fn focus_window_by_app_id(app_id: &str) -> Result<(), String> {
    let windows = list_windows()?;
    let target = windows
        .iter()
        .find(|w| w.app_id == app_id)
        .ok_or_else(|| format!("未找到 app-id={app_id} 的窗口"))?;
    let out = Command::new("niri")
        .args(["msg", "action", "focus-window", "--id", &target.id.to_string()])
        .output()
        .map_err(|e| format!("无法执行 focus-window：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "focus-window 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// 当前焦点窗口所在显示器的「逻辑尺寸 + 缩放系数」，用于计算 keytip 浮层窗口大小。
///
/// 逻辑：
///   1. `niri msg -j workspaces` 取 `is_focused` 为 true 的那个 workspace，读出它的 `output` 名
///      （如 "DP-1" / "HDMI-A-2"）——即当前焦点窗口所在的显示器。
///   2. `niri msg -j outputs` 找出该 `output` 的 `logical` 字段（`width/height/scale/x/y`），
///      返回 `(width, height, scale)`。
///
/// `width/height` 是逻辑像素（已含缩放），`scale` 是该显示器的缩放系数（如 1.0 / 1.667）。
/// keytip 用逻辑尺寸算"屏宽1/4、屏高80%"的目标，再用 scale 把高度钳到 niri 浮动窗口上限内。
///
/// 若无法确定焦点显示器（如 niri 未运行），退回第一块输出的尺寸作为兜底。
pub fn active_monitor_logical_size() -> Option<(f32, f32, f32)> {
    // 1) 焦点 workspace → output 名
    let active_output: Option<String> = (|| {
        let out = Command::new("niri")
            .args(["msg", "-j", "workspaces"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        if let serde_json::Value::Array(arr) = &v {
            for ws in arr {
                if ws.get("is_focused")?.as_bool()? {
                    return ws.get("output")?.as_str().map(|s| s.to_string());
                }
            }
        }
        None
    })();

    // 2) outputs → 对应 output 的 logical 尺寸 + scale；同时记录第一块作兜底。
    let out = Command::new("niri").args(["msg", "-j", "outputs"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;

    let mut fallback: Option<(f32, f32, f32)> = None;
    let mut matched: Option<(f32, f32, f32)> = None;
    if let serde_json::Value::Object(map) = &v {
        for (name, o) in map {
            let logical = o.get("logical")?;
            let w = logical.get("width")?.as_f64()? as f32;
            let h = logical.get("height")?.as_f64()? as f32;
            let s = logical.get("scale")?.as_f64()? as f32;
            if fallback.is_none() {
                fallback = Some((w, h, s));
            }
            if let Some(ref target) = active_output {
                if name == target {
                    matched = Some((w, h, s));
                    break;
                }
            }
        }
    }
    matched.or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: u64, app_id: &str, ws: u32, focused: bool) -> WindowInfo {
        WindowInfo {
            id,
            title: app_id.to_string(),
            app_id: app_id.to_string(),
            pid: id as u32,
            workspace_id: Some(ws),
            is_focused: focused,
        }
    }

    #[test]
    fn empty_workspace_does_not_pick_other_ws_window() {
        // keytip 在 workspace 2（空），kitty 在 workspace 1（另一个 workspace 还开着）。
        let windows = vec![
            win(1, "kitty", 1, false),
            win(2, "keytip", 2, true),
        ];
        // 情形 A：焦点就是 keytip（open-focused=true 抢焦点）
        let r = pick_target(Some(&windows[1]), &windows, Some(2));
        assert!(r.is_none(), "空 workspace 应回退 niri，而非误命中 kitty：{:?}", r);
        // 情形 B：焦点在背景（focused-window 失败，focused=None），focused_ws=2
        let r = pick_target(None, &windows, Some(2));
        assert!(r.is_none(), "空 workspace 应回退 niri，而非误命中 kitty：{:?}", r);
    }

    #[test]
    fn same_workspace_window_is_used() {
        // keytip 被 spawn 到 kitty 所在的 workspace 1。
        let windows = vec![win(1, "kitty", 1, true), win(2, "keytip", 1, false)];
        let r = pick_target(Some(&windows[0]), &windows, Some(1));
        assert_eq!(r.unwrap().app_id, "kitty");
    }

    #[test]
    fn focused_non_keytip_window_used_directly() {
        // 焦点就是普通程序（keytip 还没出现 / 未抢焦点）。
        let windows = vec![win(1, "firefox", 3, true)];
        let r = pick_target(Some(&windows[0]), &windows, Some(3));
        assert_eq!(r.unwrap().app_id, "firefox");
    }
}
