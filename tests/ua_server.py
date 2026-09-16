#!/usr/bin/env python3
"""极简 UA 记录服务器，只为验证 --user-agent / GUI 自定义 UA 是否真的生效。

用法:
    python ua_server.py <port> <ua_log_file>

行为:
    * 对 GET / 返回一个不含任何外部资源的极小 HTML（保证一次克隆只产生一次请求）；
    * 把每次请求的 User-Agent 头按行追加写入 <ua_log_file>；
    * 一直运行到被外部终止。
"""

import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PAGE = b"<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>ua</title></head><body>ua</body></html>"

LOG_PATH = ""


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):  # noqa: N802 - BaseHTTPRequestHandler 的约定名
        ua = self.headers.get("User-Agent", "")
        with open(LOG_PATH, "a", encoding="utf-8") as fp:
            fp.write(ua + "\n")

        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(PAGE)))
        self.end_headers()
        self.wfile.write(PAGE)

    def log_message(self, *args):  # 静音
        pass


def main():
    global LOG_PATH

    if len(sys.argv) < 3:
        print("usage: ua_server.py <port> <ua_log_file>", file=sys.stderr)
        return 2

    port = int(sys.argv[1])
    LOG_PATH = sys.argv[2]

    # 每次启动清空，避免上一轮的数据混淆断言
    open(LOG_PATH, "w", encoding="utf-8").close()

    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"ua server listening on 127.0.0.1:{port} -> {LOG_PATH}", flush=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    try:
        threading.Event().wait()
    except KeyboardInterrupt:
        pass
    finally:
        server.shutdown()

    return 0


if __name__ == "__main__":
    sys.exit(main())
