//! KeyTip 主入口（M1 + M2 + M3 + M4 整合）。
//!
//! 工作方式（结合 niri 配置，见 docs/plan.md §6）：
//!   1. 由 niri 配置 binds 的 `Mod+/ { spawn "keytip"; }` 唤起（绕过 portal app-id 障碍）。
//!   2. 通过 `niri msg -j focused-window` 取当前焦点窗口的 app-id（M2：niri IPC）。
//!   3. 按 app-id 从默认库+用户配置合并查询快捷键（M3）。
//!   4. 弹出 EGui 浮层展示，Esc/失焦关闭（M4）。

mod fonts;
mod ipc;
mod niri;
mod overlay;
mod store;
mod term;

use eframe::egui::ViewportBuilder;
use eframe::NativeOptions;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// 把一次唤起的诊断信息追加写到 `~/.cache/keytip/last.log`。
///
/// keytip 通常由 niri 后台 spawn，stderr 用户看不到；这个文件让用户能事后查证
/// "到底解析成了哪个 app-id、匹配到多少条快捷键"，便于定位"某程序不显示快捷键"类问题。
fn diag(line: &str) {
    if let Some(home) = std::env::var_os("HOME") {
        let dir = std::path::Path::new(&home).join(".cache/keytip");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("last.log");
        let ts = chrono_stamp();
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "[{ts}] {line}")
            });
    }
}

/// 简单的本地时间戳（避免引入 chrono 依赖，用 libc 太重，这里用 Rust 标准库算）。
fn chrono_stamp() -> String {
    // 用 std::time 自 epoch 算一个人类可读的本地时间字符串（最小精度秒）。
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 转成 UTC 日历（不依赖时区库，够用于诊断）。
    let (h, m, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let day = (secs / 86400) + 719163; // 1970-01-01 的 RD 天数偏移
    format!("RD{day} {h:02}:{m:02}:{s:02}")
}

fn main() -> eframe::Result<()> {
    // --- M5：手动补充通道（极简 CLI 子命令）---
    //   keytip add <app_id> <keys> <action> [description] [context]
    //   写入用户配置 ~/.config/keytip/shortcuts.json
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "add" {
        return cmd_add(&args[2..]);
    }
    if args.len() > 1 && args[1] == "--detect-chain" {
        // 调试用：对给定 pid（通常是终端进程）跑完整穿透探测并打印耗时与候选，不弹 GUI。
        if let Some(pid_str) = args.get(2) {
            if let Ok(pid) = pid_str.parse::<u32>() {
                let t0 = std::time::Instant::now();
                let chain = term::detect_chain(pid);
                let t1 = std::time::Instant::now();
                // 用一个占位终端名生成候选（仅用于展示优先级）。
                let candidates = term::resolve_lookup_keys("kitty", &chain);
                let t2 = std::time::Instant::now();
                println!(
                    "[detect-chain] pid={} chain={:?} | detect={:.1?} resolve={:.1?}",
                    pid, chain, t1 - t0, t2 - t1
                );
                println!("[detect-chain] candidates={:?}", candidates);
                std::process::exit(0);
            }
        }
        eprintln!("[keytip] 用法：keytip --detect-chain <terminal_pid>");
        std::process::exit(2);
    }
    if args.len() > 1 && (args[1] == "-h" || args[1] == "--help" || args[1] == "help") {
        print_help();
        std::process::exit(0);
    }

    // 在非 TTY（被 niri spawn）时把日志打到 stderr，便于排查。
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    // --- 单实例 + toggle 检查（在查询焦点窗口之前）---
    // 每次按 Mod+Slash 都会 spawn 一个新进程。先判断是否已有 keytip 实例在跑：
    //   - 成为 server（唯一实例）=> 正常弹窗；
    //   - 发现已有实例（client）=> 按当前焦点决定"关闭旧实例"或"把旧实例提到最前"，然后退出。
    match ipc::try_bind() {
        Some(listener) => {
            // ===== server：成为唯一实例，正常启动浮层 =====
            // --- M2：通过 niri IPC 取当前焦点窗口（作为要展示快捷键的目标程序） ---
            let window = match niri::focused_window() {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("[keytip] 获取焦点窗口失败：{e}");
                    eprintln!("[keytip] 需在 niri 会话中运行，且 niri 已在执行。");
                    std::process::exit(1);
                }
            };

            // --- M3：按 app-id 查快捷键（默认库 + 用户配置合并）---
            // 若焦点是终端模拟器，通用地递归穿透其内部程序（含复用器 tmux/screen），
            // 生成一串候选 app-id（越具体越优先）。取第一个在 store 中存在的作为目标。
            let candidates = resolve_candidates(&window);
            let store = store::ShortcutStore::load_all();
            let resolved_app_id = candidates
                .iter()
                .find(|k| store.get(*k).is_some())
                .cloned()
                .unwrap_or_else(|| window.app_id.clone());
            eprintln!(
                "[keytip] 当前窗口 app_id={} 标题={} => 候选 {:?} => 命中 {}",
                window.app_id, window.title, candidates, resolved_app_id
            );
            let entries = store
                .get(&resolved_app_id)
                .map(|app| app.entries.clone())
                .unwrap_or_default();
            diag(&format!(
                "focus app_id={} title={} => candidates={:?} resolved={} | matched_entries={} (store apps: {})",
                window.app_id,
                window.title,
                candidates,
                resolved_app_id,
                entries.len(),
                store.apps.keys().cloned().collect::<Vec<_>>().join(",")
            ));

            // 加载当前 app 的收藏集合（来自 favorites.json），供浮层标记/过滤收藏页。
            let favorites: std::collections::HashSet<String> = store::load_favorites()
                .get(&resolved_app_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();

            // 关闭请求标志：供 IPC server 线程与 UI 线程通信。
            let close_flag = Arc::new(AtomicBool::new(false));

            // --- M4：弹 EGui 浮层 ---
            let mut app = overlay::OverlayApp::new(
                resolved_app_id.clone(),
                window.title.clone(),
                entries,
                favorites,
                close_flag.clone(),
            );
            // 终端程序启用"延迟重探测"：窗口获得焦点后约 1.2s 再探测一次，
            // 解决"终端里打开文件后 nvim 等要等十几秒才识别"的问题。
            if term::is_terminal(&window.app_id) {
                app.enable_redetect(window.pid, &window.app_id);
            }

            // 窗口尺寸：宽 = 活动显示器逻辑宽度 / 4，高 = 逻辑高度 × 80%，竖窗口。
            // 必须在 NativeOptions 阶段就设进 with_inner_size（niri 平铺 WM 下
            // ViewportCommand::InnerSize 无效，初始尺寸只能走这里）。
            // 活动显示器由 niri IPC 按"当前焦点窗口所在屏"取得，多屏各自合适。
            //
            // 关键：实测 niri 对 keytip 浮动窗口统一把请求的 logical 值 ×2 渲染成物理像素
            //（与显示器 scale 无关，疑 winit 报 native_pixels_per_point=2.0）。
            // 而用户看到的逻辑尺寸 = 物理 / scale。所以：
            //   请求 R → 物理 R×2 → 用户逻辑 R×2/scale
            //   要让用户看到「屏逻辑宽/4」: R = 屏逻辑宽 × scale / 8 = 屏物理宽 / 8
            //   要让用户看到「屏逻辑高×80%」: R = 屏逻辑高 × scale × 0.8 / 2 = 屏物理高 × 0.4
            // 另：niri 浮动窗口高度物理上限 ~1035，故 R_h 还需 min(_, 1035/2=517)。
            let (win_w, win_h) = match niri::active_monitor_logical_size() {
                Some((mw, mh, scale)) if mw > 1.0 && mh > 1.0 => {
                    let phys_w = mw * scale; // 物理宽
                    let phys_h = mh * scale; // 物理高
                    let w = (phys_w / 8.0).max(140.0); // 宽请求值
                    let h = (phys_h * 0.4).min(517.0); // 高请求值（517 = 1035/2 上限）
                    (w, h)
                }
                _ => (240.0, 432.0), // niri 不可用时的兜底（≈ 480×864 实际@scale1.0）
            };

            let options = NativeOptions {
                viewport: ViewportBuilder::default()
                    .with_title(format!("KeyTip · {}", resolved_app_id))
                    .with_inner_size([win_w, win_h])
                    .with_app_id("keytip") // 供 niri window-rule 匹配（浮动置顶）
                    .with_transparent(true)
                    .with_decorations(false), // 无边框工具窗，niri 更可能浮动置顶
                ..Default::default()
            };

            let res = eframe::run_native(
                "keytip",
                options,
                Box::new(move |cc| {
                    // 注入系统中文字体：egui 内置字体不含 CJK，否则中文全是方块 ▯。
                    fonts::install_cjk(&cc.egui_ctx);
                    // 半透明深色视觉：配合 with_transparent(true) + App::clear_color 做浮层效果。
                    let mut visuals = eframe::egui::Visuals::dark();
                    // ===== 科技感深蓝配色（稳重） =====
                    // 强调色：科技蓝（用于选中、悬停边框、光标、超链接）。
                    let accent = eframe::egui::Color32::from_rgb(82, 160, 255);
                    // 面板：深藏蓝，近乎不透明（保证中文可读）。
                    visuals.panel_fill =
                        eframe::egui::Color32::from_rgba_unmultiplied(17, 22, 31, 248);
                    visuals.window_fill = visuals.panel_fill;
                    // 全局文本：亮蓝白；weak（窗口名/提示）与 strong（标题）由它派生，色调统一。
                    visuals.override_text_color =
                        Some(eframe::egui::Color32::from_rgb(222, 231, 243));
                    // 深坑背景（输入框 / code 底色）。
                    visuals.extreme_bg_color = eframe::egui::Color32::from_rgb(9, 12, 18);
                    visuals.code_bg_color = eframe::egui::Color32::from_rgb(22, 29, 41);
                    // 选中 / 光标：科技蓝。
                    visuals.selection.bg_fill =
                        eframe::egui::Color32::from_rgba_unmultiplied(82, 160, 255, 64);
                    visuals.selection.stroke =
                        eframe::egui::Stroke::new(1.0, accent);
                    visuals.text_cursor = eframe::egui::style::TextCursorStyle {
                        stroke: eframe::egui::Stroke::new(1.5, accent),
                        ..Default::default()
                    };
                    visuals.hyperlink_color = accent;
                    // 控件状态：深蓝分层 + 蓝灰边框，悬停/按下以科技蓝强调。
                    use eframe::egui::Stroke;
                    let text_main = eframe::egui::Color32::from_rgb(222, 231, 243);
                    let text_bright = eframe::egui::Color32::from_rgb(240, 246, 255);
                    let border = eframe::egui::Color32::from_rgb(47, 61, 82);
                    let raised = eframe::egui::Color32::from_rgb(23, 31, 43);
                    let hover = eframe::egui::Color32::from_rgb(29, 40, 56);
                    let active = eframe::egui::Color32::from_rgb(35, 48, 68);
                    let open = eframe::egui::Color32::from_rgb(24, 32, 46);
                    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text_main);
                    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
                    visuals.widgets.inactive.bg_fill = raised;
                    visuals.widgets.inactive.fg_stroke =
                        Stroke::new(1.0, eframe::egui::Color32::from_rgb(198, 211, 229));
                    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border);
                    visuals.widgets.hovered.bg_fill = hover;
                    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text_bright);
                    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
                    visuals.widgets.active.bg_fill = active;
                    visuals.widgets.active.fg_stroke = Stroke::new(1.0, text_bright);
                    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent);
                    visuals.widgets.open.bg_fill = open;
                    // 无框按钮（星号收藏）悬停时的淡蓝底。
                    visuals.faint_bg_color =
                        eframe::egui::Color32::from_rgba_unmultiplied(82, 160, 255, 28);
                    cc.egui_ctx.set_visuals(visuals);

                    // 启动 IPC server：监听来自后续 keytip 进程的 toggle 指令。
                    // focus_fn 动态反查 keytip 自身窗口 id 并让 niri 把它提到最前。
                    let focus_fn = move || {
                        if let Err(e) = niri::focus_window_by_app_id("keytip") {
                            eprintln!("[keytip] 聚焦自身窗口失败：{e}");
                        }
                    };
                    ipc::serve(listener, close_flag.clone(), cc.egui_ctx.clone(), focus_fn);

                    Ok(Box::new(app) as Box<dyn eframe::App>)
                }),
            );
            // 窗口关闭后清理 socket 文件，避免残留。
            ipc::cleanup();
            return res;
        }
        None => {
            // ===== client：已有 keytip 实例在跑 =====
            // 查询当前焦点窗口，判断旧实例是否在最前：
            //   - 焦点是 keytip（在最前）  => 通知旧实例关闭；
            //   - 焦点不是 keytip（被盖住）=> 通知旧实例提到最前。
            match niri::focused_window() {
                Ok(focused) => {
                    if focused.app_id == "keytip" {
                        eprintln!("[keytip] 已有实例在最前，发送关闭指令");
                        let _ = ipc::notify_existing("close");
                    } else {
                        eprintln!("[keytip] 已有实例被盖住，发送聚焦指令");
                        let _ = ipc::notify_existing("focus");
                    }
                }
                Err(e) => {
                    eprintln!("[keytip] 查询焦点窗口失败（{e}），默认发送关闭指令");
                    let _ = ipc::notify_existing("close");
                }
            }
            std::process::exit(0);
        }
    }
}

/// 解析"最终用于查快捷键的 app-id"。
///
/// 普通 GUI 程序（zen/firefox/...）直接返回其 `app_id`。
/// 终端模拟器（kitty/...）则探测其内部运行的 TUI 程序：若探测到 nvim 等，
/// 返回复合 id `kitty:nvim`；否则返回原始 `app_id`（终端自身）。
/// 解析"最终用于查快捷键的候选 app-id 列表"（越靠前越优先）。
///
/// 普通 GUI 程序（zen/firefox/...）直接返回 `[app_id]`。
/// 终端模拟器（kitty/...）则通用地递归穿透其内部程序：若探测到 nvim 等 TUI，
/// 返回 `kitty:nvim` 等复合候选；若内部还套着复用器（tmux/screen），则继续穿透，
/// 例如 `kitty → tmux → yazi` 会生成 `[kitty:tmux:yazi, kitty:yazi, yazi, kitty:tmux, kitty]`
/// ——优先匹配 yazi，找不到才退回 tmux / kitty。无需为某个应用写特例。
fn resolve_candidates(window: &niri::WindowInfo) -> Vec<String> {
    if term::is_terminal(&window.app_id) {
        let chain = term::detect_chain(window.pid);
        if !chain.is_empty() {
            return term::resolve_lookup_keys(&window.app_id, &chain);
        }
    }
    vec![window.app_id.clone()]
}

/// M5：手动补充快捷键到用户配置。
fn cmd_add(args: &[String]) -> eframe::Result<()> {
    if args.len() < 3 {
        eprintln!("[keytip] 用法：keytip add <app_id> <keys> <action> [description] [context]");
        std::process::exit(2);
    }
    let app_id = args[0].clone();
    let keys = args[1].clone();
    let action = args[2].clone();
    let description = args.get(3).cloned().unwrap_or_default();
    let context = args.get(4).cloned().unwrap_or_else(|| "手动补充".to_string());

    let mut store = store::ShortcutStore::load_all();
    let mut entries: Vec<store::ShortcutEntry> = store
        .get(&app_id)
        .map(|a| a.entries.clone())
        .unwrap_or_default();
    entries.push(store::ShortcutEntry {
        context,
        keys,
        action,
        description,
    });
    store.add_app_shortcuts(&app_id, entries);
    Ok(())
}

fn print_help() {
    println!("KeyTip — Wayland 快捷键提示工具（niri）");
    println!();
    println!("用法：");
    println!("  keytip              唤起浮层，显示当前活动窗口的快捷键");
    println!("  keytip add <app_id> <keys> <action> [desc] [context]");
    println!("                    手动补充某程序的快捷键到用户配置");
    println!("  keytip help        显示本帮助");
    println!();
    println!("单实例 / toggle：");
    println!("  已显示时再按 Super+/ 会关闭窗口（若被其他窗口盖住则改为提到最前）。");
    println!();
    println!("用户配置：~/.config/keytip/shortcuts.json");
    println!("内置默认库：~/.local/share/keytip/defaults/");
    println!("唤起键：Super+/（在 niri 配置 binds 中定义）");
    println!();
    println!("终端穿透（通用、可递归）：");
    println!("  焦点为终端（kitty 等）时，自动识别其中运行的 TUI 程序（如 nvim），");
    println!("  并优先展示 `kitty:nvim` 复合档案；没配则退回终端自身快捷键。");
    println!("  终端里若还套着复用器（tmux/screen），会继续穿透到其前台 pane 里的程序，");
    println!("  例如 tmux 里跑 yazi 会优先匹配 `kitty:yazi`，而非 `kitty:tmux`。");
    println!("  手动补充终端内程序：keytip add kitty:nvim <keys> <action> [desc] [context]");
}
