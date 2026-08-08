//! 验证程序 1：通过 xdg-desktop-portal 的 GlobalShortcuts 接口注册一个全局快捷键。
//! 运行后按下绑定的键（这里设为 Super_L 作为 preferred trigger），若成功捕获会打印触发信息。
//! 用途：封堵 M0 风险点 1（portal GlobalShortcuts 在 niri 下是否真的能捕获）。
//!
//! ⚠️ 已知限制（M0 实测发现，属 M1 待解决）：
//!   当前裸二进制直接调用会被 portal 拒绝：`org.freedesktop.portal.Error.NotAllowed: An app id is required`
//!   portal 要求调用方有合法 app-id（来自 .desktop 文件 + compositor 在 permission store 的登记，
//!   或通过 org.freedesktop.portal.Activation 获取 activation token）。
//!   后续 M1 解决方案：(a) 提供 keytip.desktop 并让 compositor 登记；(b) 绑定前先调
//!   Background/Activation portal 获取 token 并设 XDG_ACTIVATION_TOKEN。

use ashpd::desktop::global_shortcuts::{Activated, GlobalShortcuts, NewShortcut};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let proxy = GlobalShortcuts::new().await?;
    println!("[verify1] GlobalShortcuts proxy 创建成功（portal 接口可用）");

    // 定义快捷键：show-keytip，触发键设为 Super_L（niri 下 Super 通常即 Mod 键）
    let shortcut = NewShortcut::new("show-keytip", "Show KeyTip")
        .preferred_trigger("Super_L");

    let session = proxy.create_session().await?;
    println!("[verify1] session 创建成功");

    // bind_shortcuts 第三个参数是 parent window identifier，无窗口可传 None
    let _request = proxy
        .bind_shortcuts(&session, &[shortcut], None)
        .await?;
    println!("[verify1] 绑定成功，请按下 Super_L（左 Super 键）测试...");

    // 监听激活事件流
    let mut stream = proxy.receive_activated().await?;
    while let Some(activated) = stream.next().await {
        let activated: Activated = activated;
        println!("[verify1] >>> 全局快捷键触发: id={}", activated.shortcut_id());
    }

    Ok(())
}
