#!/usr/bin/env bash
# KeyTip 安装脚本（niri / Wayland）
set -euo pipefail

SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$SRC_DIR/target/release/keytip"

echo "[keytip] 构建 release 版本..."
cargo build --release

echo "[keytip] 安装二进制到 ~/.local/bin/"
mkdir -p ~/.local/bin
cp "$BIN" ~/.local/bin/keytip
chmod +x ~/.local/bin/keytip

echo "[keytip] 安装内置默认库到 ~/.local/share/keytip/defaults/"
mkdir -p ~/.local/share/keytip/defaults
cp "$SRC_DIR"/data/defaults/*.json ~/.local/share/keytip/defaults/

echo "[keytip] 确保用户配置目录存在 ~/.config/keytip/"
mkdir -p ~/.config/keytip

echo ""
echo "[keytip] 安装完成。"
echo "请确认 ~/.config/niri/dms/binds.kdl 含以下唤起键（若没有请添加）："
echo "    Mod+Slash { spawn \"$HOME/.local/bin/keytip\"; }"
echo "（注意 1：niri 里 / 必须写 keysym 名 Slash，写 Mod+/ 会导致整个 binds.kdl 解析失败）"
echo "（注意 2：必须用绝对路径，niri 的 PATH 不含 ~/.local/bin）"
echo ""
echo "并在 ~/.config/niri/config.kdl 加窗口规则（open-focused 必需，否则窗口一闪即关）："
echo '    window-rule {'
echo '        match app-id="keytip"'
echo '        open-floating true'
echo '        open-focused true'
echo '        default-floating-position x=50 y=0 relative-to="left"'
echo '        focus-ring { off; }'
echo '        border { off; }'
echo '        shadow { off; }'
echo '    }'
echo "校验配置：niri validate  （应显示 config is valid，niri 会自动重载）"
echo "按 Super+/ 唤起。"
