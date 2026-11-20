//! 通用工具：路径、目录体积统计、安全校验、shell 调用。

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// 统计路径占用的磁盘空间（按实际分配块计算，不跟随符号链接）。
/// 遇到无权限的目录直接跳过；`limit` 用于限制遍历的条目数，避免卡死。
pub fn disk_usage(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    let mut visited = 0usize;
    while let Some(p) = stack.pop() {
        visited += 1;
        if visited > 2_000_000 {
            break;
        }
        let Ok(meta) = fs::symlink_metadata(&p) else { continue };
        total += meta.blocks() * 512;
        if meta.is_dir() {
            if let Ok(rd) = fs::read_dir(&p) {
                for e in rd.flatten() {
                    stack.push(e.path());
                }
            }
        }
    }
    total
}

/// 判断目录是否可读（用于提示「完全磁盘访问权限」）。
pub fn readable(path: &Path) -> bool {
    fs::read_dir(path).is_ok()
}

/// 校验 `target` 必须位于 `roots` 中某个根目录之下（且不等于根本身），
/// 防止前端传入任意路径导致误删。
pub fn ensure_within(target: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let canon = target
        .canonicalize()
        .map_err(|e| format!("路径无效 {}: {e}", target.display()))?;
    for r in roots {
        if let Ok(rc) = r.canonicalize() {
            if canon.starts_with(&rc) && canon != rc {
                return Ok(canon);
            }
        }
    }
    Err(format!("拒绝操作白名单之外的路径：{}", canon.display()))
}

pub fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("执行 {cmd} 失败: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn modified_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
