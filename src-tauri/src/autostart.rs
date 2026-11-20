//! 开机自启：写入用户级 LaunchAgent（无需签名、可在应用内随时开关）。

use crate::util::{self, home};
use std::fs;
use std::path::PathBuf;

const LABEL: &str = "com.macbutler.app";

fn plist_path() -> PathBuf {
    home().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

fn uid() -> u32 {
    extern "C" {
        #[link_name = "getuid"]
        fn libc_getuid() -> u32;
    }
    unsafe { libc_getuid() }
}

pub fn is_enabled() -> bool {
    plist_path().exists()
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let path = plist_path();
    if !on {
        let _ = util::run("launchctl", &["bootout", &format!("gui/{}/{}", uid(), LABEL)]);
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| format!("无法获取应用路径：{e}"))?;
    // 从 DMG / 下载目录直接运行时路径不稳定，弹出后就会失效
    let exe_str = exe.to_string_lossy();
    if exe_str.contains("/Volumes/") || exe_str.contains("/AppTranslocation/") {
        return Err("请先把「Mac 管家」拖到「应用程序」文件夹，再开启开机自启".into());
    }
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
        exe.display()
    );
    fs::write(&path, xml).map_err(|e| format!("无法写入启动项：{e}"))?;
    // 立即注册（失败不影响下次登录时生效）
    let _ = util::run("launchctl", &["bootstrap", &format!("gui/{}", uid()), &path.to_string_lossy()]);
    Ok(())
}

#[tauri::command]
pub async fn autostart_status() -> bool {
    is_enabled()
}

#[tauri::command]
pub async fn set_autostart(enabled: bool) -> Result<(), String> {
    set_enabled(enabled)
}
