//! KeyTip 主题（配色方案）模块。
//!
//! 把浮层视觉从硬编码的单一配色抽成可切换的主题列表。当前提供 5 套方案：
//! 科技深蓝（默认）、石墨浅色、墨绿暗色、暖橙暗色、紫罗兰暗色。
//!
//! 切换入口：浮层内按 `c` 循环切换；选中索引持久化到 `~/.config/keytip/theme.txt`，
//! 重启后保留。见 `store::load_theme_index` / `store::save_theme_index`。

use eframe::egui;

/// 一套完整主题的所有颜色。字段命名对应 egui `Visuals` 中实际使用的位置。
#[derive(Clone, Copy)]
pub struct Theme {
    /// 主题显示名（出现在浮层底部提示行）。
    pub name: &'static str,
    /// 是否为浅色主题（影响星标/状态色等少量派生选择）。
    pub light: bool,
    /// 面板 / 窗口底色（含 alpha，248/255 近乎不透明，保证中文可读）。
    pub panel: (u8, u8, u8, u8),
    /// 全局主文本色（亮蓝白 / 深灰）。
    pub text: (u8, u8, u8),
    /// 强调色（选中、悬停边框、光标、超链接、分组标题）。
    pub accent: (u8, u8, u8),
    /// 输入框 / code 深坑底色。
    pub extreme: (u8, u8, u8),
    /// code 块底色。
    pub code_bg: (u8, u8, u8),
    /// 控件分层：raised / hover / active / open（由 panel 派生或显式给出）。
    pub raised: (u8, u8, u8),
    pub hover: (u8, u8, u8),
    pub active: (u8, u8, u8),
    pub open: (u8, u8, u8),
    /// 边框色（蓝灰 / 浅灰）。
    pub border: (u8, u8, u8),
    /// 弱文本（窗口名 / 提示）与强文本（标题）。
    pub text_weak: (u8, u8, u8),
    pub text_strong: (u8, u8, u8),
    /// 星标（已收藏 ★）颜色。
    pub star: (u8, u8, u8),
    /// 无框按钮（星号）悬停淡底（含 alpha）。
    pub faint: (u8, u8, u8, u8),
}

impl Theme {
    fn rgba(c: (u8, u8, u8, u8)) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
    }
    fn rgb(c: (u8, u8, u8)) -> egui::Color32 {
        egui::Color32::from_rgb(c.0, c.1, c.2)
    }

    /// 由主题构建一套完整的 egui `Visuals`。
    pub fn to_visuals(&self) -> egui::Visuals {
        let mut v = if self.light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        let accent = Self::rgb(self.accent);
        v.panel_fill = Self::rgba(self.panel);
        v.window_fill = v.panel_fill;
        v.override_text_color = Some(Self::rgb(self.text));
        v.extreme_bg_color = Self::rgb(self.extreme);
        v.code_bg_color = Self::rgb(self.code_bg);
        v.selection.bg_fill = Self::rgba((self.accent.0, self.accent.1, self.accent.2, 64));
        v.selection.stroke = egui::Stroke::new(1.0, accent);
        v.text_cursor = egui::style::TextCursorStyle {
            stroke: egui::Stroke::new(1.5, accent),
            ..Default::default()
        };
        v.hyperlink_color = accent;

        let text_main = Self::rgb(self.text);
        let text_bright = Self::rgb(self.text_strong);
        let border = Self::rgb(self.border);
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_main);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border);
        v.widgets.inactive.bg_fill = Self::rgb(self.raised);
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text_main);
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border);
        v.widgets.hovered.bg_fill = Self::rgb(self.hover);
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text_bright);
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, accent);
        v.widgets.active.bg_fill = Self::rgb(self.active);
        v.widgets.active.fg_stroke = egui::Stroke::new(1.0, text_bright);
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, accent);
        v.widgets.open.bg_fill = Self::rgb(self.open);
        v.faint_bg_color = Self::rgba(self.faint);
        v
    }
}

/// 全部可选主题（顺序即循环顺序；索引 0 为默认）。
pub const THEMES: &[Theme] = &[
    // 1) 科技深蓝（默认）
    Theme {
        name: "科技深蓝",
        light: false,
        panel: (17, 22, 31, 248),
        text: (222, 231, 243),
        accent: (82, 160, 255),
        extreme: (9, 12, 18),
        code_bg: (22, 29, 41),
        raised: (23, 31, 43),
        hover: (29, 40, 56),
        active: (35, 48, 68),
        open: (24, 32, 46),
        border: (47, 61, 82),
        text_weak: (124, 138, 165),
        text_strong: (240, 246, 255),
        star: (245, 200, 76),
        faint: (82, 160, 255, 28),
    },
    // 2) 石墨浅色
    Theme {
        name: "石墨浅色",
        light: true,
        panel: (244, 245, 247, 250),
        text: (42, 51, 64),
        accent: (24, 95, 165),
        extreme: (255, 255, 255),
        code_bg: (236, 238, 242),
        raised: (255, 255, 255),
        hover: (228, 232, 238),
        active: (214, 222, 232),
        open: (240, 243, 247),
        border: (211, 214, 221),
        text_weak: (122, 131, 146),
        text_strong: (28, 37, 48),
        star: (224, 162, 26),
        faint: (24, 95, 165, 22),
    },
    // 3) 墨绿暗色
    Theme {
        name: "墨绿暗色",
        light: false,
        panel: (15, 26, 20, 248),
        text: (221, 239, 223),
        accent: (93, 202, 165),
        extreme: (8, 18, 12),
        code_bg: (19, 33, 26),
        raised: (19, 33, 26),
        hover: (25, 43, 34),
        active: (31, 54, 42),
        open: (22, 38, 30),
        border: (34, 64, 47),
        text_weak: (110, 138, 120),
        text_strong: (232, 247, 233),
        star: (224, 178, 58),
        faint: (93, 202, 165, 28),
    },
    // 4) 暖橙暗色
    Theme {
        name: "暖橙暗色",
        light: false,
        panel: (27, 21, 18, 248),
        text: (243, 231, 221),
        accent: (240, 153, 123),
        extreme: (16, 11, 8),
        code_bg: (36, 27, 21),
        raised: (36, 27, 21),
        hover: (46, 35, 28),
        active: (58, 44, 35),
        open: (38, 29, 23),
        border: (61, 46, 36),
        text_weak: (126, 110, 98),
        text_strong: (250, 240, 232),
        star: (232, 184, 75),
        faint: (240, 153, 123, 28),
    },
    // 5) 紫罗兰暗色
    Theme {
        name: "紫罗兰暗色",
        light: false,
        panel: (22, 18, 31, 248),
        text: (234, 228, 245),
        accent: (175, 169, 236),
        extreme: (14, 10, 22),
        code_bg: (32, 26, 46),
        raised: (32, 26, 46),
        hover: (42, 34, 58),
        active: (52, 42, 72),
        open: (34, 28, 48),
        border: (50, 42, 69),
        text_weak: (139, 127, 158),
        text_strong: (240, 235, 250),
        star: (232, 201, 90),
        faint: (175, 169, 236, 28),
    },
];

/// 规范化主题索引（防止越界）。
pub fn normalize(idx: isize) -> usize {
    let n = THEMES.len() as isize;
    let mut i = idx % n;
    if i < 0 {
        i += n;
    }
    i as usize
}

/// 主题数量。
pub fn count() -> usize {
    THEMES.len()
}

/// 根据索引取主题（越界自动回绕）。
pub fn get(idx: usize) -> &'static Theme {
    &THEMES[normalize(idx as isize)]
}

/// 把指定主题应用到当前 egui 上下文（实时生效）。
pub fn apply(ctx: &egui::Context, idx: usize) {
    ctx.set_visuals(get(idx).to_visuals());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_wraps_around() {
        assert_eq!(normalize(0), 0);
        assert_eq!(normalize(4), 4);
        assert_eq!(normalize(5), 0);
        assert_eq!(normalize(6), 1);
        assert_eq!(normalize(-1), 4);
        assert_eq!(normalize(-5), 0);
    }

    #[test]
    fn themes_have_distinct_names() {
        let mut seen = std::collections::HashSet::new();
        for t in THEMES {
            assert!(seen.insert(t.name), "重复主题名：{}", t.name);
        }
        assert!(count() >= 2);
    }

    #[test]
    fn to_visuals_builds_for_every_theme() {
        // 确保每套主题都能成功构建 Visuals（不 panic、panel_fill 非透明默认值）。
        for i in 0..count() {
            let v = get(i).to_visuals();
            assert_ne!(v.panel_fill, egui::Color32::default(), "主题 {} panel_fill 异常", i);
        }
    }
}
