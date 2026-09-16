# WebClone（Rust 版）— 网页模板 / 整站克隆工具

C# + WPF 版 `WebClone` 的 **Rust + winsafe** 1:1 复刻。图形界面走 winsafe 的
原生 Win32 控件（不引入任何 UI 框架），命令行版与 C# 版完全同参。

对应 wget 的两种用法：

| 模式 | 等价 wget 命令 | 说明 |
|------|----------------|------|
| 单页模板 | `wget -r -E -np -c -k --ignore-tags=a <url>` | 只抓当前页与它引用的资源，不递归 a 链接 |
| 整站克隆 | `wget -r -np -c -k <url>` | 递归追踪站内链接（不向上级目录），抓全站 |

## 项目结构

```
webclone-rs/
├── Cargo.toml                  workspace 定义（共享依赖版本 + release profile）
├── build.sh / build.ps1        一键构建脚本（Windows 上优先用 build.ps1，理由见下）
├── crates/
│   ├── webclone-core/          核心引擎（纯 Rust，AOT 友好、零反射）
│   ├── webclone-cli/           命令行版（bin 名 webclone，与 C# 版同参）
│   └── webclone-gui/           图形界面（winsafe 原生 Win32）
│       ├── src/main.rs         窗口、控件、事件、布局
│       ├── build.rs            把视觉样式 manifest 嵌入 exe（见"界面观感"一节）
│       └── webclone-gui.manifest   comctl32 v6 依赖声明，决定控件用现代主题还是经典外观
└── tests/
    ├── make_test_site.py       生成功能验证站点（含 GBK 页面、查询参数资源、CSS 嵌套引用等）
    ├── test_server.py          支持 ETag / 304 / Range 续传的本地测试服务器
    ├── test_proxy.py           极简 HTTP / SOCKS5 双模代理
    ├── ua_server.py            UA 记录服务器（验证 --user-agent 是否真的生效）
    ├── regression.ps1          CLI 一键回归（6 组场景）
    ├── gui_probe.py            GUI 界面探针（42 条断言 + 截图）
    └── gui_smoke.sh            GUI 冒烟入口（起进程、跑探针、收尾；自动沙箱隔离）
```

`webclone-core` 的模块划分刻意与 C# 版 `WebClone.Core` 一一对应，方便对照阅读：

| C# | Rust |
|----|------|
| `CloneEngine.cs` | `engine.rs` |
| `CloneOptions.cs` / `ProxySettings.cs` | `options.rs` |
| `Models/*` | `models.rs` |
| `Net/ResourceDownloader.cs` | `net/downloader.rs` |
| `Net/HttpClientFactory.cs` | `net/http.rs` |
| `Net/Socks5Connector.cs` | 由 reqwest 的 `socks` 特性承担 |
| `Parse/HtmlScanner.cs` / `CssScanner.cs` | `parse/html_scanner.rs` / `parse/css_scanner.rs` |
| `Storage/PathMapper.cs` / `SafeFileName.cs` | `storage/path_mapper.rs` / `storage/safe_name.rs` |
| `Text/EncodingDetector.cs` / `CharsetRewriter.cs` | `text/encoding.rs` / `text/charset.rs` |

## 功能与实现要点

- **两种模式**：单页模板 / 整站递归，整站模式遵循 `-np` 语义（不下载上级目录）。
- **文件名处理**：URL 带查询参数时生成 `原名_8位哈希.ext`（如 `style_78fe4a82.css`），
  合法且可读；同时处理 Windows 保留名、非法字符、路径穿越。
- **路径改写**：所有资源引用改写为相对路径；`<base href>` 重定向为 `.`；
  跨域 CDN 资源按各自域名归档后同样以相对路径引用。
- **重定向归一**：下载时跟随服务端重定向，最终地址与请求地址共享同一本地文件，
  整站克隆不会因链接写法差异（`/doc` 与 `/doc/`）存两份。
- **类型修正**：下载完成后按响应的真实 Content-Type 修正落盘扩展名 ——
  像 Google Fonts 的 `/css?family=...` 这类无扩展名动态样式表会被正确存为 `.css`。
- **单页模式防跳转**：指向未下载页面的 a 链接统一替换为 `#`；整站模式保留外站原链。
- **代理**：HTTP(S) 代理与 SOCKS5 代理。SOCKS5 走 reqwest 的 `socks5h://`
  （域名交给代理解析，与 C# 版手写 RFC 1928 实现里按 ATYP=域名 发出的行为一致），
  支持用户名密码认证。
- **增量与断点续传**：
  - 图片、字体等二进制资源用 `If-None-Match` / `If-Modified-Since` 条件请求，304 直接跳过；
  - 中断的下载用 `Range` + `If-Range` 续传（资源变化时服务端自动降级为 200 全量）；
  - 一律先写 `.part` 再原子改名，不留残缺成品；
  - HTML / CSS 因为落盘前会被改写，始终取原始内容。
- **编码**：按 BOM → HTTP charset → meta charset / `@charset` → 严格 UTF-8 嗅探 →
  GB18030 兜底的顺序识别，统一转 UTF-8 保存并改写编码声明，GBK 页面不乱码。
- **解析器自研**：HTML / CSS 扫描器为零依赖状态机，记录原文下标做原地替换，
  不依赖任何 HTML 库。覆盖 `<link>`、`<script>`、`<img>`、`srcset`、`<video>/<audio>/<source>`、
  `<object>/<embed>`、`<iframe>`、内联 style、`<style>` 正文、CSS `url()` / `@import` /
  `@font-face`，以及 `data-src` 等懒加载属性。
- **新增：自定义浏览器 UA** —— 图形界面的"下载选项"里多了一个
  `使用自定义浏览器 UA` 复选框与配套文本框（默认填 Edge 的 UA，默认不勾选）。
  勾选后用文本框内容作为 User-Agent 发起请求，并同样持久化到 `webclone-gui.json`。
  CLI 侧对应 `--user-agent "<UA>"`。

## 构建

Windows 上优先用 PowerShell 脚本：

```powershell
.\build.ps1                  # CLI + GUI 全部构建，产物汇总到 dist\
.\build.ps1 -SkipGui         # 只构建命令行版
.\build.ps1 -SkipCli         # 只构建图形界面版
.\build.ps1 -Clean           # 先清 dist\ 与 target\ 再全量构建
.\build.ps1 -Profile debug   # 用 debug profile
```

Git Bash / MSYS 下也可以：

```bash
./build.sh            # CLI + GUI 全部构建
./build.sh cli        # 只构建命令行版
./build.sh gui        # 只构建图形界面版
./build.sh clean      # 先清 dist/ 再全量构建
```

> 为什么 Windows 上优先 `build.ps1`：`build.sh` 里调 `bash`，而部分环境下嵌套 `bash`
> 会被 WSL 接走，那会在**同一个 `target/`** 里编出 Linux 产物，与 Windows 产物互相污染
> （现象是 `cargo` 报一堆平台不匹配，或者 `target/release` 里混进 ELF）。
> `build.sh` 开头已加 `uname` 检测拦截，但用 PowerShell 更省事。

产物：

| 文件 | 说明 |
|------|------|
| `dist/webclone.exe` | 命令行版，约 3.3 MB |
| `dist/WebClone-GUI.exe` | 图形界面版，约 3.4 MB |

两个都是**独立单文件**：Rust 直接编成机器码，没有 .NET 那种 AOT / 自包含之分，
也不需要任何运行库，拷到别的机器上直接能跑。

手动构建（等效于脚本内部行为）：

```bash
cargo build --release -p webclone-cli
cargo build --release -p webclone-gui
```

## CLI 用法

```bash
webclone <网址> [选项]

# 单页模板，输出到指定目录
webclone https://example.com -o D:\site

# 整站递归，深度 3，并发 16
webclone https://example.com -m full -d 3 -c 16 -o D:\site

# 走 SOCKS5 代理（支持 user:pass 认证）
webclone https://example.com --proxy socks5://user:pass@127.0.0.1:1080

# 走 HTTP 代理
webclone https://example.com --proxy http://127.0.0.1:8080

# 指定 User-Agent
webclone https://example.com --user-agent "Mozilla/5.0 (...) Edg/126.0.0.0"
```

常用选项：`-m single|full`、`-o 目录`、`-d 深度`、`-c 并发`、`--max-pages`、
`--timeout`、`--retry`、`--no-cross-origin`（不抓 CDN）、`--no-incremental`、
`--keep-absolute`、`--keep-external-links`、`--ignore-cert-errors`、
`--user-agent`、`-v` 详细日志。

退出码与 C# 版一致：`0` 成功、`1` 有失败项 / 参数缺失、`2` 参数错误、`3` 未处理异常、
`130` 用户取消。

## GUI 选项记忆

GUI 每次启动自动回显上次的选项，配置存放在 **exe 同目录** 的 `webclone-gui.json`：

- 首次运行自动生成默认配置；点"开始克隆"或关闭窗口时自动保存当前选项
- 单个字段损坏只跳过该字段用默认值；整个文件损坏自动重建
- 配置写失败（如放在无写权限目录）会写一份 `gui-save-error.txt` 诊断，不影响克隆
- 文件字段名与 C# 版完全一致（PascalCase），**两个版本可以互读同一份配置**

配置文件长这样（新增的两个字段在末尾）：

```json
{
  "StartUrl": "https://example.com",
  "OutputDirectory": "C:\\Users\\Administrator\\Desktop\\webclone-output",
  "Mode": "single",
  "MaxDepth": 3,
  "MaxPages": 0,
  "ProxyKind": "none",
  "ProxyHost": "",
  "ProxyPort": 8080,
  "ProxyUserName": "",
  "ProxyPassword": "",
  "IncludeCrossOriginAssets": true,
  "ConvertToRelativePaths": true,
  "EnableIncremental": true,
  "NeutralizeExternalLinks": true,
  "IgnoreCertificateErrors": false,
  "Concurrency": 8,
  "TimeoutSeconds": 30,
  "RetryCount": 3,
  "UseCustomUserAgent": false,
  "CustomUserAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0"
}
```

## 测试

### CLI 一键回归

自动起服务器、跑六组场景断言、汇总 PASS/FAIL：

```powershell
.\tests\regression.ps1                    # 默认用 dist\webclone.exe
.\tests\regression.ps1 -PythonPath py     # 指定 python 命令
```

覆盖场景：单页克隆（含哈希文件名与资源落盘）、增量二跑、整站三页递归、
`-np` 不越界、GBK 转码，以及 `--user-agent` 是否真的传到了服务端。

### GUI 冒烟验证

`tests\gui_probe.py` 用纯 `ctypes` + PIL 找到窗口、读控件状态、发鼠标/按钮/键盘消息，
对界面做 **42 条断言**，并顺手截四张图。覆盖：

- 默认勾选状态是否与 C# 的 XAML 一致、配置回显；
- 代理框失能、六个数字框的默认值与微调按钮宽度、自定义 UA 勾选联动；
- 关于页渲染；
- **切页重绘**：切到"关于"再切回"克隆"，截图与初始画面逐像素比对（阈值 1%，
  修好前是 20.89%）；
- **代理端口框随单选启停**：直连时输入框 + 微调两个窗口都必须禁用；
- **网址框回车**：用 `SendInput` 发真实回车，验证能弹出校验提示；
- 点关闭按钮能否真的退出进程。

```bash
# 必须用 `.` 在当前 shell 里执行：脚本依赖"GUI 与探针同一条命令"这个前提，
# 也避免嵌套 bash 落到 WSL 上。
PYTHON=/path/to/python.exe . tests/gui_smoke.sh
```

输出形如 `RESULT: ALL PASS` / `RESULT: HAS FAILURES`，并留下
`shot-1-clone.png` / `shot-2-ua-checked.png` / `shot-3-about.png` /
`shot-4-clone-back.png` 四张截图。
最后两条断言会发 `WM_CLOSE` 并等进程退出，脚本另打印一行
`关闭验证: 进程已自行退出 [OK]` 作为独立于断言之外的第二重证据。

> 脚本会把 `dist\WebClone-GUI.exe` 复制到 `tests\.smoke-sandbox\` 里再运行。原因是 GUI 的
> 配置固定写在 **exe 同目录**的 `webclone-gui.json`：直接在 `dist\` 里跑会双向污染 ——
> 测试读到用户手调的端口/并发而误报 FAIL，用户调好的配置又被测试结果覆盖。沙箱化之后
> 两边彻底隔离，测试也总从"首次运行"的干净状态开始。
> （沙箱内的旧配置用 `mv` 改名而不是 `rm` 删除 —— 本环境的 safe-delete 策略会拦截 `rm`，
> 而 `mv` 改名一定成功。）

> 两个环境坑（脚本里已处理，手工调试时容易踩）：
> 1. **GUI 和探针必须在同一条命令里跑**。GUI 是 shell 的子进程，命令一结束父 shell
>    被回收，GUI 会被连坐杀掉，现象是探针跑到一半窗口消失、截屏得到空图。
> 2. **跨进程不能用 `TCM_GETITEMRECT`**。它不在系统跨进程封送的消息表里，传过去的
>    `RECT` 指针在目标进程里无效，会稳定返回全零矩形；探针改用"扫页签条发鼠标消息，
>    以关于页是否显示来自证"的办法。

### 手工验证

```bash
# 1. 生成测试站点并启动本地服务器（支持 304 / Range）
python tests/make_test_site.py tests/site
python tests/test_server.py 8765 tests/site

# 2. 启动双模代理（可选，验证代理功能）
python tests/test_proxy.py http 8899
python tests/test_proxy.py socks5 8898

# 3. 克隆并对比
dist/webclone.exe http://127.0.0.1:8765/ -m full -o out
```

## 界面观感与原生控件主题

原生 Win32 程序的外观**不由代码决定，而由 exe 里那份 manifest 决定**。Rust 的 MSVC
目标默认不生成 manifest，于是 exe 里没有 `Microsoft.Windows.Common-Controls` v6 的
依赖声明 —— 结果是所有原生控件（Button / CheckBox / Radio / ProgressBar / Tab /
ScrollBar）都退到**经典渲染路径**：灰色 3D 凸起按钮、方角复选框、带箭头的粗滚动条，
也就是 Win9x 那种观感。

C# WPF 版不存在这个问题，因为 WPF 的控件全是自绘的，一个像素都不来自 comctl32。

`crates/webclone-gui/webclone-gui.manifest` 补上了这条声明，`build.rs` 用 MSVC 链接器
自带的 `/MANIFEST:EMBED` + `/MANIFESTINPUT:` 把它嵌进 exe（**零第三方依赖**，
不需要 winres / embed-resource 那类 crate）：

- 体积代价 **+1.5 KB**（3,576,832 → 3,578,368 字节），业务代码 **0 改动**；
  （后续修掉下面几个 Win32 坑又加了约 1.5 KB，当前产物 3,579,904 字节）
- 未产生外部 `.manifest` 文件（`/MANIFEST:EMBED` 保证内嵌）；
- 效果：按钮、复选框、单选钮、页签、滚动条全部切换为 Windows 10/11 原生主题。

> **坑：manifest 里不能声明 DPI 感知。**
> winsafe 的 `initial_gui_setup()` 会调用 `SetProcessDPIAware()`，而该 API 在
> "DPI 感知已由 manifest 指定"时会失败并返回 `ERROR_ACCESS_DENIED`；winsafe 对它用的
> 是 `.expect(DONTFAIL)`，于是进程**直接 panic**。所以 DPI 感知必须留给 winsafe 的
> 运行时调用（system-DPI aware），manifest 只声明 comctl32 依赖 + supportedOS。
> 代价是拿不到 PerMonitorV2 的多屏独立缩放，属于 winsafe 当前版本的固有约束。

**控件字体不用管**：winsafe 会在每个控件创建时自动把系统 UI 字体
（`SystemParametersInfo(SPI_GETNONCLIENTMETRICS).lfMenuFont`，现代 Windows 上即
Segoe UI 9pt）通过 `WM_SETFONT` 设上去，跟随用户的系统设置，中文由字体链接自动 fallback。

### 离 C# WPF 版的外观还差多少

C# 版做过专门的视觉设计，不是 WPF 默认皮肤：窗口底 `#FAFAFA`、白色圆角卡片
（`CornerRadius=6` + `#DDD` 边框 + 12px 内边距）、`#1E88E5` 蓝色实心主按钮、
Consolas 12px 日志区、SemiBold 分区标题。补齐这些在 winsafe 下的成本：

| 项目 | 做法 | 量级 |
|------|------|------|
| 日志区等宽字体 | 单独 `CreateFont("Consolas")` + `WM_SETFONT` | ~5 行 |
| 分区标题字重/颜色 | 换字体 + `WM_CTLCOLORSTATIC` 里 `SetTextColor` | ~20 行 |
| 窗口/卡片底色 | 窗口类 `hbrBackground` + `WM_CTLCOLORSTATIC` 返回同色画刷 | ~30 行 |
| 圆角卡片边框 | `WM_PAINT` 自绘圆角矩形（要先 `CreateRoundRectRgn` 裁切） | ~120 行 |
| 蓝色主按钮 | owner-draw 按钮（`BS_OWNERDRAW` + `WM_DRAWITEM`），含 hover / pressed 态 | ~150 行 |
| 控件间距对齐 WPF 的 Margin 体系 | 手工挪坐标 | 视精细度 |

即：**轻量美化（前 3 项）约 50 行**，能明显提升但仍是系统配色；**追平 WPF 的卡片式
设计约 300~450 行**，主要成本在自绘按钮与圆角卡片，且要额外处理 DPI 缩放与高对比度
模式下的可读性。再往上一档才是换 UI 框架（egui / Slint / Tauri + WebView2），那要重写
整个 GUI 层（现约 1500 行），`webclone-core` 不受影响。

## 四个"看不见"的 Win32 坑

这几处都不报错、不崩溃，只是"行为不对"，全部靠探针截图 + 日志定位出来。记在这里，
下次改 GUI 时能少走一遍。

### 1. 切页重影：空心分组框撞上 `WS_CLIPCHILDREN`

**现象**：点"关于"再点回"克隆"，关于页的大段说明文字原样留在克隆页上，和克隆页
控件叠成重影，文字完全不可读。

**根因**（两条 Win32 规则互相抵消）：

- `BS_GROUPBOX` 是**空心**控件 —— 它只画边框和左上角标题，中间那块**故意不填**，
  设计上假设"父窗口会把底色擦好"；
- `TabPage` 带 `WS_CLIPCHILDREN` —— 父窗口擦背景时会把**子窗口矩形整块跳过**。

于是分组框中间那块像素成了"无人区"：程序刚启动时它是窗口初值（看着正常），一旦被
别的页盖过再回来，系统把这块矩形交给下层窗口补画 —— 父页面被 `WS_CLIPCHILDREN`
跳过、分组框自己又不擦底，旧像素就留在原地了。

**修法**：给两个分组框挂子类化，接管 `WM_ERASEBKGND` 自己把客户区刷成窗口底色
（`FillRect` + `GetSysColorBrush(COLOR_WINDOW)`），并在切页后用
`RedrawWindow(RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW)`
把新页连同子控件一起强制重绘。两者缺一不可。

> **排查记录**：一开始只加了 `RedrawWindow`，像素差异纹丝不动（20.89%），
> 一度以为是事件没触发。实测证据是：跨进程调 `RedrawWindow` 返回 1 但残影不变 ——
> 说明根本不是"没收到重绘请求"，而是"给它重绘它也不擦"。`PrintWindow` +
> z 序枚举 + `WindowFromPoint` 逐个排除后才定位到分组框。
> 修复后差异 **0.00%**。

### 2. 真实回车被 `IsDialogMessage` 吃掉

**现象**：网址框里按回车毫无反应。

**根因**：winsafe 的 `BaseWnd::run_main_loop` 里有一步（`process_dlg_msgs` 打开时）：

```rust
if process_dlg_msgs && hwnd_top_level.IsDialogMessage(&mut msg) {
    continue; // 消息被消费，不再 DispatchMessage
}
```

对话框管理器拿到 `VK_RETURN` 的 `WM_KEYDOWN` 后会去找"默认按钮"，本窗口压根没有
默认按钮 —— 那条消息就被它**直接丢弃**：既没有 Dispatch 到窗口过程，也没触发任何按钮。
`WM_KEYUP` 不受影响，所以子类化过程能收到 `WM_KEYUP` 却收不到 `WM_KEYDOWN`。

**修法**（标准做法）：在 `WM_GETDLGCODE` 里对"这一条具体的回车消息"回
`DLGC_WANTMESSAGE` —— 此时 `lParam` 正好指向待处理的 `MSG`，可以精确判断
`message == WM_KEYDOWN && wParam == VK_RETURN`，只放行回车，Tab / 方向键照旧交给
对话框管理器，键盘导航不受影响。

> **测试口径**：这条只能用 `SendInput` 发真实按键验证。`PostMessage` 的键盘消息
> 直接进窗口队列、绕开 `IsDialogMessage`，恰好把坑绕过去了（实测 `PostMessage`
> 到不了、`SendInput` 能到）——用错工具会得到"功能正常"的假阳性。

### 3. winsafe 的子类化必须在 `WM_CREATE` 之后装

winsafe 的窗口**连同所有子控件**都是在消息循环启动后才真正 `CreateWindowEx` 出来的，
`WindowMain::new` 返回时 HWND 还不存在。所以在 `events()` 里调 `SetWindowSubclass`
拿到的是空句柄，`SetWindowSubclass` 返回 FALSE、**安装静默失败**。

正确时机是 `wm_create` 回调里（`apply_runtime_init`）。这也正是坑 2 里"回车完全没反应"
的第一层原因 —— 当时子类化根本没装上，修好时机之后才暴露出 `IsDialogMessage` 那一层。

### 4. 端口的"输入框"和"微调"是两个窗口

`msctls_updown32` 只占右侧 17px 的箭头区，真正显示数字、能被点击选中文字的是它的
**伙伴 `Edit`**。所以"直连时禁用端口框"必须**两个窗口都禁用** —— 只禁 UpDown 的话，
输入框照旧能点进去选中文字。`UpDown` 的 `EnableWindow(FALSE)` 并不会连带禁用伙伴框。

## 与 C# / WPF 版的差异

功能与选项 1:1 对齐，只有下面三处是**受技术栈限制的实现差异**，不影响克隆结果：

1. **"关于"页改为纯文本呈现**。C# 版用内嵌 `WebBrowser`（IE 内核）渲染一份 HTML；
   winsafe 是纯 Win32 封装，不含 WebView 宿主，为避免额外引入 IE / WebView2 依赖，
   改成了同等内容的纯文本。
2. **状态文字不换色**。WPF 版按结果把"就绪 / 正在克隆 / 克隆完成 / 有失败 / 已取消"
   分别染成蓝 / 绿 / 红 / 橙；纯 Win32 下需要接管 `WM_CTLCOLORSTATIC` 自建画刷，
   这里选择不染，文案完全一致。
3. **不做 C# 版那套自定义视觉设计**。C# 版有 `#FAFAFA` 底 + 白色圆角卡片 +
   `#1E88E5` 蓝色主按钮；这里走 Windows 系统原生主题（控件外观 + 配色跟随系统），
   仅在 manifest 里开启了 comctl32 v6 视觉样式。理由与后续可选的投入见上一节。

技术栈层面的实现取舍：

- **UI 布局用手动定位**。winsafe 自带按"父窗口缩放比例"重排的 `resize_behavior`，
  但页签页（`TabPage`）自身会被 `TCM_ADJUSTRECT` 挪动，自动布局在这种
  "父窗口被外部挪动"的场景下原点会算错。所以 `Tab` 的尺寸、页签页的位置、
  以及页内所有控件的坐标都由 `apply_layout()` 统一手动算。
- **一切"写控件"的动作都必须发生在 `WM_CREATE` 之后**。winsafe 的窗口（连同所有
  子控件）是在 `run_main()` 内部才 `CreateWindowEx` 出来的，`WindowMain::new()`
  返回时 `HWND` 还不存在；此时调 `set_text` / `set_check` / `EnableWindow` 全是
  静默空操作。因此配置回显统一放在 `wm_create` 回调里（`App::apply_runtime_init`）。
- **首次运行时用 `GuiSettings::default()` 回填界面**。C# 版首次运行不回填，界面直接
  用 XAML 里写死的初始状态（那几个 `IsChecked="True"`）；winsafe 没有 XAML，若跳过
  回填直接读控件取值落盘，会写出四个复选框全 `false` 的错配置。
- **回车快捷键借用 `SetWindowSubclass`**。winsafe 的控件事件只暴露 `WM_COMMAND` /
  `WM_NOTIFY`，没有 `WM_KEYDOWN` 钩子，所以网址输入框的"回车即开始克隆"
  直接调 comctl32 的 `SetWindowSubclass` 挂了一层子类化过程。两个前提缺一不可：
  **安装时机**必须在 `WM_CREATE` 之后（坑 3），且要在 `WM_GETDLGCODE` 里对回车回
  `DLGC_WANTMESSAGE`，否则消息会被 `IsDialogMessage` 吞掉（坑 2）。
- **微调按钮不用 `UDS::ALIGNRIGHT`**。它会把伙伴输入框压窄一个箭头宽（96 DPI 下
  16px），而且这个压窄在控件被程序化搬动之后表现不稳定。这里改为显式摆放：输入框
  和微调按钮都自己给坐标与宽度。另外 `UDS::NOTHOUSANDS` 必须显式加 ——
  `UDS::SETBUDDYINT` 默认按用户区域格式插千分位，8080 会显示成 `8,080`，
  而伙伴框带 `ES::NUMBER`，这种文本是非法输入。
- **多行 `Edit` 的换行必须是 `\r\n`**。Win32 按回车断行，裸 `\n` 会被吃掉，
  整段文字挤成一行（关于页正文因此统一在出口补 `\r`）。
- **注册了 `wm_close` 就必须自己 `DestroyWindow`**，否则点关闭按钮毫无反应、只能
  强杀进程。winsafe 的 `on()` 通道是**同名消息后注册者胜**
  （`process_last_message` 用 `msgs.iter().rev().find(..)` 只取最后一条），而
  `DlgMain::new` 内建注册过一个 `wm_close` → `DestroyWindow`，我们一注册就把它顶掉了；
  `fn_wm_noparm_noret!` 宏在回调返回 `Ok` 时返回 `def_proc_val`（dialog = TRUE），
  `wnd_proc` 见 `user_ret` 有值就直接返回、不再走 `DefWindowProc` —— `WM_CLOSE` 被
  吃掉，窗口不销毁，`GetMessage` 永远等不到 `WM_QUIT`，消息循环卡死。
  正确做法是在自己的 `wm_close` 里补上 `hwnd().DestroyWindow()`。
  （winsafe 文档对 `wm_close` 也明说了：处理此事件即覆盖 `WindowMain` 内建的
  `DestroyWindow`；`DlgMain` 里 `wm_nc_destroy` → `PostQuitMessage(0)` 我们没碰，
  所以销毁之后仍能正常退出消息循环。）
- **reqwest 关掉了 system-proxy 默认特性**，并显式 `no_proxy()`，避免本机
  `http_proxy` / `https_proxy` 环境变量干扰"直连"语义。
- **SOCKS5 用 `socks5h://`**（h = 域名交给代理解析），与 C# 版手写实现等价。
- **HTML / CSS 扫描器按 UTF-8 字节下标工作**。原 C# 版用 UTF-16 下标；
  由于所有定界符都是 ASCII，两种下标在语义上等价（首字节处切分正确）。

## 环境备注

- 需要 Rust 1.75+（实测 1.97.1）。
- 在 Git Bash / Cygwin 会话里构建若报环境相关错误，先
  `source ~/.workbuddy/bin/env-fix.sh`（该脚本补 `ProgramFiles` / `OS` 等变量）。
- winsafe 依赖 Windows SDK 自带的 `comctl32` / `shell32` / `ole32` 等系统 DLL，
  不需要额外装 Visual Studio C++ 工具链（MSVC link.exe 由 rustup 的
  `x86_64-pc-windows-msvc` 工具链提供）。
