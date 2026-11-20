// 在 release 构建中不弹出额外的控制台窗口（Windows 专用，macOS 无影响）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mac_butler_lib::run()
}
