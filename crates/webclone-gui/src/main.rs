//! WebClone 图形界面（Rust + winsafe 原生 Win32）。
//!
//! 与 C# + WPF 版功能一一对应：同样的两个页签、同样的选项集合、同样的默认值、
//! 同样的日志与状态文案，配置同样落在 exe 同目录的 `webclone-gui.json`。
//!
//! 额外新增：**"下载选项"里的自定义浏览器 UA** —— 复选框默认不勾选，
//! 文本框默认填 Edge 的 UA；勾选后用它发起请求，并同样持久化到配置文件。
//!
//! 与 WPF 版的两点实现差异（功能等价，非缺失）：
//! 1. "关于"页用纯文本呈现（winsafe 不含 WebView 宿主，不额外引入 IE/WebView2）；
//! 2. 状态文字不换色（纯 Win32 下需接管 WM_CTLCOLORSTATIC 自建画刷），文案一致。

#![windows_subsystem = "windows"]

mod about;
mod settings;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use winsafe::{self as w, co, gui, msg, prelude::*};

use webclone_core::storage::path_util;
use webclone_core::{
    CancelToken, CloneEngine, CloneMode, CloneOptions, CloneProgress, CloneReport,
    Error as CoreError, ProxyKind, ProxySettings, DEFAULT_USER_AGENT,
};

use settings::{GuiSettings, GuiSettingsStore};

// ------------------------------------------------------------ 常量与坐标换算

/// 日志刷新定时器 ID（WM_TIMER）。
const TIMER_LOG: usize = 1;
/// 日志批量刷新间隔：100ms，避免下载高峰期每条日志都触发一次重排。
const TIMER_LOG_MS: u32 = 100;

/// 页面设计宽度（逻辑像素）。窗口变宽时超出的部分补给可拉伸控件。
const DESIGN_PAGE_W: i32 = 724;

/// 微调按钮的宽度（逻辑像素）。96 DPI 下系统默认就是这个值。
const SPIN_W: i32 = 17;

/// 日志缓冲上限：超过后裁掉前半段，避免内存无限增长。
const LOG_SOFT_LIMIT: usize = 512 * 1024;
const LOG_KEEP_TAIL: usize = 256 * 1024;

/// 网址输入框回车即开始克隆（对应 WPF 版的 `UrlBox_KeyDown`）。
const SUBCLASS_URL_EDIT: usize = 0x5743; // 'WC'
/// 分组框自擦背景的子类化 ID。
const SUBCLASS_GROUPBOX: usize = 0x5747; // 'WG'

fn dx(v: i32) -> i32 {
    gui::dpi_x(v)
}

fn dy(v: i32) -> i32 {
    gui::dpi_y(v)
}

/// 只挪位置，不动尺寸（用于 winsafe 自动按文字撑开的标签 / 复选框 / 单选框）。
fn mv<C: GuiWindow>(ctrl: &C, x: i32, y: i32) {
    mv_hwnd(ctrl.hwnd(), x, y);
}

fn mv_hwnd(hwnd: &w::HWND, x: i32, y: i32) {
    let _ = hwnd.SetWindowPos(
        w::HwndPlace::None,
        w::POINT::with(x, y),
        w::SIZE::with(0, 0),
        co::SWP::NOZORDER | co::SWP::NOACTIVATE | co::SWP::NOSIZE,
    );
}

/// 位置与尺寸一起设定。
fn put<C: GuiWindow>(ctrl: &C, x: i32, y: i32, cx: i32, cy: i32) {
    let _ = ctrl.hwnd().SetWindowPos(
        w::HwndPlace::None,
        w::POINT::with(x, y),
        w::SIZE::with(cx, cy),
        co::SWP::NOZORDER | co::SWP::NOACTIVATE,
    );
}

// ------------------------------------------------------------ 后台任务通信

enum Outcome {
    Done(Box<CloneReport>),
    Cancelled,
    Failed(String),
}

/// 后台线程与 UI 线程之间的共享信箱。UI 侧由定时器周期性取走。
struct Shared {
    logs: Mutex<Vec<String>>,
    outcome: Mutex<Option<Outcome>>,
    running: AtomicBool,
    cancel: Mutex<CancelToken>,
}

impl Shared {
    fn new() -> Self {
        Self {
            logs: Mutex::new(Vec::new()),
            outcome: Mutex::new(None),
            running: AtomicBool::new(false),
            cancel: Mutex::new(CancelToken::new()),
        }
    }

    fn push_log(&self, line: String) {
        if let Ok(mut guard) = self.logs.lock() {
            // 兜底：UI 被卡住时别让队列无限涨
            if guard.len() < 50_000 {
                guard.push(line);
            }
        }
    }
}

// ------------------------------------------------------------ 界面控件集合

#[derive(Clone)]
struct Ui {
    url: gui::Edit,

    mode: gui::RadioGroup,
    depth: gui::UpDown,
    max_pages: gui::UpDown,

    output: gui::Edit,
    browse: gui::Button,

    proxy: gui::RadioGroup,
    proxy_host: gui::Edit,
    /// 端口输入框本体。
    ///
    /// 【必须持有】`proxy_port` 只是那一对上下箭头（`msctls_updown32`，宽 17px），
    /// 用户看得见、点得到、能选中文字的是它的伙伴输入框，那是另一个窗口。
    /// 早先这里把 Edit 用 `_` 丢掉了，于是"直连"时只禁用了箭头、输入框照样能点。
    proxy_port_edit: gui::Edit,
    proxy_port: gui::UpDown,
    proxy_user: gui::Edit,
    proxy_pass: gui::Edit,

    cross_origin: gui::CheckBox,
    relative_paths: gui::CheckBox,
    incremental: gui::CheckBox,
    neutralize: gui::CheckBox,
    ignore_cert: gui::CheckBox,
    /// 新增：启用自定义浏览器 UA。
    custom_ua: gui::CheckBox,
    /// 新增：自定义 UA 文本框。
    ua_text: gui::Edit,

    // 数字框 + 微调：右边的分组要跟着窗口变宽走，所以两半都得持有引用
    conc_edit: gui::Edit,
    concurrency: gui::UpDown,
    timeout_edit: gui::Edit,
    timeout: gui::UpDown,
    retry_edit: gui::Edit,
    retry: gui::UpDown,

    // 右侧分组框（BS_GROUPBOX 的按钮，纯装饰；左侧那个位置固定，不必持有）
    panel_net: gui::Button,
    panel_opts: gui::Button,
    conc_label: gui::Label,
    timeout_label: gui::Label,
    retry_label: gui::Label,

    start: gui::Button,
    stop: gui::Button,
    status: gui::Label,
    progress: gui::ProgressBar,
    log: gui::Edit,

    about_text: gui::Edit,
}

impl Ui {
    fn build(clone_page: &gui::TabPage, about_page: &gui::TabPage) -> Self {
        // ---------------- 网址 / 模式 / 深度 ----------------
        let _url_label = label(clone_page, "网址", 12, 10);

        let url = gui::Edit::new(
            clone_page,
            gui::EditOpts {
                position: gui::dpi(12, 30),
                width: dx(700),
                height: dy(25),
                ..Default::default()
            },
        );

        let _mode_label = label(clone_page, "克隆模式", 12, 66);

        let mode = gui::RadioGroup::new(
            clone_page,
            &[
                gui::RadioButtonOpts {
                    text: "单页模板（只抓当前页与其引用的资源）",
                    position: gui::dpi(12, 86),
                    selected: true,
                    ..Default::default()
                },
                gui::RadioButtonOpts {
                    text: "整站（递归追踪站内链接）",
                    position: gui::dpi(348, 86),
                    ..Default::default()
                },
            ],
        );

        let _depth_label = label(clone_page, "递归深度", 12, 118);

        let (_depth_edit, depth) = numeric(clone_page, 76, 115, 52, (0, 100), 3);

        let _max_pages_label = label(clone_page, "最多页面数（0 不限）", 166, 118);
        let (_max_pages_edit, max_pages) =
            numeric(clone_page, 310, 115, 64, (0, 100_000), 0);

        // ---------------- 保存位置 ----------------
        let _output_label = label(clone_page, "保存位置", 12, 152);

        let output = gui::Edit::new(
            clone_page,
            gui::EditOpts {
                position: gui::dpi(12, 172),
                width: dx(596),
                height: dy(25),
                ..Default::default()
            },
        );

        let browse = gui::Button::new(
            clone_page,
            gui::ButtonOpts {
                text: "浏览...",
                position: gui::dpi(616, 170),
                width: dx(96),
                height: dy(29),
                ..Default::default()
            },
        );

        // ---------------- 网络代理 ----------------
        let panel_net = group_box(clone_page, "网络代理", 12, 212, 350, 252);

        let proxy = gui::RadioGroup::new(
            clone_page,
            &[
                gui::RadioButtonOpts {
                    text: "直连",
                    position: gui::dpi(26, 240),
                    selected: true,
                    ..Default::default()
                },
                gui::RadioButtonOpts {
                    text: "HTTP",
                    position: gui::dpi(90, 240),
                    ..Default::default()
                },
                gui::RadioButtonOpts {
                    text: "SOCKS5",
                    position: gui::dpi(158, 240),
                    ..Default::default()
                },
            ],
        );

        let _proxy_host_label = label(clone_page, "代理地址", 26, 272);
        let proxy_host = gui::Edit::new(
            clone_page,
            gui::EditOpts {
                position: gui::dpi(26, 292),
                width: dx(224),
                height: dy(25),
                ..Default::default()
            },
        );

        // 端口框要放得下 65535，别太窄
        let _proxy_port_label = label(clone_page, "端口", 258, 272);
        let (proxy_port_edit, proxy_port) =
            numeric(clone_page, 258, 292, 64, (1, 65_535), 8080);

        let _proxy_user_label = label(clone_page, "用户名", 26, 326);
        let proxy_user = gui::Edit::new(
            clone_page,
            gui::EditOpts {
                position: gui::dpi(26, 346),
                width: dx(140),
                height: dy(25),
                ..Default::default()
            },
        );

        let _proxy_pass_label = label(clone_page, "密码", 180, 326);
        let proxy_pass = gui::Edit::new(
            clone_page,
            gui::EditOpts {
                position: gui::dpi(180, 346),
                width: dx(144),
                height: dy(25),
                control_style: co::ES::AUTOHSCROLL | co::ES::NOHIDESEL | co::ES::PASSWORD,
                ..Default::default()
            },
        );

        // ---------------- 下载选项 ----------------
        let panel_opts = group_box(clone_page, "下载选项", 372, 212, 340, 252);

        let cross_origin = check(clone_page, "下载跨域资源（CDN）", 384, 240);
        let relative_paths = check(clone_page, "改写为相对路径", 384, 262);
        let incremental = check(clone_page, "增量下载（未变更的跳过）", 384, 284);
        let neutralize = check(clone_page, "单页模式下禁用外站链接", 384, 306);
        let ignore_cert = check(clone_page, "忽略 HTTPS 证书错误", 384, 328);
        // 新增项：排在原有复选框之后
        let custom_ua = check(clone_page, "使用自定义浏览器 UA", 384, 350);

        let ua_text = gui::Edit::new(
            clone_page,
            gui::EditOpts {
                text: webclone_core::EDGE_USER_AGENT,
                position: gui::dpi(384, 372),
                width: dx(316),
                height: dy(25),
                ..Default::default()
            },
        );

        let conc_label = label(clone_page, "并发", 384, 406);
        let (conc_edit, concurrency) = numeric(clone_page, 418, 403, 40, (1, 64), 8);

        let timeout_label = label(clone_page, "超时(秒)", 482, 406);
        let (timeout_edit, timeout) = numeric(clone_page, 542, 403, 42, (5, 3600), 30);

        let retry_label = label(clone_page, "重试", 608, 406);
        let (retry_edit, retry) = numeric(clone_page, 642, 403, 40, (0, 10), 3);

        // ---------------- 操作区 ----------------
        let start = gui::Button::new(
            clone_page,
            gui::ButtonOpts {
                text: "开始克隆",
                position: gui::dpi(12, 478),
                width: dx(120),
                height: dy(38),
                ..Default::default()
            },
        );

        let stop = gui::Button::new(
            clone_page,
            gui::ButtonOpts {
                text: "停止",
                position: gui::dpi(142, 478),
                width: dx(90),
                height: dy(38),
                ..Default::default()
            },
        );

        let status = label(clone_page, "就绪", 244, 488);

        let progress = gui::ProgressBar::new(
            clone_page,
            gui::ProgressBarOpts {
                position: gui::dpi(12, 524),
                size: gui::dpi(700, 10),
                range: (0, 100),
                value: 0,
                ..Default::default()
            },
        );

        let log = gui::Edit::new(
            clone_page,
            gui::EditOpts {
                position: gui::dpi(12, 542),
                width: dx(700),
                height: dy(114),
                control_style: co::ES::MULTILINE
                    | co::ES::AUTOVSCROLL
                    | co::ES::AUTOHSCROLL
                    | co::ES::READONLY
                    | co::ES::WANTRETURN,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::TABSTOP
                    | co::WS::VSCROLL
                    | co::WS::HSCROLL
                    | co::WS::BORDER,
                ..Default::default()
            },
        );
        log.limit_text(Some(2_000_000));

        // ---------------- 关于页 ----------------
        // 只留纵向滚动条：不加 ES::AUTOHSCROLL 时超长行会自动折行，比横向滚动好读。
        let about_text = gui::Edit::new(
            about_page,
            gui::EditOpts {
                text: &about::about_text(),
                position: gui::dpi(12, 12),
                width: dx(700),
                height: dy(640),
                control_style: co::ES::MULTILINE
                    | co::ES::AUTOVSCROLL
                    | co::ES::READONLY
                    | co::ES::WANTRETURN,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::VSCROLL
                    | co::WS::BORDER,
                ..Default::default()
            },
        );

        Self {
            url,
            mode,
            depth,
            max_pages,
            output,
            browse,
            proxy,
            proxy_host,
            proxy_port_edit,
            proxy_port,
            proxy_user,
            proxy_pass,
            cross_origin,
            relative_paths,
            incremental,
            neutralize,
            ignore_cert,
            custom_ua,
            ua_text,
            conc_edit,
            concurrency,
            timeout_edit,
            timeout,
            retry_edit,
            retry,
            panel_net,
            panel_opts,
            conc_label,
            timeout_label,
            retry_label,
            start,
            stop,
            status,
            progress,
            log,
            about_text,
        }
    }

    /// 自定义 UA 只有在勾选时才可编辑。
    fn refresh_ua_state(&self) {
        self.ua_text.hwnd().EnableWindow(self.custom_ua.is_checked());
    }

    /// 按当前页面尺寸重排所有控件。
    ///
    /// 全部手动定位，不走 winsafe 的自动布局：页签里的子窗口跟着 TCM_ADJUSTRECT
    /// 出来的显示区走，而自动布局在"父窗口自身被外部挪动"的场景下原点会算错。
    fn apply_layout(&self, page_w: i32, page_h: i32) {
        let extra = (page_w - dx(DESIGN_PAGE_W)).max(0);

        // 会随窗口变宽的
        put(&self.url, dx(12), dy(30), dx(700) + extra, dy(25));
        put(&self.output, dx(12), dy(172), dx(596) + extra, dy(25));
        put(&self.browse, dx(616) + extra, dy(170), dx(96), dy(29));
        put(&self.progress, dx(12), dy(524), dx(700) + extra, dy(10));

        let log_h = (page_h - dy(554)).max(dy(80));
        put(&self.log, dx(12), dy(542), dx(700) + extra, log_h);

        // 右侧分组整体右移
        put(&self.panel_opts, dx(372) + extra, dy(212), dx(340), dy(252));
        mv(&self.cross_origin, dx(384) + extra, dy(240));
        mv(&self.relative_paths, dx(384) + extra, dy(262));
        mv(&self.incremental, dx(384) + extra, dy(284));
        mv(&self.neutralize, dx(384) + extra, dy(306));
        mv(&self.ignore_cert, dx(384) + extra, dy(328));
        mv(&self.custom_ua, dx(384) + extra, dy(350));
        put(&self.ua_text, dx(384) + extra, dy(372), dx(316), dy(25));

        mv(&self.conc_label, dx(384) + extra, dy(406));
        mv(&self.timeout_label, dx(482) + extra, dy(406));
        mv(&self.retry_label, dx(608) + extra, dy(406));

        place_numeric(&self.conc_edit, &self.concurrency, dx(418) + extra, dy(403), dx(40));
        place_numeric(&self.timeout_edit, &self.timeout, dx(542) + extra, dy(403), dx(42));
        place_numeric(&self.retry_edit, &self.retry, dx(642) + extra, dy(403), dx(40));

        // 关于页的文本框铺满整页
        put(
            &self.about_text,
            dx(12),
            dy(12),
            (page_w - dx(24)).max(dx(100)),
            (page_h - dy(24)).max(dy(100)),
        );
    }
}

fn label(parent: &gui::TabPage, text: &str, x: i32, y: i32) -> gui::Label {
    gui::Label::new(
        parent,
        gui::LabelOpts {
            text,
            position: gui::dpi(x, y),
            ..Default::default()
        },
    )
}

fn check(parent: &gui::TabPage, text: &str, x: i32, y: i32) -> gui::CheckBox {
    gui::CheckBox::new(
        parent,
        gui::CheckBoxOpts {
            text,
            position: gui::dpi(x, y),
            // 不用默认的换行样式：给足宽度让文字单行显示
            control_style: co::BS::AUTOCHECKBOX,
            ..Default::default()
        },
    )
}

/// 数字输入框 + 微调按钮。
///
/// 伙伴关系靠 `UDS::AUTOBUDDY`，它取"z 序上紧邻的前一个窗口"，所以本函数内部先建
/// `Edit` 再建 `UpDown`；调用方不要在这两者之间插入别的控件。
///
/// 这里刻意**不加** `UDS::ALIGNRIGHT`：它会让微调按钮自动吸附到伙伴框右缘，同时把
/// 伙伴框压窄一个箭头宽（96 DPI 下 16px）。MSDN 写明"the width of the buddy window
/// is decreased"，而且这个压窄在控件被程序化搬动之后表现得并不一致（实测同一份代码
/// 里有的框被压了、有的没被压）。改成显式摆放，宽度所见即所得。
///
/// `UDS::NOTHOUSANDS` 也必须显式加：`UDS::SETBUDDYINT` 默认按用户区域格式插千分位，
/// 8080 会显示成 "8,080"，而伙伴框带 `ES::NUMBER`，这种文本是非法输入。
///
/// `w` 是伙伴输入框的可见宽度，微调按钮紧贴其右侧（整体占 `w + SPIN_W`）。
fn numeric(
    parent: &gui::TabPage,
    x: i32,
    y: i32,
    w: i32,
    range: (i32, i32),
    value: i32,
) -> (gui::Edit, gui::UpDown) {
    let edit = gui::Edit::new(
        parent,
        gui::EditOpts {
            position: gui::dpi(x, y),
            width: dx(w),
            height: dy(25),
            control_style: co::ES::AUTOHSCROLL | co::ES::NOHIDESEL | co::ES::NUMBER,
            ..Default::default()
        },
    );

    // winsafe 硬编码按 width=0 创建 UpDown，去掉 ALIGNRIGHT 之后必须自己给宽度，
    // 否则箭头区宽度为 0、看不见。
    let spin = gui::UpDown::new(
        parent,
        gui::UpDownOpts {
            position: gui::dpi(x + w, y),
            height: dy(25),
            control_style: co::UDS::AUTOBUDDY
                | co::UDS::SETBUDDYINT
                | co::UDS::ARROWKEYS
                | co::UDS::HOTTRACK
                | co::UDS::NOTHOUSANDS,
            range,
            value,
            ..Default::default()
        },
    );
    put(&spin, dx(x + w), dy(y), dx(SPIN_W), dy(25));

    (edit, spin)
}

/// 把一组"数字框 + 微调"整体挪到指定位置（`w` 为输入框宽度）。
fn place_numeric(edit: &gui::Edit, spin: &gui::UpDown, x: i32, y: i32, w: i32) {
    put(edit, x, y, w, dy(25));
    put(spin, x + w, y, dx(SPIN_W), dy(25));
}

/// BS_GROUPBOX 的按钮：纯画框 + 左上角标题。
fn group_box(
    parent: &gui::TabPage,
    text: &str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> gui::Button {
    gui::Button::new(
        parent,
        gui::ButtonOpts {
            text,
            position: gui::dpi(x, y),
            width: dx(w),
            height: dy(h),
            control_style: co::BS::GROUPBOX,
            // 分组框不该进 Tab 焦点链
            window_style: co::WS::CHILD | co::WS::VISIBLE | co::WS::GROUP,
            ..Default::default()
        },
    )
}

// ------------------------------------------------------------ 应用主体

#[derive(Default)]
struct UiState {
    log_buf: String,
    started_at: Option<Instant>,
}

#[derive(Clone)]
struct App {
    wnd: gui::WindowMain,
    tab: gui::Tab,
    clone_page: gui::TabPage,
    about_page: gui::TabPage,
    ui: Ui,
    shared: Arc<Shared>,
    state: Rc<RefCell<UiState>>,
}

impl App {
    fn create_and_run() -> w::AnyResult<i32> {
        let wnd = gui::WindowMain::new(gui::WindowMainOpts {
            title: "网页克隆工具 WebClone",
            size: gui::dpi(748, 712),
            style: co::WS::CAPTION
                | co::WS::SYSMENU
                | co::WS::CLIPCHILDREN
                | co::WS::BORDER
                | co::WS::MINIMIZEBOX
                | co::WS::MAXIMIZEBOX
                | co::WS::SIZEBOX
                | co::WS::VISIBLE,
            ..Default::default()
        });

        // 页签页必须先于 Tab 创建：TabOpts.pages 要在构造时拿到它们。
        let clone_page = gui::TabPage::new(&wnd, gui::TabPageOpts::default());
        let about_page = gui::TabPage::new(&wnd, gui::TabPageOpts::default());

        let tab = gui::Tab::new(
            &wnd,
            gui::TabOpts {
                position: gui::dpi(8, 8),
                size: gui::dpi(732, 696),
                pages: &[("克隆", clone_page.clone()), ("关于", about_page.clone())],
                ..Default::default()
            },
        );

        let ui = Ui::build(&clone_page, &about_page);

        let app = Self {
            wnd,
            tab,
            clone_page,
            about_page,
            ui,
            shared: Arc::new(Shared::new()),
            state: Rc::new(RefCell::new(UiState::default())),
        };

        app.events();

        app.wnd.run_main(None)
    }

    // ---------------------------------------------------------- 事件绑定

    fn events(&self) {
        // 启动：设日志定时器 + 首次排布 + 回填配置
        //
        // 注意时机：winsafe 的窗口（连同所有子控件）是在 `run_main` 内部才真正
        // CreateWindowEx 出来的，`WindowMain::new` 返回时 HWND 还不存在。所以
        // 一切 set_text / set_check / EnableWindow 都必须在 WM_CREATE 之后做，
        // 提前调用全是空操作。
        let me = self.clone();
        self.wnd.on().wm_create(move |_| {
            let _ = me.wnd.hwnd().SetTimer(TIMER_LOG, TIMER_LOG_MS, None);
            me.relayout();
            me.apply_runtime_init();
            Ok(0)
        });

        // 尺寸变化：先把 Tab 摆好，再按 TCM_ADJUSTRECT 出来的显示区排布页签页
        let me = self.clone();
        self.wnd.on().wm_size(move |_| {
            me.relayout();
            Ok(())
        });

        // 关闭：先把配置落盘（改了选项但没点"开始克隆"就关掉的，下次照样记住），
        // 再销毁窗口。
        //
        // 【必须自己 DestroyWindow，否则点关闭按钮毫无反应，只能强杀进程】
        //
        // winsafe 的 `DlgMain::new` 内建注册过一个 `wm_close`（走 `on()` 通道），
        // 干的就是 `DestroyWindow`；但同一个消息在 `on()` 通道里是"后注册者胜"
        // —— `process_last_message` 用 `msgs.iter().rev().find(..)` 只取最后一条，
        // 我们一注册就把它顶掉了。而 `fn_wm_noparm_noret!` 宏在回调返回 `Ok` 时
        // 返回 `def_proc_val`（dialog 类型 = TRUE），`wnd_proc` 见 `user_ret` 有值
        // 就直接返回、不再走 `DefWindowProc` —— 于是 WM_CLOSE 被吃掉，窗口不销毁、
        // `GetMessage` 永远拿不到 `WM_QUIT`，消息循环卡死。
        // winsafe 文档对 wm_close 也明说了："If you handle this event, you'll
        // overwrite the default handling ... calls DestroyWindow"。
        let me = self.clone();
        self.wnd.on().wm_close(move || {
            me.save_from_ui();
            let _ = me.wnd.hwnd().DestroyWindow();
            Ok(())
        });

        // 100ms 批量刷新日志 + 收尾
        let me = self.clone();
        self.wnd.on().wm_timer(TIMER_LOG, move || {
            me.flush();
            Ok(())
        });

        // 开始 / 停止
        let me = self.clone();
        self.ui.start.on().bn_clicked(move || {
            me.start_clone();
            Ok(())
        });

        let me = self.clone();
        self.ui.stop.on().bn_clicked(move || {
            if let Ok(token) = me.shared.cancel.lock() {
                token.cancel();
            }
            let _ = me.ui.status.set_text_and_resize("正在停止…");
            me.ui.stop.hwnd().EnableWindow(false);
            Ok(())
        });

        // 浏览...
        let me = self.clone();
        self.ui.browse.on().bn_clicked(move || {
            me.browse_folder();
            Ok(())
        });

        // 代理单选：刷新可用状态与默认端口
        let me = self.clone();
        self.ui.proxy.on().bn_clicked(move || {
            me.refresh_proxy_inputs();
            Ok(())
        });

        let me = self.clone();
        self.ui.proxy_host.on().en_change(move || {
            me.refresh_proxy_inputs();
            Ok(())
        });

        // 自定义 UA 勾选状态变化
        let me = self.clone();
        self.ui.custom_ua.on().bn_clicked(move || {
            me.ui.refresh_ua_state();
            Ok(())
        });

        // 切页后强制整页重绘。
        //
        // 【为什么必须自己补这一下】两个页签页是**完全重叠的兄弟窗口**（位置和尺寸
        // 都取自 Tab 的显示区）。切页时 winsafe 只做两件事：把别的页 `SW_HIDE`、
        // 把选中的页 `SWP_SHOWWINDOW`，然后指望系统自己把旧像素擦掉 —— 而系统擦不
        // 干净：暴露出来的那块矩形，落在 Tab 控件的客户区里，而 Tab 自己不会主动
        // 重绘；WindowMain 又带 `WS_CLIPCHILDREN`（画背景时排除全部子窗口），于是
        // 这块区域没有任何一个窗口负责擦背景。结果就是"关于"页的文字原样留在那儿，
        // 叠在克隆页的控件上 —— 也就是看到的重影。
        //
        // 这里挂在 `tab.on()`（user 通道）而不是 `wnd.on()`：winsafe 的切页逻辑注册在
        // WindowMain 的 **before** 通道，`process_msgs` 的顺序是 before -> user -> after，
        // 所以本回调一定跑在切换完成之后；同时也不会顶掉它（只有同一个通道里才会
        // "后注册者胜"，这也正是当初 wm_close 被吃掉的成因）。
        let me = self.clone();
        self.tab.on().tcn_sel_change(move || {
            me.repaint_active_page();
            Ok(())
        });

        // 网址框回车 = 开始克隆（WPF 版的 UrlBox_KeyDown）。
        // 注意不能在这里装：此刻窗口还没 CreateWindowEx，HWND 全是空值，
        // SetWindowSubclass 只会失败。统一挪到 apply_runtime_init（WM_CREATE 里）去做。
    }

    /// 把当前可见的页签页连同它的全部子控件强制擦除重绘一遍。
    ///
    /// `RDW_ERASE` 让窗口擦背景（这一下才是真正盖掉残留像素的关键），
    /// `RDW_ALLCHILDREN` 把子控件一并卷进来 —— 缺了它，分组框那类"自己不擦背景"
    /// 的控件内部仍然是旧的。`RDW_UPDATENOW` 同步完成，避免留一帧闪烁。
    fn repaint_active_page(&self) {
        for page in [&self.clone_page, &self.about_page] {
            if !page.hwnd().IsWindowVisible() {
                continue;
            }
            if let Ok(rc) = page.hwnd().GetClientRect() {
                let _ = page.hwnd().RedrawWindow(
                    rc,
                    &w::HRGN::NULL,
                    co::RDW::INVALIDATE
                        | co::RDW::ERASE
                        | co::RDW::ALLCHILDREN
                        | co::RDW::UPDATENOW,
                );
            }
        }
    }

    fn relayout(&self) {
        let client = match self.wnd.hwnd().GetClientRect() {
            Ok(rc) => rc,
            Err(_) => return,
        };

        // Tab 铺满客户区
        put(
            &self.tab,
            dx(8),
            dy(8),
            (client.right - dx(16)).max(dx(200)),
            (client.bottom - dy(16)).max(dy(200)),
        );

        // 问 Tab 要"页面显示区"的实际位置
        let display = match self.tab_display_rect() {
            Some(rc) => rc,
            None => return,
        };
        let page_w = (display.right - display.left).max(1);
        let page_h = (display.bottom - display.top).max(1);

        // 两个页签页都摆到同一位置；可见性由 winsafe 自己管（故不带 SHOWWINDOW）
        for page in [&self.clone_page, &self.about_page] {
            let _ = page.hwnd().SetWindowPos(
                w::HwndPlace::None,
                w::POINT::with(display.left, display.top),
                w::SIZE::with(page_w, page_h),
                co::SWP::NOZORDER | co::SWP::NOACTIVATE,
            );
        }

        self.ui.apply_layout(page_w, page_h);
    }

    /// 用 TCM_ADJUSTRECT 把 Tab 的窗口矩形换算成页面显示区（相对父窗口客户区）。
    fn tab_display_rect(&self) -> Option<w::RECT> {
        let mut rc = self.tab.hwnd().GetWindowRect().ok()?;
        rc = self.wnd.hwnd().ScreenToClientRc(rc).ok()?;
        unsafe {
            self.tab.hwnd().SendMessage(msg::TcmAdjustRect {
                display_rect: false,
                rect: &mut rc,
            });
        }
        Some(rc)
    }

    // ---------------------------------------------------------- 配置读写

    /// WM_CREATE 之后才能跑的一次性初始化：控件已就绪，可以真正写值了。
    fn apply_runtime_init(&self) {
        self.init_settings();
        self.ui.refresh_ua_state();
        self.refresh_proxy_inputs();
        // 初始态：停止按钮不可点、进度条归零（等价 WPF 里 XAML 的初始 Enable 状态）
        self.ui.stop.hwnd().EnableWindow(false);
        self.ui.progress.set_marquee(false);
        self.ui.progress.set_position(0);
        // 子类化必须等到这里（控件已在 WM_CREATE 的 before 通道创建完毕）。
        self.subclass_group_boxes();
        self.install_url_enter();
    }

    /// 启动时把上次的配置回显到界面；首次运行（或配置损坏）就把当前默认值写盘。
    ///
    /// 与 C# 版的差别：C# 首次运行时不回填，界面直接用 XAML 里写死的初始状态
    /// （`IsChecked="True"` 之类）；winsafe 没有 XAML，所以这里首次运行改为用
    /// `GuiSettings::default()` 回填 —— 两边的默认值是对齐的（下载跨域资源 /
    /// 改写为相对路径 / 增量下载 / 单页禁用外站链接 默认勾选，忽略证书错误默认不勾）。
    /// 不能省略这一步：控件刚创建时复选框一律未勾选，直接 `save_from_ui()` 落盘的
    /// 就是一份全 false 的错配置。
    fn init_settings(&self) {
        let _ = self.ui.output.set_text(&default_output_directory());

        let loaded = GuiSettingsStore::load();
        let first_run = loaded.is_none();
        self.apply_settings(&loaded.unwrap_or_default());

        // 对应 C# 的 `if (GuiSettingsStore.Load() is null) SaveSettingsFromUi();`
        if first_run {
            self.save_from_ui();
        }
    }

    fn apply_settings(&self, s: &GuiSettings) {
        let _ = self.ui.url.set_text(&s.start_url);

        if !s.output_directory.trim().is_empty() {
            let _ = self.ui.output.set_text(s.output_directory.trim());
        }

        if s.mode.eq_ignore_ascii_case("full") {
            self.ui.mode[1].select(true);
        } else {
            self.ui.mode[0].select(true);
        }

        self.ui.depth.set_pos(s.max_depth);
        self.ui.max_pages.set_pos(s.max_pages);

        match s.proxy_kind.to_lowercase().as_str() {
            "http" => self.ui.proxy[1].select(true),
            "socks5" => self.ui.proxy[2].select(true),
            _ => self.ui.proxy[0].select(true),
        }
        let _ = self.ui.proxy_host.set_text(&s.proxy_host);
        self.ui.proxy_port.set_pos(s.proxy_port);
        let _ = self.ui.proxy_user.set_text(&s.proxy_user_name);
        let _ = self.ui.proxy_pass.set_text(&s.proxy_password);

        self.ui.cross_origin.set_check(s.include_cross_origin_assets);
        self.ui.relative_paths.set_check(s.convert_to_relative_paths);
        self.ui.incremental.set_check(s.enable_incremental);
        self.ui.neutralize.set_check(s.neutralize_external_links);
        self.ui.ignore_cert.set_check(s.ignore_certificate_errors);

        // 新增项
        self.ui.custom_ua.set_check(s.use_custom_user_agent);
        let ua = if s.custom_user_agent.trim().is_empty() {
            webclone_core::EDGE_USER_AGENT
        } else {
            s.custom_user_agent.as_str()
        };
        let _ = self.ui.ua_text.set_text(ua);

        self.ui.concurrency.set_pos(s.concurrency);
        self.ui.timeout.set_pos(s.timeout_seconds);
        self.ui.retry.set_pos(s.retry_count);

        self.ui.refresh_ua_state();
    }

    fn save_from_ui(&self) {
        let settings = GuiSettings {
            start_url: normalize_url_text(&self.ui.url.text().unwrap_or_default()),
            output_directory: self.ui.output.text().unwrap_or_default().trim().to_owned(),
            mode: if self.ui.mode.selected_index() == Some(1) {
                "full".to_owned()
            } else {
                "single".to_owned()
            },
            max_depth: self.ui.depth.pos(),
            max_pages: self.ui.max_pages.pos(),
            proxy_kind: match self.ui.proxy.selected_index() {
                Some(1) => "http".to_owned(),
                Some(2) => "socks5".to_owned(),
                _ => "none".to_owned(),
            },
            proxy_host: self
                .ui
                .proxy_host
                .text()
                .unwrap_or_default()
                .trim()
                .to_owned(),
            proxy_port: self.ui.proxy_port.pos(),
            proxy_user_name: self
                .ui
                .proxy_user
                .text()
                .unwrap_or_default()
                .trim()
                .to_owned(),
            proxy_password: self.ui.proxy_pass.text().unwrap_or_default(),
            include_cross_origin_assets: self.ui.cross_origin.is_checked(),
            convert_to_relative_paths: self.ui.relative_paths.is_checked(),
            enable_incremental: self.ui.incremental.is_checked(),
            neutralize_external_links: self.ui.neutralize.is_checked(),
            ignore_certificate_errors: self.ui.ignore_cert.is_checked(),
            concurrency: self.ui.concurrency.pos(),
            timeout_seconds: self.ui.timeout.pos(),
            retry_count: self.ui.retry.pos(),
            use_custom_user_agent: self.ui.custom_ua.is_checked(),
            custom_user_agent: {
                let text = self.ui.ua_text.text().unwrap_or_default().trim().to_owned();
                if text.is_empty() {
                    webclone_core::EDGE_USER_AGENT.to_owned()
                } else {
                    text
                }
            },
        };

        GuiSettingsStore::save(&settings);
    }

    // ---------------------------------------------------------- 交互辅助

    fn refresh_proxy_inputs(&self) {
        let selected = self.ui.proxy.selected_index();
        let enabled = matches!(selected, Some(i) if i != 0);

        self.ui.proxy_host.hwnd().EnableWindow(enabled);
        // 端口是"输入框 + 上下箭头"两个窗口，两边都得禁用：只禁箭头的话，
        // 输入框还能点进去选中文字。EnableWindow(FALSE) 之后虽然理论上
        // UpDown 会连带处理伙伴框，但实测并不会，必须显式来一次。
        self.ui.proxy_port_edit.hwnd().EnableWindow(enabled);
        self.ui.proxy_port.hwnd().EnableWindow(enabled);
        self.ui.proxy_user.hwnd().EnableWindow(enabled);
        self.ui.proxy_pass.hwnd().EnableWindow(enabled);

        if !enabled {
            return;
        }

        // 首次启用或地址为空时补上默认地址；用户手动改过就不打扰
        let host = self.ui.proxy_host.text().unwrap_or_default();
        if host.trim().is_empty() {
            let _ = self.ui.proxy_host.set_text("127.0.0.1");
        }

        // 在两种协议之间切换时，端口还停在另一个协议的默认值上，跟着切过去
        let current = self.ui.proxy_port.pos();
        let socks = selected == Some(2);
        let (other, want) = if socks { (8080, 1080) } else { (1080, 8080) };
        if current == other {
            self.ui.proxy_port.set_pos(want);
        }
    }

    fn browse_folder(&self) {
        let current = self.ui.output.text().unwrap_or_default().trim().to_owned();
        if let Some(path) = pick_folder(&self.wnd, &current) {
            let _ = self.ui.output.set_text(&path);
        }
    }

    // ---------------------------------------------------------- 克隆流程

    fn try_build_options(&self) -> Option<CloneOptions> {
        let url_text = self.ui.url.text().unwrap_or_default().trim().to_owned();
        if url_text.is_empty() {
            warn(&self.wnd, "请输入要克隆的网址。");
            let _ = self.ui.url.hwnd().SetFocus();
            return None;
        }

        let normalized = normalize_url_text(&url_text);
        let start_uri = match url::Url::parse(&normalized) {
            Ok(u) => u,
            Err(_) => {
                warn(&self.wnd, "网址格式不正确。");
                let _ = self.ui.url.hwnd().SetFocus();
                return None;
            }
        };

        let output = self.ui.output.text().unwrap_or_default().trim().to_owned();
        if output.is_empty() {
            warn(&self.wnd, "请选择保存位置。");
            return None;
        }

        // 代理：启用了但没填地址
        let proxy = match self.ui.proxy.selected_index() {
            None | Some(0) => ProxySettings::none(),
            idx => {
                let host = self
                    .ui
                    .proxy_host
                    .text()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if host.is_empty() {
                    warn(&self.wnd, "请填写代理地址（例如 127.0.0.1）。");
                    return None;
                }
                let user = self
                    .ui
                    .proxy_user
                    .text()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                let pass = self.ui.proxy_pass.text().unwrap_or_default();
                ProxySettings {
                    kind: if idx == Some(2) {
                        ProxyKind::Socks5
                    } else {
                        ProxyKind::Http
                    },
                    host,
                    port: self.ui.proxy_port.pos().clamp(1, 65_535) as u16,
                    username: if user.is_empty() { None } else { Some(user) },
                    password: if pass.is_empty() { None } else { Some(pass) },
                }
            }
        };

        // 新增：自定义 UA。勾选后文本框内容生效，留空则退回默认 UA。
        let user_agent = if self.ui.custom_ua.is_checked() {
            let text = self.ui.ua_text.text().unwrap_or_default().trim().to_owned();
            if text.is_empty() {
                DEFAULT_USER_AGENT.to_owned()
            } else {
                text
            }
        } else {
            DEFAULT_USER_AGENT.to_owned()
        };

        Some(CloneOptions {
            start_uri,
            output_directory: path_util::get_full_path(&output)
                .to_string_lossy()
                .into_owned(),
            mode: if self.ui.mode.selected_index() == Some(1) {
                CloneMode::FullSite
            } else {
                CloneMode::SinglePage
            },
            max_depth: self.ui.depth.pos().clamp(0, 100),
            max_pages: self.ui.max_pages.pos().clamp(0, 100_000) as usize,
            concurrency: self.ui.concurrency.pos().clamp(1, 64) as usize,
            timeout_seconds: self.ui.timeout.pos().clamp(5, 3600) as u64,
            retry_count: self.ui.retry.pos().clamp(0, 10) as u32,
            proxy,
            user_agent,
            include_cross_origin_assets: self.ui.cross_origin.is_checked(),
            convert_to_relative_paths: self.ui.relative_paths.is_checked(),
            enable_incremental: self.ui.incremental.is_checked(),
            neutralize_external_links: self.ui.neutralize.is_checked(),
            ignore_certificate_errors: self.ui.ignore_cert.is_checked(),
            ..CloneOptions::default()
        })
    }

    fn start_clone(&self) {
        if self.shared.running.load(Ordering::SeqCst) {
            return;
        }

        let options = match self.try_build_options() {
            Some(o) => o,
            None => return,
        };

        // 用户确认过的选项先落盘，下次启动直接回显
        self.save_from_ui();

        let token = CancelToken::new();
        if let Ok(mut guard) = self.shared.cancel.lock() {
            *guard = token.clone();
        }
        self.shared.running.store(true, Ordering::SeqCst);

        self.set_busy(true);
        {
            let mut st = self.state.borrow_mut();
            st.started_at = Some(Instant::now());
        }

        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let logging = shared.clone();
            let result = CloneEngine::new(options).map(|mut engine| {
                engine.set_progress(move |p: CloneProgress| {
                    logging.push_log(format!(
                        "[{}] {}",
                        chrono::Local::now().format("%H:%M:%S"),
                        p.message
                    ));
                });
                engine.run(&token)
            });

            let outcome = match result {
                Ok(Ok(report)) => Outcome::Done(Box::new(report)),
                Ok(Err(CoreError::Cancelled)) => Outcome::Cancelled,
                Ok(Err(err)) => Outcome::Failed(err.to_string()),
                Err(err) => Outcome::Failed(err.to_string()),
            };

            if let Ok(mut guard) = shared.outcome.lock() {
                *guard = Some(outcome);
            }
            shared.running.store(false, Ordering::SeqCst);
        });
    }

    fn set_busy(&self, busy: bool) {
        let ui = &self.ui;
        ui.start.hwnd().EnableWindow(!busy);
        ui.stop.hwnd().EnableWindow(busy);
        ui.url.hwnd().EnableWindow(!busy);
        ui.browse.hwnd().EnableWindow(!busy);

        if busy {
            ui.progress.set_marquee(true);
            let _ = ui.status.set_text_and_resize("正在克隆…");
        } else {
            ui.progress.set_marquee(false);
            ui.progress.set_position(100);
        }
    }

    /// 定时器回调：把后台攒下的日志刷到界面，并处理任务终态。
    fn flush(&self) {
        // ---- 日志 ----
        let drained: Vec<String> = match self.shared.logs.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(_) => Vec::new(),
        };

        if !drained.is_empty() {
            let text = {
                let mut st = self.state.borrow_mut();
                for line in drained {
                    st.log_buf.push_str(&line);
                    st.log_buf.push_str("\r\n");
                }

                if st.log_buf.len() > LOG_SOFT_LIMIT {
                    let cut = st.log_buf.len() - LOG_KEEP_TAIL;
                    let boundary = st
                        .log_buf
                        .char_indices()
                        .map(|(i, _)| i)
                        .find(|i| *i >= cut)
                        .unwrap_or(0);
                    st.log_buf = st.log_buf.split_off(boundary);
                }

                st.log_buf.clone()
            };

            let _ = self.ui.log.set_text(&text);
            self.scroll_log_to_end();
        }

        // ---- 终态 ----
        let outcome = match self.shared.outcome.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };

        let Some(outcome) = outcome else { return };

        let elapsed = {
            let mut st = self.state.borrow_mut();
            st.started_at.take().map(|t| t.elapsed())
        };

        match outcome {
            Outcome::Done(report) => {
                let status = if report.failures > 0 {
                    format!("完成，但有 {} 项失败", report.failures)
                } else {
                    "克隆完成".to_owned()
                };
                let _ = self.ui.status.set_text_and_resize(&status);

                let mut block = String::from("\r\n");
                block.push_str(&report.summary());
                block.push_str("\r\n");

                if !report.errors.is_empty() {
                    block.push_str(&format!("—— {} 条错误详情 ——\r\n", report.errors.len()));
                    for error in report.errors.iter().take(50) {
                        block.push_str(&format!("  ✗ {}\r\n", error));
                    }
                }

                self.state.borrow_mut().log_buf.push_str(&block);
                let text = self.state.borrow().log_buf.clone();
                let _ = self.ui.log.set_text(&text);
                self.scroll_log_to_end();
            }
            Outcome::Cancelled => {
                let _ = self.ui.status.set_text_and_resize("已取消");
                self.append_log("任务已取消。已完成部分保留在输出目录。");
            }
            Outcome::Failed(message) => {
                let _ = self
                    .ui
                    .status
                    .set_text_and_resize(&format!("出错：{}", message));
                self.append_log(&format!("发生异常：{}", message));
            }
        }

        if let Some(elapsed) = elapsed {
            let _ = self.wnd.hwnd().SetWindowText(&format!(
                "网页克隆工具 WebClone（用时 {:.1} 秒）",
                elapsed.as_secs_f64()
            ));
        }

        self.set_busy(false);
    }

    fn append_log(&self, line: &str) {
        {
            let mut st = self.state.borrow_mut();
            st.log_buf.push_str(line);
            st.log_buf.push_str("\r\n");
        }
        let text = self.state.borrow().log_buf.clone();
        let _ = self.ui.log.set_text(&text);
        self.scroll_log_to_end();
    }

    fn scroll_log_to_end(&self) {
        self.ui.log.set_selection(-1, -1);
        unsafe {
            self.ui.log.hwnd().SendMessage(msg::WmVScroll {
                scroll_box_pos: 0,
                request: co::SB_REQ::BOTTOM,
                hcontrol: None,
            });
        }
    }

    /// 给网址输入框挂上"回车即开始克隆"。
    ///
    /// winsafe 的控件事件只暴露 WM_COMMAND / WM_NOTIFY，没有 WM_KEYDOWN 钩子，
    /// 所以这里直接借 comctl32 的 `SetWindowSubclass` 挂一层子类化过程。
    ///
    /// 【时机】必须在 WM_CREATE 之后调用（见 `apply_runtime_init`）：winsafe 的窗口
    /// 连同子控件都是在消息循环启动后才真正 CreateWindowEx 出来的，提前调
    /// `hwnd()` 拿到的是空句柄，`SetWindowSubclass` 会直接失败、回车功能静默失效。
    fn install_url_enter(&self) {
        let app_ptr = self as *const App as usize;
        unsafe {
            SetWindowSubclass(
                self.ui.url.hwnd().ptr(),
                url_edit_subclass,
                SUBCLASS_URL_EDIT,
                app_ptr,
            );
        }
    }

    /// 给两个分组框挂上"自擦背景"的子类化过程。
    ///
    /// 【为什么非做不可】`BS_GROUPBOX` 是"空心"控件：它只画边框和左上角标题，
    /// 中间那块**故意不填**，设计上假设父窗口会把底色擦好。而父窗口（TabPage）
    /// 带 `WS_CLIPCHILDREN` —— 擦背景时会把子窗口矩形整块跳过。两条规则一撞，
    /// 分组框中间那块像素就处于"无人区"：
    ///   - 程序刚起来时它是窗口创建时的初值，看着是白的、正常；
    ///   - 一旦被别的页签盖过（"关于"页全屏文本），再切回来，系统把这块矩形
    ///     交给下层窗口补画 —— clone 页被 CLIPCHILDREN 跳过、分组框自己又只画
    ///     边框不擦底，于是"关于"页的文字就原样留在那里，和 clone 页叠在一起。
    /// 让分组框自己把客户区刷一遍，是最小改动、且只影响这两个控件的做法。
    fn subclass_group_boxes(&self) {
        for gb in [&self.ui.panel_net, &self.ui.panel_opts] {
            unsafe {
                SetWindowSubclass(gb.hwnd().ptr(), groupbox_subclass, SUBCLASS_GROUPBOX, 0);
            }
        }
    }
}

/// 网址输入框的子类化过程：只在 `VK_RETURN` 的 `WM_KEYDOWN` 上做拦截。
unsafe extern "system" fn url_edit_subclass(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id_subclass: usize,
    ref_data: usize,
) -> isize {
    // ── 让回车能真正落到本窗口过程上 ──────────────────────────────
    //
    // winsafe 的 `run_main` 消息循环里有一步 `IsDialogMessage`（见
    // `BaseWnd::run_main_loop`，`process_dlg_msgs` 打开时）。对话框管理器
    // 拿到 `VK_RETURN` 的 `WM_KEYDOWN` 后，按标准流程去找"默认按钮"，
    // 而这个窗口压根没有默认按钮 —— 于是那条消息被它**直接丢弃**：
    // 既没有 Dispatch 到窗口过程，也没有触发任何按钮。
    // 实测证据：只挂子类化、不做下面这步时，网址框的过程能收到 `WM_KEYUP`
    // 却收不到 `WM_KEYDOWN`（真实按键和 SendMessage 都一样）。
    //
    // 解法是标准的一套：在 `WM_GETDLGCODE` 里对"这一条具体的回车消息"回
    // `DLGC_WANTMESSAGE`，等于告诉对话框管理器"这个键归我，你别管"。
    // `lParam` 此时指向那条待处理的 `MSG`，正好用来精确判断 —— 只认回车，
    // Tab / 方向键照旧交给它，键盘导航不受影响。
    if msg == (co::WM::GETDLGCODE.raw() as u32) {
        // 先取 Edit 自己的答案（通常是 WANTCHARS | HASSETSEL | WANTARROWS）
        let base = DefSubclassProc(hwnd, msg, wparam, lparam);

        if lparam != 0 {
            let pending = lparam as *const w::MSG;
            if (*pending).message == co::WM::KEYDOWN
                && ((*pending).wParam as u16) == co::VK::RETURN.raw()
            {
                return base | (co::DLGC::WANTMESSAGE.raw() as isize);
            }
        }
        return base;
    }

    if msg == (co::WM::KEYDOWN.raw() as u32)
        && (wparam as u16) == co::VK::RETURN.raw()
        && ref_data != 0
    {
        // ref_data 指向 create_and_run 里那个活到 run_main 结束的 App
        let app = unsafe { &*(ref_data as *const App) };
        app.start_clone();
        return 0;
    }

    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// 分组框的子类化过程：接管 `WM_ERASEBKGND`，把客户区刷成窗口底色。
///
/// 返回 1 表示"我已经擦了"，系统就不会再走默认处理（默认处理对 BS_GROUPBOX
/// 同样不会填中间区域，所以必须自己来）。
unsafe extern "system" fn groupbox_subclass(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id_subclass: usize,
    _ref_data: usize,
) -> isize {
    if msg == (co::WM::ERASEBKGND.raw() as u32) {
        let mut rc = w::RECT::default();
        if GetClientRect(hwnd, &mut rc) != 0 {
            // wparam 就是 HDC；系统刷子不用释放
            FillRect(
                wparam as *mut std::ffi::c_void,
                &rc,
                GetSysColorBrush(co::COLOR::WINDOW.raw()),
            );
        }
        return 1;
    }

    DefSubclassProc(hwnd, msg, wparam, lparam)
}

#[link(name = "comctl32")]
extern "system" {
    fn SetWindowSubclass(
        hwnd: *mut std::ffi::c_void,
        pfn_subclass: unsafe extern "system" fn(
            *mut std::ffi::c_void,
            u32,
            usize,
            isize,
            usize,
            usize,
        ) -> isize,
        id_subclass: usize,
        ref_data: usize,
    ) -> i32;

    fn DefSubclassProc(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize;
}

#[link(name = "user32")]
extern "system" {
    fn GetClientRect(hwnd: *mut std::ffi::c_void, out: *mut w::RECT) -> i32;

    fn FillRect(
        hdc: *mut std::ffi::c_void,
        rc: *const w::RECT,
        brush: *mut std::ffi::c_void,
    ) -> i32;

    fn GetSysColorBrush(index: i32) -> *mut std::ffi::c_void;
}

// ------------------------------------------------------------ 小工具

/// 缺省输出目录：桌面下的 webclone-output（与 WPF 版一致）。
fn default_output_directory() -> String {
    let desktop = user_desktop_directory().unwrap_or_else(|| ".".to_owned());
    std::path::Path::new(&desktop)
        .join("webclone-output")
        .to_string_lossy()
        .into_owned()
}

/// 走 shell 拿桌面路径，语义与 .NET 的 `Environment.SpecialFolder.Desktop` 一致。
fn user_desktop_directory() -> Option<String> {
    let item = w::SHCreateItemInKnownFolder::<w::IShellItem>(
        &co::KNOWNFOLDERID::Desktop,
        co::KF::DEFAULT,
        "",
    )
    .ok()?;
    item.GetDisplayName(co::SIGDN::FILESYSPATH).ok()
}

fn normalize_url_text(text: &str) -> String {
    let url = text.trim();
    if url.is_empty() {
        return String::new();
    }

    let has_scheme = (url.len() >= 7 && url[..7].eq_ignore_ascii_case("http://"))
        || (url.len() >= 8 && url[..8].eq_ignore_ascii_case("https://"));

    if has_scheme {
        url.to_owned()
    } else {
        format!("https://{}", url)
    }
}

fn warn(parent: &gui::WindowMain, text: &str) {
    let _ = parent
        .hwnd()
        .MessageBox(text, "提示", co::MB::OK | co::MB::ICONWARNING);
}

/// 系统文件夹选择对话框（IFileOpenDialog + FOS_PICKFOLDERS）。
fn pick_folder(parent: &gui::WindowMain, initial: &str) -> Option<String> {
    let dialog = w::CoCreateInstance::<w::IFileOpenDialog>(
        &co::CLSID::FileOpenDialog,
        None::<&w::IUnknown>,
        co::CLSCTX::INPROC_SERVER,
    )
    .ok()?;

    if let Ok(options) = dialog
        .GetOptions()
        .map(|o| o | co::FOS::PICKFOLDERS | co::FOS::FORCEFILESYSTEM)
    {
        let _ = dialog.SetOptions(options);
    }
    let _ = dialog.SetTitle("选择保存位置");

    if !initial.is_empty() {
        if let Ok(item) =
            w::SHCreateItemFromParsingName::<w::IShellItem>(initial, None::<&w::IBindCtx>)
        {
            let _ = dialog.SetFolder(&item);
        }
    }

    match dialog.Show(&parent.hwnd()) {
        Ok(true) => dialog
            .GetResult()
            .and_then(|item| item.GetDisplayName(co::SIGDN::FILESYSPATH))
            .ok(),
        _ => None,
    }
}

// ------------------------------------------------------------ 入口

fn main() {
    // COM 用于"浏览..."的文件夹选择对话框；guard 必须活到进程结束。
    let _com = w::CoInitializeEx(co::COINIT::APARTMENTTHREADED).ok();

    if let Err(err) = App::create_and_run() {
        let _ = w::HWND::NULL.MessageBox(
            &err.to_string(),
            "WebClone 启动失败",
            co::MB::OK | co::MB::ICONERROR,
        );
    }
}
