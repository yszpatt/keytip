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

/// 返回"真正要展示快捷键的目标窗口"：排除 keytip 自身。
///
/// - 若当前焦点窗口不是 keytip，直接返回它（正常情况）；
/// - 若焦点是 keytip（被 `spawn` 后立即聚焦，或它就是唯一窗口），从窗口列表里
///   找**最前**的非 keytip 窗口——修复"刚唤起时焦点瞬间落在 keytip 上，
///   导致按 app-id 查到的是 keytip 自身、浮层空空"的竞态；
/// - 若没有任何非 keytip 窗口（空桌面 / 唯一窗口就是 keytip），返回 `None`
///   ——调用方据此退化为展示 niri 自身快捷键。
pub fn target_window() -> Option<WindowInfo> {
    if let Ok(w) = focused_window() {
        if w.app_id != "keytip" {
            return Some(w);
        }
    }
    let windows = list_windows().ok()?;
    windows.into_iter().find(|w| w.app_id != "keytip")
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

/// 把指定 app-id 的窗口提到最前（toggle 的 "focus" 分支）。
///
/// 不依赖启动时记录的 id——启动时 `focused_window()` 返回的是 keytip 背后的程序，
/// 而非 keytip 自身，直接用那个 id 会聚焦错窗口。这里动态反查 keytip 当前真实的窗口 id，
/// 再 `niri msg action focus-window --id <id>` 把它提到最前。
pub fn focus_window_by_app_id(app_id: &str) -> Result<(), String> {
    let windows = list_windows()?;
    let target = windows
        .iter()
        .find(|w| w.app_id == app_id)
        .ok_or_else(|| format!("未找到 app-id={app_id} 的窗口"))?;
    focus_window_by_id(target.id)
}

/// 按窗口 id 聚焦指定窗口（`niri msg action focus-window --id <id>`）。
///
/// 用于 IME 初始化时的"焦点离开→回到"循环：先把焦点移到 keytip 背后的窗口，
/// 让 keytip 触发 text-input 的 `leave`，再聚焦回 keytip 触发新的 `enter`，
/// 从而让 winit 在 `ime_allowed` 已为 true 时调用 `text_input.enable()`。
pub fn focus_window_by_id(id: u64) -> Result<(), String> {
    let out = Command::new("niri")
        .args(["msg", "action", "focus-window", "--id", &id.to_string()])
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

/// 返回「除 `self_app_id` 之外」某个可见窗口的 id（IME 循环中用于把焦点挪开，
/// 让 keytip 触发 text-input 的 `leave`）。
///
/// 关键点：必须用**实时查询**的结果，不能复用启动时记录的 id——若 keytip 在
/// 启动时已带焦点（niri `open-focused true`），`focused_window()` 返回的其实是
/// keytip 自身，用它去"聚焦背后"等于没动，leave 永不触发，IME 循环卡死。
///
/// 若除 keytip 外没有其他窗口则返回 None（此时无法制造 leave，IME 可能不可用，
/// 但英文输入不受影响）。
pub fn first_other_window_id(self_app_id: &str) -> Option<u64> {
    let windows = list_windows().ok()?;
    windows
        .iter()
        .find(|w| w.app_id != self_app_id)
        .map(|w| w.id)
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
