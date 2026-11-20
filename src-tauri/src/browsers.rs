//! 浏览器缓存清理：列出常见浏览器的缓存目录与体积，浏览器未运行时才允许清理。

use crate::util::{self, home};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct BrowserInfo {
    pub key: String,
    pub name: String,
    pub process: String,
    pub running: bool,
    pub size: u64,
    pub paths: Vec<String>,
}

/// 每个浏览器的进程名与缓存目录（相对用户目录）。
fn spec(key: &str) -> Option<(&'static str, &'static str, &'static [&'static str])> {
    Some(match key {
        "chrome" => (
            "Google Chrome",
            "Chrome",
            &[
                "Library/Caches/Google/Chrome",
                "Library/Application Support/Google/Chrome/Default/Cache",
                "Library/Application Support/Google/Chrome/Default/Code Cache",
                "Library/Application Support/Google/Chrome/Default/GPUCache",
                "Library/Application Support/Google/Chrome/Default/Service Worker/CacheStorage",
                "Library/Application Support/Google/Chrome/GrShaderCache",
                "Library/Application Support/Google/Chrome/ShaderCache",
            ],
        ),
        "edge" => (
            "Microsoft Edge",
            "Edge",
            &[
                "Library/Caches/Microsoft Edge",
                "Library/Application Support/Microsoft Edge/Default/Cache",
                "Library/Application Support/Microsoft Edge/Default/Code Cache",
                "Library/Application Support/Microsoft Edge/Default/GPUCache",
                "Library/Application Support/Microsoft Edge/Default/Service Worker/CacheStorage",
            ],
        ),
        "safari" => (
            "Safari",
            "Safari",
            &[
                "Library/Caches/com.apple.Safari",
                "Library/Containers/com.apple.Safari/Data/Library/Caches",
            ],
        ),
        "firefox" => (
            "firefox",
            "Firefox",
            &["Library/Caches/Firefox"],
        ),
        "arc" => (
            "Arc",
            "Arc",
            &[
                "Library/Caches/Company.TheBrowser.Arc",
                "Library/Application Support/Arc/User Data/Default/Cache",
                "Library/Application Support/Arc/User Data/Default/Code Cache",
            ],
        ),
        _ => return None,
    })
}

fn is_running(process: &str) -> bool {
    util::run("pgrep", &["-x", process]).map(|s| !s.trim().is_empty()).unwrap_or(false)
}

fn paths_of(key: &str) -> Vec<PathBuf> {
    let h = home();
    spec(key).map(|(_, _, rels)| rels.iter().map(|r| h.join(r)).collect()).unwrap_or_default()
}

#[tauri::command]
pub async fn scan_browsers() -> Vec<BrowserInfo> {
    let mut out = vec![];
    for key in ["chrome", "edge", "safari", "firefox", "arc"] {
        let Some((process, name, _)) = spec(key) else { continue };
        let paths = paths_of(key);
        let existing: Vec<PathBuf> = paths.iter().filter(|p| p.exists()).cloned().collect();
        if existing.is_empty() {
            continue;
        }
        let size = existing.iter().map(|p| util::disk_usage(p)).sum();
        out.push(BrowserInfo {
            key: key.into(),
            name: name.into(),
            process: process.into(),
            running: is_running(process),
            size,
            paths: existing.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        });
    }
    out
}

/// 清理指定浏览器的缓存（移到废纸篓，可恢复）。浏览器运行中会拒绝执行。
#[tauri::command]
pub async fn clean_browser_cache(key: String) -> Result<u64, String> {
    let (process, name, rels) = spec(&key).ok_or("未知的浏览器")?;
    if is_running(process) {
        return Err(format!("请先退出 {name}，再清理缓存"));
    }
    let h = home();
    let allowed: Vec<PathBuf> = rels.iter().map(|r| h.join(r)).collect();
    let mut freed = 0u64;
    let mut errs = vec![];
    for p in allowed {
        if !p.exists() {
            continue;
        }
        let size = util::disk_usage(&p);
        match trash::delete(&p) {
            Ok(_) => freed += size,
            Err(e) => errs.push(format!("{}：{e}", p.display())),
        }
    }
    if freed == 0 && !errs.is_empty() {
        return Err(errs.join("；"));
    }
    Ok(freed)
}
