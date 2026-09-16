//! 给 CLI 产物 `webclone.exe` 嵌入应用图标。
//!
//! CLI 没有窗口，图标只出现在资源管理器、快捷方式、任务管理器这些地方 —— 但
//! 缺了它 exe 就是一个"白纸"图标，跟 GUI 版放一起很突兀。
//!
//! 实现与 GUI 版共用 workspace 根的 `build/embed_icon.rs`。

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../build/embed_icon.rs"
));

fn main() {
    embed_app_icon(env!("CARGO_MANIFEST_DIR"));
}
