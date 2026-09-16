"""对新旧两个 exe 分别启动 -> 等窗口 -> 截图 -> 关闭，用于观感对比。

只做"截一张主界面图"这一件事，不带断言；断言仍由 tests/gui_probe.py 负责。
"""
import ctypes
import ctypes.wintypes as wt
import sys
import time

from PIL import ImageGrab

u32 = ctypes.windll.user32
u32.SetProcessDPIAware()

TITLE = "网页克隆工具 WebClone"
WM_CLOSE = 0x0010
SW_RESTORE = 9

WNDENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def find_main():
    found = []

    def cb(h, _l):
        buf = ctypes.create_unicode_buffer(512)
        u32.GetWindowTextW(h, buf, 512)
        if buf.value == TITLE and u32.IsWindowVisible(h):
            found.append(h)
        return True

    u32.EnumWindows(WNDENUMPROC(cb), 0)
    return found[0] if found else None


def main():
    out = sys.argv[1]
    hwnd = None
    for _ in range(40):
        hwnd = find_main()
        if hwnd:
            break
        time.sleep(0.25)
    if not hwnd:
        print(f"{out}: FAIL 未找到主窗口")
        return 1

    u32.ShowWindow(hwnd, SW_RESTORE)
    u32.SetForegroundWindow(hwnd)
    time.sleep(0.8)

    r = wt.RECT()
    u32.GetWindowRect(hwnd, ctypes.byref(r))
    w, h = r.right - r.left, r.bottom - r.top
    ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom)).save(out)
    print(f"{out}: OK  {w}x{h}px")
    return 0


if __name__ == "__main__":
    sys.exit(main())
