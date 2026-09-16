# -*- coding: utf-8 -*-
"""
生成用于功能验证的测试站点。

覆盖的场景：
  - CSS / JS / 图片 / 字体 等常规资源
  - 带查询参数的资源（验证文件名的哈希化重命名）
  - CSS 内的 url() 背景图、@font-face、@import 嵌套
  - 内联 style 属性里的 url()
  - srcset 多候选
  - GBK 编码的中文页面（验证编码识别与转码）
  - 下级页面（验证整站递归）
  - 外站链接（验证单页模式下的失效处理）
"""
import base64
import os
import sys

PNG_1X1 = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)

INDEX_HTML = """<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>克隆工具测试首页</title>
<link rel="stylesheet" href="css/style.css?v=1.2&theme=dark">
<link rel="stylesheet" href="css/extra.css">
<link rel="icon" href="img/favicon.ico">
<link rel="canonical" href="http://localhost:8765/">
<script src="js/app.js?ver=3"></script>
<style>
  .inline-bg { background-image: url("img/css-bg.png?size=large"); }
  @import "css/imported.css";
  @font-face {
    font-family: 'TestFont';
    src: url('../font/test.woff2') format('woff2');
  }
</style>
</head>
<body background="img/body-bg.png">
  <h1>测试首页</h1>
  <img src="img/logo.png" alt="logo">
  <img srcset="img/a.png 1x, img/b.png?x=1 2x" alt="srcset">
  <div style="background: url(img/inline-style.png) no-repeat; width: 10px; height: 10px;"></div>
  <video poster="img/poster.png"><source src="media/clip.mp4" type="video/mp4"></video>
  <a href="sub/page2.html">下级页面</a>
  <a href="https://external.example.com/somewhere">外站链接</a>
  <a href="mailto:someone@example.com">邮件链接</a>
  <a href="#anchor">页内锚点</a>
</body>
</html>
"""

STYLE_CSS = """/* 主样式表 */
@charset "utf-8";
body {
  background: #fff url(../img/bg.jpg) repeat-x;
  font-family: 'TestFont', sans-serif;
}
.icon {
  background-image: url('../img/icon.png?v=2');
}
.hero {
  background: url(https://cdn.example.net/assets/hero.webp) center/cover;
}
"""

IMPORTED_CSS = """.imported {
  background: url(../img/imported-bg.png) no-repeat;
}
"""

EXTRA_CSS = """.extra { color: #333; }
"""

APP_JS = """document.addEventListener('DOMContentLoaded', function () {
  console.log('app loaded');
});
"""

PAGE2_HTML = """<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>下级页面</title>
<link rel="stylesheet" href="../css/style.css?v=1.2&theme=dark">
</head>
<body>
  <h1>下级页面</h1>
  <img src="../img/logo.png">
  <a href="../index.html">回到首页</a>
  <a href="deeper/page3.html">更深一层</a>
</body>
</html>
"""

PAGE3_HTML = """<!DOCTYPE html>
<html lang="zh-CN">
<head><meta charset="utf-8"><title>第三层页面</title></head>
<body><h1>第三层</h1><img src="../../img/logo.png"></body>
</html>
"""

GBK_HTML = """<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta http-equiv="Content-Type" content="text/html; charset=gb2312">
<title>中文编码测试页</title>
<link rel="stylesheet" href="css/gbk-style.css">
</head>
<body>
  <h1>这是一段GBK编码的中文内容</h1>
  <p>如果编码处理正确，这段话在本地打开应该不会乱码。</p>
  <img src="img/logo.png">
</body>
</html>
"""

GBK_CSS = """.gbk { content: "中文注释内容"; }
"""


def write(path: str, data: bytes) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def main() -> int:
    root = sys.argv[1] if len(sys.argv) > 1 else "site"
    root = os.path.abspath(root)

    for name in ["logo.png", "a.png", "b.png", "css-bg.png", "inline-style.png",
                 "body-bg.png", "poster.png", "icon.png", "imported-bg.png", "bg.jpg"]:
        write(os.path.join(root, "img", name), PNG_1X1)

    write(os.path.join(root, "img", "favicon.ico"), PNG_1X1)
    write(os.path.join(root, "font", "test.woff2"), b"wOF2" + b"\x00" * 64)
    write(os.path.join(root, "media", "clip.mp4"), b"\x00\x00\x00\x18ftypmp42" + b"\x00" * 128)

    write(os.path.join(root, "index.html"), INDEX_HTML.encode("utf-8"))
    write(os.path.join(root, "gbk.html"), GBK_HTML.encode("gb18030"))
    write(os.path.join(root, "css", "style.css"), STYLE_CSS.encode("utf-8"))
    write(os.path.join(root, "css", "imported.css"), IMPORTED_CSS.encode("utf-8"))
    write(os.path.join(root, "css", "extra.css"), EXTRA_CSS.encode("utf-8"))
    write(os.path.join(root, "css", "gbk-style.css"), GBK_CSS.encode("gb18030"))
    write(os.path.join(root, "js", "app.js"), APP_JS.encode("utf-8"))
    write(os.path.join(root, "sub", "page2.html"), PAGE2_HTML.encode("utf-8"))
    write(os.path.join(root, "sub", "deeper", "page3.html"), PAGE3_HTML.encode("utf-8"))

    print("测试站点已生成：" + root)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
