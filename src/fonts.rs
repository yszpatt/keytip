//! 中文字体加载。
//!
//! 背景：egui 内置字体（Ubuntu/Hack/NotoEmoji）只覆盖拉丁字符，
//! 渲染中文时全部退化成豆腐块 ▯。必须显式注入系统 CJK 字体。
//!
//! 注意：**不要选 `.ttc`（TrueType Collection）**。egui 依赖的 ab_glyph
//! 对 ttc 支持不佳（`NotoSansCJK-Regular.ttc` 这类会加载失败），
//! 必须挑单一字面的 `.otf` / `.ttf`。

use eframe::egui::{FontData, FontDefinitions, FontFamily};

/// 候选正文中文字体（按优先级；均为单一字面，非 ttc）。
///
/// 正文指定为思源黑体 CN（Source Han Sans CN）。思源是 CFF/OTF 轮廓（8MB），
/// 早期 ab_glyph 对其支持不稳定（曾有"加载无报错但中文不可见"的旧结论）；
/// 在 egui 0.29.1 + ttf-parser 下已实测可用（`examples/verify_font.rs`：
/// 中文/拉丁/数字/符号 `has_glyph` 全部通过）。仍保留 TTF 的 MapleMono 作后备，
/// 以防个别环境 CFF 光栅化异常。
const PROPORTIONAL_CANDIDATES: &[&str] = &[
    // 思源黑体 CN — 用户指定正文字体，优先
    "/usr/share/fonts/adobe-source-han-sans/SourceHanSansCN-Regular.otf",
    "/usr/share/fonts/adobe-source-han-sans/SourceHanSansCN-Normal.otf",
    // TrueType 后备（曾为稳定首选）
    "/usr/share/fonts/maple/MapleMono-NF-CN-Regular.ttf",
];

/// 候选等宽中文字体（用于快捷键 `ui.code()` 展示）。
const MONOSPACE_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/maple/MapleMono-NF-CN-Regular.ttf",
    "/usr/share/fonts/maple/MapleMono-CN-Regular.ttf",
    "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf",
];

/// 读取第一个存在且可读的候选字体。
fn load_first(candidates: &[&str]) -> Option<(String, Vec<u8>)> {
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            // 排除 TrueType Collection：magic 为 "ttcf"，ab_glyph 解析易失败。
            if bytes.len() >= 4 && &bytes[0..4] == b"ttcf" {
                tracing::debug!("跳过 ttc 字体：{path}");
                continue;
            }
            let name = std::path::Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("cjk")
                .to_string();
            return Some((name, bytes));
        }
    }
    None
}

/// 把系统中文字体注入 egui，使中文正常显示。
///
/// 中文字体插入到各字族的**首位**，保证汉字优先由它渲染；
/// egui 自带字体仍保留在后面作为回退（覆盖 emoji 等）。
pub fn install_cjk(ctx: &eframe::egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut installed_any = false;

    if let Some((name, bytes)) = load_first(PROPORTIONAL_CANDIDATES) {
        fonts
            .font_data
            .insert(name.clone(), FontData::from_owned(bytes));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, name.clone());
        // 等宽字族也回退到它，避免没有等宽中文字体时 code() 里的汉字变豆腐。
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push(name);
        installed_any = true;
    }

    if let Some((name, bytes)) = load_first(MONOSPACE_CANDIDATES) {
        fonts
            .font_data
            .insert(name.clone(), FontData::from_owned(bytes));
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, name);
        installed_any = true;
    }

    if installed_any {
        eprintln!("[keytip] 已加载 CJK 字体：{installed_any} 个");
        ctx.set_fonts(fonts);
    } else {
        eprintln!(
            "[keytip] 警告：未找到可用的系统中文字体，中文可能显示为方块。\n\
             [keytip] 可安装：sudo pacman -S adobe-source-han-sans-cn-fonts  （或 noto-fonts-cjk）"
        );
    }
}
