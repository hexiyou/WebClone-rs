# -*- coding: utf-8 -*-
"""
极简双模代理，用于验证克隆工具的代理功能。

  python test_proxy.py http 8899    # HTTP 代理（支持绝对 URI 的 GET 与 CONNECT 隧道）
  python test_proxy.py socks5 8899  # SOCKS5 代理（RFC1928，无认证）
"""
import http.server
import select
import socket
import socketserver
import sys
from socketserver import ThreadingTCPServer
from urllib.parse import urlsplit

RECV_TIMEOUT = 20


def relay(a: socket.socket, b: socket.socket) -> None:
    """双向转发，直到任一端关闭或超时。"""
    a.settimeout(RECV_TIMEOUT)
    b.settimeout(RECV_TIMEOUT)
    pair = [(a, b), (b, a)]
    try:
        while True:
            readable, _, _ = select.select([a, b], [], [], RECV_TIMEOUT)
            if not readable:
                return
            for src in readable:
                dst = b if src is a else a
                data = src.recv(65536)
                if not data:
                    return
                dst.sendall(data)
    except OSError:
        pass


def recvn(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("对端在握手过程中关闭了连接")
        buf += chunk
    return buf


# ---------------- HTTP 代理 ----------------

class HttpProxyHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def _hop_headers(self):
        skip = {"proxy-connection", "connection", "keep-alive", "proxy-authorization",
                "proxy-authenticate", "te", "trailers", "transfer-encoding", "upgrade"}
        for key, value in self.headers.items():
            if key.lower() not in skip:
                yield key, value

    def do_GET(self):
        parts = urlsplit(self.path)
        if not parts.hostname:
            self.send_error(400, "需要绝对 URI 形式的请求")
            return

        port = parts.port or 80
        path = parts.path or "/"
        if parts.query:
            path += "?" + parts.query

        try:
            target = socket.create_connection((parts.hostname, port), RECV_TIMEOUT)
        except OSError as exc:
            self.send_error(502, "无法连接上游：%s" % exc)
            return

        try:
            lines = ["GET %s HTTP/1.1" % path]
            for key, value in self._hop_headers():
                lines.append("%s: %s" % (key, value))
            lines.append("Connection: close")
            target.sendall(("\r\n".join(lines) + "\r\n\r\n").encode("utf-8"))

            self.connection.settimeout(RECV_TIMEOUT)
            while True:
                data = target.recv(65536)
                if not data:
                    break
                self.wfile.write(data)
        except OSError:
            pass
        finally:
            target.close()
        self.close_connection = True

    do_HEAD = do_GET

    def do_CONNECT(self):
        host, _, port = self.path.partition(":")
        try:
            target = socket.create_connection((host, int(port or 443)), RECV_TIMEOUT)
        except OSError as exc:
            self.send_error(502, "无法连接上游：%s" % exc)
            return

        self.send_response(200, "Connection Established")
        self.end_headers()
        relay(self.connection, target)
        target.close()
        self.close_connection = True


class HttpProxyServer(ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True


# ---------------- SOCKS5 代理 ----------------

class Socks5Handler(socketserver.BaseRequestHandler):
    def handle(self):
        sock = self.request
        sock.settimeout(RECV_TIMEOUT)
        try:
            ver, nmethods = recvn(sock, 2)
            if ver != 5:
                return
            recvn(sock, nmethods)  # 方法列表，测试代理固定选无认证
            sock.sendall(b"\x05\x00")

            ver, cmd, _, atyp = recvn(sock, 4)
            if cmd != 1:  # 只支持 CONNECT
                sock.sendall(b"\x05\x07\x00\x01" + b"\x00" * 6)
                return

            if atyp == 1:
                address = socket.inet_ntoa(recvn(sock, 4))
            elif atyp == 3:
                length = recvn(sock, 1)[0]
                address = recvn(sock, length).decode("utf-8", "replace")
            elif atyp == 4:
                address = socket.inet_ntop(socket.AF_INET6, recvn(sock, 16))
            else:
                sock.sendall(b"\x05\x08\x00\x01" + b"\x00" * 6)
                return

            port = int.from_bytes(recvn(sock, 2), "big")

            try:
                target = socket.create_connection((address, port), RECV_TIMEOUT)
            except OSError:
                sock.sendall(b"\x05\x04\x00\x01" + b"\x00" * 6)
                return

            sock.sendall(b"\x05\x00\x00\x01" + b"\x00" * 6)
            relay(sock, target)
            target.close()
        except (ConnectionError, OSError, TimeoutError):
            pass


class Socks5Server(ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True


def main() -> int:
    mode = sys.argv[1].lower() if len(sys.argv) > 1 else "http"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8899

    if mode == "http":
        server = HttpProxyServer(("127.0.0.1", port), HttpProxyHandler)
        print("HTTP 代理已启动：127.0.0.1:%d" % port)
    elif mode in ("socks5", "socks"):
        server = Socks5Server(("127.0.0.1", port), Socks5Handler)
        print("SOCKS5 代理已启动：127.0.0.1:%d" % port)
    else:
        print("未知模式：%s（可选 http / socks5）" % mode)
        return 1

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
