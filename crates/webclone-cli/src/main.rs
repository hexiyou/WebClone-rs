//! webclone — 网页模板 / 整站克隆命令行工具。
//!
//! 与 C# 版 `webclone` 完全同参：同样的选项名、默认值、取值范围、中文提示与退出码。

use std::io::Write;
use std::sync::OnceLock;

use webclone_core::storage::path_util;
use webclone_core::{
    CancelToken, CloneEngine, CloneMode, CloneOptions, CloneProgressKind, Error, ProxyKind,
    ProxySettings,
};

/// 命令行默认 UA（与 C# 版一致）。
const DEFAULT_USER_AGENT: &str = webclone_core::DEFAULT_USER_AGENT;

fn main() {
    setup_console();

    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        std::process::exit(if args.is_empty() { 1 } else { 0 });
    }

    let (options, verbose) = match parse_arguments(&args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("参数错误：{}", message);
            eprintln!();
            print_usage();
            std::process::exit(2);
        }
    };

    match run(options, verbose) {
        Ok(code) => std::process::exit(code),
        Err(Error::Cancelled) => {
            log("已取消。");
            std::process::exit(130);
        }
        Err(e) => {
            eprintln!("未处理的异常：{}", e);
            std::process::exit(3);
        }
    }
}

fn run(options: CloneOptions, verbose: bool) -> Result<i32, Error> {
    let cancel = CancelToken::new();
    install_cancel_handler(cancel.clone());

    let mut engine = CloneEngine::new(options.clone())?;

    let completed = std::sync::atomic::AtomicUsize::new(0);
    engine.set_progress(move |progress| {
        let index = completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let always = matches!(
            progress.kind,
            CloneProgressKind::Started
                | CloneProgressKind::PageDownloaded
                | CloneProgressKind::AssetFailed
                | CloneProgressKind::Completed
        );
        if verbose || always {
            log(&format!("[{:>5}] {}", index, progress.message));
        }
    });

    log(&format!("模式：{}", options.mode.label()));
    log(&format!("代理：{}", options.proxy));
    log(&format!("输出：{}", options.output_directory));
    log("");

    let report = engine.run(&cancel)?;

    log("");
    log(&report.summary());

    if !report.errors.is_empty() {
        log("");
        log(&format!("前 {} 条错误：", report.errors.len().min(20)));
        for error in report.errors.iter().take(20) {
            log(&format!("  - {}", error));
        }
    }

    Ok(if report.failures > 0 { 1 } else { 0 })
}

fn log(message: &str) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{}", message);
}

fn parse_arguments(args: &[String]) -> Result<(CloneOptions, bool), String> {
    let mut verbose = false;

    let mut url: Option<String> = None;
    let mut output = std::env::current_dir()
        .unwrap_or_default()
        .join("webclone-output")
        .to_string_lossy()
        .into_owned();
    let mut mode = CloneMode::SinglePage;
    let mut depth = 5i32;
    let mut concurrency = 8usize;
    let mut timeout = 30u64;
    let mut max_pages = 0usize;
    let mut retry = 3u32;
    let mut proxy: Option<String> = None;
    let mut include_cross_origin = true;
    let mut incremental = true;
    let mut convert_relative = true;
    let mut neutralize = true;
    let mut ignore_cert = false;
    let mut user_agent: Option<String> = None;

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].clone();

        if !arg.starts_with('-') {
            if url.is_none() {
                url = Some(arg);
            }
            i += 1;
            continue;
        }

        match arg.as_str() {
            "-o" | "--output" => output = next_value(args, &mut i, &arg)?,
            "-m" | "--mode" => {
                let value = next_value(args, &mut i, &arg)?.to_lowercase();
                mode = match value.as_str() {
                    "single" | "page" | "s" => CloneMode::SinglePage,
                    "full" | "site" | "f" => CloneMode::FullSite,
                    _ => {
                        return Err(format!("未知的克隆模式：{}（可选 single / full）", value))
                    }
                };
            }
            "-d" | "--depth" => depth = parse_int(&next_value(args, &mut i, &arg)?, &arg, 0, 100)? as i32,
            "-c" | "--concurrency" => {
                concurrency = parse_int(&next_value(args, &mut i, &arg)?, &arg, 1, 64)? as usize
            }
            "--timeout" => timeout = parse_int(&next_value(args, &mut i, &arg)?, &arg, 5, 3600)? as u64,
            "--retry" => retry = parse_int(&next_value(args, &mut i, &arg)?, &arg, 0, 10)? as u32,
            "--max-pages" => {
                max_pages = parse_int(&next_value(args, &mut i, &arg)?, &arg, 0, 100_000)? as usize
            }
            "--proxy" => proxy = Some(next_value(args, &mut i, &arg)?),
            "--user-agent" => user_agent = Some(next_value(args, &mut i, &arg)?),
            "--no-cross-origin" => include_cross_origin = false,
            "--no-incremental" => incremental = false,
            "--keep-absolute" => convert_relative = false,
            "--keep-external-links" => neutralize = false,
            "--ignore-cert-errors" => ignore_cert = true,
            "-v" | "--verbose" => verbose = true,
            _ => return Err(format!("未知选项：{}", arg)),
        }

        i += 1;
    }

    let url = match url {
        Some(u) if !u.trim().is_empty() => u,
        _ => return Err("必须提供要克隆的网址。".to_owned()),
    };

    let has_scheme = (url.len() >= 7 && url[..7].eq_ignore_ascii_case("http://"))
        || (url.len() >= 8 && url[..8].eq_ignore_ascii_case("https://"));
    let url = if has_scheme {
        url
    } else {
        format!("https://{}", url)
    };

    let start_uri = url::Url::parse(&url).map_err(|_| format!("网址格式不正确：{}", url))?;

    let mut options = CloneOptions {
        start_uri,
        output_directory: path_util::get_full_path(&output)
            .to_string_lossy()
            .into_owned(),
        mode,
        max_depth: depth,
        concurrency,
        timeout_seconds: timeout,
        retry_count: retry,
        max_pages,
        proxy: parse_proxy(proxy.as_deref())?,
        user_agent: user_agent.unwrap_or_else(|| DEFAULT_USER_AGENT.to_owned()),
        include_cross_origin_assets: include_cross_origin,
        enable_incremental: incremental,
        convert_to_relative_paths: convert_relative,
        neutralize_external_links: neutralize,
        ignore_certificate_errors: ignore_cert,
        ..CloneOptions::default()
    };

    // 与 C# 版一样：CLI 不走 HTML/CSS 的相对路径开关之外的额外逻辑
    options.normalize_to_utf8 = true;

    Ok((options, verbose))
}

fn next_value(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
    if *i + 1 >= args.len() {
        return Err(format!("选项 {} 缺少取值。", name));
    }
    *i += 1;
    Ok(args[*i].clone())
}

fn parse_int(value: &str, name: &str, min: i64, max: i64) -> Result<i64, String> {
    let parsed: i64 = value
        .trim()
        .parse()
        .map_err(|_| format!("选项 {} 需要一个整数。", name))?;

    if parsed < min || parsed > max {
        return Err(format!(
            "选项 {} 的取值需要在 {} 到 {} 之间。",
            name, min, max
        ));
    }

    Ok(parsed)
}

/// 解析代理地址。支持：
///   http://127.0.0.1:8080
///   http://user:pass@127.0.0.1:8080
///   socks5://127.0.0.1:1080
///   socks5://user:pass@127.0.0.1:1080
fn parse_proxy(proxy: Option<&str>) -> Result<ProxySettings, String> {
    let raw = match proxy {
        Some(p) if !p.trim().is_empty() => p.trim(),
        _ => return Ok(ProxySettings::none()),
    };

    let normalized = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("http://{}", raw)
    };

    let uri = url::Url::parse(&normalized).map_err(|_| format!("代理地址格式不正确：{}", raw))?;

    let kind = match uri.scheme().to_lowercase().as_str() {
        "http" | "https" => ProxyKind::Http,
        "socks5" | "socks" | "socks5h" => ProxyKind::Socks5,
        other => {
            return Err(format!(
                "不支持的代理协议：{}（可选 http / socks5）",
                other
            ))
        }
    };

    let username = if uri.username().is_empty() {
        None
    } else {
        Some(uri.username().to_owned())
    };
    let password = uri.password().map(|p| p.to_owned());

    // .NET 的 Uri.Port 会给 http/https 补默认端口（80/443），socks5 这种未知协议给 -1；
    // 这里按同样的口径补齐，保证"漏写端口"时两边行为一致。
    let port = uri.port().unwrap_or(match kind {
        ProxyKind::Socks5 => 1080,
        _ => match uri.scheme().to_lowercase().as_str() {
            "https" => 443,
            _ => 80,
        },
    });

    Ok(ProxySettings {
        kind,
        host: uri.host_str().unwrap_or("").to_owned(),
        port,
        username,
        password,
    })
}

fn print_usage() {
    println!(
        r#"webclone — 网页模板 / 整站克隆工具

用法:
  webclone <网址> [选项]

模式:
  -m, --mode <single|full>   克隆模式，single=单页模板（默认），full=整站递归

常用选项:
  -o, --output <目录>         保存位置，默认 ./webclone-output
  -d, --depth <层数>          整站模式最大递归深度，默认 5
  -c, --concurrency <数量>    并发下载数，默认 8
      --timeout <秒>          单请求超时，默认 30
      --retry <次数>          失败重试次数，默认 3
      --max-pages <数量>      整站模式最多抓取的页面数，0 表示不限

代理:
      --proxy <地址>          http://host:port 或 socks5://host:port
                              带认证：socks5://user:pass@host:port
      --ignore-cert-errors    忽略 HTTPS 证书错误

行为开关:
      --no-cross-origin       不下载跨域（CDN）资源
      --no-incremental        禁用增量，全部重新下载
      --keep-absolute         保留绝对路径，不改写为相对路径
      --keep-external-links   单页模式下不把外站链接替换为 #
      --user-agent <UA>       自定义 User-Agent
  -v, --verbose              输出详细日志

示例:
  webclone https://example.com -o D:\site -m full -d 3
  webclone https://example.com --proxy socks5://127.0.0.1:1080
"#
    );
}

// ------------------------------------------------------------ 控制台

/// Windows 控制台默认按本地代码页（中文系统是 936）解析输出字节，
/// 直接打 UTF-8 会花屏。这里把输出/输入代码页都切到 UTF-8。
#[cfg(windows)]
fn setup_console() {
    const CP_UTF8: u32 = 65001;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleOutputCP(code_page: u32) -> i32;
        fn SetConsoleCP(code_page: u32) -> i32;
    }

    unsafe {
        SetConsoleOutputCP(CP_UTF8);
        SetConsoleCP(CP_UTF8);
    }
}

#[cfg(not(windows))]
fn setup_console() {}

#[cfg(windows)]
static CANCEL_TOKEN: OnceLock<CancelToken> = OnceLock::new();

/// Ctrl+C 不直接杀进程，而是置位取消令牌，让已开始的任务收尾（与 C# 版一致）。
#[cfg(windows)]
fn install_cancel_handler(cancel: CancelToken) {
    let _ = CANCEL_TOKEN.set(cancel);

    extern "system" fn handler(ctrl_type: u32) -> i32 {
        if ctrl_type == 0 {
            // CTRL_C_EVENT
            if let Some(token) = CANCEL_TOKEN.get() {
                token.cancel();
            }
            log("正在取消，等待已开始的任务收尾…");
            return 1; // 已处理，阻止默认的立即终止
        }
        0
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }

    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(not(windows))]
fn install_cancel_handler(_cancel: CancelToken) {}
