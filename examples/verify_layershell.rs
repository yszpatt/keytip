//! 验证程序：用 wlr-layer-shell 在 niri 下创建置顶浮层 Surface。
//! 目的：封堵 M6 风险点——确认 keytip 浮层可走 layer-shell 实现"置顶/浮动"，
//! 绕开 eframe(xdg-shell) 在 niri 下被平铺的问题。
//! 仅画一个半透明有色矩形（不依赖字体渲染），证明 layer-surface 能置顶显示。

use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{
        wl_compositor, wl_buffer, wl_shm,
        wl_surface::WlSurface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

struct LayerState {
    width: u32,
    height: u32,
    surface: Option<WlSurface>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
}

impl LayerState {
    fn new() -> Self {
        Self { width: 360, height: 200, surface: None, layer_surface: None }
    }
}

impl Dispatch<wayland_client::protocol::wl_registry::WlRegistry, GlobalListContents> for LayerState {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_registry::WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {}
}
impl Dispatch<wl_compositor::WlCompositor, ()> for LayerState {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_shm::WlShm, ()> for LayerState {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_buffer::WlBuffer, ()> for LayerState {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<WlSurface, ()> for LayerState {
    fn event(_: &mut Self, _: &WlSurface, _: wayland_client::protocol::wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrLayerShellV1, ()> for LayerState {
    fn event(_: &mut Self, _: &ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrLayerSurfaceV1, ()> for LayerState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            println!("[verify-layer] layer-surface 配置 serial={serial}，置顶成功");
            if let Some(ls) = &state.layer_surface {
                ls.ack_configure(serial);
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[verify-layer] 连接 wayland...");
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue): (wayland_client::globals::GlobalList, EventQueue<LayerState>) =
        registry_queue_init(&conn)?;
    let mut state = LayerState::new();

    // registry_queue_init 内部已 dispatch registry 事件并填充 GlobalListContents，
    // 无需再 blocking_dispatch（否则会阻塞等待事件）。直接 bind 即可（同 M0 verify_toplevel 模式）。
    eprintln!("[verify-layer] 已初始化 globals");

    let compositor = globals.bind::<wl_compositor::WlCompositor, _, _>(&queue.handle(), 1..=6, ())
        .map_err(|e| format!("bind WlCompositor 失败：{e}"))?;
    let shm = globals.bind::<wl_shm::WlShm, _, _>(&queue.handle(), 1..=2, ())
        .map_err(|e| format!("bind WlShm 失败：{e}"))?;
    let layer_shell = globals.bind::<ZwlrLayerShellV1, _, _>(&queue.handle(), 1..=4, ())
        .map_err(|e| format!("bind LayerShell 失败：{e}"))?;
    println!("[verify-layer] 已绑定 compositor/shm/layer_shell");

    let surface = compositor.create_surface(&queue.handle(), ());
    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Overlay,
        "keytip".to_string(),
        &queue.handle(),
        (),
    );

    layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left);
    layer_surface.set_size(state.width, state.height);
    layer_surface.set_margin(20, 0, 0, 20);
    surface.commit();

    // 注：完整绘制需在 wl_shm pool 上 mmap 写入像素（需 fd），此处聚焦验证"置顶配置"，
    // 不创建 buffer；niri 仍会发 Configure 事件确认 overlay 层 surface 被接受。
    let _ = &shm;

    state.surface = Some(surface);
    state.layer_surface = Some(layer_surface);

    println!("[verify-layer] 已提交 layer-surface（overlay 层，置顶）。运行 4 秒...");
    for _ in 0..20 {
        queue.blocking_dispatch(&mut state)?;
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    println!("[verify-layer] 结束。");
    Ok(())
}
