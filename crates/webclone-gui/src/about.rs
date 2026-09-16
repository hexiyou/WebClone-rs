//! "关于"页的说明文本。
//!
//! C# 版用内嵌 WebBrowser 渲染一份 `About.html`；winsafe 是纯 Win32 封装，
//! 没有内置的 WebView 宿主，这里改为同等内容的纯文本，避免额外引入 IE/WebView2 依赖。

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 关于页正文。
///
/// 返回的换行是 `\r\n` 而不是字面量里的 `\n`：Win32 多行 `Edit` 按回车断行，
/// 裸 `\n` 会被直接吃掉，整段文字会挤成一行。字面量保持 `\n` 便于阅读，出口统一补 `\r`。
pub fn about_text() -> String {
    format!(
        r#"WebClone 网页克隆工具
版本 {VERSION} · Rust + winsafe 原生 Win32 + 命令行双形态

WebClone 是一款网页模板 / 整站离线克隆工具。它可以把网页连同其引用的全部资源
（样式表、脚本、图片、字体等）下载到本地，并把页面中的链接改写为相对路径，
生成可直接离线打开、也可作为模板二次开发的本地副本。

克隆模式
  · 单页模板：只抓取当前页面及其引用的资源，外站链接可禁用，适合保存网页模板。
  · 整站递归：沿站内链接递归抓取整个站点，可设置递归深度与最大页面数。

主要特性
  · 多层 CSS 资源追踪：@import、url() 引用与内联样式一并抓取。
  · 跨域 CDN 资源可选下载，抓回本地不再依赖外部服务。
  · GBK / GB2312 页面自动转换为 UTF-8。
  · 增量下载（ETag / 304 未变更自动跳过），断点续传（Range / 206）。
  · HTTP / SOCKS5 代理支持，并发数、超时、重试次数均可调节。
  · 可自定义浏览器 UA（默认 Edge，见“下载选项”），便于绕过部分站点的
    浏览器指纹校验。
  · 命令行版 webclone.exe 为原生编译、零运行时依赖；图形界面版单文件发布，
    开箱即用。

命令行用法
  webclone https://example.com -o D:\backup              （单页）
  webclone https://example.com -m full -d 3 -o D:\backup （整站，深度 3）

源代码仓库
  https://github.com/hexiyou/WebClone-rs

WebClone · 使用 Rust / winsafe 构建
"#
    )
    .replace('\n', "\r\n")
}
