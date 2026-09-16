#!/bin/bash
# 对"加 manifest 之前 / 之后"两个 exe 各截一张主界面图，用于观感对比。
#
# 两个 exe 放在同一目录下跑，且每次跑前清掉配置 json —— 保证两者看到的
# 初始界面状态完全一致（否则残留配置会让对比失真）。
#
# 必须在当前 shell 里跑（. _viztest/compare.sh），否则嵌套 bash 会落到 WSL。
set -u

R="C:/Users/Administrator/WorkBuddy/workBuddy-Space/webclone-rs"
PY="C:/Users/Administrator/.workbuddy/binaries/python/versions/3.13.12/python.exe"
V="$R/_viztest"

mkdir -p "$V"
cd "$V" || exit 1

cp -f "$R/target/release/WebClone-GUI.exe" "$V/WebClone-GUI.after.exe"

run_one() {
    local name="$1"
    rm -f "$V/webclone-gui.json" "$V/gui-save-error.txt"
    "$V/WebClone-GUI.$name.exe" >/dev/null 2>&1 &
    local app=$!
    sleep 3
    "$PY" "$V/shot.py" "$V/shot-$name.png"
    kill "$app" 2>/dev/null
    sleep 1
}

run_one "before"
run_one "after"
