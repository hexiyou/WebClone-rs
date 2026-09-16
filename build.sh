#!/usr/bin/env bash
# WebClone（Rust 版）一键构建脚本。
#
# 用法:
#   ./build.sh              # CLI + GUI 全部构建，产物汇总到 dist/
#   ./build.sh cli          # 只构建命令行版
#   ./build.sh gui          # 只构建图形界面版
#   ./build.sh clean        # 先清 dist/ 再全量构建
#
# 产物:
#   dist/webclone.exe        命令行版（原生 exe，单文件，无运行时依赖）
#   dist/WebClone-GUI.exe    图形界面版（原生 exe，单文件，无运行时依赖）
#
# 说明:
#   Rust 编译的是机器码，没有 .NET 那种 AOT / 自包含之分，两个产物都是
#   独立单文件，拷走即用，无需任何运行库。

set -euo pipefail

# winsafe 只支持 Windows，必须在 Windows 侧的 Bash 里跑。
# 注意：某些环境下嵌套 `bash` 会落到 WSL，那会在同一个 target/ 里编出 Linux 产物，
# 与 Windows 产物互相污染，所以这里显式拦一下。
case "$(uname -s 2>/dev/null || echo unknown)" in
    Linux|Darwin)
        echo "build.sh 需要在 Windows 侧的 Bash（Git Bash / MSYS / Cygwin）里运行。" >&2
        echo "当前检测到的是 $(uname -s) —— winsafe 仅支持 Windows。" >&2
        echo "请改用 PowerShell 侧的 build.ps1，或直接执行 cargo build --release。" >&2
        exit 2
        ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

MODE="${1:-all}"
PROFILE="release"
TARGET_DIR="target/${PROFILE}"
DIST="$ROOT/dist"

# 某些受限会话（沙箱 / 嵌套 bash）会丢掉 PATH 里的用户工具目录，
# 先把 cargo 的默认安装位置补回去。
for d in "${HOME:-}/.cargo/bin" "${USERPROFILE:-}/.cargo/bin"; do
    if [ -n "$d" ] && [ -d "$d" ]; then
        case ":$PATH:" in
            *":$d:"*) ;;
            *) PATH="$PATH:$d" ;;
        esac
    fi
done
export PATH

# Git Bash 会话里环境变量不全（缺 ProgramFiles / OS 等），部分工具链会误判。
# 若存在何工自备的 env-fix.sh 就顺手 source 一下。
if [ -f "$HOME/.workbuddy/bin/env-fix.sh" ]; then
    # shellcheck disable=SC1090
    source "$HOME/.workbuddy/bin/env-fix.sh" 2>/dev/null || true
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "找不到 cargo，请确认 Rust 工具链已安装并加入 PATH。" >&2
    exit 1
fi

if [ "$MODE" = "clean" ]; then
    echo "==> 清理 dist/ 与 target/"
    rm -rf "$DIST" "$ROOT/target"
    MODE="all"
fi

mkdir -p "$DIST"

build_cli() {
    echo "==> 构建命令行版 webclone"
    cargo build -p webclone-cli --profile "$PROFILE"
    cp -f "$TARGET_DIR/webclone.exe" "$DIST/webclone.exe"
    echo "    -> dist/webclone.exe"
}

build_gui() {
    echo "==> 构建图形界面版 WebClone-GUI"
    cargo build -p webclone-gui --profile "$PROFILE"
    cp -f "$TARGET_DIR/WebClone-GUI.exe" "$DIST/WebClone-GUI.exe"
    echo "    -> dist/WebClone-GUI.exe"
}

case "$MODE" in
    cli)  build_cli ;;
    gui)  build_gui ;;
    all)  build_cli; build_gui ;;
    *)
        echo "未知参数：$MODE（可选 cli / gui / all / clean）" >&2
        exit 2
        ;;
esac

echo
echo "==> 产物:"
ls -la "$DIST"
