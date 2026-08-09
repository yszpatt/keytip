//! M4：EGui 浮层 UI。
//!
//! 风格对齐用户已有的 dms/kando 浮层生态：半透明、置顶、快速唤起/关闭。
//! 显示当前活动窗口（app-id）的快捷键，按 context 分组，并提供搜索过滤。

use eframe::egui;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 标签页：收藏页（默认）与全集页。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    /// 已收藏的快捷键。
    Fav,
    /// 全部快捷键。
    All,
}

/// 浮层状态：持有要展示的快捷键数据 + 搜索词。
pub struct OverlayApp {
    app_id: String,
    title: String,
    /// 按 context 分组后的快捷键。
    grouped: Vec<(String, Vec<crate::store::ShortcutEntry>)>,
    /// 搜索过滤词（小写）。
    filter: String,
    /// 滚动偏移（由 j/k 控制，也同步用户滚轮）。
    scroll_offset: f32,
    /// 查找模式：true 时搜索框可获得焦点、接受输入；false 时 j/k 用于滚动列表。
    find_mode: bool,
    /// 延迟一帧聚焦搜索框：按 f 进入查找时置位，在帧末再 request_focus，
    /// 避免把触发键 'f' 自身输入进搜索框。
    focus_search_next: bool,
    /// 样式是否已初始化（字体放大 / 滚动条贴边只设一次，避免每帧重复放大）。
    style_inited: bool,
    /// 按键列统一宽度（px）：按当前 app 全部快捷键里最长的按键测量，
    /// 保证所有分组按键左对齐、说明文字左对齐。数据变化时置 dirty 重测。
    key_col_w: f32,
    /// 需要重测按键列宽的标志（首帧 / 重探测刷新后置位）。
    key_col_dirty: bool,
    /// 当前标签页（默认显示收藏页）。
    active_tab: Tab,
    /// 当前 app 已收藏的快捷键 key 集合（来自 favorites.json）。
    favorites: HashSet<String>,
    /// 关闭请求标志（由 IPC server 线程在收到 "close" 时置位）。
    /// 单实例模式下，再按一次 Mod+Slash 会让旧实例的这一标志置位，UI 线程检测到后关闭窗口。
    close_requested: Arc<AtomicBool>,
    /// 是否曾经真正获得过焦点。
    /// niri spawn 出来的窗口首帧往往还没拿到焦点，若此时就按"失焦"关闭会导致窗口一闪即退。
    /// 只有在「曾获得焦点 → 又失去焦点」时才算真正失焦。
    ever_focused: bool,
    /// 启动时刻，用于宽限期兜底（某些合成器可能始终不给焦点）。
    started: std::time::Instant,

    // --- 延迟重探测（优化"终端里开文件后 nvim 要等十几秒才识别"）---
    /// 探测用的终端进程 pid（非终端程序为 0，不做重探测）。
    detect_pid: u32,
    /// 探测用的终端 app-id（如 kitty）。
    detect_terminal_app_id: String,
    /// 重探测窗口的截止时刻：从启用起的一段时间内持续轮询，以捕捉终端内前台进程
    /// 延迟就位（如 LazyVim 加载插件耗时十几秒）的情况。超过此时间点即停止轮询。
    redetect_until: std::time::Instant,
    /// 下一次轮询探测的计划时刻；到点后探测一次，若 app-id 变了就刷新浮层。
    next_redetect: Option<std::time::Instant>,

    /// 当前主题索引（浮层内按 `c` 在 THEMES 间循环切换，并持久化到磁盘）。
    theme_idx: usize,
}

impl OverlayApp {
    pub fn new(
        app_id: String,
        title: String,
        entries: Vec<crate::store::ShortcutEntry>,
        favorites: HashSet<String>,
        close_requested: Arc<AtomicBool>,
    ) -> Self {
        let mut s = Self {
            app_id: String::new(),
            title,
            grouped: Vec::new(),
            filter: String::new(),
            scroll_offset: 0.0,
            find_mode: false,
            active_tab: Tab::Fav,
            favorites,
            close_requested,
            ever_focused: false,
            started: std::time::Instant::now(),
            detect_pid: 0,
            detect_terminal_app_id: String::new(),
            redetect_until: std::time::Instant::now(),
            next_redetect: None,
            focus_search_next: false,
            style_inited: false,
            key_col_w: 0.0,
            key_col_dirty: true,
            theme_idx: crate::store::load_theme_index(),
        };
        s.regroup(entries);
        s.app_id = app_id;
        s
    }

    /// 设置延迟重探测参数（仅对终端程序有意义）。
    ///
    /// 终端里打开文件/切换 TUI 时，前台进程可能要等几百毫秒~十几秒才就位
    /// （如 LazyVim 加载插件），首帧快照容易抓到旧程序（yazi）而非新程序（nvim）。
    /// 这里在窗口启用后先等 1.2s 让程序就位，之后每 ~1s 轮询探测一次，最多持续 20s；
    /// 一旦解析出的 app-id 变了（如从 yazi 变成 nvim），立即重新加载并刷新浮层，
    /// 无需用户反复重按键。轮询只在 app-id 真正变化时才刷新，平时零开销、不闪烁。
    pub fn enable_redetect(&mut self, terminal_pid: u32, terminal_app_id: &str) {
        self.detect_pid = terminal_pid;
        self.detect_terminal_app_id = terminal_app_id.to_string();
        let now = std::time::Instant::now();
        self.next_redetect = Some(now + std::time::Duration::from_millis(1200));
        self.redetect_until = now + std::time::Duration::from_secs(20);
    }

    /// 把一批快捷键按 context 重新分组（保持出现顺序），供初始化与重探测刷新复用。
    fn regroup(&mut self, entries: Vec<crate::store::ShortcutEntry>) {
        let mut ordered_ctx: Vec<String> = Vec::new();
        let mut map: std::collections::BTreeMap<String, Vec<crate::store::ShortcutEntry>> =
            Default::default();
        for e in entries {
            if !ordered_ctx.contains(&e.context) && !e.context.is_empty() {
                ordered_ctx.push(e.context.clone());
            }
            map.entry(if e.context.is_empty() {
                "其他".to_string()
            } else {
                e.context.clone()
            })
            .or_default()
            .push(e);
        }
        self.grouped = ordered_ctx
            .into_iter()
            .map(|ctx| (ctx.clone(), map.remove(&ctx).unwrap_or_default()))
            .collect();
    }

    /// 用新的 app-id 重新加载快捷键与收藏，并重置滚动/查找态（保留当前标签页）。
    fn reload_for(&mut self, resolved: String) {
        let store = crate::store::ShortcutStore::load_all();
        let entries = store
            .get(&resolved)
            .map(|a| a.entries.clone())
            .unwrap_or_default();
        self.app_id = resolved;
        self.regroup(entries);
        self.favorites = crate::store::load_favorites()
            .get(&self.app_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        self.scroll_offset = 0.0;
        self.filter.clear();
        self.find_mode = false;
        self.key_col_dirty = true;
    }
}

impl eframe::App for OverlayApp {
    /// 透明清屏色：配合 ViewportBuilder::with_transparent(true)，
    /// 让窗口本体透明，只由 panel_fill 绘制半透明浮层底板。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 单实例 toggle：收到 IPC "close" 指令时，close_requested 被置位，这里关闭窗口。
        if self.close_requested.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // 仅初始化一次：放大整体字体 + 滚动条改为实心常驻并强制贴右。
        // （必须在创建面板 frame 之前执行，使 Frame::central_panel 读到最新样式。）
        let first_frame = !self.style_inited;
        if first_frame {
            ctx.style_mut(|style| {
                // 整体字体放大一点（用户要求）。仅在首帧执行一次，避免每帧重复放大。
                for fid in style.text_styles.values_mut() {
                    fid.size *= 1.3;
                }
            });
            self.style_inited = true;
        }

        // 按键列统一宽度（首帧 / 数据刷新后重测一次）：
        // 取当前 app 全部按键中最大的渲染宽度，保证列表内按键与说明各自左对齐。
        if self.key_col_dirty {
            let mono_size = ctx
                .style()
                .text_styles
                .get(&egui::TextStyle::Monospace)
                .map(|f| f.size)
                .unwrap_or(16.0);
            let font_id = egui::FontId::monospace(mono_size);
            let mut max_w = 0.0f32;
            ctx.fonts(|f| {
                for (_, entries) in &self.grouped {
                    for e in entries {
                        let w = f
                            .layout_no_wrap(e.keys.clone(), font_id.clone(), egui::Color32::WHITE)
                            .size()
                            .x;
                        if w > max_w {
                            max_w = w;
                        }
                    }
                }
            });
            // 列宽需容纳：星号按钮(18) + spacing(8) + code 框内边距(~18)。
            self.key_col_w = max_w + 44.0;
            self.key_col_dirty = false;
        }

        // ===== IME（中文输入法）强制启用 =====
        // 根因（egui-winit 0.29.1 源码确认）：该版本在 Linux 上直接丢弃所有 IME 事件
        // （`WindowEvent::Ime` 处理函数里有 `if cfg!(target_os = "linux") { /* ignore */ }`，
        //  见 https://github.com/emilk/egui/issues/5008）。于是 fcitx5 的中文提交
        //  `Event::Ime(Commit)` 永远到不了 egui 的 TextEdit → 表现为「英文能打、中文不行」。
        //  此问题已通过 vendor/egui-winit-0.29.1-patched（去掉该 linux 守卫）修复。
        //
        // 修复方式：每帧常驻 IMEAllowed(true)，使窗口级 IME 始终开启。一旦 compositor
        // 把 keytip 聚焦（text-input enter），winit 在 ime_allowed==true 时就会调用
        // text_input.enable()，fcitx5 随即把中文路由到本窗口——无需任何手动焦点切换。
        //
        // ⚠️ 历史上曾用「先聚焦其他窗口制造 leave、再聚焦回本窗口制造 enter」的循环来
        // 补救 enable() 仅 enter 时调用一次的时序问题。但该循环会**主动偷走用户焦点**
        // （唤起 keytip 后跳到别的窗口），体验不可接受，已移除。现在依赖上面 vendor 补丁
        // + 每帧 IMEAllowed(true)：keytip 被正常聚焦的那一次 enter 即满足 enable 条件。
        // 当前视口焦点（自动关闭判定要用，提前取一次）。
        let focused = ctx.input(|i| i.focused);
        if focused {
            self.ever_focused = true;
        }
        // 常驻开启窗口 IME（不受搜索框焦点限制，确保 keytip 获得焦点时即可输入中文）。
        ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(true));

        // 失焦自动关闭（瞬时浮层模式：被 niri spawn 唤起，操作完即退）
        // 设 KEYTIP_NO_AUTOCLOSE 可禁用（用于无头/远程验证）。
        //
        // 关键：niri spawn 的窗口首帧通常尚未获得焦点，若直接按 !focused 关闭，
        // 窗口会一闪即退（实测现象：进程 exit 0 但窗口列表里看不到）。
        // 因此要求「先曾获得焦点，之后再失去」才关闭；并给一个宽限期兜底，
        // 避免合成器始终不给焦点时窗口永远关不掉。
        let autoclose = std::env::var_os("KEYTIP_NO_AUTOCLOSE").is_none();
        const FOCUS_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);
        let past_grace = self.started.elapsed() > FOCUS_GRACE;
        if autoclose && !focused && (self.ever_focused || past_grace) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // 未拿到焦点期间持续重绘，确保能及时观察到焦点变化。
        if !self.ever_focused {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // --- 延迟重探测（优化"终端里开文件后 nvim 要等十几秒才识别"）---
        // 终端程序的前台进程可能延迟就位（LazyVim 加载插件等耗时十几秒）。
        // 启用后的一段时间内（默认 20s）持续轮询：每 ~1s 重新穿透探测一次，
        // 一旦解析出的 app-id 变了（如从 yazi 变成 nvim），立即重新加载并刷新浮层，
        // 无需用户反复重按键。轮询只在 app-id 真正变化时刷新，平时零开销、不闪烁。
        if self.detect_pid != 0 {
            let now = std::time::Instant::now();
            if now < self.redetect_until {
                let due = self.next_redetect.map_or(true, |t| now >= t);
                if due {
                    // 安排下一次轮询（约 1s 后）。
                    self.next_redetect = Some(now + std::time::Duration::from_secs(1));
                    let chain = crate::term::detect_chain(self.detect_pid);
                    let candidates =
                        crate::term::resolve_lookup_keys(&self.detect_terminal_app_id, &chain);
                    let store = crate::store::ShortcutStore::load_all();
                    let resolved = candidates
                        .iter()
                        .find(|k| store.get(*k).is_some())
                        .cloned()
                        .unwrap_or_else(|| self.detect_terminal_app_id.clone());
                    if resolved != self.app_id {
                        self.reload_for(resolved);
                        ctx.request_repaint();
                    }
                }
                // 轮询窗口内保持重绘，确保计时器能准时触发。
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            } else {
                // 超过窗口期，停止轮询，避免长期空转。
                self.detect_pid = 0;
            }
        }

        // 搜索框 id 与焦点状态（供 Esc / f / j/k 复用）。
        let search_id = egui::Id::new("keytip_search");
        let search_focused = ctx.memory(|m| m.has_focus(search_id));
        self.find_mode = search_focused || self.focus_search_next;

        // 注：窗口级 IME 已由上方「IME 初始化」段每帧常驻开启（IMEAllowed(true)），
        // 此处不再重复发送。

        // Esc：搜索框聚焦时退出查找（失焦，恢复 j/k 滚动）；否则关闭窗口。
        let esc_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if esc_pressed {
            if search_focused {
                // 让搜索框失焦：surrender_focus 释放其焦点，j/k 恢复滚动。
                ctx.memory_mut(|m| m.surrender_focus(search_id));
                self.focus_search_next = false;
                self.find_mode = false;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }

        // f：仅在搜索框未聚焦时进入查找。延迟一帧再聚焦，
        // 避免把触发键 'f' 自身输入进搜索框（聚焦前本帧不捕获字符）。
        // 搜索框已聚焦时，f 作为普通搜索字符输入。
        let f_pressed = ctx.input(|i| i.key_pressed(egui::Key::F));
        if f_pressed && !search_focused {
            self.focus_search_next = true;
        }

        // j/k 滚动（仅在搜索框未聚焦时生效，避免与搜索框输入冲突）。
        if !search_focused {
            const SCROLL_STEP: f32 = 28.0;
            if ctx.input(|i| i.key_pressed(egui::Key::J)) {
                self.scroll_offset += SCROLL_STEP;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::K)) {
                self.scroll_offset -= SCROLL_STEP;
            }
            // t 切换标签页（收藏 / 全集），并重置滚动位置。
            if ctx.input(|i| i.key_pressed(egui::Key::T)) {
                self.active_tab = match self.active_tab {
                    Tab::Fav => Tab::All,
                    Tab::All => Tab::Fav,
                };
                self.scroll_offset = 0.0;
            }
            // c：循环切换主题（配色方案），实时生效并持久化索引。
            if ctx.input(|i| i.key_pressed(egui::Key::C)) {
                self.theme_idx = crate::theme::normalize(self.theme_idx as isize + 1);
                crate::theme::apply(ctx, self.theme_idx);
                crate::store::save_theme_index(self.theme_idx);
            }
        }

        // ====== CentralPanel：内容区天然占满整个窗口 ======
        // 不再纠结滚动条位置；用 CentralPanel + Frame 填充保证背景+内容区=窗口大小。
        // margin 直接设在 CentralPanel frame 上（不嵌套内层 Frame，避免 ScrollArea 拿不到全宽）。
        // 条目说明文字在剩余宽度内自动换行（Label::wrap()）。
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0)),
            )
            .show(ctx, |ui| {
                // 标题：⌨ KeyTip 居左（16px strong），窗口名同列靠右（12px weak，字号颜色不变）。
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("⌨ KeyTip")
                            .size(16.0)
                            .strong()
                            .color(ui.visuals().strong_text_color()),
                    );
                    // 窗口名：占用剩余宽度、右对齐，过长省略号截断。
                    if !self.title.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&self.title)
                                        .size(12.0)
                                        .color(ui.visuals().weak_text_color()),
                                )
                                .truncate(),
                            );
                        });
                    }
                });
                ui.add_space(4.0);

                // 标签页切换栏（收藏 / 全集），高亮当前页；右侧提示用弱化小字。
                ui.horizontal(|ui| {
                    let total = self.grouped.iter().map(|(_, v)| v.len()).sum::<usize>();
                    let fav_label = format!("★ 收藏 ({})", self.favorites.len());
                    let all_label = format!("☰ 全集 ({total})");
                    let fav_on = self.active_tab == Tab::Fav;
                    if ui.selectable_label(fav_on, fav_label).clicked() {
                        self.active_tab = Tab::Fav;
                        self.scroll_offset = 0.0;
                    }
                    if ui.selectable_label(!fav_on, all_label).clicked() {
                        self.active_tab = Tab::All;
                        self.scroll_offset = 0.0;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new("t 切换").weak().size(12.0));
                    });
                });
                ui.separator();

                // 搜索框：始终可交互——鼠标点击即可聚焦输入；键盘 f 进入查找。
                // 提示文字以 hint 形式内嵌于输入框，随查找状态变化。
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("🔍").size(14.0));
                    let hint = if self.find_mode {
                        "输入过滤 · Esc 退出"
                    } else {
                        "按 f 聚焦搜索…"
                    };
                    let te = egui::TextEdit::singleline(&mut self.filter)
                        .id(search_id)
                        .hint_text(hint)
                        .desired_width(ui.available_width());
                    ui.add(te);
                });
                ui.add_space(4.0);

                let filter = self.filter.to_lowercase();
                let mut found_any = false;

                // 根据当前标签页决定展示哪些条目：
                //   收藏页 => 仅 favorites 中的；全集页 => 全部。
                let visible: Vec<&(String, Vec<crate::store::ShortcutEntry>)> = self
                    .grouped
                    .iter()
                    .filter(|(_, entries)| {
                        if self.active_tab == Tab::Fav {
                            entries.iter().any(|e| {
                                self.favorites.contains(&crate::store::favorite_key_of(e))
                            })
                        } else {
                            true
                        }
                    })
                    .collect();

                let available = ui.available_height();
                let out = egui::ScrollArea::vertical()
                    .scroll_offset(egui::vec2(0.0, self.scroll_offset))
                    .max_height(available)
                    .auto_shrink([false, true])  // 水平方向不收缩 → 撑满窗口宽度；垂直方向正常收缩
                    .show(ui, |ui| {
                                for (ctx_name, entries) in &visible {
                                    // 收藏页：只保留被收藏的条目；全集页：全部。
                                    let matched: Vec<&crate::store::ShortcutEntry> = entries
                                        .iter()
                                        .filter(|e| {
                                            let in_fav = self
                                                .favorites
                                                .contains(&crate::store::favorite_key_of(e));
                                            let pass_filter = filter.is_empty()
                                                || e.keys.to_lowercase().contains(&filter)
                                                || e.action.to_lowercase().contains(&filter)
                                                || e.description.to_lowercase().contains(&filter);
                                            (self.active_tab == Tab::All || in_fav) && pass_filter
                                        })
                                        .collect();
                                    if matched.is_empty() {
                                        continue;
                                    }
                                    found_any = true;
                                    // 分类默认展开；只展示「按键 → 中文说明」，不显示英文 action。
                                    egui::CollapsingHeader::new(ctx_name.clone())
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            // Grid 两列：[星号+按键, 说明]。
                                            // 所有 Grid 共享同一个 key_col_w（min_col_width 强制），
                                            // 因此跨分组的按键左边缘与说明左边缘天然对齐。
                                            egui::Grid::new(("entries", ctx_name))
                                                .num_columns(2)
                                                .min_col_width(self.key_col_w)
                                                .spacing([8.0, 4.0])
                                                .show(ui, |ui| {
                                                    for e in &matched {
                                                        let fav = self.favorites.contains(
                                                            &crate::store::favorite_key_of(e),
                                                        );
                                                        // 列1：星号按钮 + 按键 code（左对齐）。
                                                        ui.horizontal(|ui| {
                                                            // ★ 已收藏用琥珀金强调，☆ 未收藏弱蓝灰。
                                                            let star = if fav { "★" } else { "☆" };
                                                            let star_color = if fav {
                                                                egui::Color32::from_rgb(245, 200, 76)
                                                            } else {
                                                                ui.visuals().weak_text_color()
                                                            };
                                                            if ui
                                                                .add_sized(
                                                                    [18.0, 18.0],
                                                                    egui::Button::new(
                                                                        egui::RichText::new(
                                                                            star,
                                                                        )
                                                                        .color(star_color),
                                                                    )
                                                                    .frame(false)
                                                                    .small(),
                                                                )
                                                                .clicked()
                                                            {
                                                                let now =
                                                                    crate::store::toggle_favorite(
                                                                        &self.app_id, e,
                                                                    );
                                                                if now {
                                                                    self.favorites.insert(
                                                                        crate::store::favorite_key_of(
                                                                            e,
                                                                        ),
                                                                    );
                                                                } else {
                                                                    self.favorites.remove(
                                                                        &crate::store::favorite_key_of(
                                                                            e,
                                                                        ),
                                                                    );
                                                                }
                                                                ctx.request_repaint();
                                                            }
                                                            ui.code(&e.keys);
                                                        });
                                                        // 列2：说明 wrap，左对齐。
                                                        ui.add(
                                                            egui::Label::new(&e.description).wrap(),
                                                        );
                                                        ui.end_row();
                                                    }
                                                });
                                        });
                                }

                                if !found_any {
                                    ui.label(if self.active_tab == Tab::Fav {
                                        if self.favorites.is_empty() {
                                            "（暂无收藏 · 在「全集」页点 ☆ 添加）"
                                        } else {
                                            "（无匹配结果）"
                                        }
                                    } else if filter.is_empty() {
                                        "（该程序暂无内置快捷键，可通过手动补充通道添加）"
                                    } else {
                                        "（无匹配结果）"
                                    });
                                }
                            });
                        // 同步 ScrollArea 实际偏移（含用户滚轮），保证 j/k 在滚轮基础上叠加。
                        self.scroll_offset = out.state.offset.y;

                        ui.separator();
                        let theme_name = crate::theme::get(self.theme_idx).name;
                        ui.label(
                            egui::RichText::new(format!(
                                "jk 滚动 · f 查找 · t 切换 · c 换肤[{theme_name}] · 点 ☆ 收藏 · Esc 关闭",
                            ))
                            .weak()
                            .size(12.0),
                        );
            }); // CentralPanel

        // 延迟聚焦搜索框：在本帧搜索框已渲染之后再 request_focus，
        // 这样触发键 'f' 不会被输入进搜索框（聚焦前本帧不捕获字符）。
        if self.focus_search_next {
            ctx.memory_mut(|m| m.request_focus(search_id));
            self.focus_search_next = false;
        }
    }
}
