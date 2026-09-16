"""PE 资源区与 .ico 的最小解析器（零依赖，纯 stdlib）。

只在测试里用：验证 build.rs 嵌进 exe 的图标资源确实是 `assets/app.ico` 那一份，
而不是"能打开就行"的模糊判断。
"""

import struct

RT_ICON = 3
RT_GROUP_ICON = 14
RT_MANIFEST = 24


def load_exe_resources(data: bytes) -> dict:
    """解析 PE，返回 `{资源类型 ID: {资源名 ID: 原始字节}}`。

    只处理数字 ID 的资源（微软工具链生成的图标/manifest 都是数字 ID）；
    字符串命名的资源直接跳过 —— 本项目用不到。
    """
    e_lfanew = struct.unpack("<I", data[0x3C:0x40])[0]
    if data[e_lfanew:e_lfanew + 4] != b"PE\0\0":
        raise ValueError("不是有效的 PE 文件")

    coff = e_lfanew + 4
    n_sections = struct.unpack("<H", data[coff + 2:coff + 4])[0]
    size_opt = struct.unpack("<H", data[coff + 16:coff + 18])[0]
    opt = coff + 20
    magic = struct.unpack("<H", data[opt:opt + 2])[0]
    # PE32+ (0x20B) 的 DataDirectory 比 PE32 (0x10B) 靠后 16 字节。
    dd_off = opt + (112 if magic == 0x20B else 96)

    res_rva, _res_size = struct.unpack("<II", data[dd_off + 2 * 8:dd_off + 2 * 8 + 8])
    if res_rva == 0:
        return {}

    sections = []
    sec = opt + size_opt
    for i in range(n_sections):
        off = sec + i * 40
        vsize, vaddr, raw_size, raw_ptr = struct.unpack("<IIII", data[off + 8:off + 24])
        sections.append((vaddr, max(vsize, raw_size), raw_ptr))

    def rva_to_off(rva: int) -> int:
        for vaddr, size, raw_ptr in sections:
            if vaddr <= rva < vaddr + size:
                return rva - vaddr + raw_ptr
        raise ValueError(f"RVA {rva:#x} 不在任何节内")

    base = rva_to_off(res_rva)
    out: dict = {}
    _walk_dir(data, base, 0, 0, None, None, out, rva_to_off)
    return out


def _walk_dir(data, base, dir_off, depth, type_id, name_id, out, rva_to_off) -> None:
    _chars, _tds, _maj, _min, named, ids = struct.unpack(
        "<IIHHHH", data[base + dir_off:base + dir_off + 16])
    for i in range(named + ids):
        eoff = base + dir_off + 16 + i * 8
        name, sub = struct.unpack("<II", data[eoff:eoff + 8])
        # 最高位为 1 表示"字符串名"（此处用不到），否则是数字 ID。
        cur_id = (name & 0x7FFFFFFF) if not (name & 0x80000000) else None
        if sub & 0x80000000:
            _walk_dir(
                data, base, sub & 0x7FFFFFFF, depth + 1,
                cur_id if depth == 0 else type_id,
                cur_id if depth == 1 else name_id,
                out, rva_to_off,
            )
        elif depth == 2:  # 语言层，entry 指向 IMAGE_RESOURCE_DATA_ENTRY
            rva, size, _cp, _rsv = struct.unpack("<IIII", data[base + sub:base + sub + 16])
            off = rva_to_off(rva)
            out.setdefault(type_id, {})[name_id] = data[off:off + size]


def parse_ico(data: bytes) -> list:
    """解析 .ico，返回 `[{width, height, bpp, size, offset, data}]`。"""
    _reserved, _type, count = struct.unpack("<HHH", data[:6])
    out = []
    for i in range(count):
        off = 6 + i * 16
        w, h, _nc, _rsv, _planes, bpp, size, offset = struct.unpack(
            "<BBBBHHII", data[off:off + 16])
        out.append({
            "width": 256 if w == 0 else w,
            "height": 256 if h == 0 else h,
            "bpp": bpp,
            "size": size,
            "offset": offset,
            "data": data[offset:offset + size],
        })
    return out
