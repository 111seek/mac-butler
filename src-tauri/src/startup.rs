//! 启动项管理（对标电脑管家「开机加速」）。
//!
//! 覆盖三类来源：
//! 1. ~/Library/LaunchAgents       —— 用户级后台服务，可通过 `launchctl disable/enable` 可逆地开关
//! 2. /Library/LaunchAgents、/Library/LaunchDaemons —— 系统级，修改需要管理员权限，本版本只读展示
//! 3. 登录项（System Events）       —— 需要「自动化」权限；可移除

use crate::util::{self, home};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct StartupItem {
    pub label: String,
    pub program: String,
    pub source: String,
    pub path: String,
    pub enabled: bool,
    /// 当前版本是否支持在应用内开关
    pub editable: bool,
    pub kind: String,
}

fn uid() -> u32 {
    unsafe { libc_getuid() }
}

extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// 解析 `launchctl print-disabled gui/<uid>` 输出，返回被禁用的 label 集合。
fn disabled_labels() -> HashSet<String> {
    let out = util::run("launchctl", &["print-disabled", &format!("gui/{}", uid())]).unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (k, v) = l.split_once("=>")?;
            let v = v.trim();
            if v == "disabled" || v == "true" {
                Some(k.trim().trim_matches('"').to_string())
            } else {
                None
            }
        })
        .collect()
}

fn read_plist(path: &Path) -> Option<(String, String, bool)> {
    let v: plist::Value = plist::from_file(path).ok()?;
    let d = v.as_dictionary()?;
    let label = d.get("Label")?.as_string()?.to_string();
    let program = d
        .get("Program")
        .and_then(|p| p.as_string().map(str::to_string))
        .or_else(|| {
            d.get("ProgramArguments")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|p| p.as_string().map(str::to_string))
        })
        .unwrap_or_default();
    let plist_disabled = d.get("Disabled").and_then(|b| b.as_boolean()).unwrap_or(false);
    Some((label, program, plist_disabled))
}

fn scan_dir(dir: &Path, source: &str, editable: bool, disabled: &HashSet<String>, out: &mut Vec<StartupItem>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "plist") {
            continue;
        }
        let Some((label, program, plist_disabled)) = read_plist(&p) else { continue };
        let enabled = !plist_disabled && !(editable && disabled.contains(&label));
        out.push(StartupItem {
            label,
            program,
            source: source.into(),
            path: p.to_string_lossy().into_owned(),
            enabled,
            editable,
            kind: "launchd".into(),
        });
    }
}

fn login_items() -> Vec<StartupItem> {
    let script = r#"tell application "System Events" to get the {name, path} of every login item"#;
    let Ok(out) = util::run("osascript", &["-e", script]) else { return vec![] };
    // 输出格式：name1, name2, path1, path2
    let parts: Vec<&str> = out.trim().split(", ").filter(|s| !s.is_empty()).collect();
    let n = parts.len() / 2;
    (0..n)
        .map(|i| StartupItem {
            label: parts[i].to_string(),
            program: parts[n + i].to_string(),
            source: "登录项".into(),
            path: parts[n + i].to_string(),
            enabled: true,
            editable: true,
            kind: "login_item".into(),
        })
        .collect()
}

#[tauri::command]
pub async fn list_startup_items() -> Result<Vec<StartupItem>, String> {
    let disabled = disabled_labels();
    let mut out = login_items();
    scan_dir(&home().join("Library/LaunchAgents"), "用户启动代理", true, &disabled, &mut out);
    scan_dir(Path::new("/Library/LaunchAgents"), "全局启动代理", false, &disabled, &mut out);
    scan_dir(Path::new("/Library/LaunchDaemons"), "系统守护进程", false, &disabled, &mut out);
    Ok(out)
}

/// 开关用户级 LaunchAgent。`launchctl disable` 是持久且可逆的，不删除 plist。
#[tauri::command]
pub async fn set_launch_agent_enabled(path: String, enabled: bool) -> Result<(), String> {
    let p = util::ensure_within(&PathBuf::from(&path), &[home().join("Library/LaunchAgents")])?;
    let (label, _, _) = read_plist(&p).ok_or("无法解析 plist")?;
    let domain = format!("gui/{}", uid());
    let target = format!("{domain}/{label}");
    if enabled {
        util::run("launchctl", &["enable", &target])?;
        let _ = util::run("launchctl", &["bootstrap", &domain, &p.to_string_lossy()]);
    } else {
        util::run("launchctl", &["disable", &target])?;
        let _ = util::run("launchctl", &["bootout", &target]);
    }
    Ok(())
}

/// 移除登录项（只是从登录项列表移除，不会删除应用本身）。
#[tauri::command]
pub async fn remove_login_item(name: String) -> Result<(), String> {
    let safe = name.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(r#"tell application "System Events" to delete login item "{safe}""#);
    util::run("osascript", &["-e", &script]).map(|_| ())
}
