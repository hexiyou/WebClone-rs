# -*- coding: utf-8 -*-
"""
克隆工具的功能验证服务器。

Python 自带的 SimpleHTTPRequestHandler 不支持 Range 请求，也不发 ETag，
没法验证断点续传和增量跳过，所以这里自己实现：

  - If-None-Match / If-Modified-Since -> 304
  - Range + If-Range                  -> 206（If-Range 的 ETag 不匹配时降级为 200）
  - 常规请求                           -> 200 + ETag + Last-Modified + Accept-Ranges

用法：python test_server.py <端口> <站点根目录>
"""
import email.utils
import hashlib
import http.server
import os
import re
import socketserver
import sys


class CloneTestHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "CloneTest/1.0"

    def log_message(self, fmt, *args):
        # 默认会把每个请求打到 stderr，测试时太吵，安静一点
        pass

    def do_HEAD(self):
        self._serve(body=False)

    def do_GET(self):
        self._serve(body=True)

    def _resolve(self):
        path = self.path.split("?", 1)[0].split("#", 1)[0]
        path = path.lstrip("/")
        root = self.server.site_root
        full = os.path.normpath(os.path.join(root, path))
        # 防目录穿越
        if not full.startswith(os.path.abspath(root)):
            return None
        if os.path.isdir(full):
            full = os.path.join(full, "index.html")
        return full

    def _serve(self, body: bool) -> None:
        full = self._resolve()
        if full is None or not os.path.isfile(full):
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return

        stat = os.stat(full)
        size = stat.st_size
        mtime = email.utils.formatdate(stat.st_mtime, usegmt=True)
        etag = '"%s"' % hashlib.md5(
            ("%d-%d-%s" % (size, int(stat.st_mtime), full)).encode("utf-8")
        ).hexdigest()

        # 条件请求：ETag 优先
        inm = self.headers.get("If-None-Match")
        if inm and inm.strip() == etag:
            self._send_status(304, etag, mtime, size, None, body=False)
            return

        ims = self.headers.get("If-Modified-Since")
        if ims and not inm:
            try:
                parsed = email.utils.parsedate_to_datetime(ims).timestamp()
                if int(stat.st_mtime) <= int(parsed):
                    self._send_status(304, etag, mtime, size, None, body=False)
                    return
            except Exception:
                pass

        start, end, partial = 0, size - 1, False

        range_header = self.headers.get("Range")
        if range_header:
            match = re.match(r"bytes=(\d*)-(\d*)", range_header.strip())
            if match:
                if_range = self.headers.get("If-Range")
                # If-Range 与当前 ETag 不一致说明资源已变，直接降级为全量
                range_usable = not if_range or if_range.strip() == etag
                if range_usable:
                    if match.group(1):
                        start = int(match.group(1))
                    if match.group(2):
                        end = min(int(match.group(2)), size - 1)
                    if start < size and start <= end:
                        partial = True

        content_type = self._guess_type(full)
        self._send_status(
            206 if partial else 200, etag, mtime, size,
            (start, end) if partial else None,
            body=body,
            content_type=content_type,
        )

        if not body:
            return

        with open(full, "rb") as handle:
            handle.seek(start)
            remaining = (end - start + 1) if partial else size
            while remaining > 0:
                chunk = handle.read(min(65536, remaining))
                if not chunk:
                    break
                self.wfile.write(chunk)
                remaining -= len(chunk)

    def _send_status(self, status, etag, mtime, size, byte_range, body, content_type="application/octet-stream"):
        self.send_response(status)
        self.send_header("ETag", etag)
        self.send_header("Last-Modified", mtime)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Type", content_type)

        if status == 304:
            self.send_header("Content-Length", "0")
            self.end_headers()
            return

        if byte_range is not None:
            start, end = byte_range
            self.send_header("Content-Range", "bytes %d-%d/%d" % (start, end, size))
            self.send_header("Content-Length", str(end - start + 1))
        else:
            self.send_header("Content-Length", str(size))

        self.end_headers()

    @staticmethod
    def _guess_type(path: str) -> str:
        ext = os.path.splitext(path)[1].lower()
        return {
            ".html": "text/html",
            ".htm": "text/html",
            ".css": "text/css",
            ".js": "application/javascript",
            ".png": "image/png",
            ".jpg": "image/jpeg",
            ".jpeg": "image/jpeg",
            ".gif": "image/gif",
            ".webp": "image/webp",
            ".svg": "image/svg+xml",
            ".ico": "image/x-icon",
            ".woff2": "font/woff2",
            ".woff": "font/woff",
            ".ttf": "font/ttf",
            ".mp4": "video/mp4",
            ".json": "application/json",
        }.get(ext, "application/octet-stream")


class ThreadedServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True
    site_root = "."


def main() -> int:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    site_root = os.path.abspath(sys.argv[2]) if len(sys.argv) > 2 else os.getcwd()

    ThreadedServer.site_root = site_root
    server = ThreadedServer(("127.0.0.1", port), CloneTestHandler)
    print("测试服务器已启动：http://127.0.0.1:%d  根目录：%s" % (port, site_root))
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
