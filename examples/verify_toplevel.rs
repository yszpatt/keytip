//! 验证程序 2：通过 wlr-foreign-toplevel-management 协议枚举顶层窗口并获取 app_id。
//! 用途：封堵 M0 风险点 2（niri 下能否拿到当前焦点窗口的 app_id）。
//!
//! 正确路径（wayland-protocols-wlr 0.3）：
//!   wayland_protocols_wlr::foreign_toplevel::v1::client::{manager, handle}

use std::sync::{Arc, Mutex};

use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, EventQueue, QueueHandle,
};
use wayland_client::event_created_child;
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{
        self, ZwlrForeignToplevelHandleV1,
    },
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

#[derive(Default)]
struct AppState {
    /// 收集到的所有顶层窗口 app_id
    app_ids: Vec<String>,
    /// 当前焦点窗口的 app_id（state 含 "active"）
    focused: Option<String>,
}

struct Handler {
    state: Arc<Mutex<AppState>>,
}

// WlRegistry：仅用于初始化，不需要处理事件
impl wayland_client::Dispatch<wl_registry::WlRegistry, GlobalListContents> for Handler {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<ZwlrForeignToplevelManagerV1, ()> for Handler {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // 每发现一个顶层窗口 handle，compositor 会发送 Toplevel 事件
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { .. } = event {
            println!("[verify2] 发现一个顶层窗口 handle");
        }
    }

    // Toplevel 事件会创建子对象 ZwlrForeignToplevelHandleV1，必须声明其 userdata
    event_created_child!(
        Handler,
        zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        [
            zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (
                zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
                ()
            )
        ]
    );
}

impl wayland_client::Dispatch<ZwlrForeignToplevelHandleV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        let mut st = state.state.lock().unwrap();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                println!("[verify2]   窗口 app_id = {}", app_id);
                if !st.app_ids.contains(&app_id) {
                    st.app_ids.push(app_id.clone());
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                println!("[verify2]   窗口 title  = {}", title);
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: ws } => {
                // ws 是 Vec<u8>（wlr 协议里 state 枚举的编码数组），
                // 含特定值（active=1）表示当前焦点窗口；这里仅打印原始值。
                if !ws.is_empty() {
                    println!("[verify2]   窗口 state   = {:?}", ws);
                }
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue): (wayland_client::globals::GlobalList, EventQueue<Handler>) =
        registry_queue_init(&conn)?;

    let state = Arc::new(Mutex::new(AppState::default()));
    let mut handler = Handler { state: state.clone() };

    // 绑定 wlr-foreign-toplevel-manager（版本 1..=1）
    let _manager = globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&queue.handle(), 1..=1, ())
        .expect("[verify2] 未找到 zwlr_foreign_toplevel_manager_v1 —— niri 可能不支持该协议");

    println!("[verify2] 已连接 wlr-foreign-toplevel-manager，监听 4 秒收集窗口...");

    // 事件循环：blocking_dispatch 几次以接收窗口信息
    for _ in 0..20 {
        queue.blocking_dispatch(&mut handler)?;
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    let st = state.lock().unwrap();
    println!("[verify2] ===== 收集结果 =====");
    println!("[verify2] 共发现 {} 个顶层窗口 app_id：", st.app_ids.len());
    for a in &st.app_ids {
        println!("[verify2]   - {}", a);
    }
    if st.app_ids.is_empty() {
        println!("[verify2] ⚠️ 未获取到任何窗口 —— 需排查 niri 协议支持或权限");
    } else {
        println!("[verify2] ✅ 成功获取窗口列表，M2 路径可行");
    }
    Ok(())
}
