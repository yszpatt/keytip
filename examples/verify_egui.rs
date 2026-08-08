//! 验证程序 3：用 EGui/eframe 弹出一个最简窗口，确认在 niri 平铺下能正常显示。
//! 用途：封堵 M0 风险点 4（EGui 浮层在 niri 下的置顶/显示行为）。

use eframe::NativeOptions;
use eframe::egui::ViewportBuilder;

struct DemoApp;

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("KeyTip 浮层验证");
            ui.label("若你能看到这个窗口，EGui 在 niri 下工作正常。");
            ui.label("按 Esc 或关闭窗口退出。");
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("KeyTip Verify")
            .with_inner_size([320.0, 160.0]),
        ..Default::default()
    };
    println!("[verify3] 启动 EGui 窗口...");
    eframe::run_native(
        "keytip-verify",
        options,
        Box::new(|_cc| Ok(Box::new(DemoApp) as Box<dyn eframe::App>)),
    )
}
