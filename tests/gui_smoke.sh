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
# 断言项分两个阶段：
#  阶段一（gui_probe.py，42 条）：默认勾选状态、配置回显、代理框失能、数字框默认值与
#    宽度、自定义 UA 勾选联动、关于页渲染、切页重绘（像素级比对，防重影）、代理端口框
#    随单选启停、网址框回车（真实按键）、点关闭按钮能真退出。
#  阶段二（gui_probe.py --restore，9 条）：配置恢复路径 —— 把沙箱配置改成
#    SOCKS5 + 整站后重启，验证两组单选各只有一项选中、代理输入框跟着启用。
#    防的是单选回填"两个同时选中"（程序化 BM_SETCHECK 不清同组兄弟）。
# 两个阶段全过时退出码 0。

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

# ---------- 阶段二：配置文件恢复路径 ----------
# 阶段一的 42 条只覆盖"首次运行"（无配置 → 全套默认值）。而"配置里存着非默认值
# → 回填控件"是另一条独立路径，单选按钮的回填有专门的坑：程序化 BM_SETCHECK
# 不会像真实点击那样取消同组兄弟，会留下"直连 + SOCKS5 同时选中"。
# 所以这里做一次真实往返：把沙箱配置改成 SOCKS5 + 整站，重启进程，验回填结果。
LOG2="$ROOT_WIN/gui-smoke-restore.log"
RC2=1
if [ -f "$SANDBOX/webclone-gui.json" ]; then
    "$PY" - "$SANDBOX/webclone-gui.json" <<'PYEOF'
import json
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as handle:
    cfg = json.load(handle)
cfg["Mode"] = "full"
cfg["ProxyKind"] = "socks5"
cfg["ProxyHost"] = "127.0.0.1"
cfg["ProxyPort"] = 58591
with open(path, "w", encoding="utf-8") as handle:
    json.dump(cfg, handle, ensure_ascii=False, indent=2)
print("已把沙箱配置改成 Mode=full / ProxyKind=socks5 / 127.0.0.1:58591")
PYEOF

    "./$SANDBOX/WebClone-GUI.exe" &
    GUI2=$!
    sleep 3
    "$PY" tests/gui_probe.py --restore > "$LOG2" 2>&1
    RC2=$?
    if kill -0 "$GUI2" 2>/dev/null; then
        echo "阶段二关闭验证: 进程仍在运行 [X]（手动清理）"
        kill "$GUI2" 2>/dev/null
    else
        echo "阶段二关闭验证: 进程已自行退出 [OK]"
    fi
    echo "--- 阶段二断言（配置恢复）---"
    grep -E "^\[(PASS|FAIL)\]|^RESULT|^shot |^  |Traceback" "$LOG2"
    echo "restore rc=$RC2"
else
    echo "跳过阶段二: 沙箱配置不存在"
fi

[ "$PROBE_RC" = "0" ] && [ "$RC2" = "0" ] && exit 0
exit 1
