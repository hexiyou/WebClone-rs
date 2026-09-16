#!/usr/bin/env bash
# GUI 冒烟验证：启动 WebClone-GUI.exe，跑 tests\gui_probe.py 的断言，最后清理进程。
#
# 三个必须注意的点：
#  1. 启动 GUI 和跑探针必须在**同一条命令**里完成。GUI 是这个 shell 派生出去的子进程，
#     一旦命令结束、父 shell 被回收，GUI 进程会被连坐杀掉（表现为探针跑到一半窗口
#     突然消失、截屏得到空图）。
#  2. 传给 grep 等 Windows 程序的路径要用 `pwd -W` 出来的 Windows 形式（C:/...），
#     用 MSYS 形式（/c/...）会报"系统找不到指定的路径"。
#  3. exe 会先被复制到 tests\.smoke-sandbox\ 里再运行，全程不碰 dist\。GUI 的配置
#     写在 exe 同目录，直接在 dist\ 里跑会和用户手调的 webclone-gui.json 互相污染
#     （测试读到用户的端口/并发，用户的配置又被测试结果覆盖）。
#
# 用法（Git Bash / MSYS）：
#   PYTHON=/path/to/python.exe . tests/gui_smoke.sh
#   用 `.` 在当前 shell 里执行，避免嵌套 bash 落到 WSL 上。
#
# 断言项：默认勾选状态、配置回显、代理框失能、数字框默认值与宽度、自定义 UA 勾选
# 联动、关于页渲染、切页重绘（像素级比对，防重影）、代理端口框随单选启停、网址框
# 回车（真实按键）、点关闭按钮能真退出，共 42 条；全过时退出码 0。

set -u

ROOT_MSYS="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
ROOT_WIN="$(cd "$ROOT_MSYS" && pwd -W 2>/dev/null || echo "$ROOT_MSYS")"
PY="${PYTHON:-python}"
LOG="$ROOT_WIN/gui-smoke.log"

cd "$ROOT_MSYS" || exit 1

# 先清残留进程：老实例的窗口会盖住新实例，探针 EnumerateWindows 可能抓到它。
if tasklist 2>/dev/null | grep -qi "WebClone-GUI.exe"; then
    echo "发现残留 WebClone-GUI 进程，先清理"
    taskkill //F //IM WebClone-GUI.exe >/dev/null 2>&1
    sleep 1
fi

# ---------- 沙箱隔离 ----------
# GUI 的配置固定写在"exe 同目录"的 webclone-gui.json 里。如果在 dist/ 里直接跑，
# 就会和用户自己手调的那份配置互相污染：测试读到用户的端口/并发，用户的配置被
# 测试结果覆盖。所以把 exe 复制到一个独立目录里跑，配置自然落在沙箱内。
#
# 全部用相对路径：cp/mv 这类外部程序不认 MSYS 的 /c/... 形式，会报
# "系统找不到指定的路径"。清理用 mv 而不是 rm：本环境的 safe-delete 策略会
# 拦截 rm，mv 改名则一定成功。
SANDBOX="tests/.smoke-sandbox"
mkdir -p "$SANDBOX"
if [ -f "$SANDBOX/webclone-gui.json" ]; then
    mv -f "$SANDBOX/webclone-gui.json" "$SANDBOX/webclone-gui.prev.json"
fi
if [ -f "$SANDBOX/gui-save-error.txt" ]; then
    mv -f "$SANDBOX/gui-save-error.txt" "$SANDBOX/gui-save-error.prev.txt"
fi
cp -f dist/WebClone-GUI.exe "$SANDBOX/"

"./$SANDBOX/WebClone-GUI.exe" &
GUI=$!
sleep 3

"$PY" tests/gui_probe.py > "$LOG" 2>&1
PROBE_RC=$?

# 探针最后一步会点关闭按钮：正常的话进程此刻已经自己退出了，这里只是兜底清理，
# 同时把"进程是否自行退出"作为独立于断言之外的第二重证据打出来。
if kill -0 "$GUI" 2>/dev/null; then
    echo "关闭验证: 进程仍在运行 [X]（探针未关掉，强制清理）"
    kill "$GUI" 2>/dev/null
else
    echo "关闭验证: 进程已自行退出 [OK]"
fi

echo "probe rc=$PROBE_RC"
[ -f "$SANDBOX/webclone-gui.json" ] && echo "cfg 已落盘(沙箱): yes" || echo "cfg 已落盘(沙箱): no"
echo "--- 断言 ---"
grep -E "^\[(PASS|FAIL)\]|^RESULT|^shot |^pid=|tab strip|Traceback" "$LOG"
exit "$PROBE_RC"
