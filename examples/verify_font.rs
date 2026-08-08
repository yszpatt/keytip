//! 无头验证：检查 egui 字体系统能否用指定字体文件渲染汉字（glyph 是否解析成功）。
//!
//! 背景：SourceHanSansCN-*.otf 是 CFF 轮廓（8MB），ab_glyph/ttf-parser 对它支持不稳定——
//! 可能出现"字体加载无报错，但面板内中文全部空白/豆腐块"。本程序在无显示器环境下
//! 直接检查 egui 内部字体表中各字符是否有有效 glyph，从而判断是否可用。
//!
//! 用法：cargo run --example verify_font -- <字体文件路径>
//! 默认测试 /usr/share/fonts/adobe-source-han-sans/SourceHanSansCN-Regular.otf

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily, FontId};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "/usr/share/fonts/adobe-source-han-sans/SourceHanSansCN-Regular.otf".to_string()
    });
    let bytes = std::fs::read(&path).expect("读取字体文件失败");

    // 排除 ttc（与主程序逻辑一致）
    if bytes.len() >= 4 && &bytes[0..4] == b"ttcf" {
        println!("该文件是 TrueType Collection（ttc），ab_glyph 解析易失败，跳过。");
        return;
    }

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("test_cjk".to_string(), FontData::from_owned(bytes));
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "test_cjk".to_string());
    // 保留 egui 内置字体作为回退（不删），与 install_cjk 一致。

    let ctx = Context::default();
    ctx.set_fonts(fonts);

    // 与浮层实际用到的字号一致（body 12.5 × 1.3 ≈ 16）。
    let font_id = FontId::proportional(16.0);
    let tests = ['中', '文', '打', '开', '键', '盘', 'A', 'a', '0', '⌨', '★', '▸'];
    let mut ok = 0;
    // 无头模式：必须 run 一帧，字体才会在 begin_pass 时按 set_fonts 重建。
    ctx.run(eframe::egui::RawInput::default(), |ctx| {
        for ch in tests {
            let valid = ctx.fonts(|f| f.has_glyph(&font_id, ch));
            println!("  '{ch}' (U+{:04X}) glyph={}", ch as u32, valid);
            if valid {
                ok += 1;
            }
        }
    });
    println!("结果：{ok}/{} 字符解析成功", tests.len());
    if ok >= 8 {
        println!("结论：字体可用（中文+符号均能解析）");
    } else {
        println!("结论：字体不可用或部分字符缺失（中文很可能显示为豆腐块）");
    }
}
