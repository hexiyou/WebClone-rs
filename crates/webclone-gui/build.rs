//! 把视觉样式 manifest 嵌进 exe。
//!
//! Rust 的 MSVC 目标默认不生成 manifest，于是 exe 里不会出现
//! `Microsoft.Windows.Common-Controls` v6 的依赖声明 —— 结果是所有原生控件
//! （Button / CheckBox / Edit / ProgressBar / Tab）都走经典渲染路径，呈现出
//! Win95 那种灰色 3D 凸起外观。这和 GUI 代码本身无关，纯粹是缺失的资源声明。
//!
//! 这里用 MSVC 链接器自带的 `/MANIFEST:EMBED` + `/MANIFESTINPUT:` 参数嵌入，
//! 不引入任何第三方 crate（winres / embed-resource / embed-manifest 都不需要）。
//!
//! 除此之外还负责把 `assets/app.ico` 嵌成图标资源（实现见 workspace 根的
//! `build/embed_icon.rs`，与 cli crate 共用同一份）。

// 共用 workspace 根下的图标嵌入逻辑，避免两个 crate 各写一遍。
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../build/embed_icon.rs"
));

fn main() {
    println!("cargo:rerun-if-changed=webclone-gui.manifest");

    // 应用图标：任务栏、标题栏、资源管理器里显示的那个。
    embed_app_icon(env!("CARGO_MANIFEST_DIR"));

    // build.rs 的 cfg 是宿主环境，判断目标平台要看这个环境变量。
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("webclone-gui.manifest");
    // 链接器参数用正斜杠更保险；路径含空格时整体加引号。
    let path = manifest.to_string_lossy().replace('\\', "/");
    let arg = if path.contains(' ') {
        format!("/MANIFESTINPUT:\"{path}\"")
    } else {
        format!("/MANIFESTINPUT:{path}")
    };

    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins={arg}");
}
