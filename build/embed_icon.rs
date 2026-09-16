// 把 `assets/app.ico` 作为 Win32 图标资源嵌进 exe。
//
// 由 `crates/webclone-cli/build.rs` 与 `crates/webclone-gui/build.rs` 用
// `include!` 共用（本文件本身**不是** cargo 构建脚本，只是被引入的源码片段）。
// 注意这里只能用 `//` 普通注释：`//!` 内层文档注释只能出现在模块/crate 开头，
// 被 include 到 item 位置会报 E0753。
//
// 为什么需要一个资源编译器：
// Rust 只负责生成代码段，`.ico` 要出现在 PE 的资源区（`.rsrc`）必须由链接期的
// 资源编译器完成。MSVC 工具链里 `link.exe` 直接接受 `.res` 文件作为输入，而
// `.res` 由 WinSDK 自带的 `rc.exe` 从 `.rc` 脚本编译而来 —— 两者都随 MSVC +
// WinSDK 一起装在机器上，所以这里**不引入任何第三方 crate**
// （winres / embed-resource / winresource 都不需要）。
//
// rc.exe 之所以要自己找路径：它通常不在 `PATH` 里（只有 VS 开发者命令提示符
// 会加），而 cargo 构建并不保证从那类 shell 启动。

use std::path::{Path, PathBuf};
use std::process::Command;

/// ico 相对 workspace 根的位置。
const ICON_REL: &str = "assets/app.ico";
/// `.rc` 里给图标起的资源 ID。`gui::Icon::Id(n)` 必须与之对应。
const ICON_ID: u16 = 1;

/// 在 build.rs 里调用：把应用图标嵌进当前 crate 的所有 bin 目标。
///
/// `manifest_dir` 传 `env!("CARGO_MANIFEST_DIR")`（即 `crates/<name>`），
/// 函数内部会往上两级找到 workspace 根。
pub fn embed_app_icon(manifest_dir: &str) {
    // build.rs 的 cfg 是**宿主**平台，判断目标平台要看这个环境变量。
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }

    let ico = Path::new(manifest_dir)
        .join("..")
        .join("..")
        .join(ICON_REL);
    if !ico.is_file() {
        println!(
            "cargo:warning=找不到图标文件 {}，产物将不含图标资源",
            ico.display()
        );
        return;
    }
    // ico 换了要重新编译。
    println!("cargo:rerun-if-changed={}", ico.display());

    let Some(rc) = find_rc() else {
        // 找不到 rc.exe 不该阻断编译：没有图标只是观感差一点，
        // 而缺了 WinSDK 的机器本来也编不出 MSVC 目标。
        println!("cargo:warning=未找到 rc.exe（Windows SDK 资源编译器），跳过图标嵌入");
        return;
    };

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR 未设置"));
    let res = out_dir.join("app-icon.res");

    // 每次都重新生成：rc.exe 很快（几毫秒），比判断"是否过期"更省心。
    // .rc 写成绝对路径，避免 rc.exe 相对路径解析基准不确定。
    let rc_script = out_dir.join("app-icon.rc");
    let script = format!(
        "// 由 build.rs 自动生成，请勿手工编辑\n{} ICON \"{}\"\n",
        ICON_ID,
        ico.display().to_string().replace('\\', "/")
    );
    if let Err(err) = std::fs::write(&rc_script, script) {
        println!("cargo:warning=写入 {} 失败：{err}", rc_script.display());
        return;
    }

    let output = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg(&rc_script)
        .output();

    match output {
        Ok(o) if o.status.success() && res.is_file() => {
            // link.exe 直接接受 .res 作为输入，会和 rustc 自己生成的
            // manifest 资源合并到同一个 .rsrc 段里（两者 ID 不冲突：
            // 图标是 RT_ICON(3)/RT_GROUP_ICON(14)，manifest 是 RT_MANIFEST(24)）。
            let path = res.to_string_lossy().replace('\\', "/");
            let arg = if path.contains(' ') {
                format!("\"{path}\"")
            } else {
                path
            };
            println!("cargo:rustc-link-arg-bins={arg}");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            println!(
                "cargo:warning=rc.exe 编译图标失败（{}）：{}",
                o.status,
                stderr.trim()
            );
        }
        Err(err) => println!("cargo:warning=调用 rc.exe 失败：{err}"),
    }
}

/// 定位 `rc.exe`。优先级：`RC` 环境变量 → `PATH` → Windows Kits 安装目录中版本最高者。
fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RC") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }

    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join("rc.exe");
            if cand.is_file() {
                return Some(cand);
            }
        }
    }

    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default().as_str() {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        _ => "x64",
    };

    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(pf) = std::env::var(var) {
            roots.push(PathBuf::from(pf).join("Windows Kits/10/bin"));
        }
    }
    // 兜底：环境变量被剥掉时（某些 CI / 精简 shell）直接用默认安装位置。
    for pf in [r"C:\Program Files (x86)", r"C:\Program Files"] {
        let p = PathBuf::from(pf).join("Windows Kits/10/bin");
        if !roots.contains(&p) {
            roots.push(p);
        }
    }
    // `LIB` 指向 `<SDK>\Lib\<ver>\um\<arch>`，往上四级就是 SDK 根 —— 这条路
    // 能覆盖"装在非默认盘符"的情况。
    if let Ok(lib) = std::env::var("LIB") {
        for entry in std::env::split_paths(&lib) {
            if let Some(root) = entry.ancestors().nth(4) {
                let bin = root.join("bin");
                if bin.is_dir() && !roots.contains(&bin) {
                    roots.push(bin);
                }
            }
        }
    }

    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let cand = entry.path().join(arch).join("rc.exe");
            if !cand.is_file() {
                continue;
            }
            let key = version_key(&entry.file_name().to_string_lossy());
            if best.as_ref().map_or(true, |(v, _)| key > *v) {
                best = Some((key, cand));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// 把 `10.0.26100.0` 这样的目录名转成可比较的数字序列。
fn version_key(name: &str) -> Vec<u32> {
    name.split('.')
        .map(|s| s.parse::<u32>().unwrap_or(0))
        .collect()
}
