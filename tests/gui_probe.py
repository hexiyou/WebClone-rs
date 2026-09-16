"""GUI 冒烟验证：找窗口 -> 截图 -> 点复选框 -> 切页签 -> 点关闭 -> 再截图。

纯 ctypes + PIL，不依赖 Add-Type（本机被安全策略拦）。
"""
import ctypes
import ctypes.wintypes as wt
import sys
import time
from PIL import ImageChops, ImageGrab

u32 = ctypes.windll.user32
u32.SetProcessDPIAware()

k32 = ctypes.windll.kernel32
# 这几个必须显式声明：默认把返回值当 32 位 int，HANDLE 会被截断成负数，
# 再传回 WaitForSingleObject 就找不到对象了。
k32.OpenProcess.restype = wt.HANDLE
k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
k32.WaitForSingleObject.restype = wt.DWORD
k32.WaitForSingleObject.argtypes = [wt.HANDLE, wt.DWORD]
k32.CloseHandle.restype = wt.BOOL
k32.CloseHandle.argtypes = [wt.HANDLE]

TITLE = "网页克隆工具 WebClone"
TCM_SETCURSEL = 0x130C
SW_RESTORE = 9
WM_LBUTTONDOWN = 0x0201
WM_LBUTTONUP = 0x0202
WM_CLOSE = 0x0010
WM_KEYDOWN = 0x0100
WM_SETTEXT = 0x000C
VK_RETURN = 0x0D
BM_CLICK = 0x00F5
BM_GETCHECK = 0x00F0
WM_GETTEXT = 0x000D
WM_GETTEXTLENGTH = 0x000E
SYNCHRONIZE = 0x00100000
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
WAIT_OBJECT_0 = 0
SWP_NOSIZE = 0x0001
SWP_NOZORDER = 0x0004
SWP_NOACTIVATE = 0x0010

EnumProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def find_window():
    for _ in range(40):
        hwnd = u32.FindWindowW(None, TITLE)
        if hwnd:
            return hwnd
        time.sleep(0.25)
    return 0


def children(root):
    out = []

    def cb(h, _):
        cls = ctypes.create_unicode_buffer(256)
        u32.GetClassNameW(h, cls, 256)
        n = u32.SendMessageW(h, WM_GETTEXTLENGTH, 0, 0)
        buf = ctypes.create_unicode_buffer(n + 1)
        u32.SendMessageW(h, WM_GETTEXT, n + 1, buf)
        r = wt.RECT()
        u32.GetWindowRect(h, ctypes.byref(r))
        out.append(
            {
                "hwnd": h,
                "cls": cls.value,
                "text": buf.value,
                "rect": (r.left, r.top, r.right, r.bottom),
                "enabled": bool(u32.IsWindowEnabled(h)),
                "visible": bool(u32.IsWindowVisible(h)),
                "checked": u32.SendMessageW(h, BM_GETCHECK, 0, 0),
            }
        )
        return True

    u32.EnumChildWindows(root, EnumProc(cb), 0)
    return out


def find(kids, cls, needle, visible=True):
    for k in kids:
        if k["cls"] == cls and needle in k["text"] and (not visible or k["visible"]):
            return k
    return None


def find_exact(kids, cls, text, visible=True):
    """按文本精确匹配。'HTTP' 这种要小心 —— 子串匹配会撞上'忽略 HTTPS 证书错误'。"""
    for k in kids:
        if k["cls"] == cls and k["text"] == text and (not visible or k["visible"]):
            return k
    return None


def click_tab(tab_hwnd, off, y=12):
    """点页签条：y 取客户区 12（标签行中线），off 是客户区横坐标。"""
    lp = (y << 16) | (off & 0xFFFF)
    u32.SendMessageW(tab_hwnd, WM_LBUTTONDOWN, 0x0001, lp)
    u32.SendMessageW(tab_hwnd, WM_LBUTTONUP, 0, lp)
    time.sleep(0.45)


# --------------------------------------------------------------- 真实按键
# 验证"网址框回车开始克隆"只能用 SendInput 真实按键，不能用 PostMessage：
# PostMessage 的键盘消息直接进窗口队列，绕开了 winsafe 消息循环里的
# IsDialogMessage —— 那个坑恰好就藏在那一步里，绕过去就测不出来。


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wt.WORD),
        ("wScan", wt.WORD),
        ("dwFlags", wt.DWORD),
        ("time", wt.DWORD),
        ("dwExtraInfo", ctypes.c_void_p),
    ]


class _INPUTUNION(ctypes.Union):
    # 必须凑够 32 字节（x64 下最大成员是 MOUSEINPUT）且保持 8 字节对齐，
    # 否则 sizeof(INPUT) 会是 32 而不是 40，SendInput 直接返回 0、一个事件都不发。
    _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_uint64 * 4)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wt.DWORD), ("u", _INPUTUNION)]


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [
        ("cbSize", wt.DWORD),
        ("flags", wt.DWORD),
        ("hwndActive", wt.HWND),
        ("hwndFocus", wt.HWND),
        ("hwndCapture", wt.HWND),
        ("hwndMenuOwner", wt.HWND),
        ("hwndMoveSize", wt.HWND),
        ("hwndCaret", wt.HWND),
        ("rcCaret", wt.RECT),
    ]


u32.SendInput.restype = wt.UINT
u32.SendInput.argtypes = [wt.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
u32.GetWindowThreadProcessId.restype = wt.DWORD
u32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.c_void_p]
u32.GetGUIThreadInfo.restype = wt.BOOL
u32.GetGUIThreadInfo.argtypes = [wt.DWORD, ctypes.c_void_p]
u32.GetForegroundWindow.restype = wt.HWND
u32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
u32.mouse_event.argtypes = [wt.DWORD, wt.DWORD, wt.DWORD, wt.DWORD, ctypes.c_void_p]

INPUT_KEYBOARD = 1
KEYEVENTF_KEYUP = 0x0002
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004


def send_enter():
    arr = (INPUT * 2)()
    arr[0].type = INPUT_KEYBOARD
    arr[0].u.ki = KEYBDINPUT(VK_RETURN, 0, 0, 0, None)
    arr[1].type = INPUT_KEYBOARD
    arr[1].u.ki = KEYBDINPUT(VK_RETURN, 0, KEYEVENTF_KEYUP, 0, None)
    return u32.SendInput(2, arr, ctypes.sizeof(INPUT))


def focus_of_thread(thread_id):
    gti = GUITHREADINFO()
    gti.cbSize = ctypes.sizeof(GUITHREADINFO)
    if u32.GetGUIThreadInfo(thread_id, ctypes.byref(gti)):
        return gti.hwndFocus
    return 0


def click_window(hwnd):
    """把鼠标移到窗口中心点一下（第一下激活前台，第二下才落到控件上）。"""
    r = wt.RECT()
    u32.GetWindowRect(hwnd, ctypes.byref(r))
    cx, cy = (r.left + r.right) // 2, (r.top + r.bottom) // 2
    for _ in range(2):
        u32.SetCursorPos(cx, cy)
        time.sleep(0.2)
        u32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, None)
        time.sleep(0.08)
        u32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, None)
        time.sleep(0.4)


def shot(hwnd, path, attempts=4):
    u32.ShowWindow(hwnd, SW_RESTORE)  # 最小化状态下 GetWindowRect 会给出屏幕外坐标
    time.sleep(0.3)
    r = wt.RECT()
    u32.GetWindowRect(hwnd, ctypes.byref(r))
    if r.left <= -30000 or r.right <= r.left:
        print(f"shot {path}: SKIP（窗口已最小化，bbox 落在屏幕外）")
        return None
    u32.SetForegroundWindow(hwnd)
    time.sleep(0.35)
    # ImageGrab.grab 偶发抛 OSError("screen grab failed")：多半是抓的瞬间桌面
    # 正在切换（前台窗口刚变更 / 屏保唤醒）。退避重试几次即可，不必惊动断言。
    last = None
    for i in range(attempts):
        try:
            img = ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom))
            img.save(path)
            print(f"shot {path}: {img.size[0]}x{img.size[1]}" + (f"（重试 {i} 次）" if i else ""))
            return img
        except Exception as err:  # 截屏失败不该中断断言流程
            last = err
            time.sleep(0.6)
    print(f"shot {path}: FAILED {last}")
    return None


def diff_pct(a, b):
    """两张截图的差异像素占比（忽略 0~7 的抗锯齿抖动）。"""
    if a is None or b is None:
        return None
    if a.size != b.size:
        return None
    d = ImageChops.difference(a.convert("RGB"), b.convert("RGB"))
    hist = d.convert("L").histogram()
    return sum(hist[8:]) * 100.0 / (a.size[0] * a.size[1])


def main():
    hwnd = find_window()
    if not hwnd:
        print("FAIL: window not found")
        return 1
    print(f"window hwnd={hwnd}")

    # 固定窗口位置：后面"切页重绘"的断言要拿初始截图当像素基准，
    # 窗口位置一动，两次抓到的 bbox 就对不上。
    u32.SetWindowPos(hwnd, None, 40, 40, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)
    time.sleep(0.4)

    kids = children(hwnd)
    print(f"--- children: {len(kids)} ---")
    for k in kids:
        print(
            f"{k['cls']:<24} en={int(k['enabled'])} vis={int(k['visible'])} "
            f"chk={k['checked']} {k['rect']} :: {k['text'][:58]!r}"
        )

    img_initial = shot(hwnd, "shot-1-clone.png")

    # ---------- 断言 1：默认状态 ----------
    ua_box = find(kids, "Button", "自定义浏览器 UA")
    ua_edit = find(kids, "Edit", "Mozilla/5.0")
    out_edit = find(kids, "Edit", "webclone-output")
    stop_btn = find(kids, "Button", "停止")
    ok = True

    def check(label, cond):
        nonlocal ok
        ok = ok and cond
        print(f"[{'PASS' if cond else 'FAIL'}] {label}")

    check("复选框默认未勾选", ua_box and ua_box["checked"] == 0)
    check("UA 文本框默认失能", ua_edit and not ua_edit["enabled"])
    check("UA 文本框默认值 = Edge UA", ua_edit and "Edg/" in ua_edit["text"])
    check("保存位置已回显桌面路径", out_edit is not None)
    check("停止按钮默认失能", stop_btn and not stop_btn["enabled"])
    check("端口无千分位", not any("8,080" in k["text"] for k in kids))

    # 对齐 C# XAML 的初始勾选状态
    for text, want in [
        ("下载跨域资源（CDN）", 1),
        ("改写为相对路径", 1),
        ("增量下载（未变更的跳过）", 1),
        ("单页模式下禁用外站链接", 1),
        ("忽略 HTTPS 证书错误", 0),
        ("单页模板", 1),
        ("整站（递归追踪站内链接）", 0),
        ("直连", 1),
        ("HTTP", 0),
        ("SOCKS5", 0),
    ]:
        k = find(kids, "Button", text)
        check(f"{text} 初始勾选={want}", k is not None and k["checked"] == want)

    # 微调按钮必须 17px 宽且紧贴伙伴框，没有被 UDS_ALIGNRIGHT 压坏
    spins = [k for k in kids if k["cls"] == "msctls_updown32" and k["visible"]]
    widths = sorted({k["rect"][2] - k["rect"][0] for k in spins})
    check(f"微调按钮宽度一致且非 0 {widths}", widths == [17])

    # 六个数字框的默认值必须与代码里的默认一致。
    # 这条是补的：曾经并发框显示成 13（残留实例退出时回写了 webclone-gui.json，
    # 把本轮本该走默认值的初始化带偏），单看界面根本不会注意，肉眼扫一遍就过去了。
    nums = sorted(
        (
            k
            for k in kids
            if k["cls"] == "Edit" and k["text"].strip().lstrip("-").isdigit()
        ),
        key=lambda k: (k["rect"][1] // 10, k["rect"][0]),
    )
    got = [k["text"] for k in nums]
    check(f"数字框默认值 = ['3','0','8080','8','30','3']，实际 {got}",
          got == ["3", "0", "8080", "8", "30", "3"])

    # ---------- 断言 2：勾选后 UA 框可编辑 ----------
    if ua_box:
        u32.SendMessageW(ua_box["hwnd"], BM_CLICK, 0, 0)
    time.sleep(0.5)
    kids2 = children(hwnd)
    ua_box2 = find(kids2, "Button", "自定义浏览器 UA")
    ua_edit2 = find(kids2, "Edit", "Mozilla/5.0")
    check("勾选后复选框选中", ua_box2 and ua_box2["checked"] == 1)
    check("勾选后 UA 文本框可编辑", ua_edit2 and ua_edit2["enabled"])
    shot(hwnd, "shot-2-ua-checked.png")

    # 复原（取消勾选）
    if ua_box2:
        u32.SendMessageW(ua_box2["hwnd"], BM_CLICK, 0, 0)
    time.sleep(0.4)
    kids3 = children(hwnd)
    ua_edit3 = find(kids3, "Edit", "Mozilla/5.0")
    check("取消勾选后 UA 文本框重新失能", ua_edit3 and not ua_edit3["enabled"])

    # ---------- 断言 3：关于页 ----------
    # 不能用 TCM_SETCURSEL：它只改选中项、不发 TCN_SELCHANGE，页签页不会显示。
    # 也不能用 TCM_GETITEMRECT：它不在系统跨进程封送的消息表里，传过去的 RECT
    # 指针在目标进程里无效，返回的是全零矩形。改成用鼠标消息扫页签条，跟人工点
    # 一下等价，点到哪个页签由"关于页是否显示"来自证。
    tab = None
    for k in kids:
        if k["cls"] == "SysTabControl32":
            tab = k
            break
    clicked_off = None
    if tab:
        for off in (5, 20, 35, 50, 65, 80, 95, 110, 130):
            lp = (12 << 16) | (off & 0xFFFF)  # 页签条：客户区 y≈12
            u32.SendMessageW(tab["hwnd"], WM_LBUTTONDOWN, 0x0001, lp)
            u32.SendMessageW(tab["hwnd"], WM_LBUTTONUP, 0, lp)
            time.sleep(0.35)
            if find(children(hwnd), "Edit", "WebClone 网页克隆工具"):
                clicked_off = off
                break
        print(f"tab strip probe hit at client x={clicked_off}")
    time.sleep(0.4)
    kids4 = children(hwnd)
    about = find(kids4, "Edit", "WebClone 网页克隆工具")
    check("关于页文本已渲染且可见", about is not None)
    clone_url = find(kids4, "Edit", "webclone-output")
    check("切页后克隆页控件已隐藏", clone_url is None)
    shot(hwnd, "shot-3-about.png")

    # ---------- 断言 4：切回克隆页不能有重影 ----------
    # 这个 bug 的成因：两个页签页是完全重叠的兄弟窗口，而 BS_GROUPBOX 是"空心"
    # 控件（只画边框、不填中间），父页面又带 WS_CLIPCHILDREN 会跳过子窗口矩形 ——
    # 分组框中间那块像素没有任何窗口负责擦。切页时"关于"页盖过来又走开，旧像素
    # 就留在原地，和克隆页叠成重影。
    # 判据：切回克隆页后的截图必须和最初的克隆页截图逐像素一致（阈值 1%，
    # 留出插入符闪烁之类的抖动）。修好前这里是 20.89%。
    if tab:
        click_tab(tab["hwnd"], 12)  # 点回最左边的"克隆"
        time.sleep(0.5)
        kids5 = children(hwnd)
        about_back = find(kids5, "Edit", "WebClone 网页克隆工具")
        check("切回克隆页后关于页文本已隐藏", about_back is None)
        img_back = shot(hwnd, "shot-4-clone-back.png")
        pct = diff_pct(img_initial, img_back)
        if pct is None:
            # 抓图失败 / 尺寸不一致 → 判定为"测不准"，而不是"有重影"。
            # 两种原因的排查方向完全不同，报错文案必须区分开。
            check("切回克隆页与初始画面一致（无重影）", False)
            print("  ↳ 备注：两张截图有一张没抓到（或尺寸不同），无法比对像素；"
                  "这不是重影，是抓图问题，看上面的 shot 行。")
        else:
            check(
                f"切回克隆页与初始画面一致（无重影），差异 {pct:.2f}%",
                pct < 1.0,
            )

    # ---------- 断言 5：代理端口框的启用/禁用跟随单选 ----------
    # "直连"时端口框必须整条禁用。这里踩过的坑：端口是"输入框 + 上下箭头"
    # 两个窗口，只禁用了 msctls_updown32，输入框依旧能点进去选中文字。
    port_edit = find_exact(kids, "Edit", "8080")
    port_spin = None
    if port_edit:
        # 同一行的微调按钮（同行只有端口框这一组）
        port_spin = next(
            (
                k
                for k in kids
                if k["cls"] == "msctls_updown32"
                and abs(k["rect"][1] - port_edit["rect"][1]) < 3
            ),
            None,
        )
    # 代理地址框：与端口框同一行、且在它左边的那个 Edit
    host_edit = None
    if port_edit:
        same_row = [
            k
            for k in kids
            if k["cls"] == "Edit"
            and k["visible"]
            and abs(k["rect"][1] - port_edit["rect"][1]) < 3
            and k["rect"][0] < port_edit["rect"][0]
        ]
        same_row.sort(key=lambda k: k["rect"][0])
        host_edit = same_row[0] if same_row else None

    for label, want in [("直连", False), ("HTTP", True), ("SOCKS5", True), ("直连", False)]:
        btn = find_exact(children(hwnd), "Button", label)
        if not btn:
            check(f"找到代理单选 {label}", False)
            continue
        u32.SendMessageW(btn["hwnd"], BM_CLICK, 0, 0)
        time.sleep(0.5)
        cur = children(hwnd)
        pe = next((k for k in cur if k["hwnd"] == (port_edit or {}).get("hwnd")), None)
        ps = next((k for k in cur if k["hwnd"] == (port_spin or {}).get("hwnd")), None)
        he = next((k for k in cur if k["hwnd"] == (host_edit or {}).get("hwnd")), None)
        check(f"代理={label} 时端口输入框启用={want}", pe is not None and pe["enabled"] == want)
        check(f"代理={label} 时端口微调启用={want}", ps is not None and ps["enabled"] == want)
        check(f"代理={label} 时代理地址框启用={want}", he is not None and he["enabled"] == want)

    # ---------- 断言 6：网址框回车真的能触发克隆 ----------
    # 这条守两个坑：
    #  1. 子类化装错时机：winsafe 的窗口在消息循环启动后才真正 CreateWindowEx，
    #     若在 events() 里就 SetWindowSubclass，拿到的是空句柄、安装静默失败。
    #  2. 就算装对了，winsafe 消息循环里的 IsDialogMessage 会把 VK_RETURN 的
    #     WM_KEYDOWN 当成"激活默认按钮"来处理；本窗口没有默认按钮，那条消息就被
    #     直接丢弃 —— 窗口过程只能收到 WM_KEYUP。必须在 WM_GETDLGCODE 里对这条
    #     消息回 DLGC_WANTMESSAGE 才抢得回来。
    # 用 SendInput 真实按键，不走 PostMessage（后者绕开 IsDialogMessage，测不出坑 2）。
    url_edit = min(
        (k for k in children(hwnd) if k["cls"] == "Edit" and k["visible"]),
        key=lambda k: (k["rect"][1], k["rect"][0]),
        default=None,
    )
    if url_edit:
        u32.SendMessageW(url_edit["hwnd"], WM_SETTEXT, 0, ctypes.c_wchar_p(""))
        time.sleep(0.2)
        u32.SetForegroundWindow(hwnd)
        time.sleep(0.3)
        click_window(url_edit["hwnd"])
        tid = u32.GetWindowThreadProcessId(hwnd, None)
        check(
            "点击后焦点在网址框上（真实按键的前提）",
            focus_of_thread(tid) == url_edit["hwnd"],
        )
        n = send_enter()
        check("SendInput 发出按键事件", n == 2)
        box = 0
        for _ in range(15):
            box = u32.FindWindowW("#32770", "提示")
            if box:
                break
            time.sleep(0.2)
        check("网址框回车能触发校验（弹出提示框）", bool(box))
        if box:
            u32.PostMessageW(box, WM_CLOSE, 0, 0)
            time.sleep(0.5)
    else:
        check("找到网址输入框", False)

    # ---------- 断言 4：点关闭按钮必须真的退出进程 ----------
    # 「点 X 没反应、只能强杀」这个 bug 就是这条断言要守的。winsafe 的 on() 通道
    # 是"后注册者胜"，一旦我们注册了 wm_close 就会把内建的 DestroyWindow 顶掉，
    # WM_CLOSE 被吃掉 -> 窗口不销毁 -> GetMessage 永远等不到 WM_QUIT -> 消息循环卡死。
    # 这里发 WM_CLOSE 而不是模拟点标题栏：winsafe 自己就用它定义"关窗"
    #（WindowMain::close 注释：just like if the user clicked the window X button）。
    pid = wt.DWORD()
    u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    print(f"pid={pid.value}")

    u32.PostMessageW(hwnd, WM_CLOSE, 0, 0)
    time.sleep(1.5)

    check("WM_CLOSE 后主窗口已销毁", not u32.IsWindow(hwnd))

    h = k32.OpenProcess(
        SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, False, pid.value
    )
    if h:
        rc = k32.WaitForSingleObject(h, 5000)
        k32.CloseHandle(h)
        check("进程已正常退出（非强杀）", rc == WAIT_OBJECT_0)
    else:
        # 进程已经不存在了，OpenProcess 会失败（ERROR_INVALID_PARAMETER=87）。
        print(f"OpenProcess err={ctypes.get_last_error() or k32.GetLastError()}")
        check("进程已正常退出（非强杀）", True)

    print("RESULT:", "ALL PASS" if ok else "HAS FAILURES")
    return 0


if __name__ == "__main__":
    sys.exit(main())
