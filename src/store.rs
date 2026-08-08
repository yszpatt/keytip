//! 快捷键数据模型与存储（M3 完整）。
//!
//! 统一结构：`{ app, context, keys, action, description }`
//! 数据源：内置默认库 `data/defaults/*.json` + 用户 `~/.config/keytip/shortcuts.json`，导入后合并。
//! 用户配置优先级高于默认库；并提供手动补充通道（`add_app_shortcuts`）。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 单条快捷键记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutEntry {
    /// 所属上下文/分组，如 "全局"、"编辑"、"导航"、"视图"。用于浮层分组展示。
    #[serde(default)]
    pub context: String,
    /// 按键组合，如 "Ctrl+P"、"Super+Shift+T"。字符串形式，保持灵活。
    pub keys: String,
    /// 动作名，如 "open_file"、"focus_up"。
    pub action: String,
    /// 人类可读描述。
    #[serde(default)]
    pub description: String,
}

/// 一个程序（由 app-id 标识）的快捷键集合。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppShortcuts {
    #[serde(default)]
    pub entries: Vec<ShortcutEntry>,
}

/// 全部快捷键数据库：app-id -> 该程序快捷键。
#[derive(Debug, Clone, Default)]
pub struct ShortcutStore {
    pub apps: HashMap<String, AppShortcuts>,
}

impl ShortcutStore {
    /// 从内置默认库目录加载所有 JSON 文件，合并为一个 store。
    ///
    /// 每个文件名为 `<app-id>.json`，内容为 `AppShortcuts`（或 `Vec<ShortcutEntry>`）。
    pub fn load_defaults(dir: &std::path::Path) -> Self {
        let mut store = ShortcutStore::default();
        if !dir.exists() {
            return store;
        }
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let app_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if app_id.is_empty() {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    // 容忍两种格式：直接是 entries 数组，或 { entries: [...] }
                    let app = parse_app_shortcuts(&text).unwrap_or_default();
                    store.apps.insert(app_id, app);
                }
                Err(e) => eprintln!("[store] 读取默认库失败 {}: {e}", path.display()),
            }
        }
        store
    }

    /// 完整加载：内置默认库 + 用户配置（`~/.config/keytip/shortcuts.json`）。
    /// 用户配置优先级高于默认库（按 app-id 整体覆盖，便于手动修正/补充）。
    pub fn load_all() -> Self {
        let mut store = Self::load_defaults(&defaults_dir());
        if let Some(user) = load_user_config() {
            for (app_id, app) in user.apps {
                store.apps.insert(app_id, app);
            }
        }
        store
    }

    /// 按 app-id 查询快捷键；查不到返回 None。
    pub fn get(&self, app_id: &str) -> Option<&AppShortcuts> {
        self.apps.get(app_id)
    }

    /// 手动补充/覆盖某个 app 的快捷键（用户通道）。
    /// 写入用户配置文件 `~/.config/keytip/shortcuts.json` 并即时返回更新后的 store。
    pub fn add_app_shortcuts(&mut self, app_id: &str, entries: Vec<ShortcutEntry>) {
        self.apps.insert(
            app_id.to_string(),
            AppShortcuts { entries },
        );
        self.save_user_config();
    }

    /// 把当前 store 中"用户补充的部分"持久化到用户配置（仅覆盖写过的 app）。
    /// 为简单起见：保存整个合并后的 store 到用户配置（用户配置即完整真相源之一）。
    pub fn save_user_config(&self) {
        let path = user_config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.apps) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    eprintln!("[store] 写入用户配置失败 {}: {e}", path.display());
                } else {
                    println!("[store] 已保存用户配置：{}", path.display());
                }
            }
            Err(e) => eprintln!("[store] 序列化失败：{e}"),
        }
    }
}

/// keytip 用户配置目录：`~/.config/keytip/`
pub fn config_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(&home).join(".config/keytip");
    }
    PathBuf::from(".config/keytip")
}

/// 用户快捷键配置文件：`~/.config/keytip/shortcuts.json`
/// 内容为 `{ "<app-id>": { "entries": [...] }, ... }`。
pub fn user_config_path() -> PathBuf {
    config_dir().join("shortcuts.json")
}

/// 读取用户配置（不存在或损坏时返回 None，不影响默认库）。
///
/// 容错：逐 app 解析，单个 app 数据损坏不会连累整个文件（之前曾因一条 yazi 的
/// `action` 为数组导致 `from_str::<HashMap<...>>` 整体失败、用户配置被整文件丢弃、
/// 表现为"所有快捷键都不显示"）。这里改用 tolerant 的逐 app 解析 + 跳过坏条目。
fn load_user_config() -> Option<ShortcutStore> {
    let path = user_config_path();
    if !path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    // 先整体解析为松散结构，再逐 app 用 tolerant 解析器处理。
    let raw: serde_json::Value = serde_json::from_str(&text).ok()?;
    let obj = raw.as_object()?;
    let mut apps: HashMap<String, AppShortcuts> = HashMap::new();
    for (app_id, val) in obj {
        // 逐 app：成功解析才纳入；坏数据打印告警并跳过（不连累其他 app）。
        match serde_json::from_value::<AppShortcuts>(val.clone()) {
            Ok(app) => {
                apps.insert(app_id.clone(), app);
            }
            Err(e) => {
                eprintln!(
                    "[store] 用户配置 app={} 解析失败（已跳过）：{e}",
                    app_id
                );
            }
        }
    }
    if apps.is_empty() {
        return None;
    }
    Some(ShortcutStore { apps })
}

/// 解析 AppShortcuts：支持 `{entries:[...]}` 或直接 `[...]` 数组两种 JSON 形态。
fn parse_app_shortcuts(text: &str) -> Result<AppShortcuts, serde_json::Error> {
    if let Ok(app) = serde_json::from_str::<AppShortcuts>(text) {
        return Ok(app);
    }
    // 降级：直接是 entries 数组
    let entries: Vec<ShortcutEntry> = serde_json::from_str(text)?;
    Ok(AppShortcuts { entries })
}

/// 返回 keytip 默认库目录（data/defaults）。
///
/// 定位优先级：
///   1. 环境变量 `KEYTIP_DATA_DIR`（便于测试/自定义）。
///   2. XDG 用户数据 `~/.local/share/keytip/defaults`（用户级安装/自定义）。
///   3. XDG 系统数据 `$XDG_DATA_DIRS/keytip/defaults`，如 `/usr/local/share`、`/usr/share`
///      （系统级打包安装位置，Arch PKGBUILD 装到 `/usr/share/keytip/defaults`）。
///   4. 可执行文件同级 `../data/defaults`（安装后 data 与二进制同部署）。
///   5. 开发期 `CARGO_MANIFEST_DIR/data/defaults`。
///   6. 兜底相对路径 `data/defaults`。
pub fn defaults_dir() -> PathBuf {
    if let Ok(p) = std::env::var("KEYTIP_DATA_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    // XDG 用户数据目录（用户级安装/自定义，优先于系统数据）
    if let Some(home) = std::env::var_os("HOME") {
        let xdg = PathBuf::from(&home).join(".local/share/keytip/defaults");
        if xdg.exists() {
            return xdg;
        }
    }
    // XDG 系统数据目录（Arch 打包后 /usr/share/keytip/defaults 等）
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for dir in std::env::split_paths(&dirs) {
            let candidate = dir.join("keytip/defaults");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    // 可执行文件同级 ../data/defaults
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            let candidate = bin_dir.join("../data/defaults");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    // 开发期
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        if !manifest.is_empty() {
            return PathBuf::from(manifest).join("data/defaults");
        }
    }
    PathBuf::from("data/defaults")
}

/// 收藏配置文件：`~/.config/keytip/favorites.json`
/// 内容为 `{ "<app-id>": ["<keys>|<context>|<description>", ...], ... }`。
pub fn favorites_path() -> PathBuf {
    config_dir().join("favorites.json")
}

/// 为单条快捷键生成稳定的收藏标识 key。
///
/// 组合 `keys|context|description`（不含 app-id，因为外层已按 app 分组）。
/// 用 `|` 作分隔符（按键里不会出现的字符），足够稳定区分不同条目。
pub fn favorite_key_of(entry: &ShortcutEntry) -> String {
    format!("{}|{}|{}", entry.keys, entry.context, entry.description)
}

/// 读取收藏：返回 `{ app-id -> 该 app 已收藏的 key 集合 }`。
pub fn load_favorites() -> HashMap<String, Vec<String>> {
    let path = favorites_path();
    if !path.exists() {
        return HashMap::new();
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HashMap::new(),
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// 保存收藏：整体覆盖写 `favorites.json`。
pub fn save_favorites(all: &HashMap<String, Vec<String>>) {
    let path = favorites_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(all) {
        let _ = std::fs::write(&path, text);
    }
}

/// 切换某 app 下某条快捷键的收藏状态，返回切换后的"是否已收藏"。
///
/// 直接读写磁盘（`favorites.json`），保证跨进程/重启持久化。
pub fn toggle_favorite(app_id: &str, entry: &ShortcutEntry) -> bool {
    let key = favorite_key_of(entry);
    let mut all = load_favorites();
    let list = all.entry(app_id.to_string()).or_default();
    let now_fav = if let Some(pos) = list.iter().position(|k| k == &key) {
        list.remove(pos);
        false
    } else {
        list.push(key);
        true
    };
    save_favorites(&all);
    now_fav
}
