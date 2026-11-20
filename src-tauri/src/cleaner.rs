//! 垃圾清理 + 大文件查找（对标电脑管家「垃圾清理」「大文件」）。
//!
//! 安全策略：
//! - 只扫描白名单目录；清理时再次校验路径必须位于白名单内。
//! - 一律「移到废纸篓」而非直接删除，用户可从废纸篓恢复。
//! - 废纸篓本身通过 Finder 的「清倒废纸篓」处理。

use crate::util::{self, home};
use serde::Serialize;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct JunkItem {
    pub path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Serialize)]
pub struct JunkGroup {
    pub key: String,
    pub title: String,
    pub description: String,
    pub size: u64,
    pub needs_permission: bool,
    /// 默认是否勾选（风险较高的分组默认不勾）
    pub recommended: bool,
    pub items: Vec<JunkItem>,
}

struct Rule {
    key: &'static str,
    title: &'static str,
    desc: &'static str,
    rel: &'static str,
    recommended: bool,
    /// 仅匹配这些扩展名（为空 = 目录下所有子项）
    exts: &'static [&'static str],
}

const RULES: &[Rule] = &[
    Rule { key: "user_cache", title: "应用缓存", desc: "~/Library/Caches，应用会按需重新生成", rel: "Library/Caches", recommended: true, exts: &[] },
    Rule { key: "user_logs", title: "用户日志", desc: "~/Library/Logs 中的运行日志与崩溃报告", rel: "Library/Logs", recommended: true, exts: &[] },
    Rule { key: "xcode_derived", title: "Xcode 编译缓存", desc: "DerivedData，可安全删除，下次编译重建", rel: "Library/Developer/Xcode/DerivedData", recommended: true, exts: &[] },
    Rule { key: "ios_backup", title: "iOS 设备备份", desc: "旧 iPhone/iPad 备份，删除前请确认", rel: "Library/Application Support/MobileSync/Backup", recommended: false, exts: &[] },
    Rule { key: "npm_cache", title: "npm 缓存", desc: "~/.npm/_cacache", rel: ".npm/_cacache", recommended: true, exts: &[] },
    Rule { key: "downloads_installer", title: "下载中的安装包", desc: "~/Downloads 中的 .dmg / .pkg / .zip，安装后通常不再需要", rel: "Downloads", recommended: false, exts: &["dmg", "pkg", "zip"] },
];

fn allowed_roots() -> Vec<PathBuf> {
    let h = home();
    RULES.iter().map(|r| h.join(r.rel)).collect()
}

#[tauri::command]
pub async fn scan_junk() -> Result<Vec<JunkGroup>, String> {
    let h = home();
    let mut groups = Vec::new();
    for rule in RULES {
        let root = h.join(rule.rel);
        if !root.exists() {
            continue;
        }
        let mut items = Vec::new();
        let needs_permission = !util::readable(&root);
        if let Ok(rd) = fs::read_dir(&root) {
            for e in rd.flatten() {
                let p = e.path();
                if !rule.exts.is_empty() {
                    let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                    if !rule.exts.contains(&ext.as_str()) {
                        continue;
                    }
                }
                let size = util::disk_usage(&p);
                if size < 64 * 1024 {
                    continue; // 忽略过小的条目，减少噪音
                }
                items.push(JunkItem {
                    name: e.file_name().to_string_lossy().into_owned(),
                    path: p.to_string_lossy().into_owned(),
                    size,
                });
            }
        }
        items.sort_by(|a, b| b.size.cmp(&a.size));
        groups.push(JunkGroup {
            key: rule.key.into(),
            title: rule.title.into(),
            description: rule.desc.into(),
            size: items.iter().map(|i| i.size).sum(),
            needs_permission,
            recommended: rule.recommended,
            items,
        });
    }
    // 废纸篓单独统计（读取需要「完全磁盘访问权限」）
    let trash = h.join(".Trash");
    let readable = util::readable(&trash);
    groups.push(JunkGroup {
        key: "trash".into(),
        title: "废纸篓".into(),
        description: "清倒废纸篓（不可恢复）".into(),
        size: if readable { util::disk_usage(&trash).saturating_sub(4096) } else { 0 },
        needs_permission: !readable,
        recommended: false,
        items: vec![],
    });
    Ok(groups)
}

#[derive(Serialize)]
pub struct CleanResult {
    pub cleaned: usize,
    pub freed: u64,
    pub errors: Vec<String>,
}

/// 将选中的条目移到废纸篓。
#[tauri::command]
pub async fn clean_items(paths: Vec<String>) -> Result<CleanResult, String> {
    let roots = allowed_roots();
    let mut res = CleanResult { cleaned: 0, freed: 0, errors: vec![] };
    for p in paths {
        match util::ensure_within(&PathBuf::from(&p), &roots) {
            Ok(canon) => {
                let size = util::disk_usage(&canon);
                match trash::delete(&canon) {
                    Ok(_) => {
                        res.cleaned += 1;
                        res.freed += size;
                    }
                    Err(e) => res.errors.push(format!("{p}: {e}")),
                }
            }
            Err(e) => res.errors.push(e),
        }
    }
    Ok(res)
}

#[tauri::command]
pub async fn empty_trash() -> Result<(), String> {
    util::run("osascript", &["-e", "tell application \"Finder\" to empty trash"]).map(|_| ())
}

#[derive(Serialize)]
pub struct LargeFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: u64,
}

/// 在用户目录中查找大文件（跳过 ~/Library、隐藏目录和 .app 包内部）。
#[tauri::command]
pub async fn scan_large_files(min_mb: u64, limit: usize) -> Result<Vec<LargeFile>, String> {
    let h = home();
    let min = min_mb * 1024 * 1024;
    let mut out = Vec::new();
    let mut stack = vec![h.clone()];
    let skip = [h.join("Library"), h.join(".Trash")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            let Ok(meta) = fs::symlink_metadata(&p) else { continue };
            if meta.is_dir() {
                let is_bundle = p.extension().is_some_and(|x| x == "app" || x == "photoslibrary");
                if !is_bundle && !skip.contains(&p) && name != "node_modules" {
                    stack.push(p);
                }
            } else if meta.is_file() && meta.blocks() * 512 >= min {
                out.push(LargeFile {
                    name,
                    path: p.to_string_lossy().into_owned(),
                    size: meta.blocks() * 512,
                    modified: util::modified_secs(&meta),
                });
            }
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out.truncate(limit);
    Ok(out)
}

/// 大文件由用户逐个确认后移到废纸篓（限定在用户目录内、且不在 ~/Library 中）。
#[tauri::command]
pub async fn trash_user_file(path: String) -> Result<(), String> {
    let canon = util::ensure_within(&PathBuf::from(&path), &[home()])?;
    if canon.starts_with(home().join("Library")) {
        return Err("不允许删除 ~/Library 中的文件".into());
    }
    trash::delete(&canon).map_err(|e| e.to_string())
}
