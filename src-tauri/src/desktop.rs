//! 通用文件操作：目标路径去重、在 Finder 中打开 / 显示。
//!
//! 桌面整理逻辑已迁移到 `collections.rs`（桌面集合）。

use crate::util;
use std::path::{Path, PathBuf};

/// 生成不冲突的目标路径：同名时追加 " (2)"、" (3)"…
pub(crate) fn unique_target(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|c| !c.exists())
        .unwrap()
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    util::run("open", &[&path]).map(|_| ())
}

#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    util::run("open", &["-R", &path]).map(|_| ())
}
