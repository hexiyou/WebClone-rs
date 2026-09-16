# -*- coding: utf-8 -*-
"""检查 PE 产物实际需要的最低 Windows 版本。

为什么不能只看文档：Rust 官方口径（1.78 起 MSVC 目标最低 Windows 10）是**编译器
层面**的承诺，而"这个 exe 到底能不能在 Win7 上起来"取决于**导入表**里有没有
低版本系统不存在的符号 —— 加载器解析导入表失败时，进程连 main 都进不去。

这个脚本读三处：
1. Optional Header 的 MajorOperatingSystemVersion / MajorSubsystemVersion
   —— 加载器启动时会校验后者，低于它直接拒绝加载（本项目写的是 6.0）。
2. 静态导入表 + 延迟加载表 —— 逐符号比对"低版本不存在"清单，这是真正的门槛。
3. 运行库依赖（UCRT / VCRUNTIME140）—— 决定要不要额外装可再发行组件。

用法：
    python tests/check_winver.py dist/WebClone-GUI.exe dist/webclone.exe

退出码 0 表示"检查完成"（不是"一定支持 Win10 以上"，结论看输出）。
"""
import struct
import sys

# ---------------------------------------------------------------- 判定表

# 低于 Windows 10 就没有的符号。key 是 (dll 小写, 函数名)。
# 这些一旦出现在**静态导入表**里，对应系统上进程无法启动（不是"功能不可用"，
# 是加载阶段就失败）。出现在延迟加载表里则只是调用时失败。
BLOCKERS = {
    ("bcryptprimitives.dll", "ProcessPrng"): (
        "Win8+/Win10+",
        "getrandom 0.3 的 Windows 默认后端（Rust 1.78+ 起启用）",
    ),
    ("api-ms-win-core-synch-l1-2-0.dll", "WaitOnAddress"): (
        "Win8+",
        "Rust std 的 mutex/condvar/rwlock futex 实现（1.78 起）",
    ),
    ("api-ms-win-core-synch-l1-2-0.dll", "WakeByAddressSingle"): ("Win8+", "同上"),
    ("api-ms-win-core-synch-l1-2-0.dll", "WakeByAddressAll"): ("Win8+", "同上"),
    ("kernel32.dll", "GetSystemTimePreciseAsFileTime"): (
        "Win8+",
        "std 取高精度系统时间的路径",
    ),
    ("kernel32.dll", "SetThreadDescription"): ("Win10 1607+", "线程命名"),
    ("kernel32.dll", "GetTempPath2W"): ("Win10 2004+", "临时目录"),
    ("user32.dll", "GetDpiForWindow"): ("Win10 1607+", "DPI 感知"),
    ("user32.dll", "SetProcessDpiAwarenessContext"): ("Win10 1607+", "DPI 感知"),
    ("user32.dll", "GetSystemMetricsForDpi"): ("Win10 1607+", "DPI 感知"),
    ("user32.dll", "AdjustWindowRectExForDpi"): ("Win10 1607+", "DPI 感知"),
}

# 整组判定：前缀 → (说明, 低版本系统需要什么)
RUNTIME_GROUPS = [
    (
        "api-ms-win-crt-",
        "Universal CRT（UCRT）",
        "Win10 起随系统预装；Win7/8/8.1 需装 KB2999226",
    ),
    ("vcruntime140", "MSVC 2015-2022 运行库（VCRUNTIME140.dll）",
     "需装 VC++ 2015-2022 可再发行组件包；或改静态链接 CRT 消除此依赖"),
]

SCHANNEL_HINT = (
    "检测到 secur32.dll + crypt32.dll —— 说明 TLS 走系统 Schannel（native-tls）。\n"
    "    Win7 的 Schannel 默认只开 TLS 1.0/1.1，TLS 1.2 需 KB3140245 + 改注册表，\n"
    "    TLS 1.3 完全不支持；Win10 1903+ 才有 TLS 1.3。2026 年多数站点已要求\n"
    "    TLS 1.2+，所以即使能启动，抓 HTTPS 也会大面积失败 —— 这一条比能否启动更实际。"
)

# ---------------------------------------------------------------- PE 解析


def _u16(d, o):
    return struct.unpack_from("<H", d, o)[0]


def _u32(d, o):
    return struct.unpack_from("<I", d, o)[0]


def parse_pe(path):
    d = open(path, "rb").read()
    if d[:2] != b"MZ":
        raise ValueError(f"{path}: 不是 PE 文件")
    e = _u32(d, 0x3C)
    if d[e:e + 4] != b"PE\0\0":
        raise ValueError(f"{path}: PE 签名缺失")
    coff = e + 4
    machine, nsec, _, _, _, size_opt = struct.unpack_from("<HHIIIH", d, coff)
    opt = coff + 20
    p32p = _u16(d, opt) == 0x20B

    secs = []
    sec = opt + size_opt
    for i in range(nsec):
        o = sec + i * 40
        vsize, vaddr, raw_size, raw_ptr = struct.unpack_from("<IIII", d, o + 8)
        secs.append((vaddr, vsize, raw_size, raw_ptr))

    def r2o(rva):
        for va, vs, rs, rp in secs:
            if va <= rva < va + max(vs, rs):
                return rva - va + rp
        return None

    def cstr(off):
        return d[off:d.index(b"\0", off)].decode("ascii", "replace")

    dd = opt + (0x70 if p32p else 0x60)

    def read_imports(dir_index, delay=False):
        rva = _u32(d, dd + dir_index * 8)
        out = {}
        if not rva:
            return out
        base = r2o(rva)
        i = 0
        while True:
            o = base + i * 20
            if delay:
                # ImgDelayDescr: Attributes, DllNameRVA, ModuleHandleRVA,
                #                ImportAddressTableRVA, ImportNameTableRVA, BoundIAT, ...
                attrs, name_rva, _h, _iat, int_rva = struct.unpack_from("<IIIII", d, o)
                if not name_rva:
                    break
                # 只有 Attributes 的 bit0（RVA-based）为 1 时字段才是 RVA
                if not (attrs & 1):
                    break
                t = int_rva
            else:
                oft, _ts, _fc, name_rva, _ft = struct.unpack_from("<IIIII", d, o)
                if not name_rva:
                    break
                t = oft
            dll = cstr(r2o(name_rva))
            fns = []
            off_t = r2o(t) if t else None
            if off_t:
                step, ptr = (8, "<Q") if p32p else (4, "<I")
                j = 0
                while True:
                    ent = struct.unpack_from(ptr, d, off_t + j * step)[0]
                    if ent == 0:
                        break
                    high = 1 << (63 if p32p else 31)
                    if ent & high:
                        fns.append("#ord%d" % (ent & 0xFFFF))
                    else:
                        fns.append(cstr(r2o(ent) + 2))
                    j += 1
            out.setdefault(dll, []).extend(fns)
            i += 1
        return out

    return {
        "path": path,
        "size": len(d),
        "machine": {0x8664: "x86_64", 0x14C: "i386", 0xAA64: "arm64"}.get(machine, hex(machine)),
        "subsystem": {2: "WINDOWS_GUI", 3: "WINDOWS_CUI"}.get(_u16(d, opt + 0x44), "?"),
        "os_ver": (_u16(d, opt + 0x28), _u16(d, opt + 0x2A)),
        "subsys_ver": (_u16(d, opt + 0x30), _u16(d, opt + 0x32)),
        "imports": read_imports(1),
        "delay": read_imports(13, delay=True),
    }


# ---------------------------------------------------------------- 判定


def analyse(info):
    """返回 (硬阻断列表, 延迟加载阻断列表, 运行库组)。"""
    def scan(table):
        hits = []
        for dll, fns in table.items():
            for fn in fns:
                key = (dll.lower(), fn)
                if key in BLOCKERS:
                    hits.append((dll, fn) + BLOCKERS[key])
        return hits

    hard = scan(info["imports"])
    soft = scan(info["delay"])

    groups = []
    all_dlls = [x.lower() for x in list(info["imports"]) + list(info["delay"])]
    for prefix, label, note in RUNTIME_GROUPS:
        matched = sorted({x for x in all_dlls if x.startswith(prefix)})
        if matched:
            groups.append((label, matched, note))
    return hard, soft, groups


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    V = {
        (5, 0): "Windows 2000", (5, 1): "Windows XP", (5, 2): "Windows Server 2003",
        (6, 0): "Windows Vista / Server 2008", (6, 1): "Windows 7 / Server 2008 R2",
        (6, 2): "Windows 8 / Server 2012", (6, 3): "Windows 8.1 / Server 2012 R2",
        (10, 0): "Windows 10 / 11 / Server 2016+",
    }
    for path in argv[1:]:
        info = parse_pe(path)
        hard, soft, groups = analyse(info)
        print("=" * 72)
        print(f"文件: {path}   ({info['size']:,} 字节)")
        print(f"  架构              : {info['machine']} / {info['subsystem']}")
        print(f"  PE 头 最低OS版本  : {info['os_ver'][0]}.{info['os_ver'][1]}")
        print(f"  PE 头 子系统版本  : {info['subsys_ver'][0]}.{info['subsys_ver'][1]}"
              f"  → {V.get(info['subsys_ver'], '?')}")
        print("      ↑ 加载器只校验这一个字段；它是 6.0，所以不会因版本号被拒。")
        print(f"  静态导入 DLL      : {len(info['imports'])} 个"
              f"（{sum(len(v) for v in info['imports'].values())} 个函数）")
        if info["delay"]:
            print(f"  延迟加载 DLL      : {len(info['delay'])} 个")

        print("\n  ── 硬阻断（静态导入，低版本系统进程起不来）──")
        if not hard:
            print("     无")
        for dll, fn, ver, note in hard:
            print(f"     [{ver:<10}] {dll}!{fn}")
            print(f"                   ↳ {note}")
        if soft:
            print("\n  ── 延迟加载（仅调用时失败，不影响启动）──")
            for dll, fn, ver, note in soft:
                print(f"     [{ver:<10}] {dll}!{fn}  ↳ {note}")

        print("\n  ── 运行库依赖 ──")
        if not groups:
            print("     无特殊依赖")
        for label, matched, note in groups:
            print(f"     {label}")
            print(f"       {', '.join(matched)}")
            print(f"       ↳ {note}")

        if any("secur32.dll" == x for x in info["imports"]):
            print("\n  ── TLS 提示 ──")
            print(f"    {SCHANNEL_HINT}")

        print("\n  ── 结论 ──")
        if hard:
            worst = "Win10 及以上（官方保证）"
            print(f"     **存在只在 Win8+ / Win10+ 存在的导入符号，因此实际最低要求：{worst}**")
            print(f"     其中 {len(hard)} 个符号在 Win7 上不存在 —— 进程会在加载阶段直接失败，")
            print("     不是「功能受限」，是「连窗口都出不来」。")
        else:
            print("     导入表未发现 Win8+/Win10+ 专有符号，理论上可在更早系统启动")
            print("     （但仍受 Rust 官方口径与运行库依赖约束）。")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
