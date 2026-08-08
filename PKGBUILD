# Maintainer: yszpat <yszpat@localhost>
# PKGBUILD for KeyTip — Wayland (niri) 快捷键提示工具
pkgname=keytip
pkgver=0.1.0
pkgrel=1
pkgdesc="Wayland 下的快捷键提示工具（niri 等合成器）：全局快捷键唤起，自动抓取当前程序并展示其快捷键"
arch=('x86_64')
url="https://github.com/yszpat/keytip"
license=('MIT')
depends=('niri' 'egl-wayland' 'libxkbcommon' 'vulkan-icd-loader' 'wayland')
makedepends=('cargo' 'rust')
source=("$pkgname-$pkgver.tar.gz")
# 本地构建：源码由工作区提供，不下载
noextract=("$pkgname-$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
    # 本地源码目录（CWD 为构建目录时的相对路径处理）
    if [ -d "$srcdir/$pkgname" ]; then
        cd "$srcdir/$pkgname"
    else
        cd "$srcdir"
    fi
}

build() {
    cd "$srcdir"
    export CARGO_HOME="$srcdir/cargo-home"
    export CARGO_TARGET_DIR="$srcdir/target"
    cargo build --release --locked --offline 2>/dev/null || cargo build --release --locked
}

check() {
    cd "$srcdir"
    test -x "$srcdir/target/release/$pkgname"
}

package() {
    cd "$srcdir"
    # 二进制
    install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
    # 内置默认库
    install -Dm644 data/defaults/*.json -t "$pkgdir/usr/share/$pkgname/defaults/"
    # 文档
    install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm644 LICENSE -t "$pkgdir/usr/share/licenses/$pkgname/" 2>/dev/null || true
}
