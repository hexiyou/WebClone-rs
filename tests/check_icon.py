#!/usr/bin/env python3
"""校验 exe 里的图标资源与 assets/app.ico 一致。

为什么值得单独写一个校验器：图标是"编译期嵌进 PE 资源区"的，代码里没有任何
运行时可断言的东西 —— GUI 探针能验证窗口图标存在（WM_GETICON），但验证不了
"嵌进去的那张图到底是不是同一张"。这里直接解析 PE 的 `.rsrc` 段做逐字节比对。

用法：
    python tests/check_icon.py [exe ...]
不给参数时默认校验 dist/ 下的两个产物。
"""

import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from pe_resource import load_exe_resources, parse_ico  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
ICO = ROOT / "assets" / "app.ico"
DEFAULT_EXES = [
    ROOT / "dist" / "WebClone-GUI.exe",
    ROOT / "dist" / "webclone.exe",
]

RT_ICON = 3
RT_GROUP_ICON = 14
RT_MANIFEST = 24
ICON_GROUP_ID = 1


def main() -> int:
    if not ICO.is_file():
        print(f"[FAIL] 源图标不存在：{ICO}")
        return 1

    src = parse_ico(ICO.read_bytes())
    print(f"源图标 {ICO.name}：{len(src)} 个图层 — " + ", ".join(
        f"{e['width']}x{e['height']}" for e in src))

    exes = [Path(a) for a in sys.argv[1:]] or DEFAULT_EXES
    ok = True

    for exe in exes:
        if not exe.is_file():
            print(f"\n[FAIL] {exe} 不存在")
            ok = False
            continue
        print(f"\n=== {exe.name} ({exe.stat().st_size:,} 字节) ===")
        res = load_exe_resources(exe.read_bytes())

        icons = res.get(RT_ICON, {})
        groups = res.get(RT_GROUP_ICON, {})
        manifests = res.get(RT_MANIFEST, {})

        ok &= check(f"含 RT_GROUP_ICON 资源（ID {ICON_GROUP_ID}）",
                    ICON_GROUP_ID in groups)
        ok &= check(f"含 RT_ICON 图层 {len(icons)} 个", len(icons) == len(src))
        # 只有 GUI 需要 comctl32 v6 manifest（见 crates/webclone-gui/build.rs）；
        # 控制台版没有控件，不需要也不应该带上。
        if "GUI" in exe.name:
            ok &= check("命中 comctl32 v6 manifest（RT_MANIFEST 存在）",
                        len(manifests) > 0)

        if ICON_GROUP_ID in groups:
            grp = parse_group_icon(groups[ICON_GROUP_ID])
            ok &= check(
                f"图标组声明的图层尺寸与源一致（{len(grp)} 个）",
                sorted(grp.keys()) == sorted((e["width"], e["height"]) for e in src),
            )
            # 逐字节比对：rc.exe 会把 ico 拆成独立 RT_ICON，PNG 层应当原样保留。
            mismatched = []
            for entry in src:
                size = (entry["width"], entry["height"])
                icon_id = grp.get(size)
                if icon_id is None:
                    mismatched.append(f"{size} 未在组里声明")
                elif icons.get(icon_id) != entry["data"]:
                    mismatched.append(f"{size}（ID {icon_id}）字节不一致")
            ok &= check(
                "各图层字节与源 ico 逐字节一致",
                not mismatched,
                "；".join(mismatched) if mismatched else "",
            )

    print("\nRESULT:", "ALL PASS" if ok else "HAS FAILURES")
    return 0 if ok else 1


def check(name: str, cond: bool, note: str = "") -> bool:
    print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f" — {note}" if note else ""))
    return cond


def parse_group_icon(data: bytes) -> dict:
    """解析 RT_GROUP_ICON 的 GRPICONDIR，返回 {(w,h): icon_id}。"""
    _, _, count = struct.unpack("<HHH", data[:6])
    out = {}
    for i in range(count):
        off = 6 + i * 14
        w, h, _nc, _rsv, _planes, _bpp, _size, icon_id = struct.unpack(
            "<BBBBHHIH", data[off:off + 14])
        out[(256 if w == 0 else w, 256 if h == 0 else h)] = icon_id
    return out


if __name__ == "__main__":
    sys.exit(main())
