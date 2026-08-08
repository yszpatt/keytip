//! 单实例 + toggle IPC。
//!
//! keytip 由 niri 的 `Mod+Slash` 通过 `spawn` 唤起——每次按键都会 fork 一个新进程。
//! 要让"再按一次关闭 / 提到最前"成立，必须保证同一时刻只有一个 keytip 实例在跑：
//!
//!   - 启动时尝试 bind 一个 Unix socket（位于 `$XDG_RUNTIME_DIR/keytip.sock`）；
//!   - 若 bind 成功 => 成为 server（唯一实例），在后台线程监听该 socket；
//!   - 若 bind 失败（地址已被占用）=> 已有实例在跑，本进程转为 client：
//!       查询当前焦点窗口（`niri msg focused-window`）：
//!         * 焦点是 keytip（说明它在最前）  => 通知旧实例 **关闭**；
//!         * 焦点不是 keytip（说明被盖住）  => 通知旧实例把自身 **提到最前**。
//!
//! 这样即精确实现了用户需求：keytip 在最前时再按 Mod+Slash 关闭窗口；
//! 顺带让"被盖住时再按"把它重新提到最前，而不是再开一个实例。

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eframe::egui::Context;

const SOCKET_NAME: &str = "keytip.sock";

/// 计算 socket 文件路径：优先 `$XDG_RUNTIME_DIR`，否则 `/tmp/keytip-<user>.sock`。
fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(dir).join(SOCKET_NAME)
    } else {
        let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        PathBuf::from("/tmp").join(format!("keytip-{user}.sock"))
    }
}

/// 尝试以 server 身份 bind socket。
///
/// - 返回 `Some(listener)`：成为唯一实例，应使用 server 流程。
/// - 返回 `None`：已有活实例（地址被占用且能连上），应走 client 流程。
///
/// 处理了"残留 socket 文件"情形：若文件存在但无人监听（连不上），则删掉重绑，
/// 避免上次崩溃后残留的 socket 文件把新实例误判为 client。
pub fn try_bind() -> Option<UnixListener> {
    let path = socket_path();
    match UnixListener::bind(&path) {
        Ok(l) => Some(l),
        Err(_) => {
            // 文件可能残留且无人监听：连一下探活。
            if UnixStream::connect(&path).is_ok() {
                // 真有活实例在监听
                None
            } else {
                // 残留死 socket，删掉重绑
                let _ = std::fs::remove_file(&path);
                UnixListener::bind(&path).ok()
            }
        }
    }
}

/// client：连上已有实例并发送指令（`"close"` 或 `"focus"`）。
pub fn notify_existing(msg: &str) -> std::io::Result<()> {
    let mut s = UnixStream::connect(socket_path())?;
    s.write_all(msg.as_bytes())?;
    Ok(())
}

/// server：在后台线程监听 socket。
///
/// - 收到 `"close"`：置 `close_flag`（UI 线程每帧检查后关闭窗口），并唤醒 UI。
/// - 收到 `"focus"`：执行 `focus_fn`（把自身窗口提到最前），并唤醒 UI。
///
/// `ctx.request_repaint()` 可从任意线程调用，用于唤醒 eframe 事件循环，
/// 确保 `close_flag` 被设置后窗口能及时关闭。
pub fn serve(
    listener: UnixListener,
    close_flag: Arc<AtomicBool>,
    ctx: Context,
    focus_fn: impl Fn() + Send + 'static,
) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(mut s) = stream {
                let mut buf = [0u8; 16];
                if let Ok(n) = s.read(&mut buf) {
                    let msg = String::from_utf8_lossy(&buf[..n]);
                    if msg.starts_with("close") {
                        close_flag.store(true, Ordering::SeqCst);
                        ctx.request_repaint();
                    } else if msg.starts_with("focus") {
                        focus_fn();
                        ctx.request_repaint();
                    }
                }
            }
        }
    });
}

/// 进程退出时清理 socket 文件，避免残留导致下次无法 bind。
pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
