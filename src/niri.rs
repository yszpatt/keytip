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
